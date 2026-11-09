//! Imágenes en la rejilla: sixel y kitty graphics, de punta a punta.
//!
//! Sin ventana: se escribe la secuencia en el PTY de un programa que copia su
//! entrada a la salida, y se comprueba lo que el cuadro pone a disposición de la
//! UI. El decodificado a GPU es de la UI y se verifica allí, no aquí.

use std::time::Duration;

use port_term_core::graphics::store::Pixels;
use port_term_core::pty::PtyConfig;
use port_term_core::session::{GridSize, Session};

use crate::{pump_until_quiet, text_of};

/// Programa que devuelve tal cual lo que reciba, sin eco ni modo canónico.
///
/// El modo canónico exigiría un salto de línea para entregar nada, y el eco
/// duplicaría las respuestas que la terminal escribe en el PTY.
fn raw_cat(extra: &str) -> PtyConfig {
    PtyConfig {
        command: "/bin/sh".to_string(),
        args: vec![
            "-c".to_string(),
            format!("stty -icanon -echo 2>/dev/null; {extra}cat"),
        ],
        cwd: None,
        env: Vec::new(),
    }
}

fn started(extra: &str) -> Session {
    let mut session = Session::spawn(raw_cat(extra), GridSize::new(40, 10)).expect("sesión");
    std::thread::sleep(Duration::from_millis(300));
    session.pump();
    session
}

/// Sixel de 3x6 píxeles: una columna por cada carácter, banda de seis filas.
const SIXEL: &[u8] = b"\x1bPq#0;2;100;0;0#0@A_\x1b\\";

/// PNG RGBA válido de 3x2 píxeles, en base64: lo que un programa real transmite.
const PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAMAAAACCAYAAACddGYaAAAAG0lEQVR42mP4z8DwHwwZ/gNJIGhoaAByGP4DAKv4DXUG8nAyAAAAAElFTkSuQmCC";

/// Parte el base64 en trozos de `size` caracteres.
///
/// El tamaño es múltiplo de cuatro: un programa real no parte los grupos de
/// base64 porque cada trozo se decodifica por separado.
fn base64_chunks(size: usize) -> Vec<&'static str> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < PNG_BASE64.len() {
        let end = (start + size).min(PNG_BASE64.len());
        out.push(&PNG_BASE64[start..end]);
        start = end;
    }
    out
}

#[test]
fn sixel_output_appears_in_the_frame() {
    let mut session = started("");
    session.write(SIXEL).expect("escribir el sixel");
    let _ = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(5),
    );

    let frame = session.frame();
    assert_eq!(frame.images.len(), 1, "el sixel debía pintarse");
    let image = &frame.images[0];
    assert_eq!(image.row, 0, "se coloca en la fila del cursor");
    assert_eq!(image.column, 0);
    // Celda de arranque 8x18: 3 px de ancho es una columna, 6 px de alto una fila.
    assert_eq!(image.columns, 1);
    assert_eq!(image.rows, 1);
    match image.pixels.as_ref() {
        Pixels::Rgba {
            width,
            height,
            data,
        } => {
            assert_eq!((*width, *height), (3, 6));
            assert_eq!(&data[0..4], [255, 0, 0, 255], "primer píxel rojo");
        }
        other => panic!("se esperaban píxeles RGBA, llegó {other:?}"),
    }
    assert!(
        text_of(&session).trim().is_empty(),
        "la secuencia no debe dejar texto en la rejilla"
    );
}

#[test]
fn kitty_transmit_and_display_appears_in_the_frame() {
    let mut session = started("");
    // 1x1 RGB rojo en base64: /wAA
    session
        .write(b"\x1b_Ga=T,f=24,s=1,v=1,i=7;/wAA\x1b\\")
        .expect("escribir el comando");
    let _ = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(5),
    );

    let frame = session.frame();
    assert_eq!(frame.images.len(), 1, "la imagen debía quedar colocada");
    let image = &frame.images[0];
    assert_eq!(image.image, 7);
    assert_eq!(image.row, 0);
    match image.pixels.as_ref() {
        Pixels::Rgba {
            width,
            height,
            data,
        } => {
            assert_eq!((*width, *height), (1, 1));
            assert_eq!(data, &[255, 0, 0, 255], "el píxel rojo llega como RGBA");
        }
        other => panic!("se esperaban píxeles RGBA, llegó {other:?}"),
    }
}

