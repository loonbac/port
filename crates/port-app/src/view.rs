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
    anchored, deferred, App, Bounds, Context, FocusHandle, Font, FontFallbacks, FontStyle,
    FontWeight, MouseButton, Pixels, Render, TextRun, Window, canvas, div, fill, point, px, rgb,
    size,
};
use port_plugin_api::{PluginInfo, PluginRegistry};
use port_term_core::frame::{Frame, Rgb, Run, Style};
use port_term_core::session::SessionManager;

use crate::metrics::Metrics;

/// Estado visual del menú gestor de plugins.
#[derive(Debug, Default, Clone)]
pub struct MenuState {
    pub open: bool,
    pub selected_index: usize,
}

pub struct TerminalView {
    session_manager: Rc<RefCell<SessionManager>>,
    metrics: Metrics,
    background: Rgb,
    focus_handle: FocusHandle,
    plugins: Rc<RefCell<PluginRegistry>>,
    menu_state: Rc<RefCell<MenuState>>,
}

impl TerminalView {
    pub fn new(
        session_manager: Rc<RefCell<SessionManager>>,
        metrics: Metrics,
        focus_handle: FocusHandle,
        plugins: Rc<RefCell<PluginRegistry>>,
        menu_state: Rc<RefCell<MenuState>>,
    ) -> Self {
        let background = session_manager.borrow().default_style().bg;
        Self {
            session_manager,
            metrics,
            background,
            focus_handle,
            plugins,
            menu_state,
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

        let left_inset = plugins.left_sidebar_width();
        let top_inset = plugins.top_bar_height();
        let bottom_inset = plugins.bottom_bar_height();

        let plugin_list = plugins.list_plugins();
        let menu_title = plugins.effective_menu_title().to_string();

        let menu_state = self.menu_state.borrow();
        let menu_open = menu_state.open;
        let selected_index = menu_state.selected_index;
        drop(menu_state);

        let custom_menu = if menu_open {
            plugins.render_custom_menu(&plugin_list, selected_index)
        } else {
            None
        };

        let usable_w = (width - left_inset).max(100.0);
        let usable_h = (height - top_inset - bottom_inset).max(100.0);
        let metrics = Metrics::new(font_size, self.metrics.padding);
        let grid = metrics.grid_for(usable_w, usable_h);

        let mut session_mgr = self.session_manager.borrow_mut();

        // 1. Si algún plugin solicitó crear una nueva sesión (Ctrl+Shift+T o Ctrl+Alt+T)
        if plugins.take_new_session_request() {
            if let Ok(new_id) = session_mgr.spawn_session() {
                plugins.on_session_created(new_id);
            }
        }

        // 2. Si algún plugin solicitó cerrar una sesión específica
        if let Some(id) = plugins.take_close_session_request() {
            let _ = session_mgr.close(id);
        }

        // 3. Sincroniza la sesión activa del gestor con la sesión solicitada por los plugins
        let target_session = plugins.active_session_id();
        session_mgr.select(target_session);

        session_mgr.pump();
        if session_mgr.size() != grid {
            let _ = session_mgr.resize(grid);
        }

        let frame = session_mgr.frame();
        drop(session_mgr);
        drop(plugins);

        if !self.focus_handle.is_focused(window) {
            self.focus_handle.focus(window);
        }

        let focus = self.focus_handle.clone();

        let mut root = div()
            .flex()
            .flex_row()
            .size_full()
            .bg(root_bg)
            .track_focus(&self.focus_handle)
            // Clic en cualquier parte devuelve el foco a la rejilla.
            .on_mouse_down(MouseButton::Left, move |_event, window, _cx| {
                focus.focus(window);
            });

        // 1. Barras laterales izquierdas (full height)
        for sidebar in left_sidebars {
            root = root.child(sidebar);
        }

        // 2. Columna principal (tabs arriba + canvas + barra inferior)
        let mut main_col = div().flex().flex_col().flex_1().h_full().overflow_hidden();
        for bar in top_bars {
            main_col = main_col.child(bar);
        }

        main_col = main_col.child(
            div()
                .flex_1()
                .w_full()
                .child(
                    canvas(
                        move |_bounds, _window, _cx| (),
                        move |bounds, (), window, cx| {
                            paint_frame(
                                bounds,
                                &frame,
                                &metrics,
                                effective_bg,
                                &font_family,
                                font_size,
                                &font_fallbacks,
                                window,
                                cx,
                            );
                        },
                    )
                    .size_full(),
                ),
        );

        for bar in bottom_bars {
            main_col = main_col.child(bar);
        }

        root = root.child(main_col);

        // Si el menú de plugins está abierto, se proyecta como overlay flotante
        if menu_open {
            if let Some(custom) = custom_menu {
                root = root.child(custom);
            } else {
                root = root.child(render_core_plugin_menu(
                    &plugin_list,
                    selected_index,
                    &menu_title,
                    width,
                    height,
                ));
            }
        }

        root
    }
}

/// Renderiza el modal nativo de gestión de plugins del core de PORT.
fn render_core_plugin_menu(
    plugins: &[PluginInfo],
    selected_index: usize,
    title: &str,
    window_width: f32,
    window_height: f32,
) -> impl IntoElement {
    let modal_w = 540.0f32.min(window_width - 40.0);
    let left = ((window_width - modal_w) * 0.5).max(10.0);
    let top = (window_height * 0.12).max(20.0);

    let mut list = div().flex().flex_col().gap(px(6.0));

    for (i, p) in plugins.iter().enumerate() {
        let is_selected = i == selected_index;
        let (status_text, dot_color, badge_bg, badge_border) = if p.enabled {
            ("ACTIVE", rgb(0x3fb950), rgb(0x0e2717), rgb(0x238636))
        } else {
            ("OFF", rgb(0x8b949e), rgb(0x161b22), rgb(0x30363d))
        };

        let row_bg = if is_selected {
            rgb(0x21262d)
        } else {
            rgb(0x161b22)
        };

        let row_border = if is_selected {
            rgb(0x58a6ff)
        } else {
            rgb(0x30363d)
        };

        let item = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(10.0))
            .py(px(8.0))
            .rounded(px(6.0))
            .bg(row_bg)
            .border_1()
            .border_color(row_border)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        // Indicador de estado estilo chip profesional con punto vectorial
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .px(px(8.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .bg(badge_bg)
                            .border_1()
                            .border_color(badge_border)
                            .child(
                                div()
                                    .w(px(6.0))
                                    .h(px(6.0))
                                    .rounded(px(3.0))
                                    .bg(dot_color),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(dot_color)
                                    .child(status_text),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(0xf0f6fc))
                            .child(p.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(0x8b949e))
                            .child(format!("({})", p.id)),
                    ),
            )
            .child(
                div()
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(4.0))
                    .bg(rgb(0x21262d))
                    .text_size(px(11.0))
                    .text_color(rgb(0x8b949e))
                    .child(format!("v{}", p.version)),
            );

