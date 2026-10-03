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
        let session = Session::spawn(self.default_config.clone(), self.size)?;
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
        self.sessions.get_mut(&self.active_id).expect("sesión activa")
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
        self.sessions.get(&id).and_then(|s| s.current_working_directory())
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
