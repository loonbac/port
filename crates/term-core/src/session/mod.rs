//! Sesión de terminal.
//!
//! Caso de uso que coordina las tres piezas: el proceso ([`crate::pty`]), la
//! rejilla VT de `alacritty_terminal` y el viewport que el usuario ve. Es el
//! único punto que conoce a las tres; ni la UI ni el PTY conocen a los demás.
//!
//! Los contratos con dueño propio viven en submódulos: el protocolo de ratón y
//! su política ([`mouse`]), el modelo de selección ([`selection`]), la
//! codificación del pegado ([`paste`]) y el ciclo de vida de pestañas y
//! espacios ([`manager`]). Este módulo conserva el estado compartido y la
//! coordinación de PTY, rejilla y viewport, incluida la rueda.

use std::cell::{Cell, RefCell};
use std::sync::mpsc::{channel, Receiver, Sender};

use alacritty_terminal::event::{Event as TermEvent, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;

use crate::frame::{self, Frame, Style};
use crate::input::KeyMode;
use crate::pty::{Pty, PtyConfig, RunningApp};

mod manager;
mod mouse;
mod paste;
mod selection;

use mouse::buttons_are_reported;

pub use manager::SessionManager;
pub use mouse::{MouseButton, MouseModifiers, MousePolicy};
pub use selection::SelectionKind;

/// Cuántas líneas guarda el scrollback.
pub const DEFAULT_SCROLLBACK: usize = 10_000;

/// Tamaño de la rejilla, en celdas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub columns: usize,
    pub rows: usize,
}

impl GridSize {
    pub const fn new(columns: usize, rows: usize) -> Self {
        Self { columns, rows }
    }
}

struct Dimensions_(GridSize);

impl Dimensions for Dimensions_ {
    fn total_lines(&self) -> usize {
        self.0.rows
    }

    fn screen_lines(&self) -> usize {
        self.0.rows
    }

    fn columns(&self) -> usize {
        self.0.columns
    }
}

/// Recibe los eventos que la rejilla quiere avisar a la aplicación.
#[derive(Clone)]
pub struct EventSink {
    pub sender: Sender<TermEvent>,
}

impl EventListener for EventSink {
    fn send_event(&self, event: TermEvent) {
        let _ = self.sender.send(event);
    }
}

/// Celda de la rejilla bajo el puntero, en coordenadas 1-based como las espera
/// el reporte SGR del ratón.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellPos {
    pub column: u16,
    pub row: u16,
}

impl CellPos {
    /// Construye una celda 1-based.
    ///
    /// Fuera de la rejilla se recorta a 1: el reporte SGR no admite fila ni
    /// columna cero, y un puntero sobre el margen debe seguir siendo válido.
    pub fn new(column: i32, row: i32) -> Self {
        let acotar = |valor: i32| valor.clamp(1, i32::from(u16::MAX)) as u16;
        Self {
            column: acotar(column),
            row: acotar(row),
        }
    }
}

/// Una terminal viva: proceso, rejilla y estado de vista.
pub struct Session {
    term: Term<EventSink>,
    processor: Processor,
    pty: Pty,
    events: Receiver<TermEvent>,
    size: GridSize,
    cached_frame: RefCell<Option<Frame>>,
    dirty: Cell<bool>,
    mouse_policy: MousePolicy,
}

impl Session {
    /// Arranca el shell del usuario con el tamaño dado.
    pub fn spawn(config: PtyConfig, size: GridSize) -> std::io::Result<Self> {
        let (sender, events) = channel();
        let sink = EventSink { sender };
        let term_config = alacritty_terminal::term::Config {
            scrolling_history: DEFAULT_SCROLLBACK,
            ..Default::default()
        };
        let term = Term::new(term_config, &Dimensions_(size), sink);
        let pty = Pty::spawn(config, size)?;
        Ok(Self {
            term,
            processor: Processor::new(),
            pty,
            events,
            size,
            cached_frame: RefCell::new(None),
            dirty: Cell::new(true),
            mouse_policy: MousePolicy::default(),
        })
    }

