//! Tests del contrato de [`port_term_core::graphics`].
//!
//! Verifica el escáner de secuencias crudas Sixel (DCS) y Kitty (APC), así como
//! la decodificación directa de píxeles Sixel en mapas RGBA.

use port_term_core::graphics::{GraphicsEvent, Scanner, MAX_PENDING_SEQUENCE};

#[test]
fn plain_bytes_pass_through_unchanged_and_in_order() {
    let mut scanner = Scanner::new();
    let scan = scanner.advance(b"hola");
    assert_eq!(scan.plain, b"hola");
    assert!(scan.events.is_empty());
    assert_eq!(scanner.pending_len(), 0);
}

#[test]
fn sixel_dcs_is_decoded_removed_from_plain_and_reported() {
    let mut scanner = Scanner::new();
    let fixture = b"pre\x1bPq#0;2;100;0;0~-~\x1b\\post";
    let scan = scanner.advance(fixture);

    assert_eq!(scan.plain, b"prepost");
    assert_eq!(scan.events.len(), 1);
    match &scan.events[0] {
        GraphicsEvent::Sixel(img) => {
            assert_eq!(img.width, 1);
            assert_eq!(img.height, 12);
            // Cada píxel pintado con el color #0 (rojo puro: [255, 0, 0, 255])
            for pixel in img.rgba.as_chunks::<4>().0 {
                assert_eq!(pixel, &[255, 0, 0, 255]);
            }
        }
        _ => panic!("se esperaba GraphicsEvent::Sixel"),
    }
    assert_eq!(scanner.pending_len(), 0);
}

#[test]
fn sixel_dcs_with_parameters_is_decoded() {
    let mut scanner = Scanner::new();
    let fixture = b"\x1bP0;1;8q#0;2;100;0;0~\x1b\\";
    let scan = scanner.advance(fixture);

    assert!(scan.plain.is_empty());
    assert_eq!(scan.events.len(), 1);
    match &scan.events[0] {
        GraphicsEvent::Sixel(img) => {
            assert_eq!(img.width, 1);
            assert_eq!(img.height, 6);
        }
        _ => panic!("se esperaba GraphicsEvent::Sixel"),
    }
}

#[test]
fn non_sixel_dcs_reaches_plain_byte_for_byte() {
    let mut scanner = Scanner::new();
    let fixture = b"\x1bP1$rhello\x1b\\";
    let scan = scanner.advance(fixture);

    assert_eq!(scan.plain, fixture);
    assert!(scan.events.is_empty());
    assert_eq!(scanner.pending_len(), 0);
}

/// `fish` consulta capacidades del terminal con XTGETTCAP, que es un DCS con
/// el parametro `+q`. No es un Sixel: sus parametros no son numeros, y tomarlo
/// por una imagen producia rectangulos negros donde el programa solo preguntaba.
#[test]
fn xtgettcap_dcs_is_not_decoded_as_sixel() {
    let mut scanner = Scanner::new();
    let fixture = b"\x1bP+q696e646e\x1b\\";

    let scan = scanner.advance(fixture);

    assert!(
        scan.events.is_empty(),
        "una consulta XTGETTCAP no puede producir una imagen"
    );
    assert_eq!(scan.plain, fixture);
    assert_eq!(scanner.pending_len(), 0);
}

#[test]
fn kitty_apc_reported_with_payload_including_leading_g_without_st() {
    let mut scanner = Scanner::new();
    let fixture = b"\x1b_Gi=1;payload\x1b\\";
    let scan = scanner.advance(fixture);

    assert!(scan.plain.is_empty());
    assert_eq!(scan.events.len(), 1);
    match &scan.events[0] {
        GraphicsEvent::Kitty(payload) => {
            assert_eq!(payload, b"Gi=1;payload");
        }
        _ => panic!("se esperaba GraphicsEvent::Kitty"),
    }
    assert_eq!(scanner.pending_len(), 0);
}

#[test]
fn non_kitty_apc_reaches_plain_byte_for_byte() {
    let mut scanner = Scanner::new();
    let fixture = b"\x1b_other_apc\x1b\\";
    let scan = scanner.advance(fixture);

    assert_eq!(scan.plain, fixture);
    assert!(scan.events.is_empty());
    assert_eq!(scanner.pending_len(), 0);
}

