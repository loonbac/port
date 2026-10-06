//! Estado de la rejilla: scrollback, viewport y pantalla alterna.

use std::time::Duration;

use port_term_core::session::{CellPos, GridSize, Session};

use crate::{fill_history, plain_shell, pump_until_quiet, text_of};

/// La rueda mueve el viewport sobre el scrollback: hacia arriba muestra las
/// líneas viejas (el desplazamiento crece) y hacia abajo vuelve hacia el final.
///
/// Fija la convención de signo que usa el helper puro de la vista: un valor
/// positivo de `scroll()` aleja la vista del final.
#[test]
fn scrolling_grows_the_offset_and_going_down_returns_to_the_bottom() {
    let mut session = Session::spawn(plain_shell(), GridSize::new(60, 6)).expect("sesión");
    fill_history(&mut session, 40);

    assert_eq!(
        session.frame().display_offset,
        0,
        "la vista arranca pegada al final"
    );

    // Rueda hacia arriba: se ven líneas más viejas y el desplazamiento crece.
    session.scroll(3);
    assert_eq!(
        session.frame().display_offset,
        3,
        "hacia arriba el desplazamiento debe crecer"
    );

    // Rueda hacia abajo: vuelve hacia el final sin pasarse.
    session.scroll(-2);
    assert_eq!(
        session.frame().display_offset,
        1,
        "hacia abajo el desplazamiento debe encogerse"
    );

    // Un desplazamiento mayor que el historial se recorta en el final.
    session.scroll(-100);
    assert_eq!(
        session.frame().display_offset,
        0,
        "no se puede bajar por debajo del final"
    );
}

/// La salida nueva conserva la vista desplazada: un programa que escribe sin
/// parar no puede arrastrar el viewport al final en cada cuadro, o el scroll
/// hacia arriba quedaría anulado en el mismo fotograma en que se pide.
///
/// Es el contrato de kitty y alacritty: leer historial es estable, y volver al
/// presente es una decisión del usuario (desplazarse a mano).
///
/// No se afirma un número exacto de `display_offset` porque alacritty 0.26
/// reancla el desplazamiento al empujar líneas al historial
/// (`Grid::scroll_up` en `src/grid/mod.rs` lo incrementa cuando la vista no
/// está pegada al final). El contrato observable es la región visible: las
/// mismas líneas siguen en pantalla, y el desplazamiento nunca cae a cero.
#[test]
fn la_salida_nueva_conserva_la_vista_en_el_historial() {
    let mut session = Session::spawn(plain_shell(), GridSize::new(60, 6)).expect("sesión");
    fill_history(&mut session, 40);

    session.scroll(5);
    assert_eq!(
        session.frame().display_offset,
        5,
        "la vista debe quedarse arriba tras desplazarse"
    );
    let vista_desplazada = text_of(&session);

    // El shell imprime algo nuevo: cae al final del historial, pero la vista
    // sigue mostrando exactamente el mismo pasado.
    session
        .write(b"echo linea-nueva\n")
        .expect("escribir en el shell");
    let _ = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(10),
    );
    assert!(
        session.frame().display_offset > 0,
        "la salida nueva no debe arrastrar la vista al final"
    );
    assert_eq!(
        text_of(&session),
        vista_desplazada,
        "la vista desplazada debe conservar la misma región visible"
    );

    // Bajar a mano sí vuelve al presente, y ahí aparece lo que se escribió.
    session.scroll(-100);
    assert_eq!(
        session.frame().display_offset,
        0,
        "desplazarse hacia abajo debe devolver la vista al final"
    );
    let text = text_of(&session);
    assert!(
        text.contains("linea-nueva"),
        "la salida nueva debe estar al final del historial:\n{text}"
    );
}

/// En pantalla alterna sin reportes de ratón no hay historial que desplazar:
/// alacritty no guarda scrollback en la pantalla alterna, así que el
/// desplazamiento local queda en cero. Es el comportamiento aceptado.
#[test]
fn en_pantalla_alterna_sin_reportes_no_hay_historial_que_desplazar() {
    let mut session = Session::spawn(plain_shell(), GridSize::new(60, 6)).expect("sesión");
    fill_history(&mut session, 40);

    // En la pantalla primaria sí hay historial: el desplazamiento crece.
    assert!(session.wheel(3, CellPos::new(5, 3), false));
    assert_eq!(
        session.frame().display_offset,
        3,
        "la primaria sí tiene historial"
    );
    session.scroll(-100);

    // El programa entra en la pantalla alterna, que no tiene scrollback.
    session
        .write(b"printf '\\033[?1049h'\n")
        .expect("entrar en pantalla alterna");
    let _ = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(10),
    );

    assert!(session.wheel(3, CellPos::new(5, 3), false));
    assert_eq!(
        session.frame().display_offset,
        0,
        "la pantalla alterna no tiene historial local que desplazar"
    );
}
