//! Sesión de terminal.
//!
//! Caso de uso que coordina las tres piezas: el proceso ([`crate::pty`]), la
//! rejilla VT de `alacritty_terminal` y el viewport que el usuario ve. Es el
//! único punto que conoce a las tres; ni la UI ni el PTY conocen a los demás.

use std::cell::{Cell, RefCell};
use std::sync::mpsc::{channel, Receiver, Sender};

use alacritty_terminal::event::{Event as TermEvent, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;

use crate::frame::{self, Frame, Style};
use crate::input::KeyMode;
use crate::pty::{Pty, PtyConfig};

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
    sender: Sender<TermEvent>,
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
