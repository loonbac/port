//! Contrato del caso de uso [`Session`]: shell real, PTY real, sin ventana.
//!
//! Es la verificación de integración del núcleo: si esto pasa, el shell responde
//! y la rejilla refleja lo que el shell escribe.

mod drop;
#[cfg(target_os = "linux")]
mod foreground;
mod grid;
mod manager;
mod mouse;
mod paste;
mod pty;

use std::time::{Duration, Instant};

use port_term_core::pty::PtyConfig;
use port_term_core::session::{GridSize, Session, SessionManager};

/// Arranca un shell no interactivo predecible en vez del del usuario.
fn plain_shell() -> PtyConfig {
    PtyConfig {
        command: "/bin/sh".to_string(),
        args: Vec::new(),
        cwd: None,
        env: Vec::new(),
    }
}

/// Bombea la sesión hasta que la salida estabilice o se acabe el tiempo.
fn pump_until_quiet(session: &mut Session, quiet: Duration, limit: Duration) -> String {
    let start = Instant::now();
    let mut last_change = Instant::now();
    while start.elapsed() < limit && last_change.elapsed() < quiet {
        if session.pump() {
            last_change = Instant::now();
        } else {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    text_of(session)
}

fn text_of(session: &Session) -> String {
    let frame = session.frame();
    frame
        .rows
        .iter()
        .map(|row| {
            row.runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Llena el historial del scrollback escribiendo líneas por el PTY.
///
/// No se apoya en que el shell interprete nada: el eco de la línea de comandos
/// ya pinta cada línea en la rejilla, así que la cantidad de historial no
/// depende del shell ni de su tiempo de arranque.
fn fill_history(session: &mut Session, lines: usize) {
    let mut bytes = String::new();
    for index in 0..lines {
        bytes.push_str(&format!("historial-{index}\r\n"));
    }
    session.write(bytes.as_bytes()).expect("escribir historial");
    let _ = pump_until_quiet(session, Duration::from_millis(200), Duration::from_secs(10));
}

/// Espera a que el programa haya pedido sus modos por el terminal.
///
/// `pump` ya entregó a la rejilla todo lo que el programa escribió, así que
/// cuando la salida lleva 200 ms callada los DECSET están aplicados.
fn wait_for_modes(session: &mut Session) {
    let _ = pump_until_quiet(session, Duration::from_millis(200), Duration::from_secs(10));
}

/// El entorno que pide un plugin se suma al config por defecto sin cambiarlo.
///
/// Es la garantía de que un plugin describe el entorno de sus paneles pero no
/// decide qué shell corre: que la salida del `printf` aparezca prueba las dos
/// cosas a la vez, porque ese comando y sus argumentos son los del config por
/// defecto y la variable es la que sumó el plugin.
#[test]
fn el_entorno_del_plugin_se_suma_sin_cambiar_el_shell() {
    let mut config = plain_shell();
    config.args = vec![
        "-c".to_string(),
        "printf plugin:%s \"$PORT_ENV_PLUGIN\"".to_string(),
    ];
    let mut manager = SessionManager::new(config, GridSize::new(80, 24)).expect("gestor");

    manager
        .spawn_session_with_env(vec![("PORT_ENV_PLUGIN".to_string(), "si".to_string())])
        .expect("sesión con entorno del plugin");

    let texto = pump_until_quiet(
        manager.active_session_mut(),
        Duration::from_millis(200),
        Duration::from_secs(10),
    );
    assert!(
        texto.contains("plugin:si"),
        "el entorno del plugin no llegó al panel: {texto}"
    );
}

/// El entorno extra del panel llega al proceso que corre dentro.
///
/// Es la capacidad por la que un plugin describe el entorno de sus paneles sin
/// que el núcleo tenga que conocer a ningún programa en particular.
#[test]
fn el_entorno_extra_llega_al_proceso() {
    let mut config = plain_shell();
    config.args = vec![
        "-c".to_string(),
        "printf visto:%s \"$PORT_ENV_DE_PRUEBA\"".to_string(),
    ];
    config.env = vec![("PORT_ENV_DE_PRUEBA".to_string(), "si".to_string())];

    let mut session = Session::spawn(config, GridSize::new(80, 24)).expect("sesión");
    let texto = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(10),
    );

    assert!(
        texto.contains("visto:si"),
        "el entorno no llegó al proceso: {texto}"
    );
}
