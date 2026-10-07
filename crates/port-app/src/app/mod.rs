//! Módulos de apoyo del arranque de la terminal.
//!
//! El arranque (`main.rs`) queda con la ventana y su bucle de eventos; lo que
//! tiene contrato propio vive aquí.

pub mod input;
pub mod menu_actions;
pub mod plugins;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use gpui::AnyWindowHandle;
use port_plugin_api::PluginRegistry;
use port_term_core::session::SessionManager;

use crate::menu::MenuState;

/// Referencias a las piezas vivas que comparten el interceptor de teclado y la
/// ejecución de las acciones del menú.
///
/// No posee estado: solo presta los manejadores que `run_terminal` sigue
/// poseyendo. Agruparlos deja que cada función extraída reciba un único
/// parámetro en lugar de la lista completa, sin que el interceptor ni el menú
/// pasen a ser dueños de las sesiones, del registro o de la ventana.
pub(crate) struct Handles<'a> {
    /// Gestor de sesiones del PTY.
    pub sessions: &'a Rc<RefCell<SessionManager>>,
    /// Registro de plugins compilados.
    pub plugins: &'a Rc<RefCell<PluginRegistry>>,
    /// Estado del menú gestor de plugins.
    pub menu_state: &'a Rc<RefCell<MenuState>>,
    /// Procesos de los plugins externos vivos.
    pub external_plugins: &'a Rc<RefCell<Vec<Arc<port_plugin_api::host::ExternalPlugin>>>>,
    /// Ruta del archivo de configuración de plugins.
    pub config_path: &'a Path,
    /// Ventana de la terminal, para refrescar desde trabajo en segundo plano.
    pub window_handle: AnyWindowHandle,
}
