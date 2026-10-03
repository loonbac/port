//! Contratos, capacidades y registro de plugins de PORT.
//!
//! Este crate define la API pública con la que interactúan los plugins.
//! Los plugins son componentes in-process desacoplados que extienden
//! la apariencia, la entrada, el layout, el menú o la configuración de la terminal sin tocar el núcleo.

pub mod config;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use gpui::AnyElement;
use port_term_core::frame::Rgb;
use port_term_core::input::Key;
use port_term_core::pty::RunningApp;

pub use config::{ConfigFile, PluginConfig};

/// Argumento de una llamada entre plugins.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Unit,
    Num(f32),
    Text(String),
}

/// Resultado devuelto por un servicio.
#[derive(Debug, Clone, PartialEq)]
pub enum Ret {
    Unit,
    Num(f32),
    Text(String),
    Bool(bool),
}

/// Error al invocar un servicio.
#[derive(Debug, Clone, PartialEq)]
pub enum ServiceError {
    /// Ese identificador no corresponde a ningún plugin activo.
    UnknownService(String),
    /// El servicio existe pero no expone esa acción.
    UnknownAction {
        service: String,
        action: String,
    },
    /// La acción se ejecutó pero devolvió un fallo.
    Failed(String),
}

/// Capacidad que un plugin publica para que otros puedan utilizarla.
///
/// Es la vía nativa para que un plugin llame a otro sin conocer su tipo
/// concreto: se pide por identificador estable y se invoca una acción por nombre.
pub trait Service: Send + Sync {
    /// Identificador estable con el que se busca el servicio. Suele coincidir con
    /// el `id()` del plugin que lo publica.
    fn id(&self) -> &str;

    /// Nombre legible, para diagnóstico y para la UI de configuración.
    fn name(&self) -> &str;

    /// Acciones que este servicio publica.
    fn actions(&self) -> Vec<&'static str>;

    /// Ejecuta una acción. `None` si la acción no existe en este servicio.
    fn invoke(&self, action: &str, args: &[Arg]) -> Option<Result<Ret, ServiceError>>;
}

/// Directorio de servicios publicados por los plugins.
///
/// Vive detrás de un `Arc` compartido: el registro lo crea, cada plugin publica
/// lo que expone y cualquier otro lo consulta por identificador, sin conocer su
/// implementación.
#[derive(Default)]
pub struct Services {
    inner: RwLock<HashMap<String, Arc<dyn Service>>>,
}

impl Services {
    pub fn new() -> Self {
        Self::default()
    }

    /// Publica un servicio, reemplazando cualquier versión anterior.
    pub fn publish(&self, service: Arc<dyn Service>) {
        let id = service.id().to_string();
        self.inner.write().unwrap().insert(id, service);
    }

    /// `true` si algún plugin activo publica ese servicio.
    pub fn has(&self, service_id: &str) -> bool {
        self.inner.read().unwrap().contains_key(service_id)
    }

    /// Invoca una acción sobre un servicio publicado por otro plugin.
    pub fn call(&self, service_id: &str, action: &str, args: &[Arg]) -> Result<Ret, ServiceError> {
        let service = {
            let map = self.inner.read().unwrap();
            match map.get(service_id) {
                Some(service) => Arc::clone(service),
                None => return Err(ServiceError::UnknownService(service_id.to_string())),
            }
        };

        service
            .invoke(action, args)
            .unwrap_or_else(|| {
                Err(ServiceError::UnknownAction {
                    service: service_id.to_string(),
                    action: action.to_string(),
                })
            })
    }

    /// Identificadores de todos los servicios publicados.
    pub fn published(&self) -> Vec<String> {
        let map = self.inner.read().unwrap();
        let mut ids: Vec<String> = map.keys().cloned().collect();
        ids.sort();
        ids
    }
}

/// Información básica y estado de un plugin registrado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub enabled: bool,
}

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

/// Capacidades de personalización y sustitución del menú gestor de plugins del core.
pub trait PluginManagerHook {
    /// Atajo de teclado para alternar la visibilidad del menú (por defecto "ctrl+shift+l").
    fn toggle_shortcut(&self) -> Option<&'static str> {
        None
    }

    /// Título personalizado para el encabezado del menú de plugins.
    fn menu_title(&self) -> Option<&'static str> {
        None
    }

    /// Permite a un plugin sustituir por completo el renderizado visual del menú de plugins.
    /// Recibe la lista completa de plugins registrados y el índice actualmente seleccionado.
    fn render_menu(&self, plugins: &[PluginInfo], selected_index: usize) -> Option<AnyElement> {
        let _ = (plugins, selected_index);
        None
    }
}

