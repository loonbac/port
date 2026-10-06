//! Sesión de terminal.
//!
//! Caso de uso que coordina las tres piezas: el proceso ([`crate::pty`]), la
//! rejilla VT de `alacritty_terminal` y el viewport que el usuario ve. Es el
//! único punto que conoce a las tres; ni la UI ni el PTY conocen a los demás.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::sync::mpsc::{channel, Receiver, Sender};

use alacritty_terminal::event::{Event as TermEvent, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::viewport_to_point;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;

use crate::frame::{self, Frame, Style};
use crate::input::KeyMode;
use crate::pty::{Pty, PtyConfig, RunningApp};

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

/// Tipo de selección pedido por el gesto del ratón.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionKind {
    /// Celda a celda, sin expansión.
    Simple,
    /// Se expande a la palabra completa bajo el puntero.
    Word,
    /// Selecciona líneas enteras.
    Line,
}

/// Botón del ratón reenviado al programa.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

impl MouseButton {
    /// Código base del botón en el protocolo SGR: 0, 1 y 2.
    fn code(self) -> u8 {
        match self {
            Self::Left => 0,
            Self::Middle => 1,
            Self::Right => 2,
        }
    }
}

/// Modificadores que acompañan a un evento de ratón.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// Convierte una celda del viewport (1-based) en la coordenada absoluta del
/// buffer de la rejilla.
///
/// La celda se recorta a la rejilla visible antes de convertir, de modo que un
/// arrastre más allá del borde nunca produce un punto fuera de rango. La
/// coordenada resultante es absoluta: `viewport_to_point` le resta el
/// desplazamiento del viewport, así que sobrevive al scroll.
///
/// Es pura para poder verificar la conversión sin PTY.
pub(crate) fn cell_to_point(
    display_offset: usize,
    columns: usize,
    rows: usize,
    cell: CellPos,
) -> Point {
    let column = usize::from(cell.column.saturating_sub(1)).min(columns.saturating_sub(1));
    let row = usize::from(cell.row.saturating_sub(1)).min(rows.saturating_sub(1));
    viewport_to_point(display_offset, Point::new(row, Column(column)))
}

/// Construye los bytes que se envían al PTY al pegar.
///
/// Es pura para poder probar el contrato de corchetes sin PTY: con
/// `bracketed`, el texto va envuelto entre `\e[200~` y `\e[201~`.
pub(crate) fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    if !bracketed {
        return text.as_bytes().to_vec();
    }
    let mut bytes = Vec::with_capacity(text.len() + 12);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~");
    bytes
}

/// `true` si el programa pidió reportes de ratón en cualquiera de sus modos.
///
/// El modo 1000 (clics), el 1002 (arrastre) y el 1003 (todo movimiento) son
/// mutuamente excluyentes en la rejilla, pero los tres significan que el ratón
/// pertenece al programa. Es pura para poder verificar el gating de modos sin
/// PTY.
pub(crate) fn buttons_are_reported(mode: TermMode) -> bool {
    mode.intersects(TermMode::MOUSE_MODE)
}

/// Bits de modificador que se suman al código del botón en el reporte SGR:
/// Shift 4, Alt 8, Ctrl 16.
pub(crate) fn modifier_bits(mods: MouseModifiers) -> u8 {
    (u8::from(mods.shift) << 2) | (u8::from(mods.alt) << 3) | (u8::from(mods.ctrl) << 4)
}

/// Codifica un reporte SGR del ratón: `ESC[<código;columna;fila` más `M` si el
/// evento es una pulsación o un movimiento, y `m` si es una liberación.
///
/// La celda ya llega 1-based y recortada por [`CellPos`]: aquí no se vuelve a
/// recortar. Es pura para poder verificar los bytes exactos sin PTY.
pub(crate) fn sgr_mouse_report(code: u8, cell: CellPos, released: bool) -> String {
    let final_byte = if released { 'm' } else { 'M' };
    format!("\x1b[<{code};{};{}{final_byte}", cell.column, cell.row)
}

