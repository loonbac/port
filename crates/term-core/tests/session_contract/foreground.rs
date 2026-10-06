//! Detección de la aplicación en primer plano (solo Linux: `/proc` y `tcgetpgrp`).

use std::time::Duration;

use port_term_core::pty::PtyConfig;
use port_term_core::session::{GridSize, Session, SessionManager};

use crate::plain_shell;

/// La deteccion de que programa esta en primer plano se apoya en `/proc` y en
/// `tcgetpgrp`: solo existe en Linux.
///
/// Sin esta anotacion, en macOS los tests que esperan "no hay app en primer
/// plano" pasarian sin comprobar nada, porque la deteccion nunca encuentra
/// nada ahi. Un test que pasa por la razon equivocada es peor que un test
/// ausente: da confianza que no esta ganada.
#[cfg(target_os = "linux")]
#[test]
fn idle_terminal_reports_no_foreground_app() {
    let mut session = Session::spawn(plain_shell(), GridSize::new(60, 10)).expect("sesión");
    std::thread::sleep(Duration::from_millis(300));

    // Un shell en reposo no tiene ningún hijo que no sea el propio shell,
    // así que no debe reportarse ninguna app en primer plano.
    for _ in 0..10 {
        session.pump();
        assert!(
            session.foreground_app().is_none(),
            "una terminal en reposo no debe mostrar ninguna app en las pestañas"
        );
    }
}

/// La deteccion de que programa esta en primer plano se apoya en `/proc` y en
/// `tcgetpgrp`: solo existe en Linux.
///
/// Sin esta anotacion, en macOS los tests que esperan "no hay app en primer
/// plano" pasarian sin comprobar nada, porque la deteccion nunca encuentra
/// nada ahi. Un test que pasa por la razon equivocada es peor que un test
/// ausente: da confianza que no esta ganada.
#[cfg(target_os = "linux")]
#[test]
fn a_running_program_becomes_the_foreground_app_after_two_samples() {
    let mut session = Session::spawn(plain_shell(), GridSize::new(60, 10)).expect("sesión");
    std::thread::sleep(Duration::from_millis(300));

    // Se lanza un proceso de larga duración como "app" en primer plano.
    session
        .write(b"sleep 30\n")
        .expect("lanzar proceso en primer plano");
    std::thread::sleep(Duration::from_millis(250));

    let mut seen = false;
    for _ in 0..20 {
        session.pump();
        if let Some(app) = session.foreground_app() {
            assert_eq!(app.bin, "sleep", "se esperaba el binario 'sleep'");
            seen = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    assert!(seen, "un proceso en primer plano debe detectarse como app");
}

/// La deteccion de que programa esta en primer plano se apoya en `/proc` y en
/// `tcgetpgrp`: solo existe en Linux.
///
/// Sin esta anotacion, en macOS los tests que esperan "no hay app en primer
/// plano" pasarian sin comprobar nada, porque la deteccion nunca encuentra
/// nada ahi. Un test que pasa por la razon equivocada es peor que un test
/// ausente: da confianza que no esta ganada.
#[cfg(target_os = "linux")]
#[test]
fn a_background_job_is_not_reported_as_foreground_app() {
    // Un shell con job control (`sh -i`) es necesario para que `&` cree un
    // grupo de procesos distinto al del primer plano, como haría fish o bash.
    let config = PtyConfig {
        command: "/bin/sh".to_string(),
        args: vec!["-i".to_string()],
        cwd: None,
    };
    let mut session = Session::spawn(config, GridSize::new(60, 10)).expect("sesión");
    std::thread::sleep(Duration::from_millis(400));

    // Se lanza un job en segundo plano: sigue vivo, pero no está en primer plano.
    session
        .write(b"sleep 25 &\n")
        .expect("lanzar job en background");
    std::thread::sleep(Duration::from_millis(500));

    for _ in 0..15 {
        session.pump();
        let app = session.foreground_app();
        assert!(
            app.is_none(),
            "un job lanzado con `&` no debe aparecer como app en primer plano, se obtuvo: {app:?}"
        );
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// La deteccion de que programa esta en primer plano se apoya en `/proc` y en
/// `tcgetpgrp`: solo existe en Linux.
///
/// Sin esta anotacion, en macOS los tests que esperan "no hay app en primer
/// plano" pasarian sin comprobar nada, porque la deteccion nunca encuentra
/// nada ahi. Un test que pasa por la razon equivocada es peor que un test
/// ausente: da confianza que no esta ganada.
#[cfg(target_os = "linux")]
#[test]
fn a_finished_program_stops_being_reported_as_foreground_app() {
    let mut session = Session::spawn(plain_shell(), GridSize::new(60, 10)).expect("sesión");
    std::thread::sleep(Duration::from_millis(300));

    // Programa corto: se lanza y termina casi inmediatamente.
    session.write(b"true\n").expect("lanzar job corto");
    std::thread::sleep(Duration::from_millis(400));

    // Tras estabilizarse en "ninguna app", debe seguir así aunque el valor
    // anterior fuera un programa: no puede quedar congelado en la caché.
    let mut cleared = false;
    for _ in 0..40 {
        session.pump();
        if session.foreground_app().is_none() {
            cleared = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    assert!(
        cleared,
        "un programa que ya terminó no debe seguir apareciendo en las pestañas"
    );

    // Y debe seguir limpio durante más sondeos (confirmación simétrica).
    for _ in 0..20 {
        session.pump();
        assert!(
            session.foreground_app().is_none(),
            "la terminal en reposo debe seguir sin app en primer plano"
        );
    }
}

/// La deteccion de que programa esta en primer plano se apoya en `/proc` y en
/// `tcgetpgrp`: solo existe en Linux.
///
/// Sin esta anotacion, en macOS los tests que esperan "no hay app en primer
/// plano" pasarian sin comprobar nada, porque la deteccion nunca encuentra
/// nada ahi. Un test que pasa por la razon equivocada es peor que un test
/// ausente: da confianza que no esta ganada.
#[cfg(target_os = "linux")]
#[test]
fn session_manager_reports_running_apps_across_all_sessions() {
    let mut manager =
        SessionManager::new(plain_shell(), GridSize::new(60, 10)).expect("crear manager");

    // Sin nada corriendo, no hay programas en ninguna sesión.
    manager.pump();
    assert!(
        manager.running_apps().is_empty(),
        "una terminal recién abierta no debe reportar programas"
    );
    assert!(!manager.has_running_app());

    // Se lanza un job en primer plano en la primera sesión.
    manager.write(b"sleep 20\n").expect("lanzar job");
    std::thread::sleep(Duration::from_millis(300));
    manager.pump();

    let mut detected = false;
    for _ in 0..20 {
        if manager.has_running_app() {
            detected = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
        manager.pump();
    }
    assert!(detected, "un job en primer plano debe detectarse");

    // El reporte incluye el identificador de sesión, para poder cerrar el correcto.
    let running = manager.running_apps();
    assert!(
        running
            .iter()
            .any(|(id, app)| *id == 0 && app.bin == "sleep"),
        "debe reportarse la sesión 0 ejecutando 'sleep', se obtuvo: {running:?}"
    );
}
