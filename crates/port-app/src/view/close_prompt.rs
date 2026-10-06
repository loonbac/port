//! Diálogo de confirmación de cierre.
//!
//! Responsabilidad única: mostrar qué programas se perderán al cerrar y ofrecer
//! la opción segura por defecto. No conoce la terminal ni el núcleo: solo recibe
//! la lista de programas y el estado del diálogo.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{anchored, deferred, div, point, px, rgb, FontWeight, IntoElement, MouseButton, Window};

/// Estado del diálogo de confirmación de cierre.
#[derive(Debug, Default, Clone)]
pub struct ClosePromptState {
    pub open: bool,
    pub programs: Vec<String>,
    /// Botón enfocado: 0 = "No, seguir", 1 = "Si, cerrar".
    /// Por defecto se enfoca la opción segura, para que un Enter descuidado
    /// no termine matando la terminal.
    pub selected: usize,
}

/// Cierra la ventana de verdad, saltándose la confirmación.
pub fn accept_close(state: &Rc<RefCell<ClosePromptState>>, window: &mut Window) {
    {
        let mut s = state.borrow_mut();
        s.open = false;
        s.selected = 0;
    }
    window.refresh();
    window.remove_window();
}

/// Descarta la confirmación y vuelve a la terminal.
pub fn decline_close(state: &Rc<RefCell<ClosePromptState>>, window: &mut Window) {
    {
        let mut s = state.borrow_mut();
        s.open = false;
        s.selected = 0;
    }
    window.refresh();
}

/// Modal de confirmación de cierre: avisa de qué programas se perderán.
pub(crate) fn render_close_prompt(
    programs: &[String],
    state: Rc<RefCell<ClosePromptState>>,
    window_w: f32,
    window_h: f32,
) -> impl IntoElement {
    // Centrado real en la ventana, no coordenadas fijas.
    let modal_w = 440.0f32.min(window_w - 40.0).max(240.0);
    let modal_h = 280.0f32;
    let left = ((window_w - modal_w) * 0.5).max(0.0);
    let top = ((window_h - modal_h) * 0.5).max(0.0);
    let selected = state.borrow().selected;
    let mut rows = div().flex().flex_col().gap(px(4.0));
    for bin in programs {
        rows = rows.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(8.0))
                .py(px(4.0))
                .rounded(px(4.0))
                .bg(rgb(0x1c1c2b))
                .child(
                    div()
                        .w(px(6.0))
                        .h(px(6.0))
                        .rounded(px(3.0))
                        .bg(rgb(0xf0883e)),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(0xf0f6fc))
                        .child(bin.clone()),
                ),
        );
    }

    // Ambos botones comparten estilo base: la diferencia entre el enfocado y
    // el resto es solo el foco, nunca "el que está en rojo".
    let mk_button = |label: &'static str,
                     focused: bool,
                     accent: gpui::Hsla,
                     state: Rc<RefCell<ClosePromptState>>| {
        let bg: gpui::Hsla = if focused {
            accent.opacity(0.22)
        } else {
            rgb(0x161b22).into()
        };
        let border: gpui::Hsla = if focused {
            accent
        } else {
            rgb(0x30363d).into()
        };
        let text: gpui::Hsla = if focused {
            rgb(0xffffff).into()
        } else {
            rgb(0x8b949e).into()
        };

        let mut inner = div().flex().flex_row().items_center().gap(px(8.0));
        // Punto de foco: aparece solo en la opción elegida.
        inner = if focused {
            inner.child(div().w(px(7.0)).h(px(7.0)).rounded(px(4.0)).bg(accent))
        } else {
            // Marcador de posición para que el texto no salte al aparecer.
            inner.child(div().w(px(7.0)).h(px(7.0)))
        };

        inner = inner.child(
            div()
                .text_size(px(13.0))
                .font_weight(FontWeight::BOLD)
                .text_color(text)
                .child(label),
        );

        div()
            .flex()
            .items_center()
            .justify_center()
            .px(px(18.0))
            .py(px(8.0))
            .rounded(px(6.0))
            .bg(bg)
            .border(if focused { px(2.0) } else { px(1.0) })
            .border_color(border)
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_event, window, _cx| {
                if focused {
                    accept_close(&state, window);
                } else {
                    decline_close(&state, window);
                }
            })
            .child(inner)
    };

    let cancel_btn = mk_button(
        "No, seguir",
        selected == 0,
        rgb(0x58a6ff).into(),
        Rc::clone(&state),
    );
    let confirm_btn = mk_button(
        "Si, cerrar",
        selected == 1,
        rgb(0xf85149).into(),
        Rc::clone(&state),
    );

    deferred(
        anchored().position(point(px(left), px(top))).child(
            div()
                .w(px(440.0))
                .p(px(16.0))
                .bg(rgb(0x0d1117))
                .border_1()
                .border_color(rgb(0xf0883e))
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(
                    div()
                        .text_size(px(14.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(0xf0883e))
                        .child("Hay procesos en ejecucion"),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(0x8b949e))
                        .child("Si cierras la terminal estos procesos se perderan:"),
                )
                .child(rows)
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(0x6e7681))
                        .child("[←/→] Elegir   [Enter] Confirmar   [Esc] Cancelar"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_center()
                        .gap(px(12.0))
                        .child(cancel_btn)
                        .child(confirm_btn),
                ),
        ),
    )
    .priority(200)
}
