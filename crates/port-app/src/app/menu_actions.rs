//! Ejecución de las acciones que produce el menú gestor de plugins.
//!
//! Responsabilidad única: aplicar un [`MenuAction`] ya resuelto sobre las
//! piezas vivas de PORT —el registro de plugins, la lista de externos vivos,
//! las sesiones, el archivo de configuración y el refresco de la ventana—. El
//! menú decide qué hacer; aquí solo se ejecuta, así que el interceptor de
//! teclado queda como simple despacho de la tecla.

use std::rc::Rc;
use std::sync::Arc;

use gpui::App;

use crate::app::plugins::{
    apply_mouse_policy, read_external_configs, refresh_externals, stop_external,
};
use crate::app::Handles;
use crate::menu::{ExternalPluginInfo, MenuAction};

/// Ejecuta una acción del menú sobre las piezas vivas de la terminal.
///
/// Recibe el estado del menú y los manejadores que capturó el interceptor de
/// teclado. Casi todas las acciones son síncronas; [`MenuAction::StartInstall`]
/// delega la clonación y la compilación al pool de hilos y refresca la ventana
/// cuando termina, por eso recibe el manejador de la ventana.
pub(crate) fn execute(action: MenuAction, handles: &Handles<'_>, cx: &mut App) {
    // Los manejadores siguen siendo de `run_terminal`: aquí solo se prestan y
    // se desestructuran para que el cuerpo los nombre igual que antes.
    let &Handles {
        sessions,
        plugins,
        menu_state,
        external_plugins,
        config_path,
        window_handle,
    } = handles;
    match action {
        MenuAction::Close => {
            menu_state.borrow_mut().open = false;
        }
        MenuAction::ToggleCompiled(idx) => {
            {
                let mut reg = plugins.borrow_mut();
                if let Some(_new_status) = reg.toggle_enabled_at(idx) {
                    let _ = reg.save_all_to_default_file();
                }
            }
            // Guardar la configuración también alimenta a los
            // externos: el archivo es el mismo para todos.
            let configs = read_external_configs(config_path);
            port_plugin_api::host::configure_externals(&external_plugins.borrow(), &configs);
            // El toggle cambió el registro: la política vigente
            // se reaplica a las sesiones vivas.
            apply_mouse_policy(&mut sessions.borrow_mut(), &plugins.borrow());
        }
        MenuAction::ToggleExternal(idx) => {
            let mut s = menu_state.borrow_mut();
            if let Some(p_info) = s.external_plugins.get_mut(idx) {
                if p_info.running {
                    p_info.running = false;
                    p_info.enabled = false;
                    let id = p_info.id.clone();
                    eprintln!("[tienda] plugin {id} desactivado desde el menu");
                    // La apariencia se compone en el registro:
                    // reflejar la lista nueva hace que el
                    // externo apagado deje de aportar ya.
                    stop_external(plugins, external_plugins, &id);
                    // Persistir el apagado: sin esto, el próximo
                    // arranque de PORT lo reactivaría.
                    if let Err(e) =
                        port_plugin_api::ConfigFile::set_enabled(config_path, &id, false)
                    {
                        eprintln!("no se pudo guardar enabled=false para {id}: {e}");
                    }
                } else {
                    let id = p_info.id.clone();
                    if let Some(manifest) = port_plugin_api::host::installed()
                        .into_iter()
                        .find(|m| m.id == id)
                    {
                        match port_plugin_api::host::ExternalPlugin::start(manifest) {
                            Ok(plugin) => {
                                let configs = read_external_configs(config_path);
                                port_plugin_api::host::configure_externals(
                                    &[Arc::clone(&plugin)],
                                    &configs,
                                );
                                external_plugins.borrow_mut().push(plugin);
                                refresh_externals(plugins, external_plugins);
                                p_info.running = true;
                                p_info.enabled = true;
                                eprintln!("[tienda] plugin {id} activado desde el menu");
                                if let Err(e) =
                                    port_plugin_api::ConfigFile::set_enabled(config_path, &id, true)
                                {
                                    eprintln!("no se pudo guardar enabled=true para {id}: {e}");
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
            // Encender o apagar un externo cambia el registro:
            // la política vigente se reaplica a las sesiones.
            apply_mouse_policy(&mut sessions.borrow_mut(), &plugins.borrow());
        }
        MenuAction::StartInstall(url) => {
            let menu_state_bg = Rc::clone(menu_state);
            let external_bg = Rc::clone(external_plugins);
            let plugins_bg = Rc::clone(plugins);
            let session_bg = Rc::clone(sessions);
            let config_path_bg = config_path.to_path_buf();
            let window_bg = window_handle;
            cx.spawn(async move |cx| {
                // El trabajo pesado va al pool de hilos: clonar
                // y compilar tarda segundos y en el hilo de
                // GPUI congelaria la terminal.
                let task = cx.background_executor().spawn(async move {
                    port_plugin_api::install::install(&url).map(|manifest| {
                        let started =
                            port_plugin_api::host::ExternalPlugin::start(manifest.clone());
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
                                    let configs = read_external_configs(&config_path_bg);
                                    port_plugin_api::host::configure_externals(
                                        &[Arc::clone(&plugin)],
                                        &configs,
                                    );
                                    // Instalar deja el plugin
                                    // habilitado para el próximo
                                    // arranque.
                                    let _ = port_plugin_api::ConfigFile::set_enabled(
                                        &config_path_bg,
                                        &manifest.id,
                                        true,
                                    );
                                    external_bg.borrow_mut().push(plugin);
                                    // Recién instalado y vivo:
                                    // su apariencia entra en la
                                    // composición de inmediato.
                                    refresh_externals(&plugins_bg, &external_bg);
                                    // Un plugin nuevo puede traer
                                    // una política distinta; se
                                    // reaplica a las sesiones.
                                    apply_mouse_policy(
                                        &mut session_bg.borrow_mut(),
                                        &plugins_bg.borrow(),
                                    );
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
        MenuAction::UninstallExternal(idx) => {
            // El id se obtiene antes de tocar nada: desinstalar
            // borra archivos y no debe correr con préstamos
            // vivos sobre el estado del menú.
            let target = menu_state
                .borrow()
                .external_plugins
                .get(idx)
                .map(|p| (p.id.clone(), p.running));
            match target {
                None => {
                    eprintln!("no se pudo desinstalar: no hay plugin externo en {idx}");
                }
                Some((id, was_running)) => {
                    // Un plugin vivo debe estar del todo detenido
                    // antes de borrar sus archivos: soltar su
                    // `Arc` ejecuta el `Drop` que mata el proceso.
                    if was_running {
                        if let Some(p) = menu_state.borrow_mut().external_plugins.get_mut(idx) {
                            p.running = false;
                        }
                        stop_external(plugins, external_plugins, &id);
                    }
                    match port_plugin_api::host::uninstall(&id) {
                        Ok(true) => {
                            // El plugin ya no existe: un `enabled`
                            // colgando lo reactivaría si volviera a
                            // instalarse. Se borra la clave, y si
                            // no se puede, se deja en falso.
                            if let Err(e) =
                                port_plugin_api::ConfigFile::unset(config_path, &id, "enabled")
                            {
                                eprintln!("no se pudo borrar enabled de {id}: {e}");
                                if let Err(e2) = port_plugin_api::ConfigFile::set_enabled(
                                    config_path,
                                    &id,
                                    false,
                                ) {
                                    eprintln!("no se pudo guardar enabled=false para {id}: {e2}");
                                }
                            }
                            {
                                let mut s = menu_state.borrow_mut();
                                if idx < s.external_plugins.len() {
                                    s.external_plugins.remove(idx);
                                }
                                s.pending_uninstall = None;
                                let total = s.total_items_count(plugins.borrow().len());
                                s.clamp_selection(total);
                            }
                            eprintln!("[tienda] plugin {id} desinstalado");
                        }
                        Ok(false) => {
                            eprintln!("no se pudo desinstalar plugin {id}: no está instalado");
                            menu_state.borrow_mut().pending_uninstall = None;
                        }
                        Err(e) => {
                            eprintln!("no se pudo desinstalar plugin {id}: {e}");
                            menu_state.borrow_mut().pending_uninstall = None;
                        }
                    }
                }
            }
        }
        MenuAction::Refresh | MenuAction::None => {}
    }
}
