//! Detección y extracción de secuencias gráficas del flujo de bytes crudos del PTY.
//!
//! Este módulo procesa los bytes antes del emulador de terminal para interceptar:
//! - Gráficos Sixel (`ESC P ... q ... ST`)
//! - Protocolo Kitty Graphics (`ESC _ G ... ST`)
//!
//! Los bytes ordinarios pasan limpios al emulador sin alteración ni retraso innecesario.

pub mod kitty;
pub mod sixel;
pub mod store;

/// Imagen rasterizada en memoria en formato RGBA de 8 bits por canal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Evento gráfico emitido al detectar y decodificar una secuencia en el flujo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphicsEvent {
    Sixel(Image),
    Kitty(Vec<u8>),
}

/// Resultado de procesar un bloque de bytes con el [`Scanner`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scan {
    pub plain: Vec<u8>,
    pub events: Vec<GraphicsEvent>,
}

/// Límite máximo de bytes acumulables en una secuencia incompleta (32 MiB).
pub const MAX_PENDING_SEQUENCE: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum ScannerState {
    #[default]
    Ground,
    Escape,
    Dcs,
    DcsEscape,
    Apc,
    ApcEscape,
}

/// Escáner en flujo de secuencias gráficas Sixel y Kitty.
#[derive(Clone, Debug, Default)]
pub struct Scanner {
    state: ScannerState,
    pending: Vec<u8>,
}

impl Scanner {
    /// Crea un nuevo escáner en estado inicial limpio.
    pub fn new() -> Self {
        Self::default()
    }

    /// Procesa un bloque de bytes crudos y devuelve los bytes ordinarios junto con los eventos gráficos detectados.
    pub fn advance(&mut self, bytes: &[u8]) -> Scan {
        // Camino rápido: en tierra firme y sin un solo ESC no hay nada que
        // extraer, así que el bloque entero es texto. Importa porque es el caso
        // normal de un `cat` de texto: el escáner no puede costar más que el
        // memcpy que el parser hace igualmente.
        //
        // En `Ground` el búfer de pendientes está siempre vacío: toda transición
        // a ese estado lo limpia.
        if self.state == ScannerState::Ground && !bytes.contains(&0x1b) {
            return Scan {
                plain: bytes.to_vec(),
                events: Vec::new(),
            };
        }

        let mut plain = Vec::new();
        let mut events = Vec::new();

        for &byte in bytes {
            match self.state {
                ScannerState::Ground => {
                    if byte == 0x1b {
                        self.pending.push(0x1b);
                        self.state = ScannerState::Escape;
                    } else {
                        plain.push(byte);
                    }
                }
                ScannerState::Escape => {
                    if byte == b'P' {
                        self.pending.push(b'P');
                        self.state = ScannerState::Dcs;
                    } else if byte == b'_' {
                        self.pending.push(b'_');
                        self.state = ScannerState::Apc;
                    } else if byte == 0x18 || byte == 0x1a {
                        self.pending.clear();
                        self.state = ScannerState::Ground;
                    } else if byte == 0x1b {
                        plain.push(0x1b);
                    } else {
                        plain.push(0x1b);
                        plain.push(byte);
                        self.pending.clear();
                        self.state = ScannerState::Ground;
                    }
                }
                ScannerState::Dcs => {
                    if byte == 0x18 || byte == 0x1a {
                        self.pending.clear();
                        self.state = ScannerState::Ground;
                    } else if byte == 0x1b {
                        self.pending.push(0x1b);
                        self.state = ScannerState::DcsEscape;
                    } else if self.pending.len() >= MAX_PENDING_SEQUENCE {
                        self.pending.clear();
                        self.state = ScannerState::Ground;
                    } else {
                        self.pending.push(byte);
                    }
                }
                ScannerState::DcsEscape => {
                    if byte == b'\\' {
                        self.pending.push(b'\\');
                        if let Some(sixel_body) = parse_sixel_dcs(&self.pending) {
                            if let Some(img) = sixel::decode(sixel_body) {
                                events.push(GraphicsEvent::Sixel(img));
                            }
                        } else {
                            plain.extend_from_slice(&self.pending);
                        }
                        self.pending.clear();
                        self.state = ScannerState::Ground;
                    } else if byte == 0x18 || byte == 0x1a {
                        self.pending.clear();
                        self.state = ScannerState::Ground;
                    } else if byte == 0x1b {
                        // Permanece en DcsEscape con el 0x1b ya retenido
                    } else {
                        if parse_sixel_dcs(&self.pending).is_some() {
                            self.pending.clear();
                            plain.push(byte);
                        } else {
                            plain.extend_from_slice(&self.pending);
                            plain.push(byte);
                            self.pending.clear();
                        }
                        self.state = ScannerState::Ground;
                    }
                }
                ScannerState::Apc => {
                    if byte == 0x18 || byte == 0x1a {
                        self.pending.clear();
                        self.state = ScannerState::Ground;
                    } else if byte == 0x1b {
                        self.pending.push(0x1b);
                        self.state = ScannerState::ApcEscape;
                    } else if self.pending.len() >= MAX_PENDING_SEQUENCE {
                        self.pending.clear();
                        self.state = ScannerState::Ground;
                    } else {
                        self.pending.push(byte);
                    }
                }
                ScannerState::ApcEscape => {
                    if byte == b'\\' {
                        self.pending.push(b'\\');
                        let payload = &self.pending[2..self.pending.len() - 2];
                        if payload.starts_with(b"G") {
                            events.push(GraphicsEvent::Kitty(payload.to_vec()));
                        } else {
                            plain.extend_from_slice(&self.pending);
                        }
                        self.pending.clear();
                        self.state = ScannerState::Ground;
                    } else if byte == 0x18 || byte == 0x1a {
                        self.pending.clear();
                        self.state = ScannerState::Ground;
                    } else if byte == 0x1b {
                        // Permanece en ApcEscape con el 0x1b ya retenido
                    } else {
                        let payload = &self.pending[2..self.pending.len() - 1];
                        if payload.starts_with(b"G") {
                            self.pending.clear();
                            plain.push(byte);
                        } else {
                            plain.extend_from_slice(&self.pending);
                            plain.push(byte);
                            self.pending.clear();
                        }
                        self.state = ScannerState::Ground;
                    }
                }
            }
        }

        Scan { plain, events }
    }

    /// Cantidad de bytes actualmente retenidos en el buffer de secuencia pendiente.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

/// Extrae el cuerpo Sixel si la secuencia DCS completa tiene como carácter final `'q'`.
fn parse_sixel_dcs(dcs_bytes: &[u8]) -> Option<&[u8]> {
    if dcs_bytes.len() < 4 || !dcs_bytes.starts_with(b"\x1bP") || !dcs_bytes.ends_with(b"\x1b\\") {
        return None;
    }
    let inner = &dcs_bytes[2..dcs_bytes.len() - 2];
    let mut idx = 0;
    while idx < inner.len() && (0x20..=0x3f).contains(&inner[idx]) {
        idx += 1;
    }
    if idx < inner.len() && inner[idx] == b'q' {
        Some(&inner[idx + 1..])
    } else {
        None
    }
}
