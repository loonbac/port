//! Manejo del archivo de configuración de PORT basado en bloques de código (codeblocks).
//!
//! El archivo de configuración de PORT utiliza un formato de texto limpio y legible
//! donde cada plugin dispone de su propio bloque delimitado por ```<plugin-id> ... ```.
//! Esto permite editarlo fácilmente sin necesidad de dependencias complejas y con
//! sintaxis amigable para cualquier editor.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Valores clave-valor asociados a la configuración de un plugin.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginConfig {
    pub values: BTreeMap<String, String>,
}

impl PluginConfig {
    /// Crea un conjunto de configuración vacío.
    pub fn new() -> Self {
        Self::default()
    }

    /// Asigna una clave y su valor en cadena.
    pub fn set(&mut self, key: impl Into<String>, value: impl ToString) -> &mut Self {
        self.values.insert(key.into(), value.to_string());
        self
    }

    /// Obtiene un valor como cadena si existe.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// Obtiene un valor numérico de punto flotante.
    pub fn get_f32(&self, key: &str) -> Option<f32> {
        self.get(key)?.trim().parse().ok()
    }

    /// Obtiene un valor numérico entero sin signo.
    pub fn get_u32(&self, key: &str) -> Option<u32> {
        self.get(key)?.trim().parse().ok()
    }

    /// Obtiene un valor booleano (true/false, 1/0, yes/no).
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.get(key)?.trim().to_lowercase().as_str() {
            "true" | "yes" | "1" | "on" => Some(true),
            "false" | "no" | "0" | "off" => Some(false),
            _ => None,
        }
    }

    /// Devuelve `true` si no hay propiedades configuradas.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// Gestor del archivo de configuración de PORT con soporte para codeblocks.
pub struct ConfigFile;