/// Código SGR de una pulsación o liberación de botón, o `None` si el gesto no
/// pertenece al programa.
///
/// Devuelve `None` con Shift —la salida de emergencia que devuelve el gesto a
/// la selección local de PORT, igual que en la rueda— y también cuando el
/// programa no pidió reportes de botón. La liberación comparte el mismo gate:
/// el programa que pidió reportes espera el `m` que cierra su gesto.
pub(crate) fn press_code(mode: TermMode, button: MouseButton, mods: MouseModifiers) -> Option<u8> {
    if mods.shift || !buttons_are_reported(mode) {
        return None;
    }
    Some(button.code() | modifier_bits(mods))
}

/// Código base de un movimiento, o `None` si el movimiento no se reenvía.
///
/// Con 1003 (todos los movimientos) cualquier movimiento se reenvía: el botón
/// pulsado más 32, y 35 (32 + 3, «sin botón») cuando no hay ninguno. Con solo
/// 1002 el movimiento se reenvía únicamente mientras un botón esté pulsado; un
/// mover suelto pertenece a PORT.
pub(crate) fn motion_base_code(mode: TermMode, held: Option<MouseButton>) -> Option<u8> {
    let all_motion = mode.contains(TermMode::MOUSE_MOTION);
    if !all_motion && !mode.contains(TermMode::MOUSE_DRAG) {
        return None;
    }
    if !all_motion && held.is_none() {
        return None;
    }
    Some(match held {
        Some(button) => button.code() + 32,
        None => 35,
    })
}

