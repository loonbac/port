//! Pegado: modo de corchetes (`BRACKETED_PASTE`) frente al texto crudo.

use std::time::Duration;

use port_term_core::pty::PtyConfig;
use port_term_core::session::{GridSize, Session};

use crate::{pump_until_quiet, wait_for_modes};

/// Programa que deja la entrada en crudo y devuelve a la pantalla todo lo que
/// recibe, escapando el ESC como `^[`. Con `bracketed` pide además el modo de
/// pegado entre corchetes (`\e[?2004h`); así se observan exactamente los bytes
/// que `paste` escribe al PTY.
fn echo_shell(bracketed: bool) -> PtyConfig {
    let modos = if bracketed { "\\033[?2004h" } else { "" };
    PtyConfig {
        command: "/bin/sh".to_string(),
        args: vec![
            "-c".to_string(),
            format!("stty raw -echo; printf '{modos}'; cat -v"),
        ],
        cwd: None,
    }
}

/// Con `BRACKETED_PASTE` activo, `paste` envuelve el texto entre los marcadores
/// `\e[200~` y `\e[201~`: es lo que permite pegar contenido sin que el programa
/// interprete los controles que traiga.
#[test]
fn el_pegado_respeta_el_modo_de_corchetes_del_programa() {
    let mut session = Session::spawn(echo_shell(true), GridSize::new(60, 6)).expect("sesión");
    wait_for_modes(&mut session);

    session.paste("hola");
    let text = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(10),
    );
    assert!(
        text.contains("^[[200~hola^[[201~"),
        "el pegado debe ir envuelto en los marcadores de corchetes:\n{text}"
    );
}

/// Sin `BRACKETED_PASTE`, `paste` escribe el texto tal cual, sin marcadores.
#[test]
fn el_pegado_sin_modo_de_corchetes_envia_el_texto_crudo() {
    let mut session = Session::spawn(echo_shell(false), GridSize::new(60, 6)).expect("sesión");
    wait_for_modes(&mut session);

    session.paste("hola");
    let text = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(10),
    );
    assert!(
        text.contains("hola"),
        "el texto pegado debe llegar al programa:\n{text}"
    );
    assert!(
        !text.contains("^[[200~"),
        "sin el modo activo no debe haber marcadores de corchetes:\n{text}"
    );
}
