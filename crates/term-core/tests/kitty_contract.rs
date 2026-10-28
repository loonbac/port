//! Tests del contrato de [`port_term_core::graphics::kitty`].
//!
//! Verifica el parser de comandos del protocolo Kitty Graphics (`ESC _ G ... ST`),
//! incluyendo transmisión (inline base64, archivo, temporal), visualización,
//! eliminación, consultas y respuestas sincrónicas solicitadas.

use port_term_core::graphics::kitty::{Command, Delete, Format, Payload, Placement};

#[test]
fn payload_must_start_with_g() {
    // Si no empieza por 'G', el comando se considera no soportado
    let cmd = Command::parse(b"a=q");
    assert_eq!(cmd, Command::Unsupported);

    let cmd_empty = Command::parse(b"");
    assert_eq!(cmd_empty, Command::Unsupported);
}

#[test]
fn control_and_payload_separated_by_first_semicolon() {
    // Todo lo que sigue al primer ';' es el payload y puede contener ';' adicionales
    // Por ejemplo, una ruta de archivo con punto y coma:
    let fixture = b"Ga=T,f=100,t=f;/ruta/con;punto_y_coma/imagen.png";
    let cmd = Command::parse(fixture);

    match cmd {
        Command::Transmit(t) => {
            assert_eq!(t.format, Format::Png);
            assert_eq!(
                t.payload,
                Payload::File("/ruta/con;punto_y_coma/imagen.png".to_string())
            );
            assert!(t.display.is_some());
        }
        _ => panic!("se esperaba Command::Transmit"),
    }
}

#[test]
fn transmit_action_t_and_capital_t() {
    // a=t produce Transmit sin display
    let fixture_t = b"Ga=t,f=100,i=10;AAAA";
    let cmd_t = Command::parse(fixture_t);
    match cmd_t {
        Command::Transmit(t) => {
            assert_eq!(t.image, 10);
            assert_eq!(t.format, Format::Png);
            assert_eq!(t.payload, Payload::Inline(vec![0, 0, 0]));
            assert_eq!(t.display, None);
        }
        _ => panic!("se esperaba Command::Transmit sin display"),
    }

    // a=T produce Transmit con display configurado con las mismas claves de geometría
    let fixture_cap_t = b"Ga=T,f=100,i=10,c=5,r=3,z=-1,C=1;AAAA";
    let cmd_cap_t = Command::parse(fixture_cap_t);
    match cmd_cap_t {
        Command::Transmit(t) => {
            assert_eq!(t.image, 10);
            assert_eq!(t.format, Format::Png);
            let display = t.display.expect("debe contener Placement");
            assert_eq!(display.image, 10);
            assert_eq!(display.columns, Some(5));
            assert_eq!(display.rows, Some(3));
            assert_eq!(display.z, -1);
            assert!(display.no_cursor_move);
        }
        _ => panic!("se esperaba Command::Transmit con display"),
    }
}

#[test]
fn transmit_defaults_to_capital_t_when_a_absent_but_data_present() {
    // Cuando 'a' está ausente pero hay datos (hay ';'), se asume por defecto a=T
    let fixture = b"Gf=100,i=42;AAAA";
    let cmd = Command::parse(fixture);
    match cmd {
        Command::Transmit(t) => {
            assert_eq!(t.image, 42);
            assert_eq!(t.format, Format::Png);
            let display = t.display.expect("debe incluir display por defecto a=T");
            assert_eq!(display.image, 42);
        }
        _ => panic!("se esperaba Command::Transmit con a=T por defecto"),
    }

    // Si 'a' está ausente y no hay datos, es Unsupported
    let fixture_no_data = b"Gf=100,i=42";
    assert_eq!(Command::parse(fixture_no_data), Command::Unsupported);
}

