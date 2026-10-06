//! Traducción de la rejilla de `alacritty_terminal` a nuestro [`Frame`].
//!
//! Esta es la única frontera entre la biblioteca y nuestros tipos. Traduce en un
//! solo sentido: de la rejilla hacia la UI.
//!
//! Contrato de una fila: se entregan las celdas hasta la última que aporta algo,
//! y se descartan las vacías del final que estén en el estilo por defecto. La UI
//! pinta el fondo de la fila hasta el ancho completo por su cuenta, así que esas
//! celdas no aportan información y solo inflarían el cuadro. Una celda vacía con
//! fondo propio sí se conserva, porque pintarla sí cambia lo que se ve.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape as VteCursorShape, NamedColor};

use super::{Cursor, CursorRun, CursorShape, Frame, Rgb, Row, Run, SelectionSpan, Style};

/// Paleta de los 16 colores base, mientras no exista configuración ni tema.
const PALETTE: [(NamedColor, Rgb); 16] = [
    (NamedColor::Black, Rgb::new(0x48, 0x4f, 0x58)),
    (NamedColor::Red, Rgb::new(0xff, 0x7b, 0x72)),
    (NamedColor::Green, Rgb::new(0x3f, 0xb9, 0x50)),
    (NamedColor::Yellow, Rgb::new(0xd2, 0x99, 0x22)),
    (NamedColor::Blue, Rgb::new(0x58, 0xa6, 0xff)),
    (NamedColor::Magenta, Rgb::new(0xbc, 0x8c, 0xff)),
    (NamedColor::Cyan, Rgb::new(0x39, 0xc5, 0xcf)),
    (NamedColor::White, Rgb::new(0xb1, 0xba, 0xc4)),
    (NamedColor::BrightBlack, Rgb::new(0x6e, 0x76, 0x81)),
    (NamedColor::BrightRed, Rgb::new(0xff, 0xa1, 0x98)),
    (NamedColor::BrightGreen, Rgb::new(0x56, 0xd3, 0x64)),
    (NamedColor::BrightYellow, Rgb::new(0xe3, 0xb3, 0x41)),
    (NamedColor::BrightBlue, Rgb::new(0x79, 0xc0, 0xff)),
    (NamedColor::BrightMagenta, Rgb::new(0xd2, 0xa8, 0xff)),
    (NamedColor::BrightCyan, Rgb::new(0x56, 0xd4, 0xdd)),
    (NamedColor::BrightWhite, Rgb::new(0xff, 0xff, 0xff)),
];

/// Traduce un color de la rejilla a RGB.
pub fn resolve_color<T: EventListener>(term: &Term<T>, color: Color) -> Rgb {
    match color {
        Color::Spec(rgb) => Rgb::new(rgb.r, rgb.g, rgb.b),
        Color::Named(named) => named_color(term, named),
        Color::Indexed(index) => indexed_color(index),
    }
}

fn named_color<T: EventListener>(term: &Term<T>, named: NamedColor) -> Rgb {
    match named {
        // Estos tres los resuelve la propia terminal según su configuración.
        NamedColor::Foreground | NamedColor::BrightForeground => Rgb::DEFAULT_FG,
        NamedColor::Background => Rgb::DEFAULT_BG,
        NamedColor::Cursor => Rgb::new(0x58, 0xa6, 0xff),
        NamedColor::DimForeground => Rgb::new(0x8b, 0x94, 0x9e),
        other => {
            let base = match other {
                NamedColor::DimBlack => NamedColor::Black,
                NamedColor::DimRed => NamedColor::Red,
                NamedColor::DimGreen => NamedColor::Green,
                NamedColor::DimYellow => NamedColor::Yellow,
                NamedColor::DimBlue => NamedColor::Blue,
                NamedColor::DimMagenta => NamedColor::Magenta,
                NamedColor::DimCyan => NamedColor::Cyan,
                NamedColor::DimWhite => NamedColor::White,
                same => same,
            };
            let _ = term;
            PALETTE
                .iter()
                .find(|(candidate, _)| *candidate == base)
                .map(|(_, rgb)| *rgb)
                .unwrap_or(Rgb::DEFAULT_FG)
        }
    }
}

