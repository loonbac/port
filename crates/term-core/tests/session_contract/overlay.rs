//! Capas de depuración por OSC 556, de punta a punta.
//!
//! Sin ventana: lo que se verifica aquí es el estado que la UI recibe en el
//! cuadro. Cómo se pinta es de la UI y se comprueba allí.

use std::time::Duration;

use port_term_core::overlay::Flags;
use port_term_core::pty::PtyConfig;
use port_term_core::session::{GridSize, Session};

use crate::{pump_until_quiet, text_of};

fn raw_cat() -> PtyConfig {
    PtyConfig {
        command: "/bin/sh".to_string(),
        args: vec![
            "-c".to_string(),
            "stty -icanon -echo 2>/dev/null; cat".to_string(),
        ],
        cwd: None,
        env: Vec::new(),
    }
}

fn started() -> Session {
    let mut session = Session::spawn(raw_cat(), GridSize::new(40, 10)).expect("sesión");
    std::thread::sleep(Duration::from_millis(300));
    session.pump();
    session
}

fn send(session: &mut Session, payload: &str) {
    let sequence = format!("\x1b]556;{payload}\x1b\\");
    session
        .write(sequence.as_bytes())
        .expect("escribir OSC 556");
    let _ = pump_until_quiet(session, Duration::from_millis(200), Duration::from_secs(5));
}

#[test]
fn the_sequence_turns_on_a_layer_and_never_reaches_the_grid() {
    let mut session = started();
    send(&mut session, "hud");

    let frame = session.frame();
    assert!(frame.debug.flags.hud, "el HUD debía quedar encendido");
    assert!(!frame.debug.flags.grid, "la rejilla no se pidió");
    assert!(
        !text_of(&session).contains("556"),
        "la secuencia no debe imprimirse: {:?}",
        text_of(&session)
    );
}

#[test]
fn off_turns_every_layer_off_again() {
    let mut session = started();
    send(&mut session, "all");
    assert_eq!(session.frame().debug.flags, Flags::ALL);

    send(&mut session, "off");
    assert_eq!(session.frame().debug.flags, Flags::NONE);
}

#[test]
fn the_hud_reports_measured_counters() {
    let mut session = started();
    // Un sixel deja una colocación viva, y el HUD debe contarla.
    session
        .write(b"\x1bPq#0;2;100;0;0#0@A_\x1b\\")
        .expect("escribir el sixel");
    let _ = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(5),
    );
    send(&mut session, "hud");

    let frame = session.frame();
    let stats = frame.debug.stats;
    assert_eq!(stats.columns, 40);
    assert_eq!(stats.rows, 10);
    assert_eq!(stats.images, 1, "la imagen colocada se cuenta");
    assert_eq!(stats.dropped_bytes, session.dropped_output_bytes());
    assert!(!stats.alternate, "la sesión está en la pantalla primaria");
}

#[test]
fn without_a_sequence_the_frame_carries_no_debug_state() {
    let mut session = started();
    session.write(b"hola\n").expect("escribir texto");
    let _ = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(5),
    );

    let frame = session.frame();
    assert_eq!(frame.debug.flags, Flags::NONE);
    assert_eq!(frame.debug.stats, Default::default());
}
