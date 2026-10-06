//! Traducción de teclas de GPUI al vocabulario del núcleo.
//!
//! Es un adaptador: GPUI nombra las teclas como sus-keys y el núcleo nombra las
//! teclas como las entiende un terminal. Que la traducción viva aquí evita que
//! el nombre de GPUI se disperse por la vista, y deja el núcleo sin GPUI.

use gpui::Keystroke;
use port_term_core::input::Key;

/// Acción de edición que PORT resuelve antes de mandar bytes al PTY.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditAction {
    /// Copiar la selección activa al portapapeles.
    Copy,
    /// Pegar el portapapeles en el programa activo.
    Paste,
}

/// Reconoce `Ctrl+Shift+C` y `Ctrl+Shift+V` como copiar y pegar de PORT.
///
/// Exige `Ctrl` y `Shift` a la vez y ningún `Alt`, de modo que `Ctrl+C` y
/// `Ctrl+V` pelados sigan llegando íntegros al programa: el primero es la
/// señal de interrupción y el segundo el pegado literal de la shell. Devuelve
/// `None` para cualquier otra tecla o combinación de modificadores.
pub fn edit_action(key: &Key) -> Option<EditAction> {
    if !key.ctrl || !key.shift || key.alt {
        return None;
    }
    if key.key.eq_ignore_ascii_case("c") {
        Some(EditAction::Copy)
    } else if key.key.eq_ignore_ascii_case("v") {
        Some(EditAction::Paste)
    } else {
        None
    }
}

/// Convierte una pulsación en la tecla que entiende el núcleo.
///
/// Devuelve `None` solo si no hay nada que codificar (por ejemplo, pulsar una
/// tecla modificadora sin carácter). El modo de teclas de flecha lo decide el
/// programa del otro lado, no la tecla, así que no se decide aquí.
pub fn to_core_key(keystroke: &Keystroke) -> Option<Key> {
    let name = core_name(&keystroke.key);
    if name.is_empty() {
        return None;
    }
    let mut key = Key::new(name);
    if keystroke.modifiers.control {
        key = key.ctrl();
    }
    if keystroke.modifiers.alt {
        key = key.alt();
    }
    if keystroke.modifiers.shift {
        key = key.shift();
    }
    // El carácter que el layout le puso a la tecla, si lo hubo. Con Ctrl no hace
    // falta: el núcleo ya arma el código de control a partir del nombre.
    if !keystroke.modifiers.control {
        if let Some(text) = keystroke.key_char.as_deref().filter(|t| !t.is_empty()) {
            key = key.with_text(text);
        } else if keystroke.key.chars().count() == 1 {
            key = key.with_text(&keystroke.key);
        }
    }
    Some(key)
}

/// Nombre de la tecla en el vocabulario del núcleo.
fn core_name(key: &str) -> String {
    match key {
        "enter" | "return" => "Enter".to_string(),
        "backspace" => "Backspace".to_string(),
        "tab" => "Tab".to_string(),
        "escape" | "esc" => "Escape".to_string(),
        "space" => " ".to_string(),
        "up" | "down" | "left" | "right" | "home" | "end" | "delete" | "insert" | "pageup"
        | "pagedown" => capitalize(key),
        other if is_function_key(other) => capitalize(other),
        // Letras, dígitos y signos ya vienen como los ve el layout.
        other => other.to_string(),
    }
}

fn is_function_key(key: &str) -> bool {
    let Some(number) = key.strip_prefix('f') else {
        return false;
    };
    matches!(number.parse::<u8>(), Ok(1..=12))
}