#[test]
fn sequence_aborted_by_can_or_sub_is_dropped_without_event() {
    let mut scanner = Scanner::new();

    // Abortado por CAN (0x18)
    let scan_can = scanner.advance(b"\x1bPq#0;2;100;0;0~\x18resto");
    assert_eq!(scan_can.plain, b"resto");
    assert!(scan_can.events.is_empty());
    assert_eq!(scanner.pending_len(), 0);

    // Abortado por SUB (0x1a)
    let scan_sub = scanner.advance(b"\x1b_Gi=1;payload\x1aresto");
    assert_eq!(scan_sub.plain, b"resto");
    assert!(scan_sub.events.is_empty());
    assert_eq!(scanner.pending_len(), 0);
}

#[test]
fn sequence_split_across_three_calls() {
    let mut scanner = Scanner::new();

    let scan1 = scanner.advance(b"abc\x1bP");
    assert_eq!(scan1.plain, b"abc");
    assert!(scan1.events.is_empty());
    assert_eq!(scanner.pending_len(), 2);

    let scan2 = scanner.advance(b"q#0;2;100;0;0~");
    assert!(scan2.plain.is_empty());
    assert!(scan2.events.is_empty());
    assert_eq!(scanner.pending_len(), 16);

    let scan3 = scanner.advance(b"-~\x1b\\xyz");
    assert_eq!(scan3.plain, b"xyz");
    assert_eq!(scan3.events.len(), 1);
    assert_eq!(scanner.pending_len(), 0);

    match &scan3.events[0] {
        GraphicsEvent::Sixel(img) => {
            assert_eq!(img.width, 1);
            assert_eq!(img.height, 12);
        }
        _ => panic!("se esperaba GraphicsEvent::Sixel"),
    }
}

#[test]
fn max_pending_sequence_exceeded_drops_pending_and_recovers() {
    let mut scanner = Scanner::new();

    // Inicia una secuencia y alimenta datos hasta sobrepasar el tope
    scanner.advance(b"\x1bPq");
    let huge = vec![b'~'; MAX_PENDING_SEQUENCE + 10];
    scanner.advance(&huge);

    assert_eq!(
        scanner.pending_len(),
        0,
        "el acumulador debe vaciarse al superar el tope"
    );

    // Tras el desborde, secuencias posteriores deben parsearse con normalidad
    let scan = scanner.advance(b"\x1bPq#0;2;100;0;0~\x1b\\");
    assert_eq!(scan.events.len(), 1);
    assert_eq!(scanner.pending_len(), 0);
}

#[test]
fn sixel_palette_definition_rgb_percentages_and_selection() {
    let mut scanner = Scanner::new();

    // Fixture con rojo (#0), verde (#1), azul (#2) y blanco clamp (#3)
    let fixture = b"\x1bPq\
#0;2;100;0;0#0~\
#1;2;0;100;0#1~\
#2;2;0;0;100#2~\
#3;2;150;200;120#3~\
\x1b\\";
    let scan = scanner.advance(fixture);
    assert_eq!(scan.events.len(), 1);

    match &scan.events[0] {
        GraphicsEvent::Sixel(img) => {
            assert_eq!(img.width, 4);
            assert_eq!(img.height, 6);

            // Columna 0: rojo puro
            for row in 0..6 {
                let idx = row * img.width * 4;
                assert_eq!(&img.rgba[idx..idx + 4], [255, 0, 0, 255]);
            }
            // Columna 1: verde puro
            for row in 0..6 {
                let idx = (row * img.width + 1) * 4;
                assert_eq!(&img.rgba[idx..idx + 4], [0, 255, 0, 255]);
            }
            // Columna 2: azul puro
            for row in 0..6 {
                let idx = (row * img.width + 2) * 4;
                assert_eq!(&img.rgba[idx..idx + 4], [0, 0, 255, 255]);
            }
            // Columna 3: blanco por clamp
            for row in 0..6 {
                let idx = (row * img.width + 3) * 4;
                assert_eq!(&img.rgba[idx..idx + 4], [255, 255, 255, 255]);
            }
        }
        _ => panic!("se esperaba GraphicsEvent::Sixel"),
    }
}