/// Capacidades de gestión de espacios de trabajo y sesiones múltiples (como Herdr).
pub trait SpaceHook {
    /// Identificador de la sesión PTY activa que debe recibir entrada y renderizarse.
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

    /// Hook de layout, si el plugin lo implementa.
    fn layout_hook(&self) -> Option<&dyn LayoutHook> {
        None
    }

    /// Hook para personalizar o extender el menú de gestión de plugins del core.
    fn plugin_manager_hook(&self) -> Option<&dyn PluginManagerHook> {
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

struct RegisteredPlugin {
    plugin: Box<dyn Plugin>,
    enabled: bool,
}

/// Registro central de plugins activos de la terminal.
#[derive(Default)]
pub struct PluginRegistry {
    plugins: Vec<RegisteredPlugin>,
    config_path: RefCell<Option<PathBuf>>,
    last_modified: Cell<Option<SystemTime>>,
    services: Arc<Services>,
}

impl PluginRegistry {
    /// Crea un registro de plugins vacío.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            config_path: RefCell::new(None),
            last_modified: Cell::new(None),
            services: Arc::new(Services::new()),
        }
    }

    /// Directorio de servicios, para pasarlo a plugins que llaman a otros.
    pub fn services(&self) -> Arc<Services> {
        Arc::clone(&self.services)
    }

    /// Registra un nuevo plugin en la terminal, activo por defecto.
    pub fn register<P: Plugin>(&mut self, plugin: P) {
        self.publish_services(&plugin);
        self.plugins.push(RegisteredPlugin {
            plugin: Box::new(plugin),
            enabled: true,
        });
    }

    /// Registra un plugin ya empaquetado en Box, activo por defecto.
    pub fn register_boxed(&mut self, plugin: Box<dyn Plugin>) {
        self.publish_services(plugin.as_ref());
        self.plugins.push(RegisteredPlugin {
            plugin,
            enabled: true,
        });
    }

    fn publish_services(&self, plugin: &dyn Plugin) {
        for service in plugin.services() {
            self.services.publish(service);
        }
    }

    /// Devuelve el número total de plugins registrados.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Devuelve `true` si no hay plugins registrados.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Consulta si un plugin específico está habilitado.
    pub fn is_enabled(&self, id: &str) -> bool {
        self.plugins
            .iter()
            .find(|p| p.plugin.id() == id)
            .map(|p| p.enabled)
            .unwrap_or(false)
    }

