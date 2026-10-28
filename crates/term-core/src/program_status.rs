//! Detección y escaneo puro de secuencias OSC del PTY para estado de programas.
//!
//! Este módulo implementa un analizador en flujo sin E/S que procesa los bytes
//! crudos emitidos por el proceso del terminal. Extrae:
//!
//! - Protocolo Program Status (OSC 7501): consultas de capacidad y reportes de
//!   estado (`state`, `kind`, `message`).
//! - Título de ventana (OSC 0 y OSC 2).
//! - Progreso en barra de tareas (OSC 9;4).
//!
//! Soporta secuencias partidas entre múltiples lecturas del PTY, acota el
//! buffer acumulador a un tamaño seguro ([`MAX_PENDING_OSC`]) y falla cerrado ante
//! datos corruptos o no reconocidos.

/// Respuesta exacta requerida por pi para confirmar compatibilidad con OSC 7501.
pub const PROGRAM_STATUS_RESPONSE: &[u8] = b"\x1b]7501;?port\x1b\\";

/// Tamaño máximo del buffer acumulador de secuencias OSC pendientes (8 KiB).
pub const MAX_PENDING_OSC: usize = 8192;

/// Estado de ejecución anunciado por un programa interactivo (OSC 7501).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProgramStatus {
    pub state: Option<String>,
    pub kind: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ScannerState {
    #[default]
    Ground,
    Escape,
    OscPayload,
    OscEscape,
}

/// Escáner en flujo de secuencias OSC sobre bytes crudos del PTY.
#[derive(Debug, Clone, Default)]
pub struct ProgramStatusScanner {
    state: ScannerState,
    buffer: Vec<u8>,
    status: Option<ProgramStatus>,
    title: String,
    progress: String,
    overlay: crate::overlay::Flags,
}

impl ProgramStatusScanner {
    /// Crea un nuevo escáner con estado inicial limpio.
    pub fn new() -> Self {
        Self::default()
    }

    /// Alimenta el escáner con un trozo de bytes crudos del PTY.
    ///
    /// Devuelve cuántas consultas de estado (`ESC ] 7501 ; ?`) se detectaron en este trozo
    /// para que el terminal responda inmediatamente con [`PROGRAM_STATUS_RESPONSE`].
    pub fn advance(&mut self, bytes: &[u8]) -> usize {
        let mut queries = 0;
        for &byte in bytes {
            match self.state {
                ScannerState::Ground => {
                    if byte == 0x1b {
                        self.state = ScannerState::Escape;
                    } else if byte == 0x9d {
                        self.buffer.clear();
                        self.state = ScannerState::OscPayload;
                    }
                }
                ScannerState::Escape => {
                    if byte == b']' {
                        self.buffer.clear();
                        self.state = ScannerState::OscPayload;
                    } else if byte == 0x1b {
                        // Permanece en Escape
                    } else {
                        self.state = ScannerState::Ground;
                    }
                }
                ScannerState::OscPayload => {
                    if byte == 0x07 || byte == 0x9c {
                        if self.handle_osc_payload() {
                            queries += 1;
                        }
                        self.buffer.clear();
                        self.state = ScannerState::Ground;
                    } else if byte == 0x1b {
                        self.state = ScannerState::OscEscape;
                    } else if self.buffer.len() < MAX_PENDING_OSC {
                        self.buffer.push(byte);
                    } else {
                        self.buffer.clear();
                        self.state = ScannerState::Ground;
                    }
                }
                ScannerState::OscEscape => {
                    if byte == b'\\' {
                        if self.handle_osc_payload() {
                            queries += 1;
                        }
                        self.buffer.clear();
                        self.state = ScannerState::Ground;
                    } else if byte == b']' {
                        self.buffer.clear();
                        self.state = ScannerState::OscPayload;
                    } else if byte == 0x1b {
                        // Permanece en OscEscape
                    } else {
                        self.buffer.clear();
                        self.state = ScannerState::Ground;
                    }
                }
            }
        }
        queries
    }