#[test]
fn sixel_default_vt340_palette_and_undefined_entry() {
    let mut scanner = Scanner::new();

    // Columna 0: paleta por defecto VT340 entrada 1 (azul: [51, 51, 204, 255])
    // Columna 1: paleta por defecto VT340 entrada 2 (rojo: [204, 33, 33, 255])
    // Columna 2: entrada no definida 99 (debe pintar negro: [0, 0, 0, 255])
    let fixture = b"\x1bPq#1~#2~#99~\x1b\\";
    let scan = scanner.advance(fixture);
    assert_eq!(scan.events.len(), 1);

    match &scan.events[0] {
        GraphicsEvent::Sixel(img) => {
            assert_eq!(img.width, 3);
            assert_eq!(img.height, 6);

            // Col 0: azul VT340
            let p0 = &img.rgba[0..4];
            assert_eq!(p0, [51, 51, 204, 255]);

            // Col 1: rojo VT340
            let p1 = &img.rgba[4..8];
            assert_eq!(p1, [204, 33, 33, 255]);

            // Col 2: negro por indefinido
            let p2 = &img.rgba[8..12];
            assert_eq!(p2, [0, 0, 0, 255]);
        }
        _ => panic!("se esperaba GraphicsEvent::Sixel"),
    }
}

#[test]
fn sixel_repeat_operator() {
    let mut scanner = Scanner::new();
    let fixture = b"\x1bPq#0;2;100;0;0#0!3~\x1b\\";
    let scan = scanner.advance(fixture);
    assert_eq!(scan.events.len(), 1);

    match &scan.events[0] {
        GraphicsEvent::Sixel(img) => {
            assert_eq!(img.width, 3);
            assert_eq!(img.height, 6);
            for pixel in img.rgba.as_chunks::<4>().0 {
                assert_eq!(pixel, &[255, 0, 0, 255]);
            }
        }
        _ => panic!("se esperaba GraphicsEvent::Sixel"),
    }
}

#[test]
fn sixel_newline_and_carriage_return() {
    let mut scanner = Scanner::new();
    // Banda 0: col 0 rojo. Regorno de carro ($) y salto de banda (-).
    // Banda 1: col 0 azul.
    let fixture = b"\x1bPq#0;2;100;0;0#0~$-#1;2;0;0;100#1~\x1b\\";
    let scan = scanner.advance(fixture);
    assert_eq!(scan.events.len(), 1);

    match &scan.events[0] {
        GraphicsEvent::Sixel(img) => {
            assert_eq!(img.width, 1);
            assert_eq!(img.height, 12);

            // Filas 0..6: rojo
            for row in 0..6 {
                let idx = row * 4;
                assert_eq!(&img.rgba[idx..idx + 4], [255, 0, 0, 255]);
            }
            // Filas 6..12: azul
            for row in 6..12 {
                let idx = row * 4;
                assert_eq!(&img.rgba[idx..idx + 4], [0, 0, 255, 255]);
            }
        }
        _ => panic!("se esperaba GraphicsEvent::Sixel"),
    }
}

#[test]
fn sixel_vertical_pixels_bit_mapping() {
    let mut scanner = Scanner::new();

    // Fixture con:
    // '@' (0x40 - 0x3f = 1, bit 0 activado)
    // 'A' (0x41 - 0x3f = 2, bit 1 activado)
    // '_' (0x5f - 0x3f = 32, bit 5 activado)
    let fixture = b"\x1bPq#0;2;100;0;0#0@A_\x1b\\";
    let scan = scanner.advance(fixture);
    assert_eq!(scan.events.len(), 1);

    match &scan.events[0] {
        GraphicsEvent::Sixel(img) => {
            assert_eq!(img.width, 3);
            assert_eq!(img.height, 6);

            // Columna 0 ('@'): fila 0 roja, filas 1..6 negras
            assert_eq!(&img.rgba[0..4], [255, 0, 0, 255]);
            for row in 1..6 {
                let idx = row * 3 * 4;
                assert_eq!(&img.rgba[idx..idx + 4], [0, 0, 0, 255]);
            }

            // Columna 1 ('A'): fila 1 roja, filas 0, 2..6 negras
            assert_eq!(&img.rgba[4 * 4..4 * 4 + 4], [255, 0, 0, 255]);
            for row in [0, 2, 3, 4, 5] {
                let idx = (row * 3 + 1) * 4;
                assert_eq!(&img.rgba[idx..idx + 4], [0, 0, 0, 255]);
            }

            // Columna 2 ('_'): fila 5 roja, filas 0..5 negras
            assert_eq!(&img.rgba[17 * 4..17 * 4 + 4], [255, 0, 0, 255]);
            for row in 0..5 {
                let idx = (row * 3 + 2) * 4;
                assert_eq!(&img.rgba[idx..idx + 4], [0, 0, 0, 255]);
            }
        }
        _ => panic!("se esperaba GraphicsEvent::Sixel"),
    }
}