/// Código SGR completo de un movimiento, o `None` si el gesto es de PORT.
///
/// Aplica el mismo escape de Shift que la pulsación y suma los bits de
/// modificador al código base.
pub(crate) fn motion_code(
    mode: TermMode,
    held: Option<MouseButton>,
    mods: MouseModifiers,
) -> Option<u8> {
    if mods.shift {
        return None;
    }
    motion_base_code(mode, held).map(|base| base | modifier_bits(mods))
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

    /// Convierte una celda del viewport a la coordenada absoluta de la rejilla.
    ///
    /// Se apoya en [`cell_to_point`] con el desplazamiento actual del viewport y
    /// el tamaño de la sesión.
    fn point_at(&self, cell: CellPos) -> Point {
        let display_offset = self.term.grid().display_offset();
        cell_to_point(display_offset, self.size.columns, self.size.rows, cell)
    }

    /// Inicia una selección anclada en la celda dada.
    ///
    /// El ancla usa el lado izquierdo y cada extensión el lado derecho, que es
    /// como alacritty delimita las celdas cubiertas.
    pub fn start_selection(&mut self, cell: CellPos, kind: SelectionKind) {
        let point = self.point_at(cell);
        let ty = match kind {
            SelectionKind::Simple => SelectionType::Simple,
            SelectionKind::Word => SelectionType::Semantic,
            SelectionKind::Line => SelectionType::Lines,
        };
        self.term.selection = Some(Selection::new(ty, point, Side::Left));
        self.dirty.set(true);
    }

    /// Extiende la selección activa hasta la celda dada.
    ///
    /// Sin selección activa no hace nada: una extensión suelta no crea una
    /// selección desde cero.
    pub fn extend_selection(&mut self, cell: CellPos) {
        let point = self.point_at(cell);
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, Side::Right);
            self.dirty.set(true);
        }
    }

    /// Borra la selección activa.
    pub fn clear_selection(&mut self) {
        if self.term.selection.take().is_some() {
            self.dirty.set(true);
        }
    }

    /// `true` si hay una selección activa y no vacía.
    pub fn has_selection(&self) -> bool {
        self.term
            .selection
            .as_ref()
            .is_some_and(|selection| !selection.is_empty())
    }

    /// Texto de la selección activa, si la hay.
    ///
    /// El recorte, los saltos de línea y los cuatro tipos los resuelve
    /// `alacritty_terminal`; aquí no se reimplementa la extracción.
    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }

    /// Envía texto pegado al programa, respetando el modo de pegado entre
    /// corchetes.
    ///
    /// Si el programa activó `BRACKETED_PASTE` (`\e[?2004h`), el texto va
    /// envuelto en `\e[200~`/`\e[201~` para que la aplicación lo trate como una
    /// sola entrada y no interprete saltos de línea ni controles.
    pub fn paste(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let bracketed = self.term.mode().contains(TermMode::BRACKETED_PASTE);
        let bytes = paste_bytes(text, bracketed);
        // El pegado es entrada del programa: sale por el mismo camino que los
        // reportes de protocolo y la rueda, nunca por la rejilla.
        let _ = self.pty.write(&bytes);
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

    /// Reenvía la pulsación de un botón al programa. `true` si la consumió.
    ///
    /// Contrato de terminal real, el mismo que ya sigue la rueda:
    ///
    /// - Si el programa pidió reportes de ratón (modos 1000/1002/1003) y el
    ///   usuario no mantiene Shift, la pulsación se le reenvía como reporte SGR
    ///   (`ESC[<código;x;yM`, con los bits de modificador ya sumados) y el gesto
    ///   pertenece al programa: PORT no inicia su propia selección.
    /// - Si el programa no pidió reportes, o si el usuario mantiene Shift como
    ///   salida de emergencia, devuelve `false` y el gesto queda para la
    ///   selección local.
    ///
    /// Se emite un solo reporte por evento y no se toca `dirty`: la rejilla no
    /// cambia, porque el reporte es entrada del programa y sale por el mismo
    /// camino que las respuestas de protocolo y la rueda.
    pub fn mouse_press(
        &mut self,
        button: MouseButton,
        cell: CellPos,
        mods: MouseModifiers,
    ) -> bool {
        let Some(code) = press_code(*self.term.mode(), button, mods) else {
            return false;
        };
        let report = sgr_mouse_report(code, cell, false);
        let _ = self.pty.write(report.as_bytes());
        true
    }

    /// Reenvía la liberación de un botón al programa. `true` si la consumió.
    ///
    /// Comparte el gate de la pulsación (los mismos modos y el mismo escape de
    /// Shift) y solo cambia la letra final del reporte: `ESC[<código;x;y m`. Un
    /// programa que pidió reportes espera también el `m` que cierra el gesto,
    /// así que sin esa liberación su arrastre quedaría colgado.
    pub fn mouse_release(
        &mut self,
        button: MouseButton,
        cell: CellPos,
        mods: MouseModifiers,
    ) -> bool {
        let Some(code) = press_code(*self.term.mode(), button, mods) else {
            return false;
        };
        let report = sgr_mouse_report(code, cell, true);
        let _ = self.pty.write(report.as_bytes());
        true
    }

    /// Reenvía un movimiento del ratón al programa. `true` si lo consumió.
    ///
    /// `held` es el botón pulsado, si lo hay. El movimiento se reenvía cuando:
    ///
    /// - el programa activó 1003 (todos los movimientos), con o sin botón
    ///   pulsado —es lo que necesitan el arrastre del scrollbar y el hover de
    ///   una TUI—;
    /// - o activó 1002 (arrastre) y hay un botón pulsado. Un mover suelto queda
    ///   para PORT.
    ///
    /// En cualquier otro caso devuelve `false`, y Shift sigue siendo la salida
    /// de emergencia que conserva el movimiento para la selección local.
    ///
    /// Un reporte de movimiento no cambia la rejilla: no se toca `dirty`.
    pub fn mouse_motion(
        &mut self,
        cell: CellPos,
        held: Option<MouseButton>,
        mods: MouseModifiers,
    ) -> bool {
        let Some(code) = motion_code(*self.term.mode(), held, mods) else {
            return false;
        };
        let report = sgr_mouse_report(code, cell, false);
        let _ = self.pty.write(report.as_bytes());
        true
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

/// Gestor de múltiples sesiones de terminal vivas (espacios de trabajo / pestañas).
pub struct SessionManager {
    sessions: BTreeMap<usize, Session>,
    active_id: usize,
    next_id: usize,
    default_config: PtyConfig,
    size: GridSize,
}

impl SessionManager {
    /// Inicia el gestor con una primera sesión interactiva (ID = 0).
    pub fn new(config: PtyConfig, size: GridSize) -> std::io::Result<Self> {
        let first = Session::spawn(config.clone(), size)?;
        let mut map = BTreeMap::new();
        map.insert(0, first);
        Ok(Self {
            sessions: map,
            active_id: 0,
            next_id: 1,
            default_config: config,
            size,
        })
    }

    /// Crea y añade una nueva sesión de terminal con su propio proceso PTY y la activa.
    /// Devuelve el identificador único estable asignado a la nueva sesión.
    pub fn spawn_session(&mut self) -> std::io::Result<usize> {
        self.spawn_session_config(self.default_config.clone())
    }

    /// Crea y añade una sesión que ejecuta el programa indicado y la activa.
    pub fn spawn_session_with(&mut self, config: PtyConfig) -> std::io::Result<usize> {
        self.spawn_session_config(config)
    }

    /// Arranca el PTY de la sesión nueva, la registra y le da el foco.
    fn spawn_session_config(&mut self, config: PtyConfig) -> std::io::Result<usize> {
        let session = Session::spawn(config, self.size)?;
        let id = self.next_id;
        self.next_id += 1;
        self.sessions.insert(id, session);
        self.active_id = id;
        Ok(id)
    }

    /// Selecciona la sesión activa por su ID. Devuelve `true` si el ID existe.
    pub fn select(&mut self, id: usize) -> bool {
        if self.sessions.contains_key(&id) {
            self.active_id = id;
            true
        } else {
            false
        }
    }

    /// Cierra una sesión por ID si hay más de una viva.
    /// Elimina las sesiones cuyo shell ya terminó. Devuelve los identificadores
    /// retirados para que la UI pueda quitar sus pestañas y espacios.
    ///
    /// Sin esto, ejecutar `exit` dejaba la sesión muerta en el gestor y la
    /// ventana se quedaba congelada mostrando la última rejilla.
    pub fn reap_exited(&mut self) -> Vec<usize> {
        let mut removed = Vec::new();
        self.sessions.retain(|&id, session| {
            let alive = !session.has_exited();
            if !alive {
                removed.push(id);
            }
            alive
        });

        if !self.sessions.is_empty() && !self.sessions.contains_key(&self.active_id) {
            self.active_id = *self.sessions.keys().next().unwrap();
        }

        removed
    }

    /// Cierra una sesión por su identificador.
    ///
    /// La última sesión viva no se puede cerrar desde aquí: una terminal sin
    /// sesiones debe cerrar su ventana, no quedarse hungueada. Eso lo decide la
    /// UI, que llama a `remove_window` cuando el gestor se queda vacío.
    pub fn close(&mut self, id: usize) -> bool {
        if self.sessions.len() > 1 && self.sessions.contains_key(&id) {
            self.sessions.remove(&id);
            if self.active_id == id {
                if let Some(&first_key) = self.sessions.keys().next() {
                    self.active_id = first_key;
                }
            }
            true
        } else {
            false
        }
    }

    /// Referencia a la sesión activa.
    pub fn active_session(&self) -> &Session {
        &self.sessions[&self.active_id]
    }

    /// Referencia mutable a la sesión activa.
    pub fn active_session_mut(&mut self) -> &mut Session {
        self.sessions
            .get_mut(&self.active_id)
            .expect("sesión activa")
    }

    /// ID de la sesión activa actual.
    pub fn active_id(&self) -> usize {
        self.active_id
    }

    /// Para compatibilidad con active_index.
    pub fn active_index(&self) -> usize {
        self.active_id
    }

    /// Cantidad de sesiones vivas.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Devuelve `true` si no hay sesiones.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Bombea todas las sesiones vivas. Devuelve `true` si la sesión activa tuvo cambios.
    pub fn pump(&mut self) -> bool {
        let mut active_changed = false;
        for (&id, session) in self.sessions.iter_mut() {
            let changed = session.pump();
            if id == self.active_id && changed {
                active_changed = true;
            }
        }
        active_changed
    }

    /// Redimensiona todas las sesiones a la geometría dada.
    pub fn resize(&mut self, size: GridSize) -> std::io::Result<()> {
        self.size = size;
        for session in self.sessions.values_mut() {
            session.resize(size)?;
        }
        Ok(())
    }

    /// Tamaño actual en celdas.
    pub fn size(&self) -> GridSize {
        self.size
    }

    /// Cuadro de la sesión activa para renderizado.
    pub fn frame(&self) -> Frame {
        self.active_session().frame()
    }

    /// Escribe bytes en el PTY de la sesión activa.
    pub fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.active_session_mut().write(bytes)
    }

    /// Modo de cursor de la sesión activa.
    pub fn cursor_key_mode(&self) -> KeyMode {
        self.active_session().cursor_key_mode()
    }

    /// Estilo por defecto de la sesión activa.
    pub fn default_style(&self) -> Style {
        self.active_session().default_style()
    }

    /// Mapa de todas las sesiones vivas.
    pub fn sessions(&self) -> &BTreeMap<usize, Session> {
        &self.sessions
    }

    /// Programas en primer plano de todas las sesiones, por ID de sesión.
    pub fn running_apps(&self) -> Vec<(usize, RunningApp)> {
        self.sessions
            .iter()
            .filter_map(|(&id, session)| session.foreground_app().map(|app| (id, app)))
            .collect()
    }

    /// `true` si alguna sesión tiene un programa en primer plano.
    pub fn has_running_app(&self) -> bool {
        self.sessions.values().any(|s| s.foreground_app().is_some())
    }

    /// Obtiene el directorio de trabajo de una sesión por su ID.
    pub fn session_cwd(&self, id: usize) -> Option<std::path::PathBuf> {
        self.sessions
            .get(&id)
            .and_then(|s| s.current_working_directory())
    }

    /// Obtiene el nombre de la carpeta de una sesión por su ID.
    pub fn session_folder_name(&self, id: usize) -> Option<String> {
        self.sessions.get(&id).and_then(|s| s.current_folder_name())
    }

    /// Directorio de trabajo de la sesión activa actual.
    pub fn active_cwd(&self) -> Option<std::path::PathBuf> {
        self.session_cwd(self.active_id)
    }

    /// Programa en primer plano de la sesión indicada, si lo hay.
    pub fn session_app(&self, id: usize) -> Option<RunningApp> {
        self.sessions.get(&id).and_then(|s| s.foreground_app())
    }

    /// Programa en primer plano de la sesión activa actual, si lo hay.
    pub fn active_app(&self) -> Option<RunningApp> {
        self.session_app(self.active_id)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        buttons_are_reported, cell_to_point, modifier_bits, motion_base_code, motion_code,
        paste_bytes, press_code, sgr_mouse_report, CellPos, MouseButton, MouseModifiers,
    };
    use alacritty_terminal::index::{Column, Line, Point};
    use alacritty_terminal::term::TermMode;

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

    /// Sin desplazamiento del viewport, la fila 1 de pantalla es la línea 0 del
    /// buffer y la columna 1 es la columna 0: la conversión es 1-based → 0-based.
    #[test]
    fn la_conversion_sin_desplazamiento_mapea_el_viewport_al_buffer() {
        assert_eq!(
            cell_to_point(0, 80, 24, CellPos::new(1, 1)),
            Point::new(Line(0), Column(0))
        );
        assert_eq!(
            cell_to_point(0, 80, 24, CellPos::new(5, 3)),
            Point::new(Line(2), Column(4))
        );
    }

    /// Con el viewport desplazado sobre el historial, la misma fila de pantalla
    /// apunta a una línea anterior del buffer: la conversión es absoluta.
    #[test]
    fn la_conversion_con_desplazamiento_mueve_la_linea_del_buffer() {
        assert_eq!(
            cell_to_point(3, 80, 24, CellPos::new(1, 1)),
            Point::new(Line(-3), Column(0))
        );
    }

    /// Una celda fuera de la rejilla se recorta antes de convertir, para que un
    /// arrastre más allá del borde no genere un punto fuera del buffer.
    #[test]
    fn la_conversion_recorta_las_celdas_fuera_de_la_rejilla() {
        assert_eq!(
            cell_to_point(0, 10, 4, CellPos::new(99, 99)),
            Point::new(Line(3), Column(9))
        );
    }

    /// Sin corchetes, el pegado son los bytes del texto tal cual.
    #[test]
    fn el_pegado_sin_corchetes_envia_el_texto_crudo() {
        assert_eq!(paste_bytes("hola\n", false), b"hola\n".to_vec());
    }

    /// Con corchetes, el texto queda envuelto entre los marcadores de pegado.
    #[test]
    fn el_pegado_con_corchetes_envuelve_el_texto() {
        assert_eq!(
            paste_bytes("hola", true),
            b"\x1b[200~hola\x1b[201~".to_vec()
        );
    }

    /// Una pulsación es `ESC[<código;col;filaM` con la letra `M` final. La celda
    /// ya llega 1-based y recortada por `CellPos`: el codificador no la toca.
    #[test]
    fn la_pulsacion_se_codifica_como_reporte_sgr() {
        assert_eq!(
            sgr_mouse_report(0, CellPos::new(5, 3), false),
            "\x1b[<0;5;3M"
        );
        assert_eq!(
            sgr_mouse_report(2, CellPos::new(1, 1), false),
            "\x1b[<2;1;1M"
        );
    }

    /// La liberación cambia solo la letra final: `m` en minúscula.
    #[test]
    fn la_liberacion_se_codifica_con_m_minuscula() {
        assert_eq!(
            sgr_mouse_report(0, CellPos::new(5, 3), true),
            "\x1b[<0;5;3m"
        );
    }

    /// Los botones ocupan los códigos base 0, 1 y 2 (izquierdo, medio, derecho),
    /// y con reportes de ratón activos la pulsación se reenvía tal cual.
    #[test]
    fn cada_boton_se_codifica_con_su_codigo_base() {
        let modo = TermMode::MOUSE_REPORT_CLICK;
        let sin_mods = MouseModifiers::default();
        assert_eq!(press_code(modo, MouseButton::Left, sin_mods), Some(0));
        assert_eq!(press_code(modo, MouseButton::Middle, sin_mods), Some(1));
        assert_eq!(press_code(modo, MouseButton::Right, sin_mods), Some(2));
    }

    /// Los modificadores se suman al código con los bits del protocolo: Shift 4,
    /// Alt 8, Ctrl 16. Shift solo, y el par Ctrl+Alt, son los dos casos que
    /// ejercitan bits sueltos y combinados.
    #[test]
    fn los_modificadores_suman_sus_bits_al_codigo() {
        let solo_shift = MouseModifiers {
            shift: true,
            ..Default::default()
        };
        let ctrl_alt = MouseModifiers {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        assert_eq!(modifier_bits(solo_shift), 4);
        assert_eq!(modifier_bits(ctrl_alt), 24);
        assert_eq!(modifier_bits(MouseModifiers::default()), 0);

        // El mismo código compuesto que viaja en la pulsación del botón derecho
        // con Ctrl y Alt: 2 + 16 + 8.
        assert_eq!(
            press_code(TermMode::MOUSE_REPORT_CLICK, MouseButton::Right, ctrl_alt),
            Some(26)
        );
        assert_eq!(
            sgr_mouse_report(26, CellPos::new(5, 3), false),
            "\x1b[<26;5;3M"
        );
    }

    /// Shift es la salida de emergencia, igual que en la rueda: aunque el
    /// programa pida reportes, pulsación y movimiento vuelven a PORT.
    #[test]
    fn shift_devuelve_el_gesto_a_la_seleccion_local() {
        let modo = TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION;
        let con_shift = MouseModifiers {
            shift: true,
            ..Default::default()
        };
        assert_eq!(press_code(modo, MouseButton::Left, con_shift), None);
        assert_eq!(motion_code(modo, Some(MouseButton::Left), con_shift), None);
    }

    /// El movimiento se codifica con el botón pulsado más 32, y usa 35 (32 + 3,
    /// "sin botón") cuando no hay ninguno: es lo que define el protocolo.
    #[test]
    fn el_movimiento_usa_el_boton_pulsado_mas_32() {
        let modo = TermMode::MOUSE_MOTION;
        assert_eq!(motion_base_code(modo, Some(MouseButton::Left)), Some(32));
        assert_eq!(motion_base_code(modo, Some(MouseButton::Middle)), Some(33));
        assert_eq!(motion_base_code(modo, Some(MouseButton::Right)), Some(34));
        assert_eq!(motion_base_code(modo, None), Some(35));
        assert_eq!(motion_code(modo, None, MouseModifiers::default()), Some(35));
        assert_eq!(
            sgr_mouse_report(35, CellPos::new(2, 4), false),
            "\x1b[<35;2;4M"
        );
    }

    /// Con solo 1000 el programa pidió clics: el movimiento no se reenvía, ni
    /// con botón pulsado ni sin él.
    #[test]
    fn solo_con_clics_el_movimiento_no_se_reenvia() {
        let modo = TermMode::MOUSE_REPORT_CLICK;
        assert_eq!(
            press_code(modo, MouseButton::Left, MouseModifiers::default()),
            Some(0)
        );
        assert_eq!(motion_base_code(modo, Some(MouseButton::Left)), None);
        assert_eq!(motion_base_code(modo, None), None);
    }

    /// Con 1002 el arrastre pertenece al programa solo mientras un botón esté
    /// pulsado: un mover suelto vuelve a PORT.
    #[test]
    fn con_arrastre_el_movimiento_necesita_un_boton_pulsado() {
        let modo = TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG;
        assert_eq!(motion_base_code(modo, Some(MouseButton::Left)), Some(32));
        assert_eq!(motion_base_code(modo, None), None);
    }

    /// Al añadir 1003 (todos los movimientos) también se reenvía el mover suelto:
    /// es lo que habilita el arrastre del scrollbar de pi.
    #[test]
    fn con_todos_los_movimientos_el_mover_suelto_se_reenvia() {
        let modo = TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION;
        assert_eq!(motion_base_code(modo, None), Some(35));
        assert_eq!(motion_base_code(modo, Some(MouseButton::Right)), Some(34));
    }

    /// Sin ningún modo de reporte el ratón pertenece a PORT: ni la pulsación ni
    /// el movimiento salen hacia el programa.
    #[test]
    fn sin_reportes_de_raton_el_gesto_no_sale() {
        let modo = TermMode::default();
        assert!(!buttons_are_reported(modo));
        assert_eq!(
            press_code(modo, MouseButton::Left, MouseModifiers::default()),
            None
        );
        assert_eq!(motion_base_code(modo, Some(MouseButton::Left)), None);
        assert_eq!(motion_code(modo, None, MouseModifiers::default()), None);
    }
}