    /// Habilita o deshabilita un plugin por su identificador.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> bool {
        if let Some(entry) = self.plugins.iter_mut().find(|p| p.plugin.id() == id) {
            entry.enabled = enabled;
            true
        } else {
            false
        }
    }

    /// Alterna el estado activo/inactivo de un plugin por su identificador.
    pub fn toggle_enabled(&mut self, id: &str) -> Option<bool> {
        let entry = self.plugins.iter_mut().find(|p| p.plugin.id() == id)?;
        entry.enabled = !entry.enabled;
        Some(entry.enabled)
    }

    /// Alterna el estado activo/inactivo por índice de posición.
    pub fn toggle_enabled_at(&mut self, index: usize) -> Option<bool> {
        let entry = self.plugins.get_mut(index)?;
        entry.enabled = !entry.enabled;
        Some(entry.enabled)
    }

    /// Devuelve la lista descriptiva de todos los plugins y su estado de activación.
    pub fn list_plugins(&self) -> Vec<PluginInfo> {
        self.plugins
            .iter()
            .map(|entry| PluginInfo {
                id: entry.plugin.id().to_string(),
                name: entry.plugin.name().to_string(),
                version: entry.plugin.version().to_string(),
                enabled: entry.enabled,
            })
            .collect()
    }

    /// Atajo efectivo para abrir/cerrar el menú de plugins (por defecto "ctrl+shift+l").
    pub fn effective_menu_shortcut(&self) -> &'static str {
        for entry in &self.plugins {
            if entry.enabled {
                if let Some(hook) = entry.plugin.plugin_manager_hook() {
                    if let Some(shortcut) = hook.toggle_shortcut() {
                        return shortcut;
                    }
                }
            }
        }
        "ctrl+shift+l"
    }

    /// Título efectivo para el encabezado del menú de plugins.
    pub fn effective_menu_title(&self) -> &'static str {
        for entry in &self.plugins {
            if entry.enabled {
                if let Some(hook) = entry.plugin.plugin_manager_hook() {
                    if let Some(title) = hook.menu_title() {
                        return title;
                    }
                }
            }
        }
        "Gestor de Plugins (PORT)"
    }

    /// Renderizado visual del menú provisto por un plugin personalizado, si alguno lo define.
    pub fn render_custom_menu(&self, plugins: &[PluginInfo], selected_index: usize) -> Option<AnyElement> {
        for entry in &self.plugins {
            if entry.enabled {
                if let Some(hook) = entry.plugin.plugin_manager_hook() {
                    if let Some(element) = hook.render_menu(plugins, selected_index) {
                        return Some(element);
                    }
                }
            }
        }
        None
    }

    /// Calcula la opacidad efectiva combinando los plugins de apariencia activos.
    /// Si ningún plugin especifica opacidad, el valor por defecto es `1.0` (opaco).
    pub fn effective_opacity(&self) -> f32 {
        let mut min_opacity = 1.0f32;
        for entry in &self.plugins {
            if !entry.enabled {
                continue;
            }
            if let Some(hook) = entry.plugin.appearance_hook() {
                if let Some(op) = hook.opacity() {
                    min_opacity = min_opacity.min(op.clamp(0.0, 1.0));
                }
            }
        }
        min_opacity
    }

    /// Calcula el color de fondo efectivo aplicando los tintes de los plugins activos en orden.
    pub fn effective_background(&self, mut base: Rgb) -> Rgb {
        for entry in &self.plugins {
            if !entry.enabled {
                continue;
            }
            if let Some(hook) = entry.plugin.appearance_hook() {
                base = hook.background_tint(base);
            }
        }
        base
    }

    /// Obtiene la familia de fuente configurada por los plugins activos, o el valor por defecto.
    pub fn effective_font_family(&self, default: &str) -> String {
        for entry in &self.plugins {
            if !entry.enabled {
                continue;
            }
            if let Some(hook) = entry.plugin.appearance_hook() {
                if let Some(family) = hook.font_family() {
                    return family;
                }
            }
        }
        default.to_string()
    }

    /// Obtiene el tamaño de fuente configurado por los plugins activos, o el valor por defecto.
    pub fn effective_font_size(&self, default: f32) -> f32 {
        for entry in &self.plugins {
            if !entry.enabled {
                continue;
            }
            if let Some(hook) = entry.plugin.appearance_hook() {
                if let Some(size) = hook.font_size() {
                    return size;
                }
            }
        }
        default
    }

    /// Fuentes de respaldo acumuladas de los plugins activos.
    pub fn effective_font_fallbacks(&self) -> Vec<String> {
        let mut fallbacks = Vec::new();
        for entry in &self.plugins {
            if !entry.enabled {
                continue;
            }
            if let Some(hook) = entry.plugin.appearance_hook() {
                if let Some(fbs) = hook.font_fallbacks() {
                    fallbacks.extend(fbs);
                }
            }
        }
        if fallbacks.is_empty() {
            fallbacks = vec![
                "Symbols Nerd Font Mono".to_string(),
                "DejaVu Sans Mono".to_string(),
                "FreeMono".to_string(),
            ];
        }
        fallbacks
    }

    /// Despacha una tecla a través de los hooks de entrada de los plugins activos.
    pub fn dispatch_key(&self, key: &Key) -> KeyAction {
        for entry in &self.plugins {
            if !entry.enabled {
                continue;
            }
            if let Some(hook) = entry.plugin.input_hook() {
                if hook.on_key(key) == KeyAction::Consume {
                    return KeyAction::Consume;
                }
            }
        }
        KeyAction::Pass
    }

    /// Altura acumulada de las barras superiores activas.
    pub fn top_bar_height(&self) -> f32 {
        self.plugins
            .iter()
            .filter(|p| p.enabled)
            .filter_map(|p| p.plugin.layout_hook().map(|h| h.top_bar_height()))
            .fold(0.0f32, f32::max)
    }

    /// Ancho acumulado de las barras laterales izquierdas activas.
    pub fn left_sidebar_width(&self) -> f32 {
        self.plugins
            .iter()
            .filter(|p| p.enabled)
            .filter_map(|p| p.plugin.layout_hook().map(|h| h.left_sidebar_width()))
            .fold(0.0f32, f32::max)
    }

    /// Altura acumulada de las barras inferiores activas.
    pub fn bottom_bar_height(&self) -> f32 {
        self.plugins
            .iter()
            .filter(|p| p.enabled)
            .filter_map(|p| p.plugin.layout_hook().map(|h| h.bottom_bar_height()))
            .fold(0.0f32, f32::max)
    }

    /// Identificador de la sesión activa que debe mostrarse y recibir teclado.
    pub fn active_session_id(&self) -> usize {
        for entry in &self.plugins {
            if entry.enabled {
                if let Some(hook) = entry.plugin.space_hook() {
                    return hook.active_session();
                }
            }
        }
        0
    }

    /// Índice del espacio activo según los plugins registrados.
    pub fn active_space_index(&self) -> usize {
        for entry in &self.plugins {
            if entry.enabled {
                if let Some(hook) = entry.plugin.space_hook() {
                    return hook.active_space();
                }
            }
        }
        0
    }

    /// Comprueba si algún plugin activo solicita crear una nueva sesión de shell/espacio/pestaña.
    pub fn take_new_session_request(&self) -> bool {
        for entry in &self.plugins {
            if entry.enabled {
                if let Some(hook) = entry.plugin.space_hook() {
                    if hook.take_new_session_request() {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Notifica a los plugins el ID de la sesión recién creada.
    pub fn on_session_created(&self, session_id: usize) {
        for entry in &self.plugins {
            if entry.enabled {
                if let Some(hook) = entry.plugin.space_hook() {
                    hook.on_session_created(session_id);
                }
            }
        }
    }

    /// Notifica a los plugins el directorio de trabajo actual y nombre de carpeta de una sesión.
    pub fn update_session_cwd(&self, session_id: usize, cwd: &Path, folder_name: &str) {
        for entry in &self.plugins {
            if entry.enabled {
                if let Some(hook) = entry.plugin.space_hook() {
                    hook.update_session_cwd(session_id, cwd, folder_name);
                }
            }
        }
    }

    /// Notifica a los plugins el programa en primer plano de una sesión.
    pub fn update_session_app(&self, session_id: usize, app: Option<&RunningApp>) {
        for entry in &self.plugins {
            if entry.enabled {
                if let Some(hook) = entry.plugin.space_hook() {
                    hook.update_session_app(session_id, app);
                }
            }
        }
    }

    /// Comprueba si algún plugin activo solicita cerrar una sesión específica.
    pub fn take_close_session_request(&self) -> Option<usize> {
        for entry in &self.plugins {
            if entry.enabled {
                if let Some(hook) = entry.plugin.space_hook() {
                    if let Some(idx) = hook.take_close_session_request() {
                        return Some(idx);
                    }
                }
            }
        }
        None
    }

    /// Consulta a los plugins si el cierre de la ventana necesita confirmación.
    /// Basta con que uno pida confirmar para bloquearlo.
    pub fn close_decision(&self) -> CloseDecision {
        for entry in &self.plugins {
            if entry.enabled {
                if let Some(hook) = entry.plugin.lifecycle_hook() {
                    if hook.on_close_request() == CloseDecision::Confirm {
                        return CloseDecision::Confirm;
                    }
                }
            }
        }
        CloseDecision::Allow
    }

    /// Notifica a los plugins de que una sesión ya no existe.
    pub fn on_session_closed(&self, session_id: usize) {
        for entry in &self.plugins {
            if entry.enabled {
                if let Some(hook) = entry.plugin.space_hook() {
                    hook.on_session_closed(session_id);
                }
            }
        }
    }

    /// Notifica a los plugins que el cierre fue aceptado.
    pub fn on_close_confirmed(&self) {
        for entry in &self.plugins {
            if entry.enabled {
                if let Some(hook) = entry.plugin.lifecycle_hook() {
                    hook.on_close_confirmed();
                }
            }
        }
    }

    /// Compatibilidad previa.
    pub fn take_new_space_request(&self) -> bool {
        self.take_new_session_request()
    }

    /// Compatibilidad previa.
    pub fn take_close_space_request(&self) -> Option<usize> {
        self.take_close_session_request()
    }

    /// Recopila los elementos para la barra superior (top_bar) de los plugins activos.
    pub fn top_bars(&self) -> Vec<AnyElement> {
        let mut elements = Vec::new();
        for entry in &self.plugins {
            if !entry.enabled {
                continue;
            }
            if let Some(hook) = entry.plugin.layout_hook() {
                if let Some(elem) = hook.top_bar() {
                    elements.push(elem);
                }
            }
        }
        elements
    }

    /// Recopila los elementos para la barra lateral izquierda de los plugins activos.
    pub fn left_sidebars(&self) -> Vec<AnyElement> {
        let mut elements = Vec::new();
        for entry in &self.plugins {
            if !entry.enabled {
                continue;
            }
            if let Some(hook) = entry.plugin.layout_hook() {
                if let Some(elem) = hook.left_sidebar() {
                    elements.push(elem);
                }
            }
        }
        elements
    }

    /// Recopila los elementos para la barra inferior de los plugins activos.
    pub fn bottom_bars(&self) -> Vec<AnyElement> {
        let mut elements = Vec::new();
        for entry in &self.plugins {
            if !entry.enabled {
                continue;
            }
            if let Some(hook) = entry.plugin.layout_hook() {
                if let Some(elem) = hook.bottom_bar() {
                    elements.push(elem);
                }
            }
        }
        elements
    }

    /// Recopila las configuraciones por defecto de todos los plugins registrados,
    /// incluyendo su estado enabled = true.
    pub fn default_configs(&self) -> BTreeMap<String, PluginConfig> {
        let mut map = BTreeMap::new();
        for entry in &self.plugins {
            let mut cfg = entry.plugin.default_config().unwrap_or_default();
            if !cfg.values.contains_key("enabled") {
                cfg.set("enabled", true);
            }
            map.insert(entry.plugin.id().to_string(), cfg);
        }
        map
    }

    /// Aplica la configuración leída del archivo a cada plugin registrado correspondiente,
    /// actualizando también su estado `enabled`.
    pub fn load_configs(&mut self, configs: &BTreeMap<String, PluginConfig>) {
        for entry in &mut self.plugins {
            if let Some(cfg) = configs.get(entry.plugin.id()) {
                if let Some(enabled) = cfg.get_bool("enabled") {
                    entry.enabled = enabled;
                }
                entry.plugin.load_config(cfg);
            }
        }
    }

    /// Carga la configuración desde el archivo predeterminado (`~/.config/port/config.md`)
    /// o lo crea con los valores por defecto de los plugins si no existe.
    pub fn load_or_create_default_config(&mut self) -> std::io::Result<PathBuf> {
        let path = ConfigFile::default_path();
        self.load_or_create_config(&path)?;
        Ok(path)
    }

    /// Carga la configuración desde una ruta concreta o la crea si no existe.
    pub fn load_or_create_config(&mut self, path: &Path) -> std::io::Result<()> {
        let defaults = self.default_configs();
        let configs = ConfigFile::load_or_create(path, &defaults)?;
        self.load_configs(&configs);
        *self.config_path.borrow_mut() = Some(path.to_path_buf());
        self.last_modified
            .set(std::fs::metadata(path).and_then(|m| m.modified()).ok());
        Ok(())
    }

    /// Comprueba si el archivo de configuración fue modificado en disco desde la última
    /// lectura. Si cambió, recarga las configuraciones de los plugins y devuelve `Ok(true)`.
    pub fn reload_if_modified(&mut self) -> std::io::Result<bool> {
        let path = match self.config_path.borrow().as_ref() {
            Some(p) => p.clone(),
            None => return Ok(false),
        };

        let metadata = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };

        let current_mtime = match metadata.modified() {
            Ok(t) => t,
            Err(_) => return Ok(false),
        };

        if self.last_modified.get() == Some(current_mtime) {
            return Ok(false);
        }

        let content = std::fs::read_to_string(&path)?;
        let configs = ConfigFile::parse(&content);
        self.load_configs(&configs);
        self.last_modified.set(Some(current_mtime));
        Ok(true)
    }

    /// Guarda la configuración actual de un plugin específico en el archivo, incluyendo `enabled`.
    pub fn save_plugin_config(&self, plugin_id: &str, path: &Path) -> std::io::Result<bool> {
        for entry in &self.plugins {
            if entry.plugin.id() == plugin_id {
                let mut cfg = entry.plugin.save_config().unwrap_or_default();
                cfg.set("enabled", entry.enabled);
                ConfigFile::save_plugin(path, plugin_id, &cfg)?;
                if self.config_path.borrow().as_deref() == Some(path) {
                    self.last_modified
                        .set(std::fs::metadata(path).and_then(|m| m.modified()).ok());
                }
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Guarda la configuración y estado actual de todos los plugins registrados.
    pub fn save_all_configs(&self, path: &Path) -> std::io::Result<()> {
        for entry in &self.plugins {
            let mut cfg = entry.plugin.save_config().unwrap_or_default();
            cfg.set("enabled", entry.enabled);
            ConfigFile::save_plugin(path, entry.plugin.id(), &cfg)?;
        }
        if self.config_path.borrow().as_deref() == Some(path) {
            self.last_modified
                .set(std::fs::metadata(path).and_then(|m| m.modified()).ok());
        }
        Ok(())
    }

    /// Guarda todos los plugins en la ruta por defecto configurada (`~/.config/port/config.md`).
    pub fn save_all_to_default_file(&self) -> std::io::Result<()> {
        let path = ConfigFile::default_path();
        self.save_all_configs(&path)
    }
}
