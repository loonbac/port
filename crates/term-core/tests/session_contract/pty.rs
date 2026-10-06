//! PTY y comportamiento del shell: arranque, salida, resize y señales.

use std::time::{Duration, Instant};

use port_term_core::input::KeyMode;
use port_term_core::pty::PtyConfig;
use port_term_core::session::{GridSize, Session, SessionManager};

use crate::{plain_shell, pump_until_quiet};

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

/// Una sesión creada con un programa propio ejecuta ese programa, escribe su
/// salida en la rejilla y queda activa; no arranca el shell por defecto.
#[test]
fn spawn_session_with_runs_the_given_program_and_activates_it() {
    let mut manager =
        SessionManager::new(plain_shell(), GridSize::new(60, 10)).expect("crear manager");
    assert_eq!(manager.active_index(), 0);

    let program = which("echo").expect("echo debe estar en el PATH");
    let config = PtyConfig {
        command: program,
        args: vec!["visor-de-port".to_string()],
        cwd: None,
    };
    let id = manager
        .spawn_session_with(config)
        .expect("crear sesión con programa propio");

    assert_eq!(id, 1, "el identificador es el siguiente libre");
    assert_eq!(manager.len(), 2);
    assert_eq!(
        manager.active_index(),
        id,
        "la sesión recién creada debe quedar activa"
    );

    let start = Instant::now();
    let mut text = String::new();
    while start.elapsed() < Duration::from_secs(10) {
        manager.pump();
        text = manager
            .frame()
            .rows
            .iter()
            .map(|r| r.runs.iter().map(|x| x.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        if text.contains("visor-de-port") {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        text.contains("visor-de-port"),
        "el programa indicado debe escribir en la rejilla, no el shell por defecto:\n{text}"
    );

    // La sesión inicial sigue siendo el shell silencioso: la salida del visor
    // no puede haberse colado en ella.
    assert!(manager.select(0));
    manager.pump();
    let first = manager
        .frame()
        .rows
        .iter()
        .map(|r| r.runs.iter().map(|x| x.text.as_str()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !first.contains("visor-de-port"),
        "la sesión 0 debe seguir con su shell por defecto:\n{first}"
    );
}