#[test]
fn a_chunked_kitty_transmission_paints_the_whole_image() {
    // Un programa parte la imagen en trozos: solo el primer APC lleva las
    // claves de control (acción, formato, identificador, colocación y `m=1`);
    // los siguientes traen únicamente `m` y los datos. El terminal tiene que
    // reensamblar los trozos y decodificar el contenido completo con el formato
    // del primero: si cada trozo se toma como una transmisión suelta, el último
    // decodifica la cola como píxeles crudos y no pinta nada.
    let mut whole = started("");
    whole
        .write(format!("\x1b_Ga=T,f=100,q=2,i=1;{PNG_BASE64}\x1b\\").as_bytes())
        .expect("escribir la imagen entera");
    let _ = pump_until_quiet(
        &mut whole,
        Duration::from_millis(200),
        Duration::from_secs(5),
    );
    let expected = whole.frame().images.clone();
    assert_eq!(expected.len(), 1, "la imagen entera debía pintarse");

    let chunks = base64_chunks(40);
    assert_eq!(chunks.len(), 3, "el PNG debe viajar en tres trozos");
    let mut split = Vec::new();
    for (index, chunk) in chunks.iter().enumerate() {
        let more = index + 1 < chunks.len();
        let control = if index == 0 {
            format!("a=T,f=100,q=2,i=1,C=1,m={}", u8::from(more))
        } else {
            format!("m={}", u8::from(more))
        };
        split.extend_from_slice(format!("\x1b_G{control};{chunk}\x1b\\").as_bytes());
    }

    let mut partida = started("");
    partida.write(&split).expect("escribir la imagen partida");
    let _ = pump_until_quiet(
        &mut partida,
        Duration::from_millis(200),
        Duration::from_secs(5),
    );

    let frame = partida.frame();
    assert_eq!(
        frame.images.len(),
        1,
        "los trozos debían reensamblarse en una sola imagen"
    );
    assert_eq!(
        frame.images[0].pixels.dimensions(),
        Some((3, 2)),
        "la cabecera PNG tiene que llegar entera"
    );
    assert_eq!(
        frame.images, expected,
        "la imagen partida tiene que ser la misma que la transmitida entera"
    );
}

#[test]
fn a_kitty_query_is_answered_on_the_pty() {
    // El programa pide la consulta y lee la respuesta: la terminal tiene que
    // escribirla en el PTY o este test se queda sin los bytes de OK.
    let mut session =
        started("printf '\\033_Ga=q,i=1\\033\\\\'; head -c 11 2>/dev/null | od -An -tx1; ");
    let _ = pump_until_quiet(
        &mut session,
        Duration::from_millis(300),
        Duration::from_secs(5),
    );

    // `od` parte la salida en columnas cuando el terminal es más estrecho, así
    // que se compara sin espacios ni saltos: 4f 4b es OK en hexadecimal.
    let compact: String = text_of(&session).split_whitespace().collect();
    assert!(
        compact.contains("4f4b"),
        "la respuesta no llegó al programa: {compact:?}"
    );
}

#[test]
fn deleting_an_image_removes_it_from_the_frame() {
    let mut session = started("");
    session
        .write(b"\x1b_Ga=T,f=24,s=1,v=1,i=9;/wAA\x1b\\")
        .expect("escribir el comando");
    let _ = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(5),
    );
    assert_eq!(session.frame().images.len(), 1);

    session
        .write(b"\x1b_Ga=d,d=i,i=9\x1b\\")
        .expect("escribir el borrado");
    let _ = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(5),
    );
    assert!(
        session.frame().images.is_empty(),
        "la imagen borrada no debe seguir en el cuadro"
    );
}

#[test]
fn an_image_scrolls_up_with_the_text() {
    let mut session = started("");
    // Cursor a la última fila, imagen ahí, y una línea más: se desplaza una fila.
    session
        .write(b"\n\n\n\n\n\n\n\n\n")
        .expect("bajar el cursor");
    let _ = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(5),
    );
    session.write(SIXEL).expect("escribir el sixel");
    let _ = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(5),
    );

    // La colocación baja el cursor una fila más, así que la pantalla se
    // desplaza y la imagen queda una fila por encima del borde inferior: es el
    // comportamiento de un terminal real con una imagen en la última línea.
    let before = session.frame().images.first().map(|image| image.row);
    assert_eq!(
        before,
        Some(8),
        "la imagen empieza en la última fila visible"
    );

    session.write(b"\n").expect("desplazar el texto");
    let _ = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(5),
    );

    let after = session.frame().images.first().map(|image| image.row);
    assert_eq!(after, Some(7), "la imagen sube con el texto");
}

#[test]
fn on_the_alternate_screen_the_image_stays_put() {
    let mut session = started("printf '\\033[?1049h'; ");
    let _ = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(5),
    );

    session.write(b"\n\n\n").expect("bajar el cursor");
    let _ = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(5),
    );
    session.write(SIXEL).expect("escribir el sixel");
    let _ = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(5),
    );

    let before = session.frame().images.first().map(|image| image.row);
    assert_eq!(before, Some(3));

    session.write(b"\n\n\n\n\n\n\n").expect("desplazar");
    let _ = pump_until_quiet(
        &mut session,
        Duration::from_millis(200),
        Duration::from_secs(5),
    );

    let after = session.frame().images.first().map(|image| image.row);
    assert_eq!(
        after, before,
        "en pantalla alterna la imagen no sigue al texto"
    );
}
