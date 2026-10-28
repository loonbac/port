//! Pintado del cuadro: fondos, glifos geométricos y texto.
//!
//! Única responsabilidad: convertir un [`Frame`] del núcleo en quads y texto
//! sobre el canvas.
//!
//! El texto se pinta directamente sobre un canvas con `shape_line`, como hace la
//! terminal de Zed. No se usa un `div` por carácter: el motor de layout (Taffy)
//! no garantiza que una caja de texto con altura fija conserve el glifo.
//!
//! Los elementos de bloque y los caracteres braille se dibujan como figuras
//! geométricas directas y suaves con `paint_quad`, evitando costosas búsquedas
//! en fuentes de respaldo en aplicaciones como `btop`.

use gpui::{
    fill, point, px, rgb, size, App, Bounds, Font, FontFallbacks, FontStyle, FontWeight, Pixels,
    TextRun, Window,
};
use port_term_core::frame::{Frame, PlacedImage, Rgb, Row, Run, Style};
use port_term_core::session::MousePolicy;

use crate::metrics::Metrics;
use crate::view::images::ImageCache;

/// Pinta el cuadro completo: quads de fondo, glifos geométricos y texto.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_frame(
    bounds: Bounds<Pixels>,
    frame: &Frame,
    images: &mut ImageCache,
    policy: MousePolicy,
    metrics: &Metrics,
    default_bg: Rgb,
    opacity: f32,
    font_family: &str,
    font_size_f32: f32,
    font_fallbacks: &[String],
    window: &mut Window,
    cx: &mut App,
) {
    let padding = px(metrics.padding);
    let cell_width = px(metrics.cell_width);
    let cell_height = px(metrics.cell_height);
    let font_size = px(font_size_f32);

    let fallbacks = Some(FontFallbacks::from_fonts(font_fallbacks.to_vec()));

    let regular = Font {
        family: font_family.to_string().into(),
        features: Default::default(),
        fallbacks: fallbacks.clone(),
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    };
    let bold = Font {
        weight: FontWeight::BOLD,
        ..regular.clone()
    };
    let italic = Font {
        style: FontStyle::Italic,
        ..regular.clone()
    };
    let bold_italic = Font {
        weight: FontWeight::BOLD,
        style: FontStyle::Italic,
        ..regular.clone()
    };

    let cw_f32 = metrics.cell_width;
    let ch_f32 = metrics.cell_height;

    let origin_x = bounds.origin.x + padding;
    let origin_y = bounds.origin.y + padding;

    // Primera pasada: todos los fondos. Un glifo puede tener más tinta que su
    // celda, y GPUI dibuja encima lo que se pinta después: si cada run pintara su
    // fondo junto a sus glifos, el fondo de la celda de la derecha y el de la
    // fila de abajo recortarían la tinta que sobresale. Con las dos pasadas
    // separadas los glifos quedan encima de todos los fondos.
    //
    // El margen alrededor de la rejilla se pinta antes y por separado: la rejilla
    // la cubren los fondos de las celdas, y si el margen quedara debajo de ellos
    // cada celda sumaría una capa más de alpha y volvería el rectángulo opaco.
    let grid_width = cell_width * frame.columns as f32;
    let grid_height = cell_height * frame.rows.len() as f32;
    let grid_right = origin_x + grid_width;
    let grid_bottom = origin_y + grid_height;
    let base = cell_background(default_bg, opacity);

    paint_padding(
        window,
        bounds.origin.x,
        bounds.origin.y,
        bounds.size.width,
        origin_y - bounds.origin.y,
        base,
    );
    paint_padding(
        window,
        bounds.origin.x,
        grid_bottom,
        bounds.size.width,
        bounds.origin.y + bounds.size.height - grid_bottom,
        base,
    );
    paint_padding(
        window,
        bounds.origin.x,
        origin_y,
        origin_x - bounds.origin.x,
        grid_height,
        base,
    );
    paint_padding(
        window,
        grid_right,
        origin_y,
        bounds.origin.x + bounds.size.width - grid_right,
        grid_height,
        base,
    );

    for (row_index, row) in frame.rows.iter().enumerate() {
        let top = origin_y + cell_height * row_index as f32;

        // Los fondos cubren la fila entera, columnas vacías incluidas: cada
        // píxel recibe una sola capa translúcida y la transparencia queda igual
        // en toda la ventana.
        let cursor_run = frame
            .cursor_run
            .filter(|cursor| cursor.row == row_index)
            .map(|cursor| cursor.run_index);

        for (start, end, bg) in row_background_segments(row, frame.columns, default_bg, cursor_run)
        {
            let x = origin_x + cell_width * start as f32;
            let width = cell_width * (end - start) as f32;
            window.paint_quad(fill(
                Bounds::new(point(x, top), size(width, cell_height)),
                cell_background(bg, opacity),
            ));
        }

        // El resaltado va sobre el fondo de la fila y bajo los glifos: el texto
        // sigue legible sobre el color translúcido. El color y la opacidad salen
        // de la política de la sesión; el default documentado en
        // `MousePolicy::default()` es el único que fija los valores de fábrica.
        if let Some(span) = row.selection {
            let selected_x = origin_x + cell_width * span.start as f32;
            let selected_width = cell_width * (span.end.saturating_sub(span.start) + 1) as f32;
            window.paint_quad(fill(
                Bounds::new(point(selected_x, top), size(selected_width, cell_height)),
                color(policy.highlight).opacity(policy.highlight_opacity),
            ));
        }
    }

    // Imágenes que van por debajo del texto, antes de escribir nada.
    paint_images(
        &frame.images,
        images,
        origin_x,
        origin_y,
        cw_f32,
        ch_f32,
        true,
        window,
    );

    // Segunda pasada: los glifos, ya sobre todos los fondos.
    for (row_index, row) in frame.rows.iter().enumerate() {
        let top = origin_y + cell_height * row_index as f32;
        let mut left = origin_x;

        for (run_index, run) in row.runs.iter().enumerate() {
            let is_cursor = frame
                .cursor_run
                .is_some_and(|cursor| cursor.row == row_index && cursor.run_index == run_index);
            let style = style_of(run, is_cursor);
            let width = cell_width * run.columns as f32;

            if !run.text.trim().is_empty() {
                let fg_color = color(style.fg);
                let font = match (style.bold, style.italic) {
                    (true, true) => &bold_italic,
                    (true, false) => &bold,
                    (false, true) => &italic,
                    (false, false) => &regular,
                };

                let mut text_buf = String::with_capacity(run.text.len());
                let mut text_start_col = 0usize;

                for (col_offset, ch) in run.text.chars().enumerate() {
                    let char_x: f32 = (left + cell_width * col_offset as f32).into();
                    let char_y: f32 = top.into();

                    // Intentamos pintar como elemento de bloque o braille geométrico suave
                    if paint_block_element(ch, char_x, char_y, cw_f32, ch_f32, fg_color, window)
                        || paint_braille(ch, char_x, char_y, cw_f32, ch_f32, fg_color, window)
                    {
                        // Si había texto regular acumulado previo, lo pintamos antes
                        if !text_buf.is_empty() {
                            let text_run = TextRun {
                                len: text_buf.len(),
                                font: font.clone(),
                                color: fg_color,
                                background_color: None,
                                underline: style.underline.then(|| gpui::UnderlineStyle {
                                    color: Some(fg_color),
                                    thickness: px(1.0),
                                    wavy: false,
                                }),
                                strikethrough: None,
                            };
                            let text_x = left + cell_width * text_start_col as f32;
                            window
                                .text_system()
                                .shape_line(
                                    text_buf.clone().into(),
                                    font_size,
                                    &[text_run],
                                    Some(cell_width),
                                )
                                .paint(point(text_x, top), cell_height, window, cx)
                                .ok();
                            text_buf.clear();
                        }
                    } else {
                        if text_buf.is_empty() {
                            text_start_col = col_offset;
                        }
                        text_buf.push(ch);
                    }
                }

                // Pintamos cualquier texto restante al final del run
                if !text_buf.is_empty() {
                    let text_run = TextRun {
                        len: text_buf.len(),
                        font: font.clone(),
                        color: fg_color,
                        background_color: None,
                        underline: style.underline.then(|| gpui::UnderlineStyle {
                            color: Some(fg_color),
                            thickness: px(1.0),
                            wavy: false,
                        }),
                        strikethrough: None,
                    };
                    let text_x = left + cell_width * text_start_col as f32;
                    window
                        .text_system()
                        .shape_line(text_buf.into(), font_size, &[text_run], Some(cell_width))
                        .paint(point(text_x, top), cell_height, window, cx)
                        .ok();
                }
            }

            left += width;
        }
    }

    // Imágenes que van por encima del texto: el último trazo del cuadro.
    paint_images(
        &frame.images,
        images,
        origin_x,
        origin_y,
        cw_f32,
        ch_f32,
        false,
        window,
    );

    if frame.debug.flags.any() {
        paint_debug(
            bounds, frame, &regular, font_size, origin_x, origin_y, cw_f32, ch_f32, window, cx,
        );
    }
}