#[test]
fn format_mappings_and_unsupported_formats() {
    // f=32 -> Rgba con dimensiones s y v
    let cmd32 = Command::parse(b"Ga=t,f=32,s=100,v=200;AAAA");
    match cmd32 {
        Command::Transmit(t) => {
            assert_eq!(t.format, Format::Rgba);
            assert_eq!(t.width, 100);
            assert_eq!(t.height, 200);
        }
        _ => panic!("se esperaba Format::Rgba"),
    }

    // f=24 -> Rgb
    let cmd24 = Command::parse(b"Ga=t,f=24,s=50,v=80;AAAA");
    match cmd24 {
        Command::Transmit(t) => {
            assert_eq!(t.format, Format::Rgb);
            assert_eq!(t.width, 50);
            assert_eq!(t.height, 80);
        }
        _ => panic!("se esperaba Format::Rgb"),
    }

    // f=100 -> Png
    let cmd100 = Command::parse(b"Ga=t,f=100;AAAA");
    match cmd100 {
        Command::Transmit(t) => {
            assert_eq!(t.format, Format::Png);
        }
        _ => panic!("se esperaba Format::Png"),
    }

    // Formato no reconocido -> Unsupported
    let cmd_other = Command::parse(b"Ga=t,f=99;AAAA");
    assert_eq!(cmd_other, Command::Unsupported);
}

#[test]
fn transmission_medium_inline_file_and_tempfile() {
    // t=d (por defecto si está ausente) -> Payload::Inline
    let cmd_def = Command::parse(b"Ga=t,f=100;AAAA");
    match cmd_def {
        Command::Transmit(t) => {
            assert_eq!(t.payload, Payload::Inline(vec![0, 0, 0]));
        }
        _ => panic!("se esperaba Payload::Inline por defecto"),
    }

    // t=d explícito -> Payload::Inline
    let cmd_inline = Command::parse(b"Ga=t,f=100,t=d;AAAA");
    match cmd_inline {
        Command::Transmit(t) => {
            assert_eq!(t.payload, Payload::Inline(vec![0, 0, 0]));
        }
        _ => panic!("se esperaba Payload::Inline explícito"),
    }

    // t=f -> Payload::File
    let cmd_file = Command::parse(b"Ga=t,f=100,t=f;/tmp/test.png");
    match cmd_file {
        Command::Transmit(t) => {
            assert_eq!(t.payload, Payload::File("/tmp/test.png".to_string()));
        }
        _ => panic!("se esperaba Payload::File"),
    }

    // t=t -> Payload::TempFile
    let cmd_temp = Command::parse(b"Ga=t,f=100,t=t;/tmp/kitty-temp.png");
    match cmd_temp {
        Command::Transmit(t) => {
            assert_eq!(
                t.payload,
                Payload::TempFile("/tmp/kitty-temp.png".to_string())
            );
        }
        _ => panic!("se esperaba Payload::TempFile"),
    }

    // Medio desconocido -> Unsupported
    let cmd_unknown = Command::parse(b"Ga=t,f=100,t=z;foo");
    assert_eq!(cmd_unknown, Command::Unsupported);
}

#[test]
fn display_action_p_requires_image_id() {
    // a=p con i válido produce Display
    let cmd = Command::parse(b"Ga=p,i=7,c=10,r=4,p=2,z=3");
    assert_eq!(
        cmd,
        Command::Display(Placement {
            image: 7,
            placement: Some(2),
            columns: Some(10),
            rows: Some(4),
            column_offset: 0,
            row_offset: 0,
            z: 3,
            no_cursor_move: false,
        })
    );

    // a=p sin i produce Unsupported (falta clave requerida)
    let cmd_no_i = Command::parse(b"Ga=p,c=10,r=4");
    assert_eq!(cmd_no_i, Command::Unsupported);
}

