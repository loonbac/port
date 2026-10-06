//! Gestor de sesiones: aislamiento entre espacios y recolección al salir.

use std::time::Duration;

use port_term_core::session::{GridSize, SessionManager};

use crate::plain_shell;

#[test]
fn session_manager_manages_multiple_isolated_spaces() {
    let mut manager =
        SessionManager::new(plain_shell(), GridSize::new(60, 10)).expect("crear manager");
    assert_eq!(manager.len(), 1);
    assert_eq!(manager.active_index(), 0);

    // Escribimos en la sesión 0
    manager
        .write(b"echo SESION_CERO\n")
        .expect("escribir en sesion 0");
    std::thread::sleep(Duration::from_millis(150));
    manager.pump();
    let frame0_text = manager
        .frame()
        .rows
        .iter()
        .map(|r| r.runs.iter().map(|x| x.text.as_str()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(frame0_text.contains("SESION_CERO"));

    // Creamos la sesión 1 (como hace Ctrl+Alt+T al abrir un nuevo espacio)
    let new_idx = manager.spawn_session().expect("crear nueva sesion");
    assert_eq!(new_idx, 1);
    assert_eq!(manager.len(), 2);
    assert_eq!(manager.active_index(), 1);

    // Escribimos en la sesión 1
    manager
        .write(b"echo SESION_UNO\n")
        .expect("escribir en sesion 1");
    std::thread::sleep(Duration::from_millis(150));
    manager.pump();
    let frame1_text = manager
        .frame()
        .rows
        .iter()
        .map(|r| r.runs.iter().map(|x| x.text.as_str()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(frame1_text.contains("SESION_UNO"));
    assert!(
        !frame1_text.contains("SESION_CERO"),
        "la sesión 1 debe estar completamente aislada de la 0"
    );

    // Conmutamos de vuelta a la sesión 0 (como al hacer clic en space-1)
    assert!(manager.select(0));
    assert_eq!(manager.active_index(), 0);
    let back0_text = manager
        .frame()
        .rows
        .iter()
        .map(|r| r.runs.iter().map(|x| x.text.as_str()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        back0_text.contains("SESION_CERO"),
        "la sesión 0 debe conservar intacto su contenido previo"
    );
    assert!(!back0_text.contains("SESION_UNO"));

    // Creamos una tercera sesión (sesión 2)
    let s2 = manager.spawn_session().expect("crear sesion 2");
    assert_eq!(s2, 2);
    assert_eq!(manager.len(), 3);

    // Cerramos la sesión 1 (del medio) y verificamos que la sesión 2 conserva su ID estable
    assert!(manager.close(1));
    assert_eq!(manager.len(), 2);
    assert!(manager.select(2));
    assert_eq!(manager.active_id(), 2);
}

#[test]
fn a_shell_that_exits_is_reaped_and_the_manager_empties() {
    let mut manager =
        SessionManager::new(plain_shell(), GridSize::new(60, 10)).expect("crear manager");
    assert_eq!(manager.len(), 1);

    // Un shell que ejecuta `exit` termina por su cuenta.
    manager.write(b"exit\n").expect("salir del shell");

    let mut reaped = Vec::new();
    for _ in 0..60 {
        manager.pump();
        let exited = manager.reap_exited();
        if !exited.is_empty() {
            reaped = exited;
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    assert_eq!(reaped, vec![0], "la sesión 0 debió ser retirada al salir");
    assert!(
        manager.is_empty(),
        "sin sesiones vivas la terminal debe quedar vacía, no colgada"
    );
}

#[test]
fn reaping_keeps_the_other_sessions_alive() {
    let mut manager =
        SessionManager::new(plain_shell(), GridSize::new(60, 10)).expect("crear manager");
    let second = manager.spawn_session().expect("crear segunda sesion");
    assert_eq!(manager.len(), 2);

    // `exit` siempre va a la sesión activa, así que hay que seleccionar la
    // primera explícitamente para cerrar esa y no la otra.
    assert!(manager.select(0));
    manager.write(b"exit\n").expect("salir de la sesion 0");

    let mut reaped = Vec::new();
    for _ in 0..60 {
        manager.pump();
        let exited = manager.reap_exited();
        if !exited.is_empty() {
            reaped = exited;
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    assert_eq!(reaped, vec![0]);
    assert_eq!(manager.len(), 1, "la otra sesión debe sobrevivir");
    assert!(
        manager.active_id() == second,
        "el foco debe pasar a la sesión superviviente"
    );
}
