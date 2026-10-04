//! Contrato del caso de uso [`Session`]: shell real, PTY real, sin ventana.
//!
//! Es la verificación de integración del núcleo: si esto pasa, el shell responde
//! y la rejilla refleja lo que el shell escribe.

use std::time::{Duration, Instant};

use port_term_core::input::KeyMode;
use port_term_core::pty::PtyConfig;
use port_term_core::session::{GridSize, Session, SessionManager};

/// Arranca un shell no interactivo predecible en vez del del usuario.
fn plain_shell() -> PtyConfig {
    PtyConfig {
        command: "/bin/sh".to_string(),
        args: Vec::new(),
        cwd: None,
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

/// Busca un ejecutable en el PATH, como hace una shell.
fn which(bin: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(bin))
        .find(|candidate| candidate.is_file())
        .map(|candidate| candidate.to_string_lossy().to_string())
}

/// Regresión del cuelgue de 39 minutos: `pump` no puede bloquearse esperando
/// salida que no llega. Si vuelve a pasar, este test lo dice en un segundo.
#[test]
fn pump_returns_immediately_when_the_shell_is_silent() {
    let mut session = Session::spawn(plain_shell(), GridSize::new(40, 8)).expect("sesión");

    // Se deja que el shell arranque y consuma lo que tenga pendiente.
    std::thread::sleep(Duration::from_millis(300));
    for _ in 0..20 {
        session.pump();
    }

    // Ahora está callado: cien llamadas deben costar casi nada.
    let start = Instant::now();
    for _ in 0..100 {
        session.pump();
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(500),
        "pump bloqueó: 100 llamadas con el shell callado tardaron {elapsed:?}"
    );
}

#[test]
fn shell_output_reaches_the_grid() {
    let size = GridSize::new(60, 10);
    let mut session = Session::spawn(plain_shell(), size).expect("arrancar la sesión");

    session
        .write(b"echo hola-port\n")
        .expect("escribir en el shell");

    let text = pump_until_quiet(
        &mut session,
        Duration::from_millis(150),
        Duration::from_secs(10),
    );
    assert!(
        text.contains("hola-port"),
        "el eco del shell debe aparecer en la rejilla:\n{text}"
    );
}

#[test]
fn multiline_output_advances_rows() {
    let size = GridSize::new(40, 8);
    let mut session = Session::spawn(plain_shell(), size).expect("arrancar la sesión");

    session
        .write(b"echo uno; echo dos\n")
        .expect("escribir en el shell");

    let text = pump_until_quiet(
        &mut session,
        Duration::from_millis(150),
        Duration::from_secs(10),
    );
    assert!(text.contains("uno"), "falta la primera línea:\n{text}");
    assert!(text.contains("dos"), "falta la segunda línea:\n{text}");
    let uno = text.find("uno").unwrap();
    let dos = text.find("dos").unwrap();
    assert!(uno < dos, "\"dos\" debe quedar debajo de \"uno\":\n{text}");
}

#[test]
fn resize_is_applied_to_the_session() {
    let mut session = Session::spawn(plain_shell(), GridSize::new(40, 8)).expect("sesión");
    session
        .resize(GridSize::new(100, 30))
        .expect("redimensionar");

    assert_eq!(session.size(), GridSize::new(100, 30));
    assert_eq!(session.frame().columns, 100);
    session.write(b"echo sigue\n").expect("escribir");
    let text = pump_until_quiet(
        &mut session,
        Duration::from_millis(150),
        Duration::from_secs(10),
    );
    assert!(text.contains("sigue"), "el shell debe seguir vivo:\n{text}");
}

#[test]
fn ctrl_d_ends_the_shell() {
    let mut session = Session::spawn(plain_shell(), GridSize::new(40, 8)).expect("sesión");
    std::thread::sleep(Duration::from_millis(200));
    session.pump();

    // Ctrl+D (fin de entrada) cierra el sh no interactivo.
    session.write(&[0x04]).expect("escribir ctrl+d");

    let start = Instant::now();
    let mut exited = false;
    while start.elapsed() < Duration::from_secs(5) {
        session.pump();
        if session.has_exited() {
            exited = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(exited, "el shell debió terminar tras ctrl+d");
}

#[test]
fn cursor_key_mode_is_normal_until_the_program_enables_application_keys() {
    let mut session = Session::spawn(plain_shell(), GridSize::new(40, 8)).expect("sesión");
    assert_eq!(
        session.cursor_key_mode(),
        KeyMode::NORMAL,
        "por defecto las flechas van en modo normal"
    );

    session
        .write(b"printf '\\033[?1h'\n")
        .expect("activar modo aplicación");
    pump_until_quiet(
        &mut session,
        Duration::from_millis(150),
        Duration::from_secs(10),
    );

    assert_eq!(
        session.cursor_key_mode(),
        KeyMode::APPLICATION,
        "tras \\e[?1h las flechas deben ir en modo aplicación (SS3)"
    );
}

#[test]
fn fish_device_queries_are_replied_automatically_without_blocking() {
    // La ruta de fish esta cableada a NixOS, asi que el test no puede
    // correr en un runner de GitHub, que no tiene /run/current-system. Se
    // busca en el PATH y, si no esta, el test se salta en lugar de fallar
    // por una dependencia del entorno.
    let Some(fish) = std::env::var("SHELL")
        .ok()
        .filter(|shell| shell.ends_with("fish"))
        .or_else(|| which("fish"))
    else {
        eprintln!("fish no esta disponible: se omite el test");
        return;
    };

    let config = PtyConfig {
        command: fish,
        args: vec!["-l".to_string()],
        cwd: None,
    };
    let mut session = Session::spawn(config, GridSize::new(100, 30)).expect("sesión fish");
    let start = Instant::now();
    let mut interactive = false;
    while start.elapsed() < Duration::from_secs(4) {
        session.pump();
        let frame = session.frame();
        for row in &frame.rows {
            let text: String = row.runs.iter().map(|r| r.text.clone()).collect();
            if text.contains("➜") {
                interactive = true;
                break;
            }
        }
        if interactive {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    assert!(
        interactive,
        "fish debe responder con el prompt interactivo de inmediato sin esperar 10s"
    );
}

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
