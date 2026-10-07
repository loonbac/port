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
use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    px, size, App, Application, Bounds, WindowBackgroundAppearance, WindowBounds, WindowOptions,
};
use port_plugin_api::{CloseDecision, PluginRegistry};
use port_plugin_herdr::HerdrPlugin;
use port_plugin_selection::SelectionPlugin;
use port_term_core::pty::PtyConfig;
use port_term_core::session::SessionManager;

use crate::app::plugins::{apply_mouse_policy, read_external_configs};
use crate::menu::{ExternalPluginInfo, MenuState};
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

        // La configuración se carga ANTES de arrancar externos: el `enabled`
        // persistido decide quién arranca. `load_or_create_default_config`
        // crea el archivo con los defaults de los plugins en proceso.
        let config_path = plugin_registry
            .load_or_create_default_config()
            .unwrap_or_else(|_| port_plugin_api::ConfigFile::default_path());
        let external_configs = read_external_configs(&config_path);

        // Plugins externos instalados con `port plugin add` o desde el menú.
        // Se cargan y arrancan sin bloquear el inicio si alguno falla.
        let installed = port_plugin_api::host::installed();
        let mut running_external: Vec<Arc<port_plugin_api::host::ExternalPlugin>> = Vec::new();
        let mut external_infos: Vec<ExternalPluginInfo> = Vec::new();

        for manifest in
            port_plugin_api::host::enabled_manifests(installed.clone(), &external_configs)
        {
            match port_plugin_api::host::ExternalPlugin::start(manifest.clone()) {
                Ok(plugin) => {
                    // El bloque del plugin llega una vez arrancado, igual que
                    // en cada recarga del archivo de configuración.
                    port_plugin_api::host::configure_externals(
                        &[Arc::clone(&plugin)],
                        &external_configs,
                    );
                    running_external.push(plugin);
                    external_infos.push(ExternalPluginInfo {
                        id: manifest.id,
                        name: manifest.name,
                        version: manifest.version,
                        enabled: true,
                        running: true,
                    });
                }
                Err(e) => {
                    eprintln!("plugin no cargado: {e}");
                    external_infos.push(ExternalPluginInfo {
                        id: manifest.id,
                        name: manifest.name,
                        version: manifest.version,
                        // La configuración lo deja habilitado, pero el
                        // proceso no arrancó: el menú refleja ambas cosas.
                        enabled: true,
                        running: false,
                    });
                }
            }
        }

        // Los apagados por configuración se listan igual, para poder encenderlos.
        for manifest in installed {
            if external_infos.iter().any(|info| info.id == manifest.id) {
                continue;
            }
            external_infos.push(ExternalPluginInfo {
                id: manifest.id,
                name: manifest.name,
                version: manifest.version,
                enabled: false,
                running: false,
            });
        }
        if !running_external.is_empty() {
            eprintln!("{} plugin(s) externo(s) cargados", running_external.len());
        }

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

        // Latido reactivo: redibuja cuando el PTY tiene datos nuevos, cuando el
        // directorio de trabajo cambia (cd) o cuando cambia el programa en primer plano.
        let session_for_poller = Rc::clone(&session_manager);
        let plugins_for_poller = Rc::clone(&plugins);
        let window_for_poller = window;
        let mut last_cwd = None;
        let mut last_app = None;
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
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
                    window_for_poller
                        .update(cx, |_, window, _cx| {
                            window.remove_window();
                        })
                        .ok();
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
                    window_for_poller
                        .update(cx, |_, window, _cx| window.refresh())
                        .ok();
                }
            }
        })
        .detach();

        // Vigila el archivo de configuración y lo recarga en tiempo real ante cualquier cambio en disco.
        let plugins_for_watcher = Rc::clone(&plugins);
        let external_plugins_for_watcher = Rc::clone(&external_plugins);
        let session_for_watcher = Rc::clone(&session_manager);
        let config_path_for_watcher = config_path.clone();
        let window_for_watcher = window;
        cx.spawn(async move |cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let reloaded = plugins_for_watcher
                .borrow_mut()
                .reload_if_modified()
                .unwrap_or(false);
            if reloaded {
                // El archivo es compartido: al recargarlo también se reenvía
                // su bloque a cada plugin externo.
                let configs = read_external_configs(&config_path_for_watcher);
                port_plugin_api::host::configure_externals(
                    &external_plugins_for_watcher.borrow(),
                    &configs,
                );
                // Recargar el archivo cambia el registro: la política vigente
                // se reaplica a las sesiones vivas.
                apply_mouse_policy(
                    &mut session_for_watcher.borrow_mut(),
                    &plugins_for_watcher.borrow(),
                );
                window_for_watcher
                    .update(cx, |_, window, _cx| window.refresh())
                    .ok();
            }
        })
        .detach();

        cx.activate(true);
    });
}