    /// Procesa el buffer acumulado de una secuencia OSC completa.
    /// Devuelve `true` si la secuencia era una consulta de capacidad (OSC 7501;?).
    fn handle_osc_payload(&mut self) -> bool {
        let Ok(payload) = std::str::from_utf8(&self.buffer) else {
            return false;
        };

        if let Some(rest) = payload.strip_prefix("7501;") {
            if rest == "?" || rest.starts_with('?') {
                return true;
            }
            let mut state = None;
            let mut kind = None;
            let mut message = None;

            for pair in rest.split(':') {
                if let Some((k, v)) = pair.split_once('=') {
                    match k {
                        "state" => state = Some(v.to_string()),
                        "kind" => kind = Some(v.to_string()),
                        "msg" if !v.is_empty() => {
                            message = decode_base64(v).or_else(|| Some(v.to_string()));
                        }
                        _ => {}
                    }
                }
            }

            self.status = Some(ProgramStatus {
                state,
                kind,
                message,
            });
            return false;
        }

        // OSC 556 solo cambia capas de depuración: no aporta estado de
        // programa, así que va antes y no toca el resto de los campos.
        if let Some(rest) = payload.strip_prefix("556;") {
            self.overlay = crate::overlay::Flags::parse(rest);
            return false;
        }
        if payload.trim() == "556" {
            self.overlay = crate::overlay::Flags::parse("");
            return false;
        }

        if let Some(title) = payload
            .strip_prefix("0;")
            .or_else(|| payload.strip_prefix("2;"))
        {
            self.title = title.to_string();
            return false;
        }

        if let Some(rest) = payload.strip_prefix("9;") {
            if rest.starts_with("4;") || rest == "4" {
                self.progress = rest.to_string();
                return false;
            }
        }

        false
    }

    /// Estado actual del programa anunciado por OSC 7501, si se ha recibido alguno.
    pub fn status(&self) -> Option<&ProgramStatus> {
        self.status.as_ref()
    }

    /// Título anunciado vía OSC 0 u OSC 2.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Progreso anunciado vía OSC 9;4.
    pub fn progress(&self) -> &str {
        &self.progress
    }

    /// Capas de depuración pedidas por el programa vía OSC 556.
    pub fn overlay(&self) -> crate::overlay::Flags {
        self.overlay
    }
}

fn decode_base64_byte(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}

