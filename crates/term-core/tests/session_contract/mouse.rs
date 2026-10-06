//! Reportes de ratón y política: reenvío al programa frente al scrollback local.

use std::time::Duration;

use port_term_core::frame::Rgb;
use port_term_core::pty::PtyConfig;
use port_term_core::session::{CellPos, GridSize, MousePolicy, Session};

use crate::{fill_history, plain_shell, pump_until_quiet, text_of, wait_for_modes};

/// Programa que pide reportes de ratón por el terminal y, si se pide, entra
/// en pantalla alternativa: es el escenario de pi (1000 + SGR 1006 dentro de
/// la pantalla alterna).
///
/// `stty raw -echo` deja la entrada en crudo, para que los reportes lleguen al
/// programa sin esperar un salto de línea; `cat -v` devuelve a la pantalla todo
/// lo que recibe, escapando el ESC como `^[`, y eso es lo que hace observable
/// el reporte que PORT escribe al PTY.
fn mouse_reporting_shell(alternate_screen: bool) -> PtyConfig {
    let mut modos = String::from("\\033[?1000h\\033[?1006h");
    if alternate_screen {
        modos.push_str("\\033[?1049h");
    }
    PtyConfig {
        command: "/bin/sh".to_string(),
        args: vec![
            "-c".to_string(),
            format!("stty raw -echo; printf '{modos}'; cat -v"),
        ],
        cwd: None,
    }
}

/// Con reportes de ratón activos, la rueda se reenvía al programa como reporte
/// SGR con la celda bajo el puntero, en vez de mover el scrollback local.
#[test]
fn la_rueda_se_reenvia_al_programa_cuando_hay_reportes_de_raton() {
    let mut session =
        Session::spawn(mouse_reporting_shell(false), GridSize::new(60, 6)).expect("sesión");
    wait_for_modes(&mut session);

    let celda = CellPos::new(5, 3);

    // Rueda hacia arriba: código 64.
    assert!(
        session.wheel(1, celda, false),
        "la rueda con reportes activos debe consumirse"
    );
    let text = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(10),
    );
    assert!(
        text.contains("^[[<64;5;3M"),
        "falta el reporte de rueda arriba con la celda bajo el puntero:\n{text}"
    );
    assert_eq!(
        session.frame().display_offset,
        0,
        "reenviar la rueda no debe mover el scrollback local"
    );

    // Rueda hacia abajo: código 65.
    assert!(session.wheel(-1, celda, false));
    let text = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(10),
    );
    assert!(
        text.contains("^[[<65;5;3M"),
        "falta el reporte de rueda abajo:\n{text}"
    );
}

/// Shift es la salida de emergencia estándar: aunque el programa pida reportes
/// de ratón, con Shift la rueda mueve el scrollback local y no se le envía nada.
#[test]
fn shift_mas_rueda_desplaza_el_historial_aunque_haya_reportes() {
    let mut session =
        Session::spawn(mouse_reporting_shell(false), GridSize::new(60, 6)).expect("sesión");
    wait_for_modes(&mut session);
    fill_history(&mut session, 40);

    assert!(session.wheel(3, CellPos::new(5, 3), true));
    assert_eq!(
        session.frame().display_offset,
        3,
        "con Shift la rueda debe desplazar el historial local"
    );
    assert!(
        !text_of(&session).contains("^[[<"),
        "con Shift no debe salir ningún reporte al programa:\n{}",
        text_of(&session)
    );
}

/// Sin reportes de ratón activos la rueda se comporta como siempre: mueve el
/// scrollback local del núcleo.
#[test]
fn sin_reportes_de_raton_la_rueda_desplaza_el_historial() {
    let mut session = Session::spawn(plain_shell(), GridSize::new(60, 6)).expect("sesión");
    fill_history(&mut session, 40);

    assert!(session.wheel(3, CellPos::new(5, 3), false));
    assert_eq!(
        session.frame().display_offset,
        3,
        "sin reportes la rueda debe desplazar el historial local"
    );

    // Un evento sin desplazamiento no consume nada ni mueve la vista.
    assert!(!session.wheel(0, CellPos::new(5, 3), false));
    assert_eq!(
        session.frame().display_offset,
        3,
        "una rueda nula no debe mover la vista"
    );
}

/// El caso real de pi: pantalla alterna con reportes de ratón activos. La
/// pantalla alterna no aporta nada aquí porque el reporte se reenvía al
/// programa; el scrollback local no se toca.
#[test]
fn la_pantalla_alterna_reenvia_la_rueda_si_el_programa_pide_reportes() {
    let mut session =
        Session::spawn(mouse_reporting_shell(true), GridSize::new(60, 6)).expect("sesión");
    wait_for_modes(&mut session);

    assert!(session.wheel(1, CellPos::new(5, 3), false));
    let text = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(10),
    );
    assert!(
        text.contains("^[[<64;5;3M"),
        "en pantalla alterna con reportes la rueda también se reenvía:\n{text}"
    );
}

/// `spawn` arranca con la política por defecto y `set_mouse_policy` conserva lo
/// asignado: el núcleo solo guarda y devuelve la política, la UI la lee.
#[test]
fn la_politica_de_raton_se_guarda_y_se_devuelve() {
    let mut session = Session::spawn(plain_shell(), GridSize::new(40, 8)).expect("sesión");
    assert_eq!(session.mouse_policy(), MousePolicy::default());

    let policy = MousePolicy {
        forward_clicks: false,
        copy_on_select: false,
        highlight: Rgb::new(1, 2, 3),
        highlight_opacity: 0.1,
        ..MousePolicy::default()
    };
    session.set_mouse_policy(policy);
    assert_eq!(session.mouse_policy(), policy);
}
