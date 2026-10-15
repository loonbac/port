//! Soltar archivos sobre la terminal: la ubicación llega al programa escapada.

use std::path::PathBuf;
use std::time::Duration;

use port_term_core::session::{GridSize, Session};

use crate::paste::echo_shell;
use crate::{pump_until_quiet, wait_for_modes};

/// Al soltar elementos sobre la rejilla, PORT pega su ubicación al programa con
/// el escapado del shell: una ruta con espacios llega como un solo argumento y
/// una ruta simple llega sin comillas añadidas.
#[test]
fn la_ubicacion_soltada_llega_al_programa_escapada() {
    let mut session = Session::spawn(echo_shell(false), GridSize::new(90, 6)).expect("sesión");
    wait_for_modes(&mut session);

    session.paste_dropped_paths(&[
        PathBuf::from("/tmp/port/notas.md"),
        PathBuf::from("/tmp/port/foto 1.png"),
    ]);

    let text = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(10),
    );
    assert!(
        text.contains("/tmp/port/notas.md '/tmp/port/foto 1.png'"),
        "las rutas soltadas deben llegar escapadas al programa:\n{text}"
    );
}