    pub fn size(&self) -> GridSize {
        self.size
    }

    /// Aplica a la rejilla lo que el PTY ya tenga listo.
    ///
    /// Nunca bloquea: si el shell no ha escrito nada, devuelve `false` de
    /// inmediato. La UI la llama en cada cuadro.
    pub fn pump(&mut self) -> bool {
        let bytes = self.pty.try_read();
        if bytes.is_empty() {
            return false;
        }
        self.processor.advance(&mut self.term, &bytes);
        self.handle_internal_events();
        // La salida nueva NO toca el viewport: una vista desplazada sobre el
        // historial se conserva aunque el programa escriba sin parar, que es lo
        // que hacen kitty y alacritty. Volver al presente es decisión del
        // usuario (`scroll` hacia abajo); con `display_offset == 0` la salida ya
        // sigue el final por sí sola.
        self.dirty.set(true);
        true
    }

    /// Responde a las consultas de protocolo VT que el programa del otro lado
    /// envía al terminal (como identificar atributos de dispositivo `\e[0c`,
    /// color de fondo `\e]11;?`, o tamaño en píxeles).
    fn handle_internal_events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                TermEvent::PtyWrite(text) => {
                    let _ = self.pty.write(text.as_bytes());
                }
                TermEvent::ColorRequest(_index, formatter) => {
                    let resp = formatter(alacritty_terminal::vte::ansi::Rgb {
                        r: 13,
                        g: 17,
                        b: 23,
                    });
                    let _ = self.pty.write(resp.as_bytes());
                }
                TermEvent::TextAreaSizeRequest(formatter) => {
                    let size = alacritty_terminal::event::WindowSize {
                        num_lines: self.size.rows as u16,
                        num_cols: self.size.columns as u16,
                        cell_width: 8,
                        cell_height: 19,
                    };
                    let resp = formatter(size);
                    let _ = self.pty.write(resp.as_bytes());
                }
                _ => {}
            }
        }
    }

    /// El shell terminó.
    pub fn has_exited(&mut self) -> bool {
        self.pty.has_exited()
    }

    /// Manda bytes al programa del otro lado.
    pub fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.pty.write(bytes)
    }

    /// Ajusta la rejilla y el PTY al nuevo tamaño en celdas.
    pub fn resize(&mut self, size: GridSize) -> std::io::Result<()> {
        if size == self.size {
            return Ok(());
        }
        self.size = size;
        self.term.resize(Dimensions_(size));
        self.dirty.set(true);
        self.pty.resize(size)
    }

    /// Desplaza el viewport sobre el scrollback.
    pub fn scroll(&mut self, lines: i32) {
        self.term
            .scroll_display(alacritty_terminal::grid::Scroll::Delta(lines));
        self.dirty.set(true);
    }

    /// Aplica una entrada de rueda del ratón y dice si la consumió.
    ///
    /// `lines` es el desplazamiento pedido (positivo hacia arriba) y `cell` la
    /// celda bajo el puntero. Contrato de terminal real:
    ///
    /// - Si el programa pidió reportes de ratón (modos 1000/1002/1003) y el
    ///   usuario no mantiene Shift, la rueda se le reenvía como reporte SGR
    ///   (`ESC[<64;x;yM` arriba, `ESC[<65;x;yM` abajo) y el scrollback local no
    ///   se toca: dentro de una TUI el historial de PORT no significa nada.
    /// - En cualquier otro caso (sin reportes, o con Shift como salida de
    ///   emergencia) se mueve el scrollback local.
    ///
    /// Se emite un solo reporte por evento, no uno por línea: un evento de
    /// rueda es una muesca física y `lines` solo decide la dirección, igual que
    /// en kitty o alacritty. Solo se usa la codificación SGR (1006), que es la
    /// que habilitan las TUI modernas.
    ///
    /// La pantalla alterna sin reportes no tiene historial propio —alacritty no
    /// guarda scrollback ahí— así que el desplazamiento local queda en cero. Es
    /// el comportamiento aceptado, no un modo nuevo que inventar.
    pub fn wheel(&mut self, lines: i32, cell: CellPos, shift_held: bool) -> bool {
        if lines == 0 {
            return false;
        }
        if !shift_held && self.mouse_reporting() {
            let code = if lines > 0 { 64 } else { 65 };
            let report = format!("\x1b[<{code};{};{}M", cell.column, cell.row);
            // El reporte es entrada del programa, no salida de la rejilla: sale
            // por el mismo camino que las respuestas de protocolo del terminal.
            let _ = self.pty.write(report.as_bytes());
            return true;
        }
        self.scroll(lines);
        true
    }

    /// `true` si el programa del otro lado pidió reportes de ratón.
    ///
    /// Los tres modos de reporte son mutuamente exclusivos en la rejilla, pero
    /// cualquiera de ellos significa que el ratón le pertenece al programa. La
    /// decisión vive en [`buttons_are_reported`] para poder verificarla sin PTY.
    fn mouse_reporting(&self) -> bool {
        buttons_are_reported(*self.term.mode())
    }

    /// Directorio de trabajo actual del shell en tiempo real.
    pub fn current_working_directory(&self) -> Option<std::path::PathBuf> {
        self.pty.current_working_directory()
    }

    /// Nombre de la carpeta de trabajo actual. Si es el directorio HOME, devuelve `~`.
    pub fn current_folder_name(&self) -> Option<String> {
        let path = self.current_working_directory()?;
        if let Ok(home) = std::env::var("HOME") {
            if path == std::path::Path::new(&home) {
                return Some("~".to_string());
            }
        }
        let folder = path.file_name()?.to_string_lossy().to_string();
        Some(folder)
    }

    /// Programa ejecutándose en primer plano en esta sesión, si lo hay.
    pub fn foreground_app(&self) -> Option<RunningApp> {
        self.pty.foreground_app()
    }

    /// El cuadro que la UI debe pintar. Si la rejilla no ha cambiado desde la
    /// última llamada, devuelve la versión en caché sin volver a recorrer celdas.
    pub fn frame(&self) -> Frame {
        if !self.dirty.get() {
            if let Some(cached) = self.cached_frame.borrow().as_ref() {
                return cached.clone();
            }
        }
        let frame = frame::build(&self.term);
        *self.cached_frame.borrow_mut() = Some(frame.clone());
        self.dirty.set(false);
        frame
    }

    /// El estilo efectivo del terminal, para que la UI tenga los colores base.
    pub fn default_style(&self) -> Style {
        Style::plain()
    }

    /// Modo de teclas de flecha que espera el programa del otro lado.
    ///
    /// Lo decide el propio programa con DECCKM (`\e[?1h`), no el usuario: un
    /// editor de líneas necesita `\eOA` donde un shell necesita `\e[A`.
    pub fn cursor_key_mode(&self) -> KeyMode {
        if self.term.mode().contains(TermMode::APP_CURSOR) {
            KeyMode::APPLICATION
        } else {
            KeyMode::NORMAL
        }
    }

    /// Eventos de la rejilla acumulados desde la última llamada: título,
    /// campana, respuesta a consultas.
    pub fn drain_events(&self) -> Vec<TermEvent> {
        self.events.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::CellPos;

    /// El reporte SGR no admite fila ni columna cero: un puntero sobre el margen
    /// debe seguir produciendo una celda válida.
    #[test]
    fn una_celda_fuera_de_la_rejilla_se_recorta_a_la_primera() {
        assert_eq!(CellPos::new(0, 0), CellPos::new(1, 1));
        assert_eq!(CellPos::new(-7, -3), CellPos::new(1, 1));
    }

    #[test]
    fn una_celda_valida_se_conserva_tal_cual() {
        let cell = CellPos::new(80, 24);
        assert_eq!((cell.column, cell.row), (80, 24));
    }
}
