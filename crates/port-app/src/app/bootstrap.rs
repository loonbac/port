//! Servicios de arranque de PORT: lo que hay que levantar y mantener vivo para
//! que la ventana funcione.
//!
//! Responsabilidad única: reunir el arranque de los plugins externos, la
//! lectura y la vigilancia del archivo de configuración y el latido que
//! redibuja la terminal cuando el PTY cambia. La ventana y su bucle de eventos
//! siguen viviendo en `run_terminal`; aquí solo está lo que la alimenta.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, WindowHandle};
use port_plugin_api::{PluginConfig, PluginRegistry};
use port_term_core::session::SessionManager;

use crate::app::plugins::{apply_mouse_policy, read_external_configs};
use crate::menu::ExternalPluginInfo;
use crate::view::TerminalView;

/// Lee —o crea— la configuración central y devuelve su ruta con los bloques
/// externos ya parseados.
pub(crate) fn load_config(
    plugin_registry: &mut PluginRegistry,
) -> (PathBuf, BTreeMap<String, PluginConfig>) {
    // La configuración se carga ANTES de arrancar externos: el `enabled`
    // persistido decide quién arranca. `load_or_create_default_config`
    // crea el archivo con los defaults de los plugins en proceso.
    let config_path = plugin_registry
        .load_or_create_default_config()
        .unwrap_or_else(|_| port_plugin_api::ConfigFile::default_path());
    let external_configs = read_external_configs(&config_path);
    (config_path, external_configs)
}

/// Arranca los plugins externos que la configuración deja habilitados y arma
/// la lista que ve el menú.
///
/// Devuelve los procesos vivos y la información del menú, incluidos los
/// apagados por configuración, que se listan igual para poder encenderlos.
pub(crate) fn start_external_plugins(
    external_configs: &BTreeMap<String, PluginConfig>,
) -> (
    Vec<Arc<port_plugin_api::host::ExternalPlugin>>,
    Vec<ExternalPluginInfo>,
) {
    // Plugins externos instalados con `port plugin add` o desde el menú.
    // Se cargan y arrancan sin bloquear el inicio si alguno falla.
    let installed = port_plugin_api::host::installed();
    let mut running_external: Vec<Arc<port_plugin_api::host::ExternalPlugin>> = Vec::new();
    let mut external_infos: Vec<ExternalPluginInfo> = Vec::new();

    for manifest in port_plugin_api::host::enabled_manifests(installed.clone(), external_configs) {
        match port_plugin_api::host::ExternalPlugin::start(manifest.clone()) {
            Ok(plugin) => {
                // El bloque del plugin llega una vez arrancado, igual que
                // en cada recarga del archivo de configuración.
                port_plugin_api::host::configure_externals(
                    &[Arc::clone(&plugin)],
                    external_configs,
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

    (running_external, external_infos)
}

/// Lanza el latido que redibuja la ventana mientras la terminal esté viva.
///
/// Conserva `last_cwd` y `last_app` entre iteraciones para redibujar solo ante
/// un cambio real.
pub(crate) fn spawn_poller(
    cx: &mut App,
    session_manager: &Rc<RefCell<SessionManager>>,
    plugins: &Rc<RefCell<PluginRegistry>>,
    window: WindowHandle<TerminalView>,
) {
    // Latido reactivo: redibuja cuando el PTY tiene datos nuevos, cuando el
    // directorio de trabajo cambia (cd) o cuando cambia el programa en primer plano.
    let session_for_poller = Rc::clone(session_manager);
    let plugins_for_poller = Rc::clone(plugins);
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
}

/// Lanza la vigilancia del archivo de configuración.
///
/// Ante cualquier cambio en disco recarga el registro, reenvía su bloque a
/// cada plugin externo, reaplica la política vigente a las sesiones y refresca
/// la ventana.
pub(crate) fn spawn_config_watcher(
    cx: &mut App,
    session_manager: &Rc<RefCell<SessionManager>>,
    plugins: &Rc<RefCell<PluginRegistry>>,
    external_plugins: &Rc<RefCell<Vec<Arc<port_plugin_api::host::ExternalPlugin>>>>,
    config_path: &Path,
    window: WindowHandle<TerminalView>,
) {
    // Vigila el archivo de configuración y lo recarga en tiempo real ante cualquier cambio en disco.
    let plugins_for_watcher = Rc::clone(plugins);
    let external_plugins_for_watcher = Rc::clone(external_plugins);
    let session_for_watcher = Rc::clone(session_manager);
    let config_path_for_watcher = config_path.to_path_buf();
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
}
