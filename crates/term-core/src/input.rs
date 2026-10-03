//! Tecla → bytes para el PTY.
//!
//! Lógica pura y headless: entra una tecla con sus modificadores y sale la
//! secuencia que espera el programa del otro lado. No conoce GPUI ni el PTY, así
//! que se prueba sin abrir una ventana ni un proceso.
//!
//! Se cubre el juego de secuencias de xterm que usa cualquier shell interactivo.
//! Lo que no se cubre devuelve `None` a propósito: es preferible no enviar nada
//! que enviar una secuencia equivocada.

/// Una tecla ya normalizada por la capa de UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    /// Nombre de la tecla tal como lo reporta la plataforma: `a`, `Enter`,
    /// `Up`, `Backspace`, `F5`.
    pub key: String,
    /// Carácter que produciría la tecla en el layout activo, si lo hay.
    pub text: Option<String>,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Key {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            text: None,
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    pub fn ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }

    pub fn alt(mut self) -> Self {
        self.alt = true;
        self
    }

    pub fn shift(mut self) -> Self {
        self.shift = true;
        self
    }
}

/// Estado del terminal que cambia la codificación de algunas teclas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyMode {
    /// DECCKM: las flechas se envían en forma de aplicación.
    pub application_cursor: bool,
}

impl KeyMode {
    /// Modo por defecto: secuencias ANSI normales (`\e[A`).
    pub const NORMAL: Self = Self {
        application_cursor: false,
    };

    /// Modo aplicación: las flechas se envían en forma SS3 (`\eOA`).
    pub const APPLICATION: Self = Self {
        application_cursor: true,
    };
}

/// Bytes que hay que escribir en el PTY, o `None` si la tecla no se maneja.
pub fn encode(key: &Key, mode: KeyMode) -> Option<Vec<u8>> {
    let payload = payload_of(key, mode)?;
    if payload.is_empty() {
        return None;
    }
    if key.alt && !payload.starts_with(b"\x1b") {
        // Alt es el prefijo ESC para el programa del otro lado.
        let mut bytes = Vec::with_capacity(payload.len() + 1);
        bytes.push(0x1b);
        bytes.extend_from_slice(&payload);
        return Some(bytes);
    }
    Some(payload)
}

/// Secuencia sin el prefijo de Alt.
fn payload_of(key: &Key, mode: KeyMode) -> Option<Vec<u8>> {
    if let Some(control) = control_code(key) {
        return Some(vec![control]);
    }

    match key.key.as_str() {
        "Enter" | "Return" => return Some(b"\r".to_vec()),
        "Backspace" => return Some(b"\x7f".to_vec()),
        "Tab" => return Some(b"\t".to_vec()),
        "Escape" | "Esc" => return Some(b"\x1b".to_vec()),
        "Up" => return Some(cursor_sequence(b'A', mode)),
        "Down" => return Some(cursor_sequence(b'B', mode)),
        "Right" => return Some(cursor_sequence(b'C', mode)),
        "Left" => return Some(cursor_sequence(b'D', mode)),
        "Home" => return Some(b"\x1b[H".to_vec()),
        "End" => return Some(b"\x1b[F".to_vec()),
        "Delete" => return Some(b"\x1b[3~".to_vec()),
        "Insert" => return Some(b"\x1b[2~".to_vec()),
        "PageUp" => return Some(b"\x1b[5~".to_vec()),
        "PageDown" => return Some(b"\x1b[6~".to_vec()),
        "F1" => return Some(b"\x1bOP".to_vec()),
        "F2" => return Some(b"\x1bOQ".to_vec()),
        "F3" => return Some(b"\x1bOR".to_vec()),
        "F4" => return Some(b"\x1bOS".to_vec()),
        "F5" => return Some(b"\x1b[15~".to_vec()),
        "F6" => return Some(b"\x1b[17~".to_vec()),
        "F7" => return Some(b"\x1b[18~".to_vec()),
        "F8" => return Some(b"\x1b[19~".to_vec()),
        "F9" => return Some(b"\x1b[20~".to_vec()),
        "F10" => return Some(b"\x1b[21~".to_vec()),
        "F11" => return Some(b"\x1b[23~".to_vec()),
        "F12" => return Some(b"\x1b[24~".to_vec()),
        _ => {}
    }

    // Una tecla cualquiera produce texto si el layout le dio un carácter
    // o si el nombre de la tecla es directamente el carácter (letras, dígitos, signos).
    if let Some(text) = key.text.as_deref().filter(|text| !text.is_empty()) {
        return Some(text.as_bytes().to_vec());
    }
    if key.key.chars().count() == 1 {
        return Some(key.key.as_bytes().to_vec());
    }
    None
}

/// Código de control de una letra con Ctrl, o de Ctrl+Espacio.
fn control_code(key: &Key) -> Option<u8> {
    if !key.ctrl {
        return None;
    }
    if key.key == " " {
        return Some(0x00);
    }
    let mut chars = key.key.chars();
    let first = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let lower = first.to_ascii_lowercase();
    if lower.is_ascii_lowercase() {
        return Some(lower as u8 - b'a' + 1);
    }
    match first {
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        '@' => Some(0x00),
        _ => None,
    }
}

fn cursor_sequence(final_byte: u8, mode: KeyMode) -> Vec<u8> {
    let introducer = if mode.application_cursor { b'O' } else { b'[' };
    vec![0x1b, introducer, final_byte]
}
