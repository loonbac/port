//! Hooks que un plugin puede implementar y el contrato base `Plugin`.
//!
//! Esta pieza define *qué* puede aportar un plugin (apariencia, entrada, ratón,
//! layout, espacios y ciclo de vida) y el trait [`Plugin`] que devuelve esos
//! hooks. No ejecuta nada ni lleva estado: el registro es quien los invoca y
//! quien aplica la política de watchdog.

use std::path::Path;
use std::sync::Arc;

use gpui::AnyElement;
use port_term_core::frame::Rgb;
use port_term_core::input::Key;
use port_term_core::pty::{PtyConfig, RunningApp};
use port_term_core::session::MousePolicy;

use crate::config::PluginConfig;
use crate::services::Service;

/// Acción resultante tras procesar una pulsación en un hook de entrada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// La tecla continúa su curso normal hacia el PTY u otros plugins.
    Pass,
    /// El plugin consumió la tecla. No se envía al PTY.
    Consume,
}

/// Capacidades de personalización visual (fondo, opacidad, paleta, fuentes).
pub trait AppearanceHook {
    /// Opacidad deseada para el fondo de la ventana (0.0 translúcido ..= 1.0 opaco).
    fn opacity(&self) -> Option<f32> {
        None
    }

    /// Tinte o modificación sobre el color de fondo base de la terminal.
    fn background_tint(&self, base: Rgb) -> Rgb {
        base
    }

    /// Nombre de la familia de fuente preferida (ej. "JetBrainsMono Nerd Font Mono").
    fn font_family(&self) -> Option<String> {
        None
    }

    /// Tamaño de fuente en puntos o píxeles lógicos.
    fn font_size(&self) -> Option<f32> {
        None
    }

    /// Lista ordenada de fuentes de respaldo para símbolos y caracteres especiales.
    fn font_fallbacks(&self) -> Option<Vec<String>> {
        None
    }
}

/// Capacidades de intercepción y atajos de teclado.
pub trait InputHook {
    /// Permite a un plugin interceptar o consumir una tecla antes del PTY.
    fn on_key(&self, key: &Key) -> KeyAction {
        let _ = key;
        KeyAction::Pass
    }
}

/// Capacidades de ratón y selección.
pub trait MouseHook {
    /// Política vigente. El valor por defecto es el comportamiento de PORT.
    fn mouse_policy(&self) -> MousePolicy {
        MousePolicy::default()
    }
}

/// Capacidades de inserción de cromo en el layout de la ventana (tabs, sidebar, status).
pub trait LayoutHook {
    /// Elemento que se inserta en la parte superior (ej. barra de pestañas).
    fn top_bar(&self) -> Option<AnyElement> {
        None
    }

    /// Altura en píxeles que ocupa la barra superior, para descontarla de la rejilla PTY.
    fn top_bar_height(&self) -> f32 {
        0.0
    }

    /// Elemento que se inserta a la izquierda (ej. barra de espacios/sidebar).
    fn left_sidebar(&self) -> Option<AnyElement> {
        None
    }

    /// Ancho en píxeles que ocupa la barra lateral izquierda, para descontarla de la rejilla PTY.
    fn left_sidebar_width(&self) -> f32 {
        0.0
    }

    /// Elemento que se inserta en la parte inferior (ej. barra de estado).
    fn bottom_bar(&self) -> Option<AnyElement> {
        None
    }

    /// Altura en píxeles que ocupa la barra inferior, para descontarla de la rejilla PTY.
    fn bottom_bar_height(&self) -> f32 {
        0.0
    }
}

/// Capacidades de gestión de espacios de trabajo y sesiones múltiples (como Herdr).
pub trait SpaceHook {
    /// Identificador de la sesión PTY activa que debe recibir entrada y renderizarse.
    ///
    /// El registro central consulta este identificador a través de
    /// [`crate::registry::PluginRegistry::active_session_id`], que devuelve
    /// `Option<usize>` (`None` si ningún plugin activo responde o implementa el
    /// hook). Cuando devuelve `None`, el host conserva la sesión que ya está en
    /// pantalla en lugar de seleccionar forzosamente la sesión 0.
    fn active_session(&self) -> usize {
        self.active_space()
    }

    /// Índice del espacio actualmente activo.
    fn active_space(&self) -> usize {
        0
    }

    /// Comprueba si el plugin solicita crear una nueva sesión de shell/espacio/pestaña en el núcleo.
    /// Consume la solicitud y devuelve `true` si se debe crear una nueva sesión.
    fn take_new_session_request(&self) -> bool {
        self.take_new_space_request()
    }

