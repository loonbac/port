//! Tests del contrato de [`port_term_core::frame`].
//!
//! Se monta una rejilla real de `alacritty_terminal` y se le inyectan bytes, sin
//! PTY ni ventana: por eso el `frame` es verificable de forma aislada.

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;
use port_term_core::frame::{build, CursorShape, Rgb, Style};

struct Size(usize, usize);

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.1
    }
    fn screen_lines(&self) -> usize {
        self.1
    }
    fn columns(&self) -> usize {
        self.0
    }
}

/// Runs de una fila dejando fuera el del cursor.
///
/// Casi todas las aserciones hablan del texto pintado, no del cursor: el run del
/// cursor se comprueba en sus propios tests. Omitirlo aquí deja las expectativas
/// legibles y evita repetir el mismo ajuste en cada caso.
fn painted_runs(frame: &port_term_core::frame::Frame, row: usize) -> &[port_term_core::frame::Run] {
    let runs = &frame.rows[row].runs;
    match frame.cursor_run {
        Some(cursor) if cursor.row == row => &runs[..cursor.run_index],
        _ => runs,
    }
}

fn make_term(columns: usize, rows: usize) -> Term<VoidListener> {
    Term::new(Config::default(), &Size(columns, rows), VoidListener)
}

fn feed(term: &mut Term<VoidListener>, bytes: &[u8]) {
    let mut processor: Processor<alacritty_terminal::vte::ansi::StdSyncHandler> = Processor::new();
    processor.advance(term, bytes);
}

#[test]
fn debug_prompt_and_typed_runs() {
    let mut term = make_term(80, 5);
    feed(&mut term, b"\x1b[32m[user@host:~]$\x1b[0m hello");
    let frame = build(&term);
    println!("=== DEBUG RUNS ===");
    for (i, run) in frame.rows[0].runs.iter().enumerate() {
        println!(
            "run[{i}]: text={:?} cols={} fg={:?} bg={:?}",
            run.text, run.columns, run.style.fg, run.style.bg
        );
    }
    println!("cursor: {:?}", frame.cursor);
    println!("cursor_run: {:?}", frame.cursor_run);
}

#[test]
fn plain_text_becomes_one_run() {
    let mut term = make_term(20, 3);
    feed(&mut term, b"hola");

    let frame = build(&term);
    assert_eq!(frame.columns, 20);
    assert_eq!(frame.rows.len(), 3);
    let runs = painted_runs(&frame, 0);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "hola");
    assert_eq!(runs[0].columns, 4);
}

#[test]
fn adjacent_cells_with_different_colors_split_into_runs() {
    let mut term = make_term(20, 2);
    // "rojo" en rojo (31), "azul" en azul (34)
    feed(&mut term, b"\x1b[31mrojo\x1b[34mazul");

    let frame = build(&term);
    let runs = painted_runs(&frame, 0);
    assert_eq!(runs.len(), 2, "dos colores, dos runs: {runs:?}");
    assert_eq!(runs[0].text, "rojo");
    assert_eq!(runs[1].text, "azul");
    assert_ne!(runs[0].style.fg, runs[1].style.fg);
    assert_eq!(runs[0].style.fg, Rgb::new(0xff, 0x7b, 0x72));
    assert_eq!(runs[1].style.fg, Rgb::new(0x58, 0xa6, 0xff));
}

#[test]
fn many_cells_of_one_style_stay_one_run() {
    let mut term = make_term(200, 2);
    let filler = vec![b'x'; 150];
    feed(&mut term, &filler);

    let frame = build(&term);
    assert_eq!(
        painted_runs(&frame, 0).len(),
        1,
        "150 celdas del mismo estilo no deben ser 150 runs"
    );
    assert_eq!(painted_runs(&frame, 0)[0].columns, 150);
}

#[test]
fn wide_characters_occupy_two_columns() {
    let mut term = make_term(10, 2);
    feed(&mut term, "日本".as_bytes());

    let frame = build(&term);
    let runs = painted_runs(&frame, 0);
    assert_eq!(runs[0].columns, 4);
    assert_eq!(
        runs.iter().map(|run| run.columns).sum::<usize>(),
        4,
        "cada carácter ancho ocupa dos columnas"
    );
}

#[test]
fn cursor_reports_position_and_shape() {
    let mut term = make_term(20, 3);
    feed(&mut term, b"ab");

    let frame = build(&term);
    let cursor = frame.cursor.expect("el cursor debe existir");
    assert_eq!(cursor.row, 0);
    assert_eq!(cursor.column, 2);
    assert_eq!(cursor.shape, CursorShape::Block);
    assert!(cursor.visible);
}