#[test]
fn sixel_output_size_rules() {
    let mut scanner = Scanner::new();

    // Banda 1 no pinta ningún píxel (termina con '-' sin píxeles en la segunda banda):
    // El alto debe ser exactamente 6 (la última banda con al menos un píxel).
    let fixture1 = b"\x1bPq#0;2;100;0;0#0~-\x1b\\";
    let scan1 = scanner.advance(fixture1);
    match &scan1.events[0] {
        GraphicsEvent::Sixel(img) => {
            assert_eq!(img.width, 1);
            assert_eq!(img.height, 6);
        }
        _ => panic!("se esperaba GraphicsEvent::Sixel"),
    }

    // Columnas 0 y 1 son '?' (val = 0, sin píxeles pintados). Columna 2 es '~' (pintada).
    // El ancho debe ser columna más a la derecha pintada (2) + 1 = 3.
    let fixture2 = b"\x1bPq#0;2;100;0;0#0!2?~\x1b\\";
    let scan2 = scanner.advance(fixture2);
    match &scan2.events[0] {
        GraphicsEvent::Sixel(img) => {
            assert_eq!(img.width, 3);
            assert_eq!(img.height, 6);
            // Cols 0 y 1 son negras
            assert_eq!(&img.rgba[0..4], [0, 0, 0, 255]);
            assert_eq!(&img.rgba[4..8], [0, 0, 0, 255]);
            // Col 2 es roja
            assert_eq!(&img.rgba[8..12], [255, 0, 0, 255]);
        }
        _ => panic!("se esperaba GraphicsEvent::Sixel"),
    }
}

#[test]
fn sixel_raster_attributes_and_unknown_bytes_ignored() {
    let mut scanner = Scanner::new();
    let fixture = b"\x1bPq\"1;1;100;100#0;2;100;0;0#0   ~ \x1b\\";
    let scan = scanner.advance(fixture);
    assert_eq!(scan.events.len(), 1);

    match &scan.events[0] {
        GraphicsEvent::Sixel(img) => {
            assert_eq!(img.width, 1);
            assert_eq!(img.height, 6);
        }
        _ => panic!("se esperaba GraphicsEvent::Sixel"),
    }
}

#[test]
fn ansi_escape_sequences_pass_through_intact() {
    let mut scanner = Scanner::new();
    let text = b"\x1b[31mrojo\x1b[0m normal";
    let scan = scanner.advance(text);
    assert_eq!(scan.plain, text);
    assert!(scan.events.is_empty());
    assert_eq!(scanner.pending_len(), 0);
}

#[test]
fn multiple_events_in_a_single_advance_call() {
    let mut scanner = Scanner::new();
    let stream = b"inicio \x1bPq#0;2;100;0;0~-~\x1b\\ medio \x1b_Gi=42;payload\x1b\\ fin";
    let scan = scanner.advance(stream);

    assert_eq!(scan.plain, b"inicio  medio  fin");
    assert_eq!(scan.events.len(), 2);

    match &scan.events[0] {
        GraphicsEvent::Sixel(img) => {
            assert_eq!(img.width, 1);
            assert_eq!(img.height, 12);
        }
        _ => panic!("se esperaba Sixel en primer evento"),
    }

    match &scan.events[1] {
        GraphicsEvent::Kitty(payload) => {
            assert_eq!(payload, b"Gi=42;payload");
        }
        _ => panic!("se esperaba Kitty en segundo evento"),
    }
    assert_eq!(scanner.pending_len(), 0);
}

#[test]
fn sixel_malformed_body_returns_no_image_and_never_panics() {
    let mut scanner = Scanner::new();

    // Secuencia Sixel vacía
    let scan = scanner.advance(b"\x1bPq\x1b\\");
    assert!(scan.events.is_empty());

    // Secuencia con sólo bytes '?' (ningún píxel pintado)
    let scan = scanner.advance(b"\x1bPq????\x1b\\");
    assert!(scan.events.is_empty());

    // Bytes corruptos / arbitrarios
    let scan = scanner.advance(b"\x1bPq!@#$%\x1b\\");
    assert!(scan.events.is_empty());
}
