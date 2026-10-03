//! La vista: pinta un [`Frame`].
//!
//! No sabe nada de PTY, ni de `alacritty_terminal`, ni de celdas. Solo consume
//! el cuadro que le da el núcleo y lo dibuja.
//!
//! El texto se pinta directamente sobre un canvas con `shape_line`, como hace la
//! terminal de Zed. No se usa un `div` por carácter: el motor de layout (Taffy)
//! no garantiza que una caja de texto con altura fija conserve el glifo.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    App, Bounds, Context, FocusHandle, Font, FontStyle, FontWeight, MouseButton, Render, TextRun,
    Window, canvas, div, fill, point, px, rgb, size,
};
use port_term_core::frame::{Frame, Rgb, Run, Style};
use port_term_core::session::Session;

use crate::metrics::Metrics;

pub struct TerminalView {
    session: Rc<RefCell<Session>>,
    metrics: Metrics,
    background: Rgb,
    focus_handle: FocusHandle,
}

impl TerminalView {
    pub fn new(session: Rc<RefCell<Session>>, metrics: Metrics, focus_handle: FocusHandle) -> Self {
        let background = session.borrow().default_style().bg;
        Self {
            session,
            metrics,
            background,
            focus_handle,
        }
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let (width, height): (f32, f32) = (viewport.width.into(), viewport.height.into());
        let grid = self.metrics.grid_for(width, height);

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
        let metrics = self.metrics.clone();
        let background = self.background;
        let focus = self.focus_handle.clone();

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(color(background))
            .track_focus(&self.focus_handle)
            // Clic en cualquier parte devuelve el foco a la rejilla.
            .on_mouse_down(MouseButton::Left, move |_event, window, _cx| {
                focus.focus(window);
            })
            // El canvas no deduce su tamaño: sin esto se pinta sobre una caja
            // de altura cero y no aparece nada en pantalla.
            .child(
                canvas(
                    move |_bounds, _window, _cx| (),
                    move |_bounds, (), window, cx| {
                        paint_frame(&frame, &metrics, window, cx);
                    },
                )
                .size_full(),
            )
    }
}

/// Pinta el cuadro completo: un quad de fondo por celda y el texto encima.
fn paint_frame(frame: &Frame, metrics: &Metrics, window: &mut Window, cx: &mut App) {
    let padding = px(metrics.padding);
    let cell_width = px(metrics.cell_width);
    let cell_height = px(metrics.cell_height);
    let font_size = px(metrics.font_size);

    let regular = Font {
        family: metrics.font_family.clone(),
        features: Default::default(),
        fallbacks: None,
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

    for (row_index, row) in frame.rows.iter().enumerate() {
        let top = padding + cell_height * row_index as f32;

        for (run_index, run) in row.runs.iter().enumerate() {
            let is_cursor = frame
                .cursor_run
                .is_some_and(|cursor| cursor.row == row_index && cursor.run_index == run_index);
            let style = style_of(run, is_cursor);
            let left = padding + cell_width * run_offset(&row.runs, run_index);

            // El fondo ocupa exactamente las columnas del run: así una celda con
            // color de fondo pinta su rectángulo aunque el glifo sea un espacio.
            let width = cell_width * run.columns as f32;
            window.paint_quad(fill(
                Bounds::new(point(left, top), size(width, cell_height)),
                color(style.bg),
            ));

            if run.text.trim().is_empty() {
                continue;
            }

            let font = match (style.bold, style.italic) {
                (true, true) => &bold_italic,
                (true, false) => &bold,
                (false, true) => &italic,
                (false, false) => &regular,
            };

            let text_run = TextRun {
                len: run.text.len(),
                font: font.clone(),
                color: color(style.fg),
                background_color: None,
                underline: style.underline.then(|| gpui::UnderlineStyle {
                    color: Some(color(style.fg)),
                    thickness: px(1.0),
                    wavy: false,
                }),
                strikethrough: None,
            };

            window
                .text_system()
                .shape_line(run.text.clone().into(), font_size, &[text_run], Some(cell_width))
                .paint(point(left, top), cell_height, window, cx)
                .ok();
        }
    }
}

/// Columna inicial de un run dentro de su fila.
fn run_offset(runs: &[Run], run_index: usize) -> f32 {
    runs[..run_index]
        .iter()
        .map(|run| run.columns as f32)
        .sum()
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