fn capitalize(key: &str) -> String {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Modifiers;
    use port_term_core::input::{encode, KeyMode};

    fn keystroke(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.to_string(),
            key_char: key_char.map(str::to_string),
        }
    }

    fn bytes(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> Option<Vec<u8>> {
        let keystroke = keystroke(key, key_char, modifiers);
        let core = to_core_key(&keystroke)?;
        encode(&core, KeyMode::NORMAL)
    }

    fn none() -> Modifiers {
        Modifiers::default()
    }

    #[test]
    fn named_keys_are_translated() {
        assert_eq!(bytes("enter", None, none()), Some(vec![b'\r']));
        assert_eq!(bytes("backspace", None, none()), Some(vec![0x7f]));
        assert_eq!(bytes("tab", None, none()), Some(vec![b'\t']));
        assert_eq!(bytes("escape", None, none()), Some(vec![0x1b]));
        assert_eq!(bytes("up", None, none()), Some(b"\x1b[A".to_vec()));
    }

    #[test]
    fn function_keys_are_translated() {
        assert_eq!(bytes("f1", None, none()), Some(b"\x1bOP".to_vec()));
        assert_eq!(bytes("f12", None, none()), Some(b"\x1b[24~".to_vec()));
        assert_eq!(
            bytes("f13", None, none()),
            None,
            "F13 no existe en este teclado"
        );
    }

    #[test]
    fn text_goes_through_as_utf8() {
        assert_eq!(bytes("a", Some("a"), none()), Some("a".as_bytes().to_vec()));
        assert_eq!(
            bytes("n", Some("ñ"), none()),
            Some("ñ".as_bytes().to_vec()),
            "el layout decide el carácter, no el nombre de la tecla"
        );
    }

    #[test]
    fn control_letters_become_control_codes() {
        let modifiers = Modifiers {
            control: true,
            ..none()
        };
        assert_eq!(bytes("c", Some("c"), modifiers), Some(vec![3]));
        assert_eq!(bytes("d", Some("d"), modifiers), Some(vec![4]));
        assert_eq!(bytes("space", Some(" "), modifiers), Some(vec![0]));
    }

    #[test]
    fn alt_prefixes_the_escape() {
        let modifiers = Modifiers {
            alt: true,
            ..none()
        };
        assert_eq!(bytes("b", Some("b"), modifiers), Some(b"\x1bb".to_vec()));
    }

    #[test]
    fn modifier_without_a_key_sends_nothing() {
        assert_eq!(bytes("shift", None, none()), None);
    }

    fn tecla(key: &str, ctrl: bool, shift: bool, alt: bool) -> Key {
        let mut k = Key::new(key);
        k.ctrl = ctrl;
        k.shift = shift;
        k.alt = alt;
        k
    }

    /// `Ctrl+Shift+C` copia y `Ctrl+Shift+V` pega: son atajos de PORT, no bytes.
    #[test]
    fn ctrl_shift_c_copia_y_ctrl_shift_v_pega() {
        assert_eq!(
            edit_action(&tecla("c", true, true, false)),
            Some(EditAction::Copy)
        );
        assert_eq!(
            edit_action(&tecla("v", true, true, false)),
            Some(EditAction::Paste)
        );
    }

    /// Falta `Ctrl` o falta `Shift`: la combinación no es el atajo de edición.
    #[test]
    fn sin_ctrl_o_sin_shift_no_hay_atajo_de_edicion() {
        assert_eq!(edit_action(&tecla("c", true, false, false)), None);
        assert_eq!(edit_action(&tecla("c", false, true, false)), None);
        assert_eq!(edit_action(&tecla("v", true, false, false)), None);
        assert_eq!(edit_action(&tecla("v", false, true, false)), None);
        // `Alt` añadido tampoco vale como atajo de edición.
        assert_eq!(edit_action(&tecla("c", true, true, true)), None);
    }

    /// Otras letras con la misma combinación no son atajos de edición.
    #[test]
    fn otras_letras_con_ctrl_shift_no_disparan_nada() {
        assert_eq!(edit_action(&tecla("a", true, true, false)), None);
        assert_eq!(edit_action(&tecla("x", true, true, false)), None);
    }

    /// `Ctrl+C` y `Ctrl+V` pelados siguen siendo del shell: la señal de
    /// interrupción y el pegado literal, no el portapapeles de PORT.
    #[test]
    fn ctrl_c_y_ctrl_v_pelados_no_se_tragan() {
        assert_eq!(edit_action(&tecla("c", true, false, false)), None);
        assert_eq!(edit_action(&tecla("v", true, false, false)), None);
    }

    #[test]
    fn arrows_follow_the_program_cursor_mode() {
        let keystroke = keystroke("right", None, none());
        let key = to_core_key(&keystroke).expect("flecha derecha");
        assert_eq!(encode(&key, KeyMode::NORMAL), Some(b"\x1b[C".to_vec()));
        assert_eq!(encode(&key, KeyMode::APPLICATION), Some(b"\x1bOC".to_vec()));
    }
}