#[test]
fn delete_action_all_image_and_placement() {
    // a=d por defecto o con d=a -> Delete::All
    let cmd_all_def = Command::parse(b"Ga=d");
    assert_eq!(cmd_all_def, Command::Delete(Delete::All));

    let cmd_all_exp = Command::parse(b"Ga=d,d=a");
    assert_eq!(cmd_all_exp, Command::Delete(Delete::All));

    // a=d con d=i requiere i -> Delete::Image(i)
    let cmd_img = Command::parse(b"Ga=d,d=i,i=15");
    assert_eq!(cmd_img, Command::Delete(Delete::Image(15)));

    let cmd_img_no_i = Command::parse(b"Ga=d,d=i");
    assert_eq!(cmd_img_no_i, Command::Unsupported);

    // a=d con d=p requiere i y p -> Delete::Placement { image, placement }
    let cmd_plc = Command::parse(b"Ga=d,d=p,i=15,p=3");
    assert_eq!(
        cmd_plc,
        Command::Delete(Delete::Placement {
            image: 15,
            placement: 3,
        })
    );

    let cmd_plc_no_p = Command::parse(b"Ga=d,d=p,i=15");
    assert_eq!(cmd_plc_no_p, Command::Unsupported);

    let cmd_plc_no_i = Command::parse(b"Ga=d,d=p,p=3");
    assert_eq!(cmd_plc_no_i, Command::Unsupported);

    // Otro valor de d -> Unsupported
    let cmd_invalid_d = Command::parse(b"Ga=d,d=z");
    assert_eq!(cmd_invalid_d, Command::Unsupported);
}

#[test]
fn query_action_replies_ok_and_other_commands_reply_none() {
    // a=q con i -> Query { image: Some(i) } y respuesta exacta
    let cmd_q_i = Command::parse(b"Ga=q,i=42");
    assert_eq!(cmd_q_i, Command::Query { image: Some(42) });
    assert_eq!(cmd_q_i.reply(), Some(b"\x1b_Gi=42;OK\x1b\\".to_vec()));

    // a=q sin i -> Query { image: None } y respuesta con i=0
    let cmd_q_no_i = Command::parse(b"Ga=q");
    assert_eq!(cmd_q_no_i, Command::Query { image: None });
    assert_eq!(cmd_q_no_i.reply(), Some(b"\x1b_Gi=0;OK\x1b\\".to_vec()));

    // Cualquier otro comando no genera respuesta (S7: no unsolicited replies)
    let cmd_transmit = Command::parse(b"Ga=t,f=100;AAAA");
    assert_eq!(cmd_transmit.reply(), None);

    let cmd_display = Command::parse(b"Ga=p,i=1");
    assert_eq!(cmd_display.reply(), None);

    let cmd_delete = Command::parse(b"Ga=d");
    assert_eq!(cmd_delete.reply(), None);

    let cmd_unsupported = Command::Unsupported;
    assert_eq!(cmd_unsupported.reply(), None);
}

#[test]
fn base64_decoding_tolerates_whitespace_and_padding_variants() {
    // Padded con '=' (3 bytes a 4 caracteres base64)
    // "TWFu" -> "Man"
    let cmd_full = Command::parse(b"Ga=t,f=100;TWFu");
    match cmd_full {
        Command::Transmit(t) => assert_eq!(t.payload, Payload::Inline(b"Man".to_vec())),
        _ => panic!("se esperaba Transmit"),
    }

    // Padded con un '=': "TWE=" -> "Ma"
    let cmd_pad1 = Command::parse(b"Ga=t,f=100;TWE=");
    match cmd_pad1 {
        Command::Transmit(t) => assert_eq!(t.payload, Payload::Inline(b"Ma".to_vec())),
        _ => panic!("se esperaba Transmit"),
    }

    // Unpadded: "TWE" -> "Ma"
    let cmd_unpad1 = Command::parse(b"Ga=t,f=100;TWE");
    match cmd_unpad1 {
        Command::Transmit(t) => assert_eq!(t.payload, Payload::Inline(b"Ma".to_vec())),
        _ => panic!("se esperaba Transmit"),
    }

    // Padded con dos '=': "TQ==" -> "M"
    let cmd_pad2 = Command::parse(b"Ga=t,f=100;TQ==");
    match cmd_pad2 {
        Command::Transmit(t) => assert_eq!(t.payload, Payload::Inline(b"M".to_vec())),
        _ => panic!("se esperaba Transmit"),
    }

    // Unpadded: "TQ" -> "M"
    let cmd_unpad2 = Command::parse(b"Ga=t,f=100;TQ");
    match cmd_unpad2 {
        Command::Transmit(t) => assert_eq!(t.payload, Payload::Inline(b"M".to_vec())),
        _ => panic!("se esperaba Transmit"),
    }

    // Con espacios en blanco intercalados (\n, \r, \t, espacio)
    let cmd_ws = Command::parse(b"Ga=t,f=100; T\r\nW\tF u\n");
    match cmd_ws {
        Command::Transmit(t) => assert_eq!(t.payload, Payload::Inline(b"Man".to_vec())),
        _ => panic!("se esperaba Transmit con espacios"),
    }

    // Base64 inválido produce Unsupported
    let cmd_invalid = Command::parse(b"Ga=t,f=100;TWFu!@#$");
    assert_eq!(cmd_invalid, Command::Unsupported);

    // Padding inválido en el medio produce Unsupported
    let cmd_bad_pad = Command::parse(b"Ga=t,f=100;T=Qu");
    assert_eq!(cmd_bad_pad, Command::Unsupported);
}