        list = list.child(item);
    }

    let modal = div()
        .w(px(modal_w))
        .p(px(16.0))
        .rounded(px(8.0))
        .bg(rgb(0x0d1117))
        .border_1()
        .border_color(rgb(0x30363d))
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(4.0))
                                .bg(rgb(0x1f293d))
                                .border_1()
                                .border_color(rgb(0x388bfd))
                                .text_size(px(10.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(0x58a6ff))
                                .child("PLUGINS"),
                        )
                        .child(
                            div()
                                .text_size(px(14.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(0xf0f6fc))
                                .child(title.to_string()),
                        ),
                )
                .child(
                    div()
                        .px(px(6.0))
                        .py(px(2.0))
                        .rounded(px(4.0))
                        .bg(rgb(0x161b22))
                        .border_1()
                        .border_color(rgb(0x30363d))
                        .text_size(px(11.0))
                        .text_color(rgb(0x8b949e))
                        .child("PORT Core"),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(0x8b949e))
                        .child("↑↓ Navegar"),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(0x8b949e))
                        .child("Espacio / Enter Alternar"),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(0x8b949e))
                        .child("Esc Cerrar"),
                ),
        )
        .child(list);

    deferred(
        anchored()
            .position(point(px(left), px(top)))
            .child(modal),
    )
    .priority(100)
}

/// Pinta el cuadro completo: quads de fondo, glifos geométricos y texto.
#[allow(clippy::too_many_arguments)]
fn paint_frame(
    bounds: Bounds<Pixels>,
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

    let origin_x = bounds.origin.x + padding;
    let origin_y = bounds.origin.y + padding;

    for (row_index, row) in frame.rows.iter().enumerate() {
        let top = origin_y + cell_height * row_index as f32;
        let mut left = origin_x;

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
