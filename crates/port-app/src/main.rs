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
use gpui::{
    App, Application, Bounds, WindowBackgroundAppearance, WindowBounds, WindowOptions, px, size,
};
use port_plugin_api::{KeyAction, PluginRegistry};
use port_plugin_close_guard::CloseGuardPlugin;
use port_plugin_font::FontPlugin;
use port_plugin_font_zoom::FontZoomPlugin;
use port_plugin_herdr::HerdrPlugin;
use port_plugin_menu_customizer::MenuCustomizerPlugin;
use port_plugin_shortcuts::ShortcutsPlugin;
use port_plugin_transparency::TransparencyPlugin;
use port_term_core::input::Key;
use port_term_core::pty::PtyConfig;
use port_term_core::session::SessionManager;

use crate::metrics::Metrics;
use crate::view::{ClosePromptState, MenuState, TerminalView};

/// Tamaño de fuente base de fábrica si no hay configuración.
const BASE_FONT_SIZE: f32 = 14.0;
/// Margen interior de la ventana, en píxeles lógicos.
const PADDING: f32 = 10.0;
/// Tamaño inicial de la ventana, en píxeles lógicos.
const INITIAL_SIZE: (f32, f32) = (960.0, 620.0);

fn main() {
    Application::new().run(|cx: &mut App| {
        // Registro de plugins y carga de configuración central
        let mut plugin_registry = PluginRegistry::new();
        plugin_registry.register(TransparencyPlugin::default());
        plugin_registry.register(FontPlugin::new("FiraCode Nerd Font Mono"));
        plugin_registry.register(FontZoomPlugin::new(BASE_FONT_SIZE));

        // Los atajos de zoom se registran en el plugin de atajos, que es el
        // unico dueno de las teclas. La llamada se resuelve por servicio, asi
        // que main no necesita conocer la implementacion de font-zoom.
        let shortcuts = ShortcutsPlugin::new().with_services(plugin_registry.services());
        shortcuts.bind_service("ctrl+=", "font-zoom", "zoom_in");
        shortcuts.bind_service("ctrl+shift+=", "font-zoom", "zoom_in");
        shortcuts.bind_service("ctrl+-", "font-zoom", "zoom_out");
        shortcuts.bind_service("ctrl+0", "font-zoom", "reset");
        plugin_registry.register(shortcuts);
        plugin_registry.register(MenuCustomizerPlugin::default());
        plugin_registry.register(HerdrPlugin::new());

        // Guardián de cierre: pregunta si hay programas corriendo al cerrar.
        let close_guard = CloseGuardPlugin::new();
        let close_prompt = Rc::new(RefCell::new(ClosePromptState::default()));
        plugin_registry.register(close_guard.clone());

        // Carga la configuración desde ~/.config/port/config.md o la crea con los defaults
        let _ = plugin_registry.load_or_create_default_config();

        // El tamaño de fuente inicial se deriva de los plugins (ej. font-zoom)
        let initial_font_size = plugin_registry.effective_font_size(BASE_FONT_SIZE);
        let metrics = Metrics::new(initial_font_size, PADDING);
        let (width, height) = INITIAL_SIZE;
        let bounds = Bounds::centered(None, size(px(width), px(height)), cx);

        let session_manager = Rc::new(RefCell::new(
            SessionManager::new(PtyConfig::default(), metrics.grid_for(width, height))
                .expect("no se pudo arrancar el shell"),
        ));

        let plugins = Rc::new(RefCell::new(plugin_registry));
        let menu_state = Rc::new(RefCell::new(MenuState::default()));

        // En una terminal cada tecla le pertenece al PTY, salvo que el menú de plugins
        // esté activo o un plugin decida consumirla.
        let session_for_keys = Rc::clone(&session_manager);
        let plugins_for_keys = Rc::clone(&plugins);
        let menu_state_for_keys = Rc::clone(&menu_state);
        let close_prompt_for_keys = Rc::clone(&close_prompt);
        cx.intercept_keystrokes(move |ev, window, _cx| {
            if let Some(key) = keys::to_core_key(&ev.keystroke) {
                // El diálogo de cierre es modal: captura el teclado entero.
                if close_prompt_for_keys.borrow().open {
                    match key.key.as_str() {
                        // Atajos directos de una tecla.
                        "Escape" | "n" | "N" => view::decline_close(&close_prompt_for_keys, window),
                        "y" | "Y" => view::accept_close(&close_prompt_for_keys, window),
                        // Navegación entre los dos botones.
                        "Left" | "Right" | "Up" | "Down" | "h" | "l" | "Tab" => {
                            {
                                let mut s = close_prompt_for_keys.borrow_mut();
                                s.selected = 1 - s.selected;
                            }
                            // Sin este repintado el foco cambiaba de estado
                            // pero se seguía viendo el botón anterior.
                            window.refresh();
                        }
                        "Enter" | "Return" | " " => {
                            let confirm = close_prompt_for_keys.borrow().selected == 1;
                            if confirm {
                                view::accept_close(&close_prompt_for_keys, window);
                            } else {
                                view::decline_close(&close_prompt_for_keys, window);
                            }
                        }
                        // Cualquier otra tecla no hace nada: el diálogo no se cierra solo.
                        _ => {}
                    }
                    return;
                }

                // Comprobación de atajo de apertura/cierre del menú de plugins (Ctrl+Shift+L por defecto)
                let shortcut = plugins_for_keys.borrow().effective_menu_shortcut();
                if matches_shortcut(&key, shortcut) {
                    let mut s = menu_state_for_keys.borrow_mut();
                    s.open = !s.open;
                    s.selected_index = 0;
                    drop(s);
                    window.refresh();
                    return;
                }

                // Si el menú está abierto, intercepta la navegación y acciones
                if menu_state_for_keys.borrow().open {
                    match key.key.as_str() {
                        "Escape" => {
                            menu_state_for_keys.borrow_mut().open = false;
                        }
                        "Up" | "k" => {
                            let mut s = menu_state_for_keys.borrow_mut();
                            s.selected_index = s.selected_index.saturating_sub(1);
                        }
                        "Down" | "j" => {
                            let mut s = menu_state_for_keys.borrow_mut();
                            let max = plugins_for_keys.borrow().len().saturating_sub(1);
                            s.selected_index = (s.selected_index + 1).min(max);
                        }
                        "Enter" | "Return" | " " => {
                            let idx = menu_state_for_keys.borrow().selected_index;
                            let mut reg = plugins_for_keys.borrow_mut();
                            if let Some(_new_status) = reg.toggle_enabled_at(idx) {
                                let _ = reg.save_all_to_default_file();
                            }
                        }
                        _ => {}
                    }
                    window.refresh();
                    return;
                }

                // Si un plugin consume la tecla (ej. atajo de tab o sidebar), no se envía al PTY
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

        let session_for_view = Rc::clone(&session_manager);
        let metrics_for_view = metrics.clone();
        let plugins_for_view = Rc::clone(&plugins);
        let menu_state_for_view = Rc::clone(&menu_state);
        let close_prompt_for_view = Rc::clone(&close_prompt);

        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_background: WindowBackgroundAppearance::Transparent,
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
                            menu_state_for_view,
                            close_prompt_for_view,
                        )
                    })
                },
            )
            .expect("no se pudo abrir la ventana");

        // Cierre protegido: si algún plugin detecta trabajo en curso, cancela el
        // cierre y muestra su diálogo. Si no, la ventana se cierra sin preguntar.
        let plugins_for_close = Rc::clone(&plugins);
        let session_for_close = Rc::clone(&session_manager);
        let prompt_for_close = Rc::clone(&close_prompt);
        let _ = window.update(cx, move |_, window, inner_cx| {
            let app: &App = inner_cx;
            window.on_window_should_close(app, move |close_window, _cx| {
                // Programas en primer plano de todas las sesiones vivas.
                let bins: Vec<String> = {
                    let mgr = session_for_close.borrow();
                    mgr.running_apps()
                        .into_iter()
                        .map(|(_, app)| app.bin)
                        .collect()
                };

                let ask = close_guard.request_close(bins);
                if ask {
                    {
                        let mut prompt = prompt_for_close.borrow_mut();
                        prompt.programs = close_guard.pending_programs();
                        prompt.selected = 0; // siempre sobre la opción segura
                        prompt.open = true;
                    }
                    // Sin este repintado el diálogo no aparecía hasta que
                    // otro evento posterior la redibujara.
                    close_window.refresh();
                } else {
                    plugins_for_close.borrow().on_close_confirmed();
                }

                // `false` cancela el cierre; `true` deja que la ventana se cierre.
                !ask
            });
        });

        // Latido reactivo: redibuja cuando el PTY tiene datos nuevos, cuando el
        // directorio de trabajo cambia (cd) o cuando cambia el programa en primer plano.
        let session_for_poller = Rc::clone(&session_manager);
        let plugins_for_poller = Rc::clone(&plugins);
        let window_for_poller = window.clone();
        let mut last_cwd = None;
        let mut last_app = None;
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(16)).await;
                let (changed, cwd, app, exited, empty) = {
                    let mut mgr = session_for_poller.borrow_mut();
                    let c = mgr.pump();
                    // Se recogen los shells terminados aquí, y no en el
                    // render: si un shell muere sin escribir nada, nunca
                    // habria un redibujado y la ventana se quedaria pegada.
                    let exited = mgr.reap_exited();
                    let empty = mgr.is_empty();
                    (c, mgr.active_cwd(), mgr.active_app(), exited, empty)
                };

                if !exited.is_empty() {
                    for id in &exited {
                        plugins_for_poller.borrow().on_session_closed(*id);
                    }
                }

                // Sin sesiones no hay terminal que mostrar: se cierra la ventana.
                if empty {
                    window_for_poller.update(cx, |_, window, _cx| {
                        window.remove_window();
                    }).ok();
                    break;
                }

                let cwd_changed = cwd != last_cwd;
                if cwd_changed {
                    last_cwd = cwd;
                }
                let app_changed = app != last_app;
                if app_changed {
                    last_app = app;
                }
                if changed || cwd_changed || app_changed || !exited.is_empty() {
                    window_for_poller.update(cx, |_, window, _cx| window.refresh()).ok();
                }
            }
        })
        .detach();

        // Vigila el archivo de configuración y lo recarga en tiempo real ante cualquier cambio en disco.
        let plugins_for_watcher = Rc::clone(&plugins);
        let window_for_watcher = window.clone();
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(150)).await;
                let reloaded = plugins_for_watcher
                    .borrow_mut()
                    .reload_if_modified()
                    .unwrap_or(false);
                if reloaded {
                    window_for_watcher.update(cx, |_, window, _cx| window.refresh()).ok();
                }
            }
        })
        .detach();

        cx.activate(true);
    });
}

/// Comprueba si una pulsación de tecla coincide con una cadena de atajo (ej. "ctrl+shift+l").
fn matches_shortcut(key: &Key, pattern: &str) -> bool {
    let parts: Vec<&str> = pattern.split('+').map(str::trim).collect();
    if parts.is_empty() {
        return false;
    }
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut target_key = "";

    for (i, part) in parts.iter().enumerate() {
        let lower = part.to_lowercase();
        if i == parts.len() - 1 {
            target_key = part;
        } else {
            match lower.as_str() {
                "ctrl" | "control" => ctrl = true,
                "alt" => alt = true,
                "shift" => shift = true,
                _ => return false,
            }
        }
    }

    if key.ctrl != ctrl || key.alt != alt || key.shift != shift {
        return false;
    }

    let k_lower = key.key.to_lowercase();
    let target_lower = target_key.to_lowercase();
    if k_lower == target_lower {
        return true;
    }
    if let Some(text) = &key.text {
        if text.to_lowercase() == target_lower {
            return true;
        }
    }
    false
}