#[test]
fn placement_geometry_keys_columns_rows_offsets_z_and_no_cursor_move() {
    let fixture = b"Ga=p,i=5,c=20,r=10,X=2,Y=4,z=-5,C=1";
    let cmd = Command::parse(fixture);
    assert_eq!(
        cmd,
        Command::Display(Placement {
            image: 5,
            placement: None,
            columns: Some(20),
            rows: Some(10),
            column_offset: 2,
            row_offset: 4,
            z: -5,
            no_cursor_move: true,
        })
    );

    // Claves x e y minúsculas son desplazamientos en píxeles ignorables
    let fixture_xy = b"Ga=p,i=5,c=20,r=10,x=100,y=200";
    let cmd_xy = Command::parse(fixture_xy);
    assert_eq!(
        cmd_xy,
        Command::Display(Placement {
            image: 5,
            placement: None,
            columns: Some(20),
            rows: Some(10),
            column_offset: 0,
            row_offset: 0,
            z: 0,
            no_cursor_move: false,
        })
    );
}

#[test]
fn transmission_flags_more_and_quiet() {
    // m=1 -> more: true; q=2 -> quiet: true
    let fixture = b"Ga=t,f=100,m=1,q=2;AAAA";
    let cmd = Command::parse(fixture);
    match cmd {
        Command::Transmit(t) => {
            assert!(t.more);
            assert!(t.quiet);
        }
        _ => panic!("se esperaba Transmit con more y quiet"),
    }

    // m=0 o ausente -> more: false; q ausente -> quiet: false
    let fixture_def = b"Ga=t,f=100;AAAA";
    match Command::parse(fixture_def) {
        Command::Transmit(t) => {
            assert!(!t.more);
            assert!(!t.quiet);
        }
        _ => panic!("se esperaba Transmit sin flags"),
    }
}

#[test]
fn malformed_input_sweep_never_panics() {
    let malformed_cases: &[&[u8]] = &[
        b"",
        b"G",
        b"G;",
        b"G;;;",
        b"Ga",
        b"Ga=",
        b"Ga=invalid",
        b"Ga=t",
        b"Ga=t,f=100",
        b"Ga=t,f=100;???",
        b"Ga=t,f=abc;AAAA",
        b"Ga=p,i=abc",
        b"Ga=d,d=i,i=notanumber",
        b"Ga=d,d=p,i=1,p=invalid",
        b"Ga=q,i=99999999999999999999999999",
        b"G\xff\xfe=1;AAAA",
        b"G,,,,;",
        b"Ga=T,z=abc;AAAA",
        b"Ga=T,C=invalid;AAAA",
    ];

    for &case in malformed_cases {
        let cmd = Command::parse(case);
        assert_eq!(
            cmd,
            Command::Unsupported,
            "el caso {:?} debió retornar Unsupported",
            String::from_utf8_lossy(case)
        );
        assert_eq!(cmd.reply(), None);
    }
}
