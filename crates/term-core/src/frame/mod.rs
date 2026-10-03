//! Datos que la UI necesita para pintar un cuadro de terminal.
//!
//! La UI nunca ve tipos de `alacritty_terminal`: recibe [`Frame`] con filas ya
//! agrupadas en runs del mismo estilo. Agrupar es lo que mantiene barato el
//! pintado: una fila de 200 columnas suele ser un puñado de runs, no 200 celdas.
//!
//! Este módulo tiene dos mitades: los datos ([`Frame`], [`Row`], [`Run`]) y el
//! constructor que los saca de la rejilla, en [`builder`].

pub mod builder;

pub use builder::build;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::term::Term;

/// Color en RGB de 8 bits por canal, independiente de cualquier biblioteca.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Color por defecto del texto.
    pub const DEFAULT_FG: Rgb = Rgb::new(0xc9, 0xd1, 0xd9);
    /// Color por defecto del fondo.
    pub const DEFAULT_BG: Rgb = Rgb::new(0x0d, 0x11, 0x17);
}

/// Estilo de un tramo de texto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub fg: Rgb,
    pub bg: Rgb,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
}

impl Style {
    pub const fn plain() -> Self {
        Self {
            fg: Rgb::DEFAULT_FG,
            bg: Rgb::DEFAULT_BG,
            bold: false,
            italic: false,
            underline: false,
            reverse: false,
        }
    }
}

/// Tramo contiguo de celdas con el mismo estilo.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub style: Style,
    /// Ancho en columnas de terminal, que no tiene por qué ser `text.len()`.
    pub columns: usize,
}

/// Dónde está la celda del cursor dentro de los runs de una fila.
///
/// La UI no calcula columnas ni sabe de la rejilla: solo necesita saber qué run
/// invertir para pintar el bloque. El núcleo hace esa cuenta porque es quien
/// conoce el ancho real de cada celda (un carácter ocupa dos columnas).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorRun {
    pub row: usize,
    pub run_index: usize,
}

/// Forma del cursor, para que la UI elija cómo dibujarlo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Beam,
    Underline,
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub row: usize,
    pub column: usize,
    pub shape: CursorShape,
    pub visible: bool,
}

/// Una fila visible, ya agrupada en runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub runs: Vec<Run>,
}

impl Row {
    pub fn width(&self) -> usize {
        self.runs.iter().map(|run| run.columns).sum()
    }
}

/// Un cuadro completo: lo que la UI pinta y nada más.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub rows: Vec<Row>,
    pub columns: usize,
    pub cursor: Option<Cursor>,
    /// Run exacto de la celda del cursor, ya recortado para poder pintarlo.
    pub cursor_run: Option<CursorRun>,
    /// Cuántas líneas se ha desplazado el viewport respecto del final.
    pub display_offset: usize,
}

/// Construye un cuadro desde una rejilla de alacritty.
///
/// Existe como función libre además de [`build`] para que los tests puedan
/// llamarla con una rejilla montada a mano, sin PTY ni ventana.
pub fn frame_of<T: EventListener>(term: &Term<T>) -> Frame {
    build(term)
}
