//! Registro central de plugins activos.
//!
//! Es el único lugar donde se combinan los plugins in-process y los externos
//! vivos, se invocan sus hooks y se persiste su configuración. Los hooks que
//! invoca están definidos en [`crate::hooks`], los servicios que publica viven
//! en [`crate::services`] y la política de salud y watchdog con que se ejecutan
//! vive en [`crate::health`].

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui::AnyElement;
use port_term_core::frame::Rgb;
use port_term_core::input::Key;
use port_term_core::pty::{PtyConfig, RunningApp};
use port_term_core::session::MousePolicy;

use crate::config::{ConfigFile, PluginConfig};
use crate::health::RegisteredPlugin;
use crate::hooks::{CloseDecision, KeyAction, Plugin};
use crate::host::ExternalPlugin;
use crate::services::Services;

pub use crate::health::{PluginHealth, DEFAULT_HOOK_BUDGET};

/// Información básica y estado de un plugin registrado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub enabled: bool,
    pub health: PluginHealth,
}

/// Registro central de plugins activos de la terminal.
pub struct PluginRegistry {
    plugins: Vec<RegisteredPlugin>,
    /// Externos vivos ahora mismo: procesos que hablaron el protocolo.
    ///
    /// El registro es el único lugar donde se combinan las dos fuentes de
    /// apariencia (in-process y externos), para que la vista siga consultando
    /// un solo objeto y no existan dos rutas de cálculo del mismo valor.
    ///
    /// La lista la refresca quien arranca y apaga los procesos
    /// (`port-app`): aquí solo se guarda lo que hoy está en ejecución, y un
    /// plugin apagado en el menú sale de la lista, con lo que deja de aportar.
    externals: Vec<Arc<ExternalPlugin>>,
    hook_budget: Duration,
    config_path: RefCell<Option<PathBuf>>,
    last_modified: Cell<Option<SystemTime>>,
    services: Arc<Services>,
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self {
            plugins: Vec::new(),
            externals: Vec::new(),
            hook_budget: DEFAULT_HOOK_BUDGET,
            config_path: RefCell::new(None),
            last_modified: Cell::new(None),
            services: Arc::new(Services::new()),
        }
    }
}

impl PluginRegistry {
    /// Crea un registro de plugins vacío con el presupuesto de hook por defecto.
    pub fn new() -> Self {
        Self::default()
    }

    /// Presupuesto de tiempo asignado a cada invocación de hook.
    pub fn hook_budget(&self) -> Duration {
        self.hook_budget
    }

    /// Configura el presupuesto de tiempo asignado a cada invocación de hook.
    pub fn set_hook_budget(&mut self, budget: Duration) {
        self.hook_budget = budget;
    }

    /// Sustituye la lista de plugins externos actualmente en ejecución.
    ///
    /// La llama `port-app` en cada cambio de la lista de vivos: al arrancar,
    /// al encender o apagar un externo desde el menú y al instalar uno nuevo.
    /// La apariencia de cada externo ya viene cacheada de su saludo, así que
    /// esto solo copia punteros: no lanza procesos ni habla por la tubería.
    pub fn set_externals(&mut self, externals: Vec<Arc<ExternalPlugin>>) {
        self.externals = externals;
    }

