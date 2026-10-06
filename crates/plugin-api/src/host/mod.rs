//! Anfitrión de plugins externos: instalación y estado en disco.
//!
//! Aquí vive lo que un plugin instalado es fuera de su proceso: el manifiesto
//! que lo describe, su ejecutable, el directorio de datos y las operaciones que
//! lo listan, lo guardan y lo eliminan. El proceso vivo —arranque, protocolo
//! por stdio y apagado— vive en [`process`].

mod process;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::config::{ConfigFile, PluginConfig};
use crate::protocol::PluginManifest;

pub use self::process::{
    configure_externals, event_channel, is_alive, EventReceiver, EventSender, ExternalPlugin,
};

/// Raíz de datos de los plugins instalados.
pub fn plugins_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("PORT_PLUGIN_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".local/share")
        });
    base.join("port/plugins")
}

/// Fallos que puede producir un plugin externo.
///
/// Todos son recuperables a proposito: quien los recibe decide si el plugin se
/// apaga o se ignora, y en ninguno de los dos casos se cae el terminal.
#[derive(Debug)]
pub enum PluginError {
    Spawn {
        executable: String,
        source: std::io::Error,
    },
    NoStdin(String),
    NoStdout(String),
    Thread {
        id: String,
        source: std::io::Error,
    },
    Closed(String),
    Timeout(String),
    Poisoned(String),
    Protocol(String),
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn { executable, source } => {
                write!(f, "no se pudo lanzar {executable}: {source}")
            }
            Self::NoStdin(id) => write!(f, "el plugin {id} no expone stdin"),
            Self::NoStdout(id) => write!(f, "el plugin {id} no expone stdout"),
            Self::Thread { id, source } => write!(f, "no se pudo crear el hilo de {id}: {source}"),
            Self::Closed(id) => write!(f, "el plugin {id} cerró la conexión"),
            Self::Timeout(id) => write!(f, "el plugin {id} no respondió a tiempo"),
            Self::Poisoned(id) => write!(f, "estado del plugin {id} corrupto"),
            Self::Protocol(message) => write!(f, "error de protocolo: {message}"),
        }
    }
}

impl std::error::Error for PluginError {}

/// Manifiestos instalados que la configuración persistida deja arrancar.
///
/// Un plugin recién instalado sin bloque cuenta como habilitado; solo
/// `enabled = false` lo excluye. El bucle de arranque de PORT usa esto para no
/// levantar los plugins que el usuario apagó.
pub fn enabled_manifests(
    manifests: Vec<PluginManifest>,
    configs: &BTreeMap<String, PluginConfig>,
) -> Vec<PluginManifest> {
    manifests
        .into_iter()
        .filter(|manifest| ConfigFile::is_enabled(configs, &manifest.id))
        .collect()
}

/// Lee los manifiestos instalados sin arrancar nada.
///
/// Sirve para que `port plugin list` y el gestor de la interfaz sean rápidos:
/// enumerar no puede costar un lanzamiento de proceso.
pub fn installed() -> Vec<PluginManifest> {
    installed_in(&plugins_dir())
}

/// Igual que [`installed`], pero sobre un directorio concreto.
///
/// Existe para que las pruebas puedan usar un directorio propio sin tocar
/// `PORT_PLUGIN_DIR`, que es global y se pisa entre hilos.
pub fn installed_in(dir: &Path) -> Vec<PluginManifest> {
    let mut manifests = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return manifests;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(manifest) = serde_json::from_str::<PluginManifest>(&text) {
                    manifests.push(manifest);
                }
            }
        }
    }
    manifests.sort_by(|a, b| a.id.cmp(&b.id));
    manifests
}

/// Guarda un manifiesto junto al ejecutable del plugin.
pub fn write_manifest(manifest: &PluginManifest) -> std::io::Result<()> {
    write_manifest_in(&plugins_dir(), manifest)
}

/// Igual que [`write_manifest`], sobre un directorio concreto.
pub fn write_manifest_in(dir: &Path, manifest: &PluginManifest) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.json", manifest.id));
    let text =
        serde_json::to_string_pretty(manifest).map_err(|e| std::io::Error::other(e.to_string()))?;
    std::fs::write(path, format!("{text}\n"))
}

/// Elimina el ejecutable y el manifiesto de un plugin.
pub fn uninstall(id: &str) -> std::io::Result<bool> {
    uninstall_from(&plugins_dir(), id)
}

/// Igual que [`uninstall`], sobre un directorio concreto.
pub fn uninstall_from(dir: &Path, id: &str) -> std::io::Result<bool> {
    let Some(manifest) = installed_in(dir).into_iter().find(|m| m.id == id) else {
        return Ok(false);
    };
    let _ = std::fs::remove_file(&manifest.executable);
    let _ = std::fs::remove_file(dir.join(format!("{id}.json")));
    Ok(true)
}

/// Cuánto lleva esperando el núcleo a un plugin, para poder avisar en la interfaz.
pub fn age(then: Instant) -> Duration {
    then.elapsed()
}

/// Ruta de un ejecutable dentro del directorio de plugins.
pub fn executable_path(id: &str) -> PathBuf {
    plugins_dir().join(id)
}

/// Deja el directorio de plugins listo para escritura.
pub fn ensure_plugins_dir() -> std::io::Result<PathBuf> {
    let dir = plugins_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Ejecutable instalado con ese nombre, si existe.
pub fn find_executable(id: &str) -> Option<PathBuf> {
    let path = executable_path(id);
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Capability;

    #[test]
    fn plugins_dir_is_not_empty() {
        assert!(!plugins_dir().as_os_str().is_empty());
    }

    /// Cada prueba usa su propio directorio. Tocar `PORT_PLUGIN_DIR` no vale:
    /// es global y `cargo test` corre las pruebas en paralelo.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "port-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn installing_and_uninstalling_is_symmetric() {
        let dir = scratch("plugins");

        assert!(installed_in(&dir).is_empty());

        let m = PluginManifest {
            id: "demo".into(),
            name: "Demo".into(),
            version: "0.1.0".into(),
            executable: dir.join("demo").to_string_lossy().to_string(),
            source: "https://example.invalid/demo".into(),
            capabilities: vec![Capability::Appearance],
        };
        write_manifest_in(&dir, &m).unwrap();
        let found = installed_in(&dir);
        assert_eq!(found.len(), 1, "debería encontrar el manifiesto");
        assert_eq!(found[0].id, "demo");

        assert!(uninstall_from(&dir, "demo").unwrap());
        assert!(installed_in(&dir).is_empty());
        assert!(!uninstall_from(&dir, "demo").unwrap());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn un_manifiesto_sin_origen_se_lee_con_origen_vacio() {
        // Instalación anterior a que se guardara el origen: el campo no está.
        // Debe leerse igual (origen vacío) en lugar de descartarse en
        // silencio, que ocultaría el plugin del listado y de la interfaz.
        let dir = scratch("legacy-source");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("legacy.json"),
            r#"{
  "id": "legacy",
  "name": "Legacy",
  "version": "0.1.0",
  "executable": "/no/existe/legacy",
  "capabilities": []
}
"#,
        )
        .unwrap();

        let found = installed_in(&dir);
        assert_eq!(
            found.len(),
            1,
            "un manifiesto antiguo no debe descartarse en silencio"
        );
        assert_eq!(found[0].id, "legacy");
        assert_eq!(found[0].source, "", "sin origen registrado queda vacío");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
