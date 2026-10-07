//! PORT: arranque y composición.
//!
//! Aquí solo se conectan piezas: se arranca el shell, se decide el tamaño
//! inicial de la ventana y se entrega la vista.

mod app;

mod cli;
mod keys;
mod menu;
mod metrics;
mod view;

use std::cell::RefCell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    px, size, App, Application, Bounds, WindowBackgroundAppearance, WindowBounds, WindowOptions,
};
use port_plugin_api::{CloseDecision, PluginRegistry};
use port_plugin_herdr::HerdrPlugin;
use port_plugin_selection::SelectionPlugin;
use port_term_core::pty::PtyConfig;
use port_term_core::session::SessionManager;

use crate::app::plugins::apply_mouse_policy;
use crate::menu::MenuState;
use crate::metrics::Metrics;
use crate::view::{ClosePromptState, TerminalView};

/// Tamaño de fuente base de fábrica si no hay configuración.
const BASE_FONT_SIZE: f32 = 14.0;
/// Margen interior de la ventana, en píxeles lógicos.
const PADDING: f32 = 10.0;
/// Tamaño inicial de la ventana, en píxeles lógicos.
const INITIAL_SIZE: (f32, f32) = (960.0, 620.0);

fn main() {
    // Los subcomandos de gestion de plugins se resuelven antes de abrir
    // ninguna ventana: instalar un plugin no necesita sesion grafica.
    match cli::parse(std::env::args()) {
        cli::Command::RunTerminal => run_terminal(),
        cli::Command::Help => {
            println!("{}", cli::HELP);
        }
        cli::Command::Version => {
            println!("{}", cli::version());
        }
        cli::Command::Plugin(command) => {
            std::process::exit(cli::run(command));
        }
    }
}

fn run_terminal() {
    Application::new().run(|cx: &mut App| {
        // Registro de plugins y carga de configuración central
        let mut plugin_registry = PluginRegistry::new();
        // herdr dibuja con GPUI, asi que va compilado dentro de PORT.
        plugin_registry.register(HerdrPlugin::new());
        // La politica de raton y seleccion la posee el plugin: la UI y el
        // nucleo la consultan antes de cada gesto.
        plugin_registry.register(SelectionPlugin::new());
        let close_prompt = Rc::new(RefCell::new(ClosePromptState::default()));

        let (config_path, external_configs) = app::bootstrap::load_config(&mut plugin_registry);
        let (running_external, external_infos) =
            app::bootstrap::start_external_plugins(&external_configs);

        // El registro compone la apariencia de ambas fuentes, así que recibe la
        // lista de externos vivos antes de calcular el tamaño inicial: un
        // `font` externo también decide el tamaño de arranque.
        plugin_registry.set_externals(running_external.clone());

        // El tamaño de fuente inicial se deriva de los plugins (ej. font-zoom)
        let initial_font_size = plugin_registry.effective_font_size(BASE_FONT_SIZE);
        let metrics = Metrics::new(initial_font_size, PADDING);
        let (width, height) = INITIAL_SIZE;
        let bounds = Bounds::centered(None, size(px(width), px(height)), cx);

        let session_manager = Rc::new(RefCell::new(
            SessionManager::new(PtyConfig::default(), metrics.grid_for(width, height))
                .expect("no se pudo arrancar el shell"),
        ));
        // La primera sesión también nace con la política vigente, no con su
        // default interno: el registro ya compone la lista de externos vivos.
        apply_mouse_policy(&mut session_manager.borrow_mut(), &plugin_registry);

        let external_plugins = Rc::new(RefCell::new(running_external));
        let plugins = Rc::new(RefCell::new(plugin_registry));
        let menu_state = Rc::new(RefCell::new(MenuState {
            external_plugins: external_infos,
            ..Default::default()
        }));

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

        // En una terminal cada tecla le pertenece al PTY, salvo que el menú de plugins
        // esté activo o un plugin decida consumirla.
        let session_for_keys = Rc::clone(&session_manager);
        let plugins_for_keys = Rc::clone(&plugins);
        let menu_state_for_keys = Rc::clone(&menu_state);
        let close_prompt_for_keys = Rc::clone(&close_prompt);
        let external_plugins_for_keys = Rc::clone(&external_plugins);
        let config_path_for_keys = config_path.clone();
        let window_for_install = window;
        cx.intercept_keystrokes(move |ev, window, cx| {
            app::input::handle_key(
                ev,
                window,
                cx,
                &close_prompt_for_keys,
                &app::Handles {
                    sessions: &session_for_keys,
                    plugins: &plugins_for_keys,
                    menu_state: &menu_state_for_keys,
                    external_plugins: &external_plugins_for_keys,
                    config_path: &config_path_for_keys,
                    window_handle: window_for_install.into(),
                },
            );
        })
        .detach();

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

                let ask = plugins_for_close.borrow().close_decision() == CloseDecision::Confirm;
                if ask {
                    {
                        let mut prompt = prompt_for_close.borrow_mut();
                        prompt.programs = bins;
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

        app::bootstrap::spawn_poller(cx, &session_manager, &plugins, window);

        app::bootstrap::spawn_config_watcher(
            cx,
            &session_manager,
            &plugins,
            &external_plugins,
            &config_path,
            window,
        );

        cx.activate(true);
    });
}
