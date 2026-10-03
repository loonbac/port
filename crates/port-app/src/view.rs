//! La vista: pinta un [`Frame`].
//!
//! No sabe nada de PTY, ni de `alacritty_terminal`, ni de celdas. Solo consume
//! el cuadro que le da el núcleo y lo dibuja.
//!
//! El texto se pinta directamente sobre un canvas con `shape_line`, como hace la
//! terminal de Zed. No se usa un `div` por carácter: el motor de layout (Taffy)
//! no garantiza que una caja de texto con altura fija conserve el glifo.
//!
//! Los elementos de bloque y los caracteres braille se dibujan como figuras
//! geométricas directas y suaves con `paint_quad`, evitando costosas búsquedas
//! en fuentes de respaldo en aplicaciones como `btop`.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    App, Bounds, Context, FocusHandle, Font, FontFallbacks, FontStyle, FontWeight, MouseButton,
    Render, TextRun, Window, canvas, div, fill, point, px, rgb, size,
};
use port_plugin_api::PluginRegistry;
use port_term_core::frame::{Frame, Rgb, Run, Style};
use port_term_core::session::Session;

use crate::metrics::Metrics;

pub struct TerminalView {
    session: Rc<RefCell<Session>>,
    metrics: Metrics,
    background: Rgb,
    focus_handle: FocusHandle,
    plugins: Rc<RefCell<PluginRegistry>>,
}

impl TerminalView {
    pub fn new(
        session: Rc<RefCell<Session>>,
        metrics: Metrics,
        focus_handle: FocusHandle,
        plugins: Rc<RefCell<PluginRegistry>>,
    ) -> Self {
        let background = session.borrow().default_style().bg;
        Self {
            session,
            metrics,
            background,
            focus_handle,
            plugins,
        }
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let (width, height): (f32, f32) = (viewport.width.into(), viewport.height.into());

        let plugins = self.plugins.borrow();
        let effective_bg = plugins.effective_background(self.background);
        let opacity = plugins.effective_opacity();
        let root_bg = color(effective_bg).opacity(opacity);

        let font_family = plugins.effective_font_family("FiraCode Nerd Font Mono");
        let font_size = plugins.effective_font_size(self.metrics.font_size);
        let font_fallbacks = plugins.effective_font_fallbacks();

        let top_bars = plugins.top_bars();
        let left_sidebars = plugins.left_sidebars();
        let bottom_bars = plugins.bottom_bars();

        let metrics = Metrics::new(font_size, self.metrics.padding);
        let grid = metrics.grid_for(width, height);

        drop(plugins);

        {
            let mut session = self.session.borrow_mut();
            session.pump();
            if session.size() != grid {
                let _ = session.resize(grid);
            }
        }

        if !self.focus_handle.is_focused(window) {
            self.focus_handle.focus(window);
        }

        let frame = self.session.borrow().frame();
        let focus = self.focus_handle.clone();

        let mut root = div()
            .flex()
            .flex_col()
            .size_full()
            .bg(root_bg)
            .track_focus(&self.focus_handle)
            // Clic en cualquier parte devuelve el foco a la rejilla.
            .on_mouse_down(MouseButton::Left, move |_event, window, _cx| {
                focus.focus(window);
            });

        for bar in top_bars {
            root = root.child(bar);
        }

        let mut center = div().flex().flex_row().flex_1();
        for sidebar in left_sidebars {
            center = center.child(sidebar);
        }

        center = center.child(
            canvas(
                move |_bounds, _window, _cx| (),
                move |_bounds, (), window, cx| {
                    paint_frame(
                        &frame,
                        &metrics,
                        effective_bg,
                        font_family,
                        font_size,
                        &font_fallbacks,
                        window,
                        cx,
                    );
                },
            )
            .size_full(),
        );

        root = root.child(center);

        for bar in bottom_bars {
            root = root.child(bar);
        }

        root
    }
}

/// Pinta el cuadro completo: quads de fondo, glifos geométricos y texto.
#[allow(clippy::too_many_arguments)]
fn paint_frame(
    frame: &Frame,
    metrics: &Metrics,
    default_bg: Rgb,
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

    for (row_index, row) in frame.rows.iter().enumerate() {
        let top = padding + cell_height * row_index as f32;
        let mut left = padding;

        for (run_index, run) in row.runs.iter().enumerate() {
            let is_cursor = frame
                .cursor_run
                .is_some_and(|cursor| cursor.row == row_index && cursor.run_index == run_index);
            let style = style_of(run, is_cursor);
            let width = cell_width * run.columns as f32;

            // Si el color coincide con el fondo por defecto y no es el cursor,
            // dejamos que se vea el fondo del contenedor principal (con su opacidad).
            if style.bg != default_bg || is_cursor {
                window.paint_quad(fill(
                    Bounds::new(point(left, top), size(width, cell_height)),
                    color(style.bg),
                ));
            }

            if !run.text.trim().is_empty() {
                let fg_color = color(style.fg);
                let font = match (style.bold, style.italic) {
                    (true, true) => &bold_italic,
                    (true, false) => &bold,
                    (false, true) => &italic,
                    (false, false) => &regular,
                };

                let mut col_offset = 0usize;
                let mut text_buf = String::with_capacity(run.text.len());
                let mut text_start_col = 0usize;

                for ch in run.text.chars() {
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
                                .shape_line(text_buf.clone().into(), font_size, &[text_run], Some(cell_width))
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
                    col_offset += 1;
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
                Bounds::new(point(px(dot_x), px(dot_y)), size(px(dot_size), px(dot_size))),
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

fn color(rgb_value: Rgb) -> gpui::Hsla {
    let packed =
        (rgb_value.r as u32) << 16 | (rgb_value.g as u32) << 8 | (rgb_value.b as u32);
    rgb(packed).into()
}
