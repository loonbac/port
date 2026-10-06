//! PORT: arranque y composición.
//!
//! Aquí solo se conectan piezas: se arranca el shell, se decide el tamaño
//! inicial de la ventana y se entrega la vista.

mod cli;
mod keys;
mod menu;
mod metrics;
mod view;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    px, size, App, Application, Bounds, WindowBackgroundAppearance, WindowBounds, WindowOptions,
};
use port_plugin_api::{CloseDecision, KeyAction, PluginRegistry};
use port_plugin_herdr::HerdrPlugin;
use port_term_core::input::Key;
use port_term_core::pty::PtyConfig;
use port_term_core::session::SessionManager;

use crate::menu::{ExternalPluginInfo, MenuAction, MenuState};
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
                let shortcut = crate::menu::MENU_SHORTCUT;
                if matches_shortcut(&key, shortcut) {
                    let mut s = menu_state_for_keys.borrow_mut();
                    if s.open {
                        s.close_menu();
                    } else {
                        s.open_menu();
                    }
                    drop(s);
                    window.refresh();
                    return;
                }

                // Si el menú está abierto, delega la entrada al módulo de menú
                if menu_state_for_keys.borrow().open {
                    // Pegar no llega como tecla: hay que ir al portapapeles.
                    // Sin esto, `Ctrl+V` caía en la rama que ignora las teclas
                    // con modificador y no pasaba absolutamente nada.
                    let pegando_url = {
                        let s = menu_state_for_keys.borrow();
                        matches!(s.mode, crate::menu::MenuMode::Adding { .. })
                    };
                    if pegando_url
                        && key.ctrl
                        && !key.alt
                        && key.key.eq_ignore_ascii_case("v")
                    {
                        if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                            menu_state_for_keys.borrow_mut().input_paste(&text);
                        }
                        window.refresh();
                        return;
                    }

                    let (total_items, compiled_count) = {
                        let s = menu_state_for_keys.borrow();
                        let c_count = plugins_for_keys.borrow().len();
                        (s.total_items_count(c_count), c_count)
                    };
                    let action = menu_state_for_keys.borrow_mut().handle_key(
                        &key,
                        total_items,
                        compiled_count,
                    );
                    match action {
                        MenuAction::Close => {
                            menu_state_for_keys.borrow_mut().open = false;
                        }
                        MenuAction::ToggleCompiled(idx) => {
                            {
                                let mut reg = plugins_for_keys.borrow_mut();
                                if let Some(_new_status) = reg.toggle_enabled_at(idx) {
                                    let _ = reg.save_all_to_default_file();
                                }
                            }
                            // Guardar la configuración también alimenta a los
                            // externos: el archivo es el mismo para todos.
                            let configs = read_external_configs(&config_path_for_keys);
                            port_plugin_api::host::configure_externals(
                                &external_plugins_for_keys.borrow(),
                                &configs,
                            );
                        }
                        MenuAction::ToggleExternal(idx) => {
                            let mut s = menu_state_for_keys.borrow_mut();
                            if let Some(p_info) = s.external_plugins.get_mut(idx) {
                                if p_info.running {
                                    p_info.running = false;
                                    p_info.enabled = false;
                                    let id = p_info.id.clone();
                                    eprintln!("[tienda] plugin {id} desactivado desde el menu");
                                    external_plugins_for_keys
                                        .borrow_mut()
                                        .retain(|p| p.manifest().id != id);
                                    // La apariencia se compone en el registro:
                                    // reflejar la lista nueva hace que el
                                    // externo apagado deje de aportar ya.
                                    refresh_externals(
                                        &plugins_for_keys,
                                        &external_plugins_for_keys,
                                    );
                                    // Persistir el apagado: sin esto, el próximo
                                    // arranque de PORT lo reactivaría.
                                    if let Err(e) = port_plugin_api::ConfigFile::set_enabled(
                                        &config_path_for_keys,
                                        &id,
                                        false,
                                    ) {
                                        eprintln!("no se pudo guardar enabled=false para {id}: {e}");
                                    }
                                } else {
                                    let id = p_info.id.clone();
                                    if let Some(manifest) = port_plugin_api::host::installed()
                                        .into_iter()
                                        .find(|m| m.id == id)
                                    {
                                        match port_plugin_api::host::ExternalPlugin::start(manifest)
                                        {
                                            Ok(plugin) => {
                                                let configs =
                                                    read_external_configs(&config_path_for_keys);
                                                port_plugin_api::host::configure_externals(
                                                    &[Arc::clone(&plugin)],
                                                    &configs,
                                                );
                                                external_plugins_for_keys.borrow_mut().push(plugin);
                                                refresh_externals(
                                                    &plugins_for_keys,
                                                    &external_plugins_for_keys,
                                                );
                                                p_info.running = true;
                                                p_info.enabled = true;
                                                eprintln!("[tienda] plugin {id} activado desde el menu");
                                                if let Err(e) =
                                                    port_plugin_api::ConfigFile::set_enabled(
                                                        &config_path_for_keys,
                                                        &id,
                                                        true,
                                                    )
                                                {
                                                    eprintln!(
                                                        "no se pudo guardar enabled=true para {id}: {e}"
                                                    );
                                                }
                                            }
                                            Err(e) => {
                                                eprintln!("no se pudo iniciar plugin {id}: {e}");
                                                p_info.running = false;
                                                // La configuración sigue diciendo
                                                // habilitado; solo el proceso
                                                // no está vivo.
                                                p_info.enabled = true;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        MenuAction::StartInstall(url) => {
                            let menu_state_bg = Rc::clone(&menu_state_for_keys);
                            let external_bg = Rc::clone(&external_plugins_for_keys);
                            let plugins_bg = Rc::clone(&plugins_for_keys);
                            let config_path_bg = config_path_for_keys.clone();
                            let window_bg = window_for_install;
                            cx.spawn(async move |cx| {
                                // El trabajo pesado va al pool de hilos: clonar
                                // y compilar tarda segundos y en el hilo de
                                // GPUI congelaria la terminal.
                                let task = cx.background_executor().spawn(async move {
                                    port_plugin_api::install::install(&url).map(|manifest| {
                                        let started =
                                            port_plugin_api::host::ExternalPlugin::start(
                                                manifest.clone(),
                                            );
                                        (manifest, started)
                                    })
                                });
                                let install_res = task.await;
                                window_bg
                                    .update(cx, move |_, window, _cx| {
                                        let mut s = menu_state_bg.borrow_mut();
                                        match install_res {
                                            Ok((manifest, start_res)) => match start_res {
                                                Ok(plugin) => {
                                                    let configs =
                                                        read_external_configs(&config_path_bg);
                                                    port_plugin_api::host::configure_externals(
                                                        &[Arc::clone(&plugin)],
                                                        &configs,
                                                    );
                                                    // Instalar deja el plugin
                                                    // habilitado para el próximo
                                                    // arranque.
                                                    let _ =
                                                        port_plugin_api::ConfigFile::set_enabled(
                                                            &config_path_bg,
                                                            &manifest.id,
                                                            true,
                                                        );
                                                    external_bg.borrow_mut().push(plugin);
                                                    // Recién instalado y vivo:
                                                    // su apariencia entra en la
                                                    // composición de inmediato.
                                                    refresh_externals(&plugins_bg, &external_bg);
                                                    s.add_or_update_external(ExternalPluginInfo {
                                                        id: manifest.id.clone(),
                                                        name: manifest.name.clone(),
                                                        version: manifest.version.clone(),
                                                        enabled: true,
                                                        running: true,
                                                    });
                                                    s.finish_install(
                                                        true,
                                                        format!(
                                                            "Plugin '{}' v{} installed and started successfully.",
                                                            manifest.name, manifest.version
                                                        ),
                                                    );
                                                }
                                                Err(err) => {
                                                    s.add_or_update_external(ExternalPluginInfo {
                                                        id: manifest.id.clone(),
                                                        name: manifest.name.clone(),
                                                        version: manifest.version.clone(),
                                                        enabled: false,
                                                        running: false,
                                                    });
                                                    s.finish_install(
                                                        false,
                                                        format!(
                                                            "Plugin '{}' v{} installed, but failed to start: {}",
                                                            manifest.name, manifest.version, err
                                                        ),
                                                    );
                                                }
                                            },
                                            Err(err) => {
                                                s.finish_install(
                                                    false,
                                                    format!("Failed to install plugin: {}", err),
                                                );
                                            }
                                        }
                                        window.refresh();
                                    })
                                    .ok();
                            })
                            .detach();
                        }
                        MenuAction::Refresh | MenuAction::None => {}
                    }
                    window.refresh();
                    return;
                }

                // Si un plugin consume la tecla (ej. atajo de tab o sidebar), no se envía al PTY
                if plugins_for_keys.borrow().dispatch_key(&key) == KeyAction::Consume {
                    window.refresh();
                    return;
                }

                // Plugins externos: resolver combinaciones registradas
                if let Some((plugin, action)) =
                    port_plugin_api::host::ExternalPlugin::resolve_action(
                        &external_plugins_for_keys.borrow(),
                        &key.key,
                        key.ctrl,
                        key.alt,
                        key.shift,
                    )
                {
                    let _ = plugin.invoke(&action);
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
                window_for_watcher
                    .update(cx, |_, window, _cx| window.refresh())
                    .ok();
            }
        })
        .detach();

        cx.activate(true);
    });
}

/// Copia al registro la lista de externos vivos.
///
/// El registro es quien compone la apariencia de las dos fuentes, así que hay
/// que avisarle en cada cambio de la lista: al arrancar, al encender o apagar
/// un externo desde el menú y al instalar uno nuevo. Un solo punto de refresco
/// evita que la vista y el registro discrepen.
fn refresh_externals(
    plugins: &Rc<RefCell<PluginRegistry>>,
    externals: &Rc<RefCell<Vec<Arc<port_plugin_api::host::ExternalPlugin>>>>,
) {
    plugins
        .borrow_mut()
        .set_externals(externals.borrow().clone());
}

/// Lee los bloques del archivo de configuración, o un mapa vacío si aún no existe.
///
/// Se usa tanto al arrancar como en cada recarga/guardado para reenviar su
/// bloque a cada plugin externo.
fn read_external_configs(
    path: &Path,
) -> std::collections::BTreeMap<String, port_plugin_api::PluginConfig> {
    std::fs::read_to_string(path)
        .map(|text| port_plugin_api::ConfigFile::parse(&text))
        .unwrap_or_default()
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
