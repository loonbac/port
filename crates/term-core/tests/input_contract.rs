//! Tests del contrato de entrada de teclado.
//!
//! Escritos antes de la implementación: definen qué se espera de cada tecla.

use port_term_core::input::{encode, Key, KeyMode};

fn plain(key: &str) -> Vec<u8> {
    encode(&Key::new(key), KeyMode::default()).expect("tecla manejada")
}

#[test]
fn plain_character_goes_through_as_utf8() {
    assert_eq!(
        encode(
            &Key::new("a").with_text("a"),
            KeyMode::default()
        ),
        Some(b"a".to_vec())
    );
}

#[test]
fn shifted_character_uses_the_typed_character() {
    assert_eq!(
        encode(&Key::new("A").with_text("A").shift(), KeyMode::default()),
        Some(b"A".to_vec())
    );
}

#[test]
fn non_ascii_character_is_encoded_as_utf8() {
    assert_eq!(
        encode(&Key::new("n").with_text("ñ"), KeyMode::default()),
        Some("ñ".as_bytes().to_vec())
    );
}

#[test]
fn control_letter_becomes_the_control_code() {
    // ctrl+c -> 0x03, ctrl+d -> 0x04, ctrl+z -> 0x1a
    assert_eq!(
        encode(&Key::new("c").ctrl(), KeyMode::default()),
        Some(vec![0x03])
    );
    assert_eq!(
        encode(&Key::new("d").ctrl(), KeyMode::default()),
        Some(vec![0x04])
    );
    assert_eq!(
        encode(&Key::new("z").ctrl(), KeyMode::default()),
        Some(vec![0x1a])
    );
}

#[test]
fn ctrl_space_is_nul() {
    assert_eq!(
        encode(&Key::new(" ").ctrl(), KeyMode::default()),
        Some(vec![0x00])
    );
}

#[test]
fn enter_backspace_and_tab() {
    assert_eq!(plain("Enter"), b"\r".to_vec());
    assert_eq!(plain("Backspace"), b"\x7f".to_vec());
    assert_eq!(plain("Tab"), b"\t".to_vec());
    assert_eq!(plain("Escape"), b"\x1b".to_vec());
}

#[test]
fn arrows_in_normal_and_application_mode() {
    assert_eq!(plain("Up"), b"\x1b[A".to_vec());
    assert_eq!(plain("Down"), b"\x1b[B".to_vec());
    assert_eq!(plain("Right"), b"\x1b[C".to_vec());
    assert_eq!(plain("Left"), b"\x1b[D".to_vec());

    let app = KeyMode {
        application_cursor: true,
    };
    assert_eq!(
        encode(&Key::new("Up"), app),
        Some(b"\x1bOA".to_vec())
    );
}

#[test]
fn navigation_keys() {
    assert_eq!(plain("Home"), b"\x1b[H".to_vec());
    assert_eq!(plain("End"), b"\x1b[F".to_vec());
    assert_eq!(plain("Delete"), b"\x1b[3~".to_vec());
    assert_eq!(plain("PageUp"), b"\x1b[5~".to_vec());
    assert_eq!(plain("PageDown"), b"\x1b[6~".to_vec());
}

#[test]
fn function_keys() {
    assert_eq!(plain("F1"), b"\x1bOP".to_vec());
    assert_eq!(plain("F5"), b"\x1b[15~".to_vec());
    assert_eq!(plain("F12"), b"\x1b[24~".to_vec());
}

#[test]
fn alt_prefixes_escape() {
    assert_eq!(
        encode(&Key::new("f").alt().with_text("f"), KeyMode::default()),
        Some(b"\x1bf".to_vec())
    );
}

#[test]
fn unhandled_key_sends_nothing() {
    assert_eq!(encode(&Key::new("CapsLock"), KeyMode::default()), None);
    assert_eq!(encode(&Key::new("Meta"), KeyMode::default()), None);
}