    /// Consulta el estado de salud de un plugin por su identificador.
    pub fn plugin_health(&self, id: &str) -> Option<PluginHealth> {
        self.plugins
            .iter()
            .find(|p| p.plugin.id() == id)
            .map(|p| p.health.get())
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
            health: Cell::new(PluginHealth::Ok),
        });
    }

    /// Registra un plugin ya empaquetado en Box, activo por defecto.
    pub fn register_boxed(&mut self, plugin: Box<dyn Plugin>) {
        self.publish_services(plugin.as_ref());
        self.plugins.push(RegisteredPlugin {
            plugin,
            enabled: true,
            health: Cell::new(PluginHealth::Ok),
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
            .map(|p| p.enabled && p.health.get() != PluginHealth::Disabled)
            .unwrap_or(false)
    }

    /// Habilita o deshabilita un plugin por su identificador.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> bool {
        if let Some(entry) = self.plugins.iter_mut().find(|p| p.plugin.id() == id) {
            if entry.health.get() == PluginHealth::Disabled {
                return false;
            }
            entry.enabled = enabled;
            true
        } else {
            false
        }
    }

    /// Alterna el estado activo/inactivo de un plugin por su identificador.
    pub fn toggle_enabled(&mut self, id: &str) -> Option<bool> {
        let entry = self.plugins.iter_mut().find(|p| p.plugin.id() == id)?;
        if entry.health.get() == PluginHealth::Disabled {
            return None;
        }
        entry.enabled = !entry.enabled;
        Some(entry.enabled)
    }

    /// Alterna el estado activo/inactivo por índice de posición.
    pub fn toggle_enabled_at(&mut self, index: usize) -> Option<bool> {
        let entry = self.plugins.get_mut(index)?;
        if entry.health.get() == PluginHealth::Disabled {
            return None;
        }
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
                health: entry.health.get(),
            })
            .collect()
    }

    /// Atajo efectivo para abrir/cerrar el menú de plugins (por defecto "ctrl+shift+l").
    /// Calcula la opacidad efectiva combinando los plugins de apariencia activos.
    /// Si ningún plugin especifica opacidad, el valor por defecto es `1.0` (opaco).
    ///
    /// Fuentes: el registro in-process y los externos vivos (una lista que
    /// mantiene al día quien arranca y apaga los procesos). Regla: gana la
    /// opacidad **mínima** de todas, que es la que ya aplicaba solo entre
    /// plugins in-process; un externo solo puede volver la ventana más
    /// translúcida, nunca tapar el ajuste de otro. Un valor fuera de rango se
    /// recorta a `0.0..=1.0`, igual que el in-process.
    pub fn effective_opacity(&self) -> f32 {
        let mut min_opacity = 1.0f32;
        for entry in &self.plugins {
            if let Some(op) = entry.invoke_hook(self.hook_budget, None, || {
                entry.plugin.appearance_hook().and_then(|h| h.opacity())
            }) {
                min_opacity = min_opacity.min(op.clamp(0.0, 1.0));
            }
        }
        for external in &self.externals {
            if let Some(op) = external.appearance().opacity {
                min_opacity = min_opacity.min(op.clamp(0.0, 1.0));
            }
        }
        min_opacity
    }

    /// Calcula el color de fondo efectivo aplicando los tintes de los plugins activos en orden.
    pub fn effective_background(&self, mut base: Rgb) -> Rgb {
        for entry in &self.plugins {
            base = entry.invoke_hook(self.hook_budget, base, || {
                match entry.plugin.appearance_hook() {
                    Some(hook) => hook.background_tint(base),
                    None => base,
                }
            });
        }
        base
    }

    /// Obtiene la familia de fuente configurada por los plugins activos, o el valor por defecto.
    ///
    /// Orden de precedencia: primero el registro **in-process** y después los
    /// externos vivos, en el orden en que están instalados. El in-process es la
    /// configuración propia del núcleo (lo que PORT trae y controla); los
    /// externos son añadidos instalados, así que no pueden pisarla. Gana el
    /// primer valor no vacío, igual que antes; un externo sin familia cede ante
    /// el siguiente.
    pub fn effective_font_family(&self, default: &str) -> String {
        for entry in &self.plugins {
            if let Some(family) = entry.invoke_hook(self.hook_budget, None, || {
                entry.plugin.appearance_hook().and_then(|h| h.font_family())
            }) {
                if !family.trim().is_empty() {
                    return family;
                }
            }
        }
        for external in &self.externals {
            if let Some(family) = external.appearance().font_family {
                if !family.trim().is_empty() {
                    return family;
                }
            }
        }
        default.to_string()
    }

    /// Obtiene el tamaño de fuente configurado por los plugins activos, o el valor por defecto.
    ///
    /// Misma precedencia que [`PluginRegistry::effective_font_family`]: primero
    /// el registro in-process —la configuración propia del núcleo— y después
    /// los externos vivos, por orden de instalación. Gana el primer valor
    /// presente.
    pub fn effective_font_size(&self, default: f32) -> f32 {
        for entry in &self.plugins {
            if let Some(size) = entry.invoke_hook(self.hook_budget, None, || {
                entry.plugin.appearance_hook().and_then(|h| h.font_size())
            }) {
                return size;
            }
        }
        for external in &self.externals {
            if let Some(size) = external.appearance().font_size {
                return size;
            }
        }
        default
    }

    /// Fuentes de respaldo acumuladas de los plugins activos.
    ///
    /// Solo mira el registro in-process: el protocolo [`protocol::Appearance`]
    /// que declaran los externos no lleva fallbacks (solo opacidad, familia,
    /// tamaño y color de fondo), así que no hay nada que componer sin ampliar
    /// el protocolo. Un externo no puede aportar respaldos hoy.
    pub fn effective_font_fallbacks(&self) -> Vec<String> {
        let mut fallbacks = Vec::new();
        for entry in &self.plugins {
            if let Some(fbs) = entry.invoke_hook(self.hook_budget, None, || {
                entry
                    .plugin
                    .appearance_hook()
                    .and_then(|h| h.font_fallbacks())
            }) {
                fallbacks.extend(fbs);
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
            let action = entry.invoke_hook(self.hook_budget, KeyAction::Pass, || {
                entry
                    .plugin
                    .input_hook()
                    .map(|h| h.on_key(key))
                    .unwrap_or(KeyAction::Pass)
            });
            if action == KeyAction::Consume {
                return KeyAction::Consume;
            }
        }
        KeyAction::Pass
    }

    /// Política de ratón y selección vigente, compuesta con los plugins activos.
    ///
    /// Regla: el **primer** plugin registrado que implemente el
    /// [`MouseHook`] es el dueño de la política; es la misma precedencia
    /// «gana el primero» que la familia de fuente. Un plugin deshabilitado o
    /// con el watchdog disparado no aporta —`invoke_hook` ya devuelve el valor
    /// de respaldo cuando corresponde— así que cede ante el siguiente, y si
    /// ninguno la aporta rige [`MousePolicy::default()`]. Leer la política pasa
    /// por el mismo aislamiento de pánicos y el mismo presupuesto de hook que
    /// el resto de hooks.
    pub fn mouse_policy(&self) -> MousePolicy {
        for entry in &self.plugins {
            if !entry.enabled || entry.health.get() == PluginHealth::Disabled {
                continue;
            }
            if entry.plugin.mouse_hook().is_some() {
                return entry.invoke_hook(self.hook_budget, MousePolicy::default(), || {
                    entry
                        .plugin
                        .mouse_hook()
                        .map(|hook| hook.mouse_policy())
                        .unwrap_or_default()
                });
            }
        }
        MousePolicy::default()
    }

    /// Altura acumulada de las barras superiores activas.
    pub fn top_bar_height(&self) -> f32 {
        let mut max_h = 0.0f32;
        for entry in &self.plugins {
            let h = entry.invoke_hook(self.hook_budget, 0.0, || {
                entry
                    .plugin
                    .layout_hook()
                    .map(|h| h.top_bar_height())
                    .unwrap_or(0.0)
            });
            max_h = max_h.max(h);
        }
        max_h
    }

    /// Ancho acumulado de las barras laterales izquierdas activas.
    pub fn left_sidebar_width(&self) -> f32 {
        let mut max_w = 0.0f32;
        for entry in &self.plugins {
            let w = entry.invoke_hook(self.hook_budget, 0.0, || {
                entry
                    .plugin
                    .layout_hook()
                    .map(|h| h.left_sidebar_width())
                    .unwrap_or(0.0)
            });
            max_w = max_w.max(w);
        }
        max_w
    }

    /// Altura acumulada de las barras inferiores activas.
    pub fn bottom_bar_height(&self) -> f32 {
        let mut max_h = 0.0f32;
        for entry in &self.plugins {
            let h = entry.invoke_hook(self.hook_budget, 0.0, || {
                entry
                    .plugin
                    .layout_hook()
                    .map(|h| h.bottom_bar_height())
                    .unwrap_or(0.0)
            });
            max_h = max_h.max(h);
        }
        max_h
    }

    /// Identificador de la sesión activa que debe mostrarse y recibir teclado.
    pub fn active_session_id(&self) -> usize {
        for entry in &self.plugins {
            if !entry.enabled || entry.health.get() == PluginHealth::Disabled {
                continue;
            }
            if entry.plugin.space_hook().is_some() {
                return entry.invoke_hook(self.hook_budget, 0, || {
                    entry
                        .plugin
                        .space_hook()
                        .map(|h| h.active_session())
                        .unwrap_or(0)
                });
            }
        }
        0
    }

    /// Índice del espacio activo según los plugins registrados.
    pub fn active_space_index(&self) -> usize {
        for entry in &self.plugins {
            if !entry.enabled || entry.health.get() == PluginHealth::Disabled {
                continue;
            }
            if entry.plugin.space_hook().is_some() {
                return entry.invoke_hook(self.hook_budget, 0, || {
                    entry
                        .plugin
                        .space_hook()
                        .map(|h| h.active_space())
                        .unwrap_or(0)
                });
            }
        }
        0
    }

    /// Comprueba si algún plugin activo solicita crear una nueva sesión de shell/espacio/pestaña.
    pub fn take_new_session_request(&self) -> bool {
        for entry in &self.plugins {
            let requested = entry.invoke_hook(self.hook_budget, false, || {
                entry
                    .plugin
                    .space_hook()
                    .map(|h| h.take_new_session_request())
                    .unwrap_or(false)
            });
            if requested {
                return true;
            }
        }
        false
    }

    /// Sesión nueva pedida por un plugin con un programa concreto, si la hay.
    pub fn take_spawn_session_request(&self) -> Option<PtyConfig> {
        for entry in &self.plugins {
            let request = entry.invoke_hook(self.hook_budget, None, || {
                entry
                    .plugin
                    .space_hook()
                    .and_then(|h| h.take_spawn_session_request())
            });
            if request.is_some() {
                return request;
            }
        }
        None
    }

    /// Notifica a los plugins el ID de la sesión recién creada.
    pub fn on_session_created(&self, session_id: usize) {
        for entry in &self.plugins {
            entry.invoke_hook(self.hook_budget, (), || {
                if let Some(hook) = entry.plugin.space_hook() {
                    hook.on_session_created(session_id);
                }
            });
        }
    }

    /// Notifica a los plugins el directorio de trabajo actual y nombre de carpeta de una sesión.
    pub fn update_session_cwd(&self, session_id: usize, cwd: &Path, folder_name: &str) {
        for entry in &self.plugins {
            entry.invoke_hook(self.hook_budget, (), || {
                if let Some(hook) = entry.plugin.space_hook() {
                    hook.update_session_cwd(session_id, cwd, folder_name);
                }
            });
        }
    }

    /// Notifica a los plugins el programa en primer plano de una sesión.
    pub fn update_session_app(&self, session_id: usize, app: Option<&RunningApp>) {
        for entry in &self.plugins {
            entry.invoke_hook(self.hook_budget, (), || {
                if let Some(hook) = entry.plugin.space_hook() {
                    hook.update_session_app(session_id, app);
                }
            });
        }
    }

    /// Comprueba si algún plugin activo solicita cerrar una sesión específica.
    pub fn take_close_session_request(&self) -> Option<usize> {
        for entry in &self.plugins {
            let request = entry.invoke_hook(self.hook_budget, None, || {
                entry
                    .plugin
                    .space_hook()
                    .and_then(|h| h.take_close_session_request())
            });
            if request.is_some() {
                return request;
            }
        }
        None
    }

    /// Consulta a los plugins si el cierre de la ventana necesita confirmación.
    /// Basta con que uno pida confirmar para bloquearlo.
    pub fn close_decision(&self) -> CloseDecision {
        for entry in &self.plugins {
            let decision = entry.invoke_hook(self.hook_budget, CloseDecision::Allow, || {
                entry
                    .plugin
                    .lifecycle_hook()
                    .map(|h| h.on_close_request())
                    .unwrap_or(CloseDecision::Allow)
            });
            if decision == CloseDecision::Confirm {
                return CloseDecision::Confirm;
            }
        }
        CloseDecision::Allow
    }

    /// Notifica a los plugins de que una sesión ya no existe.
    pub fn on_session_closed(&self, session_id: usize) {
        for entry in &self.plugins {
            entry.invoke_hook(self.hook_budget, (), || {
                if let Some(hook) = entry.plugin.space_hook() {
                    hook.on_session_closed(session_id);
                }
            });
        }
    }

    /// Notifica a los plugins que el cierre fue aceptado.
    pub fn on_close_confirmed(&self) {
        for entry in &self.plugins {
            entry.invoke_hook(self.hook_budget, (), || {
                if let Some(hook) = entry.plugin.lifecycle_hook() {
                    hook.on_close_confirmed();
                }
            });
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
            if let Some(elem) = entry.invoke_hook(self.hook_budget, None, || {
                entry.plugin.layout_hook().and_then(|h| h.top_bar())
            }) {
                elements.push(elem);
            }
        }
        elements
    }

    /// Recopila los elementos para la barra lateral izquierda de los plugins activos.
    pub fn left_sidebars(&self) -> Vec<AnyElement> {
        let mut elements = Vec::new();
        for entry in &self.plugins {
            if let Some(elem) = entry.invoke_hook(self.hook_budget, None, || {
                entry.plugin.layout_hook().and_then(|h| h.left_sidebar())
            }) {
                elements.push(elem);
            }
        }
        elements
    }

    /// Recopila los elementos para la barra inferior de los plugins activos.
    pub fn bottom_bars(&self) -> Vec<AnyElement> {
        let mut elements = Vec::new();
        for entry in &self.plugins {
            if let Some(elem) = entry.invoke_hook(self.hook_budget, None, || {
                entry.plugin.layout_hook().and_then(|h| h.bottom_bar())
            }) {
                elements.push(elem);
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