/// Pinta las imágenes colocadas de una de las dos capas.
///
/// Se llama dos veces: antes del texto para las que van debajo (`z < 0`) y
/// después para el resto. Así un programa puede dejar una imagen de fondo y
/// escribir encima sin que el texto desaparezca, que es lo que promete el
/// z-index del protocolo de kitty.
#[allow(clippy::too_many_arguments)]
fn paint_images(
    images: &[PlacedImage],
    cache: &mut ImageCache,
    origin_x: Pixels,
    origin_y: Pixels,
    cell_width: f32,
    cell_height: f32,
    below_text: bool,
    window: &mut Window,
) {
    for placed in images {
        if (placed.z < 0) != below_text {
            continue;
        }
        let Some(render) = cache.render(placed) else {
            continue;
        };
        let left = origin_x + px(cell_width * placed.column as f32);
        let top = origin_y + px(cell_height * placed.row as f32);
        let bounds = Bounds::new(
            point(left, top),
            size(
                px(cell_width * placed.columns as f32),
                px(cell_height * placed.rows as f32),
            ),
        );
        // El canvas recorta lo que se sale de la rejilla, así que una imagen
        // que empieza por encima del borde superior se dibuja parcialmente sin
        // ninguna cuenta extra aquí.
        let _ = window.paint_image(bounds, gpui::Corners::default(), render, 0, false);
    }
}