#[test]
fn cursor_cell_is_delivered_as_its_own_run() {
    // La UI no sabe nada de la rejilla: si quiere pintar el cursor, necesita que
    // el cuadro le diga qué run es la celda del cursor.
    let mut term = make_term(20, 3);
    feed(&mut term, b"ab\r\x1b[C");

    let frame = build(&term);
    let cursor = frame.cursor.expect("cursor");
    let cursor_run = frame
        .cursor_run
        .expect("la celda del cursor debelocalizarse");

    assert_eq!(cursor_run.row, cursor.row);
    let row = &frame.rows[cursor_run.row];
    let run = &row.runs[cursor_run.run_index];
    assert_eq!(
        run.text, "b",
        "el run del cursor debe traer solo su carácter"
    );
    assert_eq!(run.columns, 1);
    assert_eq!(cursor.column, 1);
}

#[test]
fn cursor_on_blank_keeps_a_run_to_paint_the_block() {
    // El cursor al final de la línea cae sobre un hueco vacío que el recorte
    // descartaría. Sin ese run la UI no tendría nada que pintar de cursor.
    let mut term = make_term(20, 3);
    feed(&mut term, b"ab");

    let frame = build(&term);
    let cursor_run = frame
        .cursor_run
        .expect("la celda del cursor debe localizarse");
    let row = &frame.rows[cursor_run.row];
    let run = &row.runs[cursor_run.run_index];

    assert_eq!(
        cursor_run.run_index,
        row.runs.len() - 1,
        "debe ser el último run"
    );
    assert_eq!(run.columns, 1);
    assert_eq!(run.text, " ");
}

#[test]
fn cursor_cell_survives_trailing_blank_trimming() {
    let mut term = make_term(20, 3);
    feed(&mut term, b"ab");

    let frame = build(&term);
    let cursor_run = frame.cursor_run.expect("cursor");
    let run = &frame.rows[cursor_run.row].runs[cursor_run.run_index];
    assert!(
        !run.text.is_empty(),
        "el recorte no debe borrar la celda del cursor"
    );
}

#[test]
fn cursor_moves_to_next_row_after_newline() {
    let mut term = make_term(20, 3);
    feed(&mut term, b"linea1\r\n");

    let frame = build(&term);
    let cursor = frame.cursor.expect("cursor");
    assert_eq!(cursor.row, 1);
    assert_eq!(cursor.column, 0);
}

#[test]
fn reverse_video_swaps_foreground_and_background() {
    let mut term = make_term(20, 2);
    feed(&mut term, b"\x1b[7mX");

    let frame = build(&term);
    let style = frame.rows[0].runs[0].style;
    assert_eq!(style.fg, Rgb::DEFAULT_BG);
    assert_eq!(style.bg, Rgb::DEFAULT_FG);
}

#[test]
fn attributes_are_reported() {
    let mut term = make_term(20, 2);
    feed(&mut term, b"\x1b[1mA\x1b[0m\x1b[3mB\x1b[0m\x1b[4mC");

    let frame = build(&term);
    let texts: Vec<(&str, Style)> = frame.rows[0]
        .runs
        .iter()
        .map(|run| (run.text.as_str(), run.style))
        .collect();
    let bold = texts.iter().find(|(text, _)| *text == "A").expect("A");
    let italic = texts.iter().find(|(text, _)| *text == "B").expect("B");
    let underline = texts.iter().find(|(text, _)| *text == "C").expect("C");
    assert!(bold.1.bold);
    assert!(italic.1.italic);
    assert!(underline.1.underline);
}

#[test]
fn indexed_palette_follows_the_xterm_cube() {
    let mut term = make_term(20, 2);
    // 16 es el primer color del cubo 6x6x6: negro puro.
    feed(&mut term, b"\x1b[38;5;16mX");
    let frame = build(&term);
    assert_eq!(frame.rows[0].runs[0].style.fg, Rgb::new(0, 0, 0));

    let mut term = make_term(20, 2);
    // 196 = rojo al máximo dentro del cubo.
    feed(&mut term, b"\x1b[38;5;196mX");
    let frame = build(&term);
    assert_eq!(frame.rows[0].runs[0].style.fg, Rgb::new(255, 0, 0));
}

#[test]
fn scrolled_viewport_reports_its_offset() {
    let mut term = make_term(20, 2);
    for index in 0..10 {
        feed(&mut term, format!("linea{index}\r\n").as_bytes());
    }
    // Sin desplazamiento, la primera fila visible ya no es "linea0".
    let frame = build(&term);
    assert_eq!(frame.display_offset, 0);
    assert_eq!(frame.rows.len(), 2);
}