/// Decodifica una cadena en base64 estándar o URL-safe a UTF-8.
pub fn decode_base64(input: &str) -> Option<String> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;

    for &b in bytes {
        if b == b'=' {
            break;
        }
        if b.is_ascii_whitespace() {
            continue;
        }
        let val = decode_base64_byte(b)?;
        buf = (buf << 6) | u32::from(val);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Extrae las últimas `max_lines` líneas no vacías del conjunto de líneas dado,
/// recortadas de espacios alrededor y unidas por `\n`.
pub fn screen_tail_from_lines<I, S>(lines: I, max_lines: usize) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    if max_lines == 0 {
        return String::new();
    }
    let non_empty: Vec<String> = lines
        .into_iter()
        .map(|s| s.as_ref().trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let start = non_empty.len().saturating_sub(max_lines);
    non_empty[start..].join("\n")
}

/// Extrae las últimas `max_lines` líneas no vacías de una cadena con saltos de línea.
pub fn screen_tail_from_text(text: &str, max_lines: usize) -> String {
    screen_tail_from_lines(text.lines(), max_lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parseo_de_state_working() {
        let mut scanner = ProgramStatusScanner::new();
        scanner.advance(b"\x1b]7501;state=working\x1b\\");
        assert_eq!(
            scanner.status(),
            Some(&ProgramStatus {
                state: Some("working".into()),
                kind: None,
                message: None,
            })
        );

        let mut scanner_with_msg = ProgramStatusScanner::new();
        // "working" en base64 es "d29ya2luZw=="
        scanner_with_msg.advance(b"\x1b]7501;app=pi:state=working:msg=d29ya2luZw==\x07");
        assert_eq!(
            scanner_with_msg.status(),
            Some(&ProgramStatus {
                state: Some("working".into()),
                kind: None,
                message: Some("working".into()),
            })
        );
    }

    #[test]
    fn parseo_de_state_blocked_kind_question() {
        let mut scanner = ProgramStatusScanner::new();
        scanner.advance(b"\x1b]7501;state=blocked:kind=question\x1b\\");
        assert_eq!(
            scanner.status(),
            Some(&ProgramStatus {
                state: Some("blocked".into()),
                kind: Some("question".into()),
                message: None,
            })
        );
    }

    #[test]
    fn parseo_de_state_clear() {
        let mut scanner = ProgramStatusScanner::new();
        scanner.advance(b"\x1b]7501;state=clear\x1b\\");
        assert_eq!(
            scanner.status(),
            Some(&ProgramStatus {
                state: Some("clear".into()),
                kind: None,
                message: None,
            })
        );
    }

    #[test]
    fn consulta_produce_respuesta_esperada() {
        let mut scanner = ProgramStatusScanner::new();
        let q1 = scanner.advance(b"\x1b]7501;?\x1b\\");
        assert_eq!(q1, 1, "la consulta ST debe detectarse");

        let q2 = scanner.advance(b"\x1b]7501;?\x07");
        assert_eq!(q2, 1, "la consulta BEL debe detectarse");

        assert_eq!(PROGRAM_STATUS_RESPONSE, b"\x1b]7501;?port\x1b\\");
    }

    #[test]
    fn titulo_y_progreso() {
        let mut scanner = ProgramStatusScanner::new();
        scanner.advance(b"\x1b]0;Primer Titulo\x07");
        assert_eq!(scanner.title(), "Primer Titulo");

        scanner.advance(b"\x1b]2;Segundo Titulo\x1b\\");
        assert_eq!(scanner.title(), "Segundo Titulo");

        scanner.advance(b"\x1b]9;4;3\x1b\\");
        assert_eq!(scanner.progress(), "4;3");

        scanner.advance(b"\x1b]9;4;0\x07");
        assert_eq!(scanner.progress(), "4;0");
    }

    #[test]
    fn secuencia_partida_en_dos_trozos() {
        let mut scanner = ProgramStatusScanner::new();
        scanner.advance(b"\x1b]7501;state=work");
        assert_eq!(
            scanner.status(),
            None,
            "aún incompleta no debe emitir estado"
        );

        scanner.advance(b"ing\x1b\\");
        assert_eq!(
            scanner.status(),
            Some(&ProgramStatus {
                state: Some("working".into()),
                kind: None,
                message: None,
            })
        );

        let mut title_scanner = ProgramStatusScanner::new();
        title_scanner.advance(b"\x1b]2;titulo partido\x1b");
        assert_eq!(title_scanner.title(), "");
        title_scanner.advance(b"\\");
        assert_eq!(title_scanner.title(), "titulo partido");

        let mut query_scanner = ProgramStatusScanner::new();
        query_scanner.advance(b"\x1b");
        let q = query_scanner.advance(b"]7501;?\x07");
        assert_eq!(q, 1);
    }

    #[test]
    fn buffer_sobredimensionado_no_entra_en_panico() {
        let mut scanner = ProgramStatusScanner::new();
        scanner.advance(b"\x1b]");
        let huge = vec![b'a'; 10_000];
        scanner.advance(&huge);
        scanner.advance(b"\x07");

        // Falla cerrado: nada almacenado, pero sigue funcionando normalmente
        assert_eq!(scanner.title(), "");
        assert_eq!(scanner.status(), None);

        scanner.advance(b"\x1b]0;recuperado\x07");
        assert_eq!(scanner.title(), "recuperado");
    }

    #[test]
    fn screen_tail_sobre_texto_dado() {
        assert_eq!(screen_tail_from_text("", 5), "");
        assert_eq!(screen_tail_from_text("linea 1\nlinea 2", 0), "");

        let sample = "linea 1\n\n  linea 2  \n   \nlinea 3\n\n";
        assert_eq!(
            screen_tail_from_text(sample, 2),
            "linea 2\nlinea 3",
            "debe tomar las últimas 2 no vacías recortadas"
        );
        assert_eq!(
            screen_tail_from_text(sample, 5),
            "linea 1\nlinea 2\nlinea 3",
            "debe tomar todas las no vacías si max_lines supera la cantidad"
        );
    }
}