/// Cubo de 6x6x6 (16-231) y escala de grises (232-255), como manda xterm.
fn indexed_color(index: u8) -> Rgb {
    const STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match index {
        0..=15 => {
            let named = match index {
                0 => NamedColor::Black,
                1 => NamedColor::Red,
                2 => NamedColor::Green,
                3 => NamedColor::Yellow,
                4 => NamedColor::Blue,
                5 => NamedColor::Magenta,
                6 => NamedColor::Cyan,
                7 => NamedColor::White,
                8 => NamedColor::BrightBlack,
                9 => NamedColor::BrightRed,
                10 => NamedColor::BrightGreen,
                11 => NamedColor::BrightYellow,
                12 => NamedColor::BrightBlue,
                13 => NamedColor::BrightMagenta,
                14 => NamedColor::BrightCyan,
                _ => NamedColor::BrightWhite,
            };
            PALETTE
                .iter()
                .find(|(candidate, _)| *candidate == named)
                .map(|(_, rgb)| *rgb)
                .unwrap_or(Rgb::DEFAULT_FG)
        }
        16..=231 => {
            let offset = index - 16;
            Rgb::new(
                STEPS[(offset / 36) as usize],
                STEPS[((offset % 36) / 6) as usize],
                STEPS[(offset % 6) as usize],
            )
        }
        _ => {
            let level = 8u8.saturating_add((index - 232).saturating_mul(10));
            Rgb::new(level, level, level)
        }
    }
}

fn style_of<T: EventListener>(term: &Term<T>, fg: Color, bg: Color, flags: Flags) -> Style {
    let mut style = Style {
        fg: resolve_color(term, fg),
        bg: resolve_color(term, bg),
        bold: flags.contains(Flags::BOLD) || flags.contains(Flags::BOLD_ITALIC),
        italic: flags.contains(Flags::ITALIC) || flags.contains(Flags::BOLD_ITALIC),
        underline: flags.intersects(Flags::ALL_UNDERLINES),
        reverse: flags.contains(Flags::INVERSE),
    };
    if style.reverse {
        std::mem::swap(&mut style.fg, &mut style.bg);
    }
    style
}

/// Construye el cuadro visible de la rejilla.
pub fn build<T: EventListener>(term: &Term<T>) -> Frame {
    let content = term.renderable_content();
    let rows_count = term.screen_lines();
    let columns = term.columns();
    // Se copian los datos del cursor antes de consumir el iterador de celdas:
    // `display_iter` se mueve y despues no se puede volver a mirar `content`.
    let display_offset = content.display_offset;
    let cursor_shape = content.cursor.shape;
    let cursor_point = content.cursor.point;
    let show_cursor = content.mode.contains(TermMode::SHOW_CURSOR);
    // El rango de selección ya viene resuelto por alacritty (`Selection::to_range`),
    // en coordenadas absolutas del buffer. Se copia antes de consumir el iterador
    // de celdas porque `SelectionRange` es `Copy` y `display_iter` se mueve.
    let selection_range = content.selection;

    let mut rows: Vec<Row> = Vec::with_capacity(rows_count);
    let mut current: Vec<Run> = Vec::new();
    let mut current_row: Option<i32> = None;
    let mut last_column = 0usize;
    // Columnas mínima y máxima seleccionadas de la fila que se está armando.
    let mut current_span: Option<(usize, usize)> = None;
    // Fila y columna del cursor, necesarias durante la iteración porque la celda
    // se separa en su propio run mientras se construye la fila.
    let cursor_position = if show_cursor {
        Some((
            cursor_point.line.0 + display_offset as i32,
            cursor_point.column.0,
        ))
    } else {
        None
    };
    let mut cursor_run: Option<CursorRun> = None;
    // El run del cursor queda cerrado: lo que viene detrás no se le pega.
    let mut sealed_run = false;

    for indexed in content.display_iter {
        let cell = indexed.cell;
        let row_index = indexed.point.line.0 + content.display_offset as i32;

        if current_row != Some(row_index) {
            if current_row.is_some() {
                rows.push(Row {
                    runs: std::mem::take(&mut current),
                    selection: current_span.map(|(start, end)| SelectionSpan { start, end }),
                });
            }
            current_row = Some(row_index);
            last_column = 0;
            sealed_run = false;
            current_span = None;
        }

        // La selección se comprueba antes de descartar los huecos de carácter
        // ancho: `contains_cell` marca la celda ancha cuando su hueco queda
        // dentro del rango. Se pasa `Hidden` para que `contains_cell` no excluya
        // la celda del cursor en bloque: el span describe la geometría de la
        // selección y la UI pinta el cursor por su cuenta.
        if let Some(range) = &selection_range {
            if range.contains_cell(&indexed, indexed.point, VteCursorShape::Hidden) {
                let column = indexed.point.column.0;
                current_span = Some(match current_span {
                    Some((start, end)) => (start.min(column), end.max(column)),
                    None => (column, column),
                });
            }
        }

        // El segundo hueco de un carácter ancho no se pinta: ya lo ocupa el
        // primero, y pintarlo duplicaría el ancho de la fila.
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            last_column += 1;
            continue;
        }

        let mut style = style_of(term, cell.fg, cell.bg, cell.flags);
        let cell_columns = if cell.flags.contains(Flags::WIDE_CHAR) {
            2
        } else {
            1
        };

        // Si la celda es un espacio sin fondo propio, ni subrayado ni invertido,
        // su color de texto no produce ningún glifo visible: normalizarlo al
        // estilo plano evita fragmentar filas en cientos de runs de un solo espacio
        // con colores residuales dejados por btop u otros programas TUI.
        if cell.c == ' ' && style.bg == Rgb::DEFAULT_BG && !style.underline && !style.reverse {
            style.fg = Rgb::DEFAULT_FG;
        }

        // La celda del cursor se entrega aislada: así la UI solo tiene que
        // invertir ese run para pintar el bloque, sin saber nada de la rejilla.
        let on_cursor = cursor_position == Some((row_index, indexed.point.column.0));

        if indexed.point.column.0 > last_column {
            let gap = indexed.point.column.0 - last_column;
            append_spaces(&mut current, gap, sealed_run);
            sealed_run = false;
        }

        // Si el estilo coincide y no es la celda del cursor, añadimos el carácter
        // directamente al buffer del último run sin ninguna asignación en el heap.
        if !sealed_run && !on_cursor && current.last().is_some_and(|last| last.style == style) {
            let last = current.last_mut().unwrap();
            last.text.push(cell.c);
            if let Some(zerowidth) = cell.zerowidth() {
                last.text.extend(zerowidth.iter());
            }
            last.columns += cell_columns;
        } else {
            let mut text = String::with_capacity(32);
            text.push(cell.c);
            if let Some(zerowidth) = cell.zerowidth() {
                text.extend(zerowidth.iter());
            }
            if on_cursor {
                current.push(Run {
                    text,
                    style,
                    columns: cell_columns,
                });
                cursor_run = Some(CursorRun {
                    row: row_index.max(0) as usize,
                    run_index: current.len() - 1,
                });
                sealed_run = true;
            } else {
                current.push(Run {
                    text,
                    style,
                    columns: cell_columns,
                });
                sealed_run = false;
            }
        }
        last_column = indexed.point.column.0 + cell_columns;
    }
    if current_row.is_some() {
        rows.push(Row {
            runs: current,
            selection: current_span.map(|(start, end)| SelectionSpan { start, end }),
        });
    }

    for (index, row) in rows.iter_mut().enumerate() {
        let protected = cursor_run
            .filter(|cursor| cursor.row == index)
            .map(|cursor| cursor.run_index);
        trim_trailing_blanks(row, protected);
    }

    while rows.len() < rows_count {
        rows.push(Row {
            runs: Vec::new(),
            selection: None,
        });
    }
    rows.truncate(rows_count);

    let cursor = if show_cursor {
        let row = cursor_point.line.0 + display_offset as i32;
        if row < 0 || row as usize >= rows_count {
            None
        } else {
            let shape = match cursor_shape {
                VteCursorShape::Block | VteCursorShape::HollowBlock => CursorShape::Block,
                VteCursorShape::Beam => CursorShape::Beam,
                VteCursorShape::Underline => CursorShape::Underline,
                VteCursorShape::Hidden => CursorShape::Hidden,
            };
            Some(Cursor {
                row: row as usize,
                column: cursor_point.column.0,
                shape,
                visible: shape != CursorShape::Hidden,
            })
        }
    } else {
        None
    };

    Frame {
        rows,
        columns,
        cursor,
        cursor_run,
        display_offset,
    }
}