/// Pinta las capas de depuración que el programa pidió por OSC 556.
///
/// La rejilla va primero, debajo de todo, para que las líneas no tapen el
/// texto. El HUD se ancla a la esquina superior derecha, que es donde menos
/// molesta: quien lo enciende ya sabe que está depurando.
#[allow(clippy::too_many_arguments)]
fn paint_debug(
    bounds: Bounds<Pixels>,
    frame: &Frame,
    font: &Font,
    font_size: Pixels,
    origin_x: Pixels,
    origin_y: Pixels,
    cell_width: f32,
    cell_height: f32,
    window: &mut Window,
    cx: &mut App,
) {
    let state = frame.debug;

    if state.flags.grid {
        let line = color(Rgb {
            r: 0x3a,
            g: 0x3a,
            b: 0x46,
        })
        .opacity(0.7);
        for column in 0..=frame.columns {
            let x = origin_x + px(cell_width * column as f32);
            window.paint_quad(fill(
                Bounds::new(point(x, origin_y), size(px(1.0), bounds.size.height)),
                line,
            ));
        }
        for row in 0..=frame.rows.len() {
            let y = origin_y + px(cell_height * row as f32);
            window.paint_quad(fill(
                Bounds::new(point(origin_x, y), size(bounds.size.width, px(1.0))),
                line,
            ));
        }
    }

    if !state.flags.hud {
        return;
    }

    let stats = state.stats;
    let lines = [
        format!("grid {}x{}", stats.columns, stats.rows),
        format!("pending {}", bytes(stats.pending_bytes as u64)),
        format!("dropped {}", bytes(stats.dropped_bytes)),
        format!("images {}", stats.images),
        format!(
            "scroll {}{}",
            stats.scrollback,
            if stats.alternate { " alt" } else { "" }
        ),
    ];

    // La fuente del terminal es monoespaciada, así que el ancho del cuadro sale
    // de contar caracteres: no hace falta medir el texto.
    let pad = 6.0_f32;
    let widest = lines.iter().map(|line| line.len()).max().unwrap_or(1) as f32;
    let box_width = widest * cell_width + pad * 2.0;
    let box_height = cell_height * lines.len() as f32 + pad * 2.0;
    let box_origin = point(
        bounds.origin.x + bounds.size.width - px(box_width) - px(pad),
        bounds.origin.y + px(pad),
    );
    window.paint_quad(fill(
        Bounds::new(box_origin, size(px(box_width), px(box_height))),
        color(Rgb {
            r: 0x10,
            g: 0x10,
            b: 0x18,
        })
        .opacity(0.85),
    ));

    for (index, line) in lines.iter().enumerate() {
        let text_run = TextRun {
            len: line.len(),
            font: font.clone(),
            color: color(Rgb {
                r: 0xff,
                g: 0xff,
                b: 0xff,
            }),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let text_origin = point(
            box_origin.x + px(pad),
            box_origin.y + px(pad + cell_height * index as f32),
        );
        window
            .text_system()
            .shape_line(
                line.clone().into(),
                font_size,
                &[text_run],
                Some(px(cell_width)),
            )
            .paint(text_origin, px(cell_height), window, cx)
            .ok();
    }
}

/// Formatea un tamaño de bytes para el HUD.
fn bytes(value: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    if value >= MIB {
        format!("{:.1} MiB", value as f64 / MIB as f64)
    } else if value >= KIB {
        format!("{:.1} KiB", value as f64 / KIB as f64)
    } else {
        format!("{value} B")
    }
}

/// Dibuja caracteres braille (U+2800..=U+28FF) como puntos circulares suaves y anti-aliased.
fn paint_braille(
    ch: char,
    x: f32,
    y: f32,
    cell_w: f32,
    cell_h: f32,
    color: gpui::Hsla,
    window: &mut Window,
) -> bool {
    let cp = ch as u32;
    if !(0x2800..=0x28FF).contains(&cp) {
        return false;
    }
    let bits = (cp - 0x2800) as u8;
    if bits == 0 {
        return true;
    }

    // Matriz de 2 columnas x 4 filas con puntos redondos en vez de cuadrados
    let dot_size = (cell_w * 0.32).max(2.0).round();
    let col_step = cell_w * 0.45;
    let row_step = cell_h * 0.22;
    let offset_x = (cell_w - (col_step + dot_size)) * 0.5;
    let offset_y = (cell_h - (row_step * 3.0 + dot_size)) * 0.5;
    let radius = px(dot_size * 0.5);

    let dot_map: [(u8, f32, f32); 8] = [
        (0x01, 0.0, 0.0),
        (0x02, 0.0, 1.0),
        (0x04, 0.0, 2.0),
        (0x08, 1.0, 0.0),
        (0x10, 1.0, 1.0),
        (0x20, 1.0, 2.0),
        (0x40, 0.0, 3.0),
        (0x80, 1.0, 3.0),
    ];

    for (bit, col, row) in dot_map {
        if bits & bit != 0 {
            let dot_x = x + offset_x + col * col_step;
            let dot_y = y + offset_y + row * row_step;
            window.paint_quad(gpui::quad(
                Bounds::new(
                    point(px(dot_x), px(dot_y)),
                    size(px(dot_size), px(dot_size)),
                ),
                radius,
                color,
                gpui::Edges::default(),
                gpui::transparent_black(),
                gpui::BorderStyle::default(),
            ));
        }
    }
    true
}

/// Dibuja elementos de bloque geométricos (U+2580..=U+259F y U+25A0).
fn paint_block_element(
    ch: char,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: gpui::Hsla,
    window: &mut Window,
) -> bool {
    let cp = ch as u32;
    let (bx, by, bw, bh) = match cp {
        // ▀ mitad superior
        0x2580 => (0.0, 0.0, 1.0, 0.5),
        // ▁▂▃▄▅▆▇█ bloques inferiores de 1..=8 octavos
        0x2581..=0x2588 => {
            let eighths = (cp - 0x2580) as f32 / 8.0;
            (0.0, 1.0 - eighths, 1.0, eighths)
        }
        // ▉▊▋▌▍▎▏ bloques izquierdos de 7..=1 octavos
        0x2589..=0x258F => {
            let eighths = (0x2590 - cp) as f32 / 8.0;
            (0.0, 0.0, eighths, 1.0)
        }
        // ▐ mitad derecha
        0x2590 => (0.5, 0.0, 0.5, 1.0),
        // ▔ octavo superior
        0x2594 => (0.0, 0.0, 1.0, 0.125),
        // ▕ octavo derecho
        0x2595 => (0.875, 0.0, 0.125, 1.0),
        // ■ cuadrado negro con bordes sutilmente redondeados
        0x25A0 => {
            let sq_w = (w * 0.65).round();
            let sq_h = (h * 0.55).round();
            let sq_x = x + (w - sq_w) * 0.5;
            let sq_y = y + (h - sq_h) * 0.5;
            window.paint_quad(gpui::quad(
                Bounds::new(point(px(sq_x), px(sq_y)), size(px(sq_w), px(sq_h))),
                px(2.0),
                color,
                gpui::Edges::default(),
                gpui::transparent_black(),
                gpui::BorderStyle::default(),
            ));
            return true;
        }
        // ░ sombra suave
        0x2591 => {
            window.paint_quad(fill(
                Bounds::new(point(px(x), px(y)), size(px(w), px(h))),
                color.opacity(0.25),
            ));
            return true;
        }
        // ▒ sombra media
        0x2592 => {
            window.paint_quad(fill(
                Bounds::new(point(px(x), px(y)), size(px(w), px(h))),
                color.opacity(0.50),
            ));
            return true;
        }
        // ▓ sombra densa
        0x2593 => {
            window.paint_quad(fill(
                Bounds::new(point(px(x), px(y)), size(px(w), px(h))),
                color.opacity(0.75),
            ));
            return true;
        }
        _ => return false,
    };

    window.paint_quad(fill(
        Bounds::new(
            point(px(x + bx * w), px(y + by * h)),
            size(px(bw * w), px(bh * h)),
        ),
        color,
    ));
    true
}

/// Estilo con el que se pinta un run, invirtiendo el del cursor.
///
/// El cursor es un bloque: se invierte el par de colores de esa celda.
fn style_of(run: &Run, is_cursor: bool) -> Style {
    if is_cursor {
        Style {
            fg: run.style.bg,
            bg: run.style.fg,
            ..run.style
        }
    } else {
        run.style
    }
}

/// Fondos de una fila como segmentos disjuntos que cubren todas las columnas.
///
/// La ventana es translúcida y el alpha no se reemplaza, se compone: lo que se
/// pinta sobre un fondo ya translúcido queda más opaco que el resto. Para que
/// toda la superficie tenga la misma transparencia, cada píxel debe recibir una
/// sola capa; por eso los fondos cubren la fila completa, sin huecos ni
/// solapes, y lo que ningún run ocupa se rellena con el fondo del tema.
fn row_background_segments(
    row: &Row,
    columns: usize,
    default_bg: Rgb,
    cursor_run: Option<usize>,
) -> Vec<(usize, usize, Rgb)> {
    let mut segments: Vec<(usize, usize, Rgb)> = Vec::new();
    let mut column = 0usize;

    for (run_index, run) in row.runs.iter().enumerate() {
        let end = (column + run.columns).min(columns);
        if end > column {
            let style = style_of(run, cursor_run == Some(run_index));
            push_segment(&mut segments, column, end, style.bg);
        }
        column = end;
    }

    if column < columns {
        push_segment(&mut segments, column, columns, default_bg);
    }
    segments
}

/// Añade un segmento, fusionándolo con el anterior si comparten color y son
/// contiguos: menos quads por fila y la misma cobertura.
fn push_segment(segments: &mut Vec<(usize, usize, Rgb)>, start: usize, end: usize, bg: Rgb) {
    if end <= start {
        return;
    }
    if let Some(last) = segments.last_mut() {
        if last.2 == bg && last.1 == start {
            last.1 = end;
            return;
        }
    }
    segments.push((start, end, bg));
}

/// Única capa de fondo de una celda, con la opacidad de la ventana.
///
/// La ventana es translúcida y el alpha no reemplaza, se compone: una celda
/// pintada sobre un fondo ya translúcido queda más opaca que el resto, que es
/// justo el rectángulo negro que se veía. Por eso aquí toda celda pinta su
/// fondo —también la que usa el del tema— y ninguna se apoya en el contenedor.
fn cell_background(style_bg: Rgb, opacity: f32) -> gpui::Hsla {
    color(style_bg).opacity(opacity)
}

/// Pinta una franja del área de la rejilla que ningún renglón ocupa.
fn paint_padding(
    window: &mut Window,
    x: Pixels,
    y: Pixels,
    width: Pixels,
    height: Pixels,
    background: gpui::Hsla,
) {
    if width <= px(0.0) || height <= px(0.0) {
        return;
    }
    window.paint_quad(fill(
        Bounds::new(point(x, y), size(width, height)),
        background,
    ));
}

pub(crate) fn color(rgb_value: Rgb) -> gpui::Hsla {
    let packed = (rgb_value.r as u32) << 16 | (rgb_value.g as u32) << 8 | (rgb_value.b as u32);
    rgb(packed).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fondo del tema: el mismo valor que `Rgb::DEFAULT_BG`.
    const THEME_BG: Rgb = Rgb::new(0x0d, 0x11, 0x17);
    /// Fondo que un programa puede pedir por su cuenta y que no es el del tema.
    const PROGRAM_BG: Rgb = Rgb::new(0x00, 0x00, 0x00);

    fn run(text: &str, columns: usize, bg: Rgb) -> Run {
        Run {
            text: text.to_string(),
            style: Style {
                bg,
                ..Style::plain()
            },
            columns,
        }
    }

    /// Columnas cubiertas por una lista de segmentos, con sus solapes contados.
    fn coverage(segments: &[(usize, usize, Rgb)], columns: usize) -> Vec<usize> {
        let mut hits = vec![0usize; columns];
        for (start, end, _) in segments {
            for hit in hits.iter_mut().take(*end).skip(*start) {
                *hit += 1;
            }
        }
        hits
    }

    #[test]
    fn row_segments_cover_every_column_exactly_once() {
        // Tres tramos de cinco columnas en una terminal de veinte: quedan cinco
        // columnas libres al final, que tambien necesitan su capa de fondo o la
        // ventana se veria translucida a distinto nivel en la misma fila.
        let row = Row {
            runs: vec![
                run("hola", 5, PROGRAM_BG),
                run("mundo", 5, THEME_BG),
                run("fin", 5, PROGRAM_BG),
            ],
            selection: None,
        };

        let segments = row_background_segments(&row, 20, THEME_BG, None);

        let hits = coverage(&segments, 20);
        assert!(
            hits.iter().all(|hits| *hits == 1),
            "cada columna debe recibir exactamente una capa, y el conteo es {hits:?}"
        );
    }

    #[test]
    fn the_columns_no_run_occupies_use_the_theme_background() {
        let row = Row {
            runs: vec![run("hola", 5, PROGRAM_BG)],
            selection: None,
        };

        let segments = row_background_segments(&row, 8, THEME_BG, None);

        assert_eq!(segments, vec![(0, 5, PROGRAM_BG), (5, 8, THEME_BG)]);
    }

    #[test]
    fn neighbouring_runs_with_the_same_background_share_one_layer() {
        let row = Row {
            runs: vec![run("hola", 5, PROGRAM_BG), run("mundo", 5, PROGRAM_BG)],
            selection: None,
        };

        let segments = row_background_segments(&row, 10, THEME_BG, None);

        assert_eq!(segments, vec![(0, 10, PROGRAM_BG)]);
    }

    #[test]
    fn the_cursor_run_uses_the_inverted_background() {
        // El cursor es un bloque: invierte el par de colores, asi que su fondo
        // es el del texto y no el de la celda.
        let row = Row {
            runs: vec![run("ab", 2, THEME_BG), run("cd", 2, THEME_BG)],
            selection: None,
        };

        let segments = row_background_segments(&row, 4, THEME_BG, Some(1));

        assert_eq!(segments, vec![(0, 2, THEME_BG), (2, 4, Style::plain().fg),]);
    }

    #[test]
    fn every_layer_carries_the_window_opacity() {
        // Uniformidad: el fondo del tema y el que pide un programa se pintan con
        // la misma opacidad, porque cada uno es la unica capa de su pixel.
        for bg in [THEME_BG, PROGRAM_BG] {
            let alpha = cell_background(bg, 0.85).a;
            assert!(
                (alpha - 0.85).abs() < 1e-6,
                "el alpha deberia ser la opacidad de la ventana (0.85) y es {alpha}"
            );
        }
    }

    #[test]
    fn an_opaque_window_paints_opaque_cells() {
        let bg = cell_background(PROGRAM_BG, 1.0);
        assert!((bg.a - 1.0).abs() < 1e-6);
    }
}
