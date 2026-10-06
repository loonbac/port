//! Codificación del pegado hacia el programa.
//!
//! Convierte el texto pegado a los bytes que se envían al PTY, respetando el
//! modo de pegado entre corchetes del programa. [`paste_bytes`] es pura para
//! poder probar el contrato de corchetes sin PTY.

use alacritty_terminal::term::TermMode;

use super::Session;

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

impl Session {
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
}

#[cfg(test)]
mod tests {
    use super::paste_bytes;

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
}