/// Añade espacios a la fila, fusionando con el run anterior si ya era de estilo plano.
fn append_spaces(runs: &mut Vec<Run>, count: usize, force_new: bool) {
    if count == 0 {
        return;
    }
    let plain = Style::plain();
    if !force_new {
        if let Some(last) = runs.last_mut() {
            if last.style == plain {
                last.text.extend(std::iter::repeat_n(' ', count));
                last.columns += count;
                return;
            }
        }
    }
    let mut text = String::with_capacity(count.max(16));
    text.extend(std::iter::repeat_n(' ', count));
    runs.push(Run {
        text,
        style: plain,
        columns: count,
    });
}

/// Quita del final de una fila las celdas vacías que no cambian nada al pintar.
fn trim_trailing_blanks(row: &mut Row, protected: Option<usize>) {
    let plain = Style::plain();
    while let Some(index) = row.runs.len().checked_sub(1) {
        // La celda del cursor se conserva aunque sea un hueco: sin ella la UI no
        // tiene nada que pintar de cursor.
        if protected == Some(index) {
            break;
        }
        let last = &mut row.runs[index];
        // Si el run tiene fondo por defecto, sin subrayado ni reverse, y sólo contiene espacios:
        if last.style.bg == plain.bg && !last.style.underline && !last.style.reverse {
            let trimmed_len = last.text.trim_end_matches(' ').len();
            if trimmed_len == 0 {
                row.runs.pop();
                continue;
            }
            if trimmed_len < last.text.len() {
                let removed = last.text.len() - trimmed_len;
                last.text.truncate(trimmed_len);
                last.columns = last.columns.saturating_sub(removed);
            }
            break;
        } else {
            break;
        }
    }
}
