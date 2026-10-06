//! Ciclo de vida de los plugins externos que coordina el arranque de PORT.
//!
//! Reúne las operaciones con las que `run_terminal` mantiene vivos a los
//! externos: refrescar su lista en el registro, detener uno, leer los bloques
//! del archivo de configuración y reaplicar a las sesiones la política de ratón
//! y selección vigente. El registro compone; el ciclo de vida de los procesos
//! se decide y se ejecuta aquí.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use port_plugin_api::PluginRegistry;
use port_term_core::session::SessionManager;

/// Copia al registro la lista de externos vivos.
///
/// El registro es quien compone la apariencia de las dos fuentes, así que hay
/// que avisarle en cada cambio de la lista: al arrancar, al encender o apagar
/// un externo desde el menú y al instalar uno nuevo. Un solo punto de refresco
/// evita que la vista y el registro discrepen.
pub(crate) fn refresh_externals(
    plugins: &Rc<RefCell<PluginRegistry>>,
    externals: &Rc<RefCell<Vec<Arc<port_plugin_api::host::ExternalPlugin>>>>,
) {
    plugins
        .borrow_mut()
        .set_externals(externals.borrow().clone());
}

/// Aplica a cada sesión viva la política de ratón y selección del registro.
///
/// El registro compone la política (el primer plugin con `MouseHook` gana) y la
/// sesión es la fuente de verdad que después lee la vista. Se llama en cada
/// punto donde el registro puede cambiar —arranque, encendido o apagado de un
/// plugin, instalación y recarga de la configuración—. El gestor la guarda como
/// la vigente y la aplica también a las sesiones que se creen después.
pub(crate) fn apply_mouse_policy(sessions: &mut SessionManager, registry: &PluginRegistry) {
    sessions.set_mouse_policy(registry.mouse_policy());
}

/// Detiene un plugin externo vivo y lo saca de la lista de procesos.
///
/// Soltar la última referencia `Arc` ejecuta el `Drop` de `ExternalPlugin`, que
/// manda `Shutdown` y espera al hijo: al volver, el proceso ya no existe. Se
/// refresca el registro para que su apariencia deje de contar de inmediato.
/// Devuelve `true` si había un proceso registrado con ese id.
pub(crate) fn stop_external(
    plugins: &Rc<RefCell<PluginRegistry>>,
    externals: &Rc<RefCell<Vec<Arc<port_plugin_api::host::ExternalPlugin>>>>,
    id: &str,
) -> bool {
    let removed = {
        let mut list = externals.borrow_mut();
        let before = list.len();
        list.retain(|p| p.manifest().id != id);
        before != list.len()
    };
    if removed {
        refresh_externals(plugins, externals);
    }
    removed
}

/// Lee los bloques del archivo de configuración, o un mapa vacío si aún no existe.
///
/// Se usa tanto al arrancar como en cada recarga/guardado para reenviar su
/// bloque a cada plugin externo.
pub(crate) fn read_external_configs(
    path: &Path,
) -> std::collections::BTreeMap<String, port_plugin_api::PluginConfig> {
    std::fs::read_to_string(path)
        .map(|text| port_plugin_api::ConfigFile::parse(&text))
        .unwrap_or_default()
}
