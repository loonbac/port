//! PORT: arranque y composición.
//!
//! Aquí solo se conectan piezas: se arranca el shell, se decide el tamaño
//! inicial de la ventana y se entrega la vista.

mod keys;
mod metrics;
mod view;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{App, Application, Bounds, WindowBounds, WindowOptions, px, size};
use port_plugin_api::{KeyAction, PluginRegistry};
use port_plugin_transparency::TransparencyPlugin;
use port_term_core::pty::PtyConfig;
use port_term_core::session::Session;

use crate::metrics::Metrics;
use crate::view::TerminalView;

/// Tamaño de fuente inicial. El resto de la geometría sale de ahí.
const FONT_SIZE: f32 = 14.0;
/// Margen interior de la ventana, en píxeles lógicos.
const PADDING: f32 = 10.0;
/// Tamaño inicial de la ventana, en píxeles lógicos.
const INITIAL_SIZE: (f32, f32) = (960.0, 620.0);

fn main() {
    Application::new().run(|cx: &mut App| {
        let metrics = Metrics::new(FONT_SIZE, PADDING);
        let (width, height) = INITIAL_SIZE;
        let bounds = Bounds::centered(None, size(px(width), px(height)), cx);

        let session = Rc::new(RefCell::new(
            Session::spawn(PtyConfig::default(), metrics.grid_for(width, height))
                .expect("no se pudo arrancar el shell"),
        ));

        let mut plugin_registry = PluginRegistry::new();
        plugin_registry.register(TransparencyPlugin::default());
        let plugins = Rc::new(RefCell::new(plugin_registry));

        // En una terminal cada tecla le pertenece al PTY, salvo que un plugin
        // registrado decida consumirla primero.
        let session_for_keys = Rc::clone(&session);
        let plugins_for_keys = Rc::clone(&plugins);
        cx.intercept_keystrokes(move |ev, window, _cx| {
            if let Some(key) = keys::to_core_key(&ev.keystroke) {
                // Si un plugin consume la tecla (ej. atajo de tab o sidebar),
                // no se envía al PTY.
                if plugins_for_keys.borrow().dispatch_key(&key) == KeyAction::Consume {
                    window.refresh();
                    return;
                }
                let mut session = session_for_keys.borrow_mut();
                let mode = session.cursor_key_mode();
                if let Some(bytes) = port_term_core::input::encode(&key, mode) {
                    let _ = session.write(&bytes);
                }
            }
            window.refresh();
        })
        .detach();

        let session_for_view = Rc::clone(&session);
        let metrics_for_view = metrics.clone();
        let plugins_for_view = Rc::clone(&plugins);

        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    app_id: Some("port".to_string()),
                    ..Default::default()
                },
                move |_, cx| {
                    let focus_handle = cx.focus_handle();
                    cx.new(|_| {
                        TerminalView::new(
                            session_for_view,
                            metrics_for_view,
                            focus_handle,
                            plugins_for_view,
                        )
                    })
                },
            )
            .expect("no se pudo abrir la ventana");

        // Latido: mantiene la ventana al día ante eventos del PTY y compositor.
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(16)).await;
                window.update(cx, |_, window, _cx| window.refresh()).ok();
            }
        })
        .detach();

        cx.activate(true);
    });
}