impl ConfigFile {
    /// Ruta predeterminada del archivo de configuración: `$XDG_CONFIG_HOME/port/config.md`
    /// o `~/.config/port/config.md`. También respeta la variable de entorno `PORT_CONFIG`.
    pub fn default_path() -> PathBuf {
        if let Ok(custom) = std::env::var("PORT_CONFIG") {
            if !custom.trim().is_empty() {
                return PathBuf::from(custom);
            }
        }

        let base = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|p| !p.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                PathBuf::from(home).join(".config")
            });

        base.join("port").join("config.md")
    }

    /// Analiza el contenido de un archivo de texto con codeblocks.
    ///
    /// Cada bloque con formato:
    ///
    /// ````text
    /// ```<plugin-id>
    /// clave = valor
    /// ```
    /// ````
    ///
    /// se asocia a la configuración de ese plugin. Cualquier texto fuera de los
    /// bloques se ignora de forma segura.
    pub fn parse(content: &str) -> BTreeMap<String, PluginConfig> {
        let mut result = BTreeMap::new();
        let mut current_plugin: Option<String> = None;
        let mut current_config = PluginConfig::new();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                let tag = trimmed.trim_start_matches('`').trim();
                if tag.is_empty() {
                    // Cierre del codeblock
                    if let Some(plugin_id) = current_plugin.take() {
                        result.insert(plugin_id, std::mem::take(&mut current_config));
                    }
                } else {
                    // Inicio de un nuevo codeblock
                    if let Some(plugin_id) = current_plugin.take() {
                        result.insert(plugin_id, std::mem::take(&mut current_config));
                    }
                    current_plugin = Some(tag.to_string());
                }
            } else if current_plugin.is_some() {
                if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
                    continue;
                }
                if let Some((key, value)) = trimmed.split_once('=') {
                    let k = key.trim();
                    let v = value.trim();
                    if !k.is_empty() {
                        current_config.set(k, v);
                    }
                }
            }
        }

        if let Some(plugin_id) = current_plugin.take() {
            result.insert(plugin_id, current_config);
        }

        result
    }

    /// Genera un nuevo archivo con el encabezado y las secciones de configuración dadas.
    pub fn render_new(configs: &BTreeMap<String, PluginConfig>) -> String {
        let mut out = String::new();
        out.push_str("# Configuración de PORT\n\n");
        out.push_str("<!-- Cada sección delimitada por un codeblock pertenece a un plugin -->\n\n");

        for (plugin_id, config) in configs {
            out.push_str(&format!("```{plugin_id}\n"));
            for (k, v) in &config.values {
                out.push_str(&format!("{k} = {v}\n"));
            }
            out.push_str("```\n\n");
        }

        out
    }

    /// Actualiza un bloque existente en el contenido del archivo o añade uno nuevo al final,
    /// conservando comentarios y demás bloques intactos.
    pub fn update_or_append(content: &str, plugin_id: &str, config: &PluginConfig) -> String {
        let lines: Vec<&str> = content.lines().collect();
        let mut out = String::new();
        let mut found = false;
        let mut inside_target = false;

        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];
            let trimmed = line.trim();

            if trimmed.starts_with("```") {
                let tag = trimmed.trim_start_matches('`').trim();
                if tag == plugin_id {
                    // Inicio del bloque objetivo
                    found = true;
                    inside_target = true;
                    out.push_str(&format!("```{plugin_id}\n"));
                    for (k, v) in &config.values {
                        out.push_str(&format!("{k} = {v}\n"));
                    }
                    i += 1;
                    continue;
                } else if inside_target && tag.is_empty() {
                    // Cierre del bloque objetivo
                    inside_target = false;
                    out.push_str("```\n");
                    i += 1;
                    continue;
                }
            }

            if !inside_target {
                out.push_str(line);
                out.push('\n');
            }

            i += 1;
        }

        if !found {
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push('\n');
            }
            out.push_str(&format!("```{plugin_id}\n"));
            for (k, v) in &config.values {
                out.push_str(&format!("{k} = {v}\n"));
            }
            out.push_str("```\n");
        }

        out
    }

    /// Carga el archivo en la ruta dada o lo crea con la configuración por defecto proporcionada.
    /// Si el archivo ya existe pero faltan bloques de plugins recién registrados, añade
    /// automáticamente sus secciones por defecto preservando el resto del archivo.
    pub fn load_or_create(
        path: &Path,
        defaults: &BTreeMap<String, PluginConfig>,
    ) -> std::io::Result<BTreeMap<String, PluginConfig>> {
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            let mut configs = Self::parse(&content);
            let mut missing = false;
            let mut updated_content = content;
            for (plugin_id, default_cfg) in defaults {
                if !configs.contains_key(plugin_id) {
                    updated_content =
                        Self::update_or_append(&updated_content, plugin_id, default_cfg);
                    configs.insert(plugin_id.clone(), default_cfg.clone());
                    missing = true;
                }
            }
            if missing {
                std::fs::write(path, updated_content)?;
            }
            Ok(configs)
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let rendered = Self::render_new(defaults);
            std::fs::write(path, &rendered)?;
            Ok(defaults.clone())
        }
    }

    /// Guarda o actualiza la configuración de un plugin específico en el archivo.
    pub fn save_plugin(path: &Path, plugin_id: &str, config: &PluginConfig) -> std::io::Result<()> {
        let content = if path.exists() {
            std::fs::read_to_string(path)?
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            String::new()
        };

        let updated = Self::update_or_append(&content, plugin_id, config);
        std::fs::write(path, updated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_codeblocks_correctly() {
        let content = r#"
# Configuración general

Notas del usuario aquí.

```font-zoom
default_size = 18
step = 2.0
```

Texto intermedio que debe ignorarse.

```transparency
opacity = 0.80
# Comentario interno
active = true
```
"#;

        let parsed = ConfigFile::parse(content);
        assert_eq!(parsed.len(), 2);

        let font_zoom = parsed.get("font-zoom").unwrap();
        assert_eq!(font_zoom.get_f32("default_size"), Some(18.0));
        assert_eq!(font_zoom.get_f32("step"), Some(2.0));

        let transparency = parsed.get("transparency").unwrap();
        assert_eq!(transparency.get_f32("opacity"), Some(0.80));
        assert_eq!(transparency.get_bool("active"), Some(true));
    }

    #[test]
    fn update_existing_codeblock_in_place() {
        let initial = r#"# Encabezado

```font-zoom
default_size = 14
```

```transparency
opacity = 0.85
```
"#;

        let mut updated_zoom = PluginConfig::new();
        updated_zoom.set("default_size", 20);

        let result = ConfigFile::update_or_append(initial, "font-zoom", &updated_zoom);
        assert!(result.contains("default_size = 20"));
        assert!(result.contains("```transparency\nopacity = 0.85\n```"));
        assert!(!result.contains("default_size = 14"));
    }

    #[test]
    fn append_new_codeblock_if_not_present() {
        let initial = r#"# Encabezado

```font-zoom
default_size = 14
```
"#;

        let mut font_cfg = PluginConfig::new();
        font_cfg.set("family", "JetBrainsMono");

        let result = ConfigFile::update_or_append(initial, "font", &font_cfg);
        assert!(result.contains("```font-zoom\ndefault_size = 14\n```"));
        assert!(result.contains("```font\nfamily = JetBrainsMono\n```"));
    }
}
