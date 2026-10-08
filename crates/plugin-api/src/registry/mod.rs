//! Registro central de plugins activos.
//!
//! Es el único lugar donde se combinan los plugins in-process y los externos
//! vivos, se invocan sus hooks y se persiste su configuración. Los hooks que
//! invoca están definidos en [`crate::hooks`], los servicios que publica viven
//! en [`crate::services`] y la política de salud y watchdog con que se ejecutan
//! vive en [`crate::health`].
//!
//! El registro tiene dos mitades, cada una con su propia razón para cambiar:
//!
//! - este módulo: la contabilidad del registro —qué plugins existen, cuáles
//!   están habilitados, la lista que se muestra, sus servicios, su
//!   configuración en disco y la composición con los externos vivos—.
//! - el submódulo `hooks`: la invocación de cada hook bajo el presupuesto del
//!   registro.

mod hooks;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::config::{ConfigFile, PluginConfig};
use crate::health::RegisteredPlugin;
use crate::hooks::Plugin;
use crate::host::ExternalPlugin;
use crate::services::Services;

pub use crate::health::{PluginHealth, DEFAULT_HOOK_BUDGET, DEFAULT_PROBE_COOLDOWN};

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
    probe_cooldown: Duration,
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
            probe_cooldown: DEFAULT_PROBE_COOLDOWN,
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

    /// Periodo de enfriamiento tras el cual un plugin deshabilitado recibe una prueba.
    pub fn probe_cooldown(&self) -> Duration {
        self.probe_cooldown
    }

    /// Configura el periodo de enfriamiento para llamadas de prueba.
    pub fn set_probe_cooldown(&mut self, cooldown: Duration) {
        self.probe_cooldown = cooldown;
        for plugin in &self.plugins {
            plugin.cooldown.set(cooldown);
        }
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
        self.plugins
            .push(RegisteredPlugin::new(Box::new(plugin), self.probe_cooldown));
    }

    /// Registra un plugin ya empaquetado en Box, activo por defecto.
    pub fn register_boxed(&mut self, plugin: Box<dyn Plugin>) {
        self.publish_services(plugin.as_ref());
        self.plugins
            .push(RegisteredPlugin::new(plugin, self.probe_cooldown));
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
}

impl PluginRegistry {
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