    /// Petición de una sesión nueva que ejecuta un programa propio en lugar del
    /// shell por defecto (por ejemplo, un visor de otro proceso). `None` si el
    /// plugin no pide nada.
    fn take_spawn_session_request(&self) -> Option<PtyConfig> {
        None
    }

    /// Notifica al plugin el identificador de la sesión recién creada en el núcleo.
    fn on_session_created(&self, session_id: usize) {
        let _ = session_id;
    }

    /// Notifica al plugin el directorio de trabajo actual y nombre de carpeta de una sesión.
    fn update_session_cwd(&self, session_id: usize, cwd: &Path, folder_name: &str) {
        let _ = (session_id, cwd, folder_name);
    }

    /// Notifica al plugin qué programa se está ejecutando en primer plano en una sesión.
    fn update_session_app(&self, session_id: usize, app: Option<&RunningApp>) {
        let _ = (session_id, app);
    }

    /// Comprueba si el plugin solicita cerrar una sesión específica.
    fn take_close_session_request(&self) -> Option<usize> {
        self.take_close_space_request()
    }

    /// Notifica al plugin de que una sesión desapareció porque su shell terminó.
    /// El plugin debe quitar su pestaña y, si el espacio se queda sin pestañas,
    /// eliminar también ese espacio.
    fn on_session_closed(&self, session_id: usize) {
        let _ = session_id;
    }

    /// Compatibilidad previa.
    fn take_new_space_request(&self) -> bool {
        false
    }

    /// Compatibilidad previa.
    fn take_close_space_request(&self) -> Option<usize> {
        None
    }
}

/// Decisión que un plugin toma cuando el usuario pide cerrar la ventana.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseDecision {
    /// Cerrar sin preguntar.
    Allow,
    /// Bloquear el cierre y pedir confirmación al usuario.
    Confirm,
}

/// Capacidades sobre el ciclo de vida de la ventana.
pub trait LifecycleHook {
    /// Se consulta al pedir cerrar la ventana. `Confirm` cancela el cierre y
    /// deja que el plugin muestre su diálogo.
    fn on_close_request(&self) -> CloseDecision {
        CloseDecision::Allow
    }

    /// Notifica al plugin de que el cierre ya fue aceptado, para que limpia su
    /// estado de diálogo si lo tenía abierto.
    fn on_close_confirmed(&self) {}
}

/// Interfaz base que todo plugin de PORT debe implementar.
pub trait Plugin: 'static {
    /// Identificador único del plugin en formato kebab-case (ej. "transparency", "font-zoom").
    fn id(&self) -> &'static str;

    /// Nombre legible del plugin.
    fn name(&self) -> &'static str;

    /// Versión semántica del plugin.
    fn version(&self) -> &'static str {
        "0.1.0"
    }

    /// Hook de apariencia, si el plugin lo implementa.
    fn appearance_hook(&self) -> Option<&dyn AppearanceHook> {
        None
    }

    /// Hook de entrada, si el plugin lo implementa.
    fn input_hook(&self) -> Option<&dyn InputHook> {
        None
    }

    /// Hook de ratón y selección, si el plugin lo implementa.
    fn mouse_hook(&self) -> Option<&dyn MouseHook> {
        None
    }

    /// Hook de layout, si el plugin lo implementa.
    fn layout_hook(&self) -> Option<&dyn LayoutHook> {
        None
    }

    /// Hook de gestión de espacios de trabajo y sesiones, si el plugin lo implementa.
    fn space_hook(&self) -> Option<&dyn SpaceHook> {
        None
    }

    /// Hook de ciclo de vida de la ventana, si el plugin lo implementa.
    fn lifecycle_hook(&self) -> Option<&dyn LifecycleHook> {
        None
    }

    /// Capacidades que este plugin publica para que otros plugins las utilization.
    /// El registro las publica en el directorio de servicios al registrarlo.
    fn services(&self) -> Vec<Arc<dyn Service>> {
        Vec::new()
    }

    /// Configuración por defecto que este plugin define al crear el archivo.
    fn default_config(&self) -> Option<PluginConfig> {
        None
    }

    /// Carga la configuración persistida leída del archivo para este plugin.
    fn load_config(&self, config: &PluginConfig) {
        let _ = config;
    }

    /// Configuración actual que este plugin desea guardar en el archivo.
    fn save_config(&self) -> Option<PluginConfig> {
        None
    }
}
