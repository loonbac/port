//! Recarga de plugins mientras se desarrolla.
//!
//! # El coste caro va una vez
//!
//! Un plugin se compila al instalarlo, no al cargarlo. Cargar es lanzar un
//! proceso: unos milisegundos. Por eso el ciclo de desarrollo puede permitirse
//! recompilar sin que nadie note una pausa en la terminal.
//!
//! # Qué se vigila y por qué
//!
//! Se vigila el binario instalado, no el codigo fuente. El codigo cambia
//! constantemente durante una sesion de trabajo y casi nunca produce un binario
//! distinto; el binario es el punto en el que la recompilacion se ha hecho
//! real. Vigilarlo evita reiniciar el plugin por cada guardado que no toco el
//! plugin, que es la mayoria.
//!
//! # Por qué se mide tamaño y fecha
//!
//! Comparar solo la fecha de modificación pierde cambios: dos escrituras dentro
//! del mismo tick del reloj dejan la misma fecha. Por eso la firma son las dos
//! cosas. Un reemplazo que conserve tamaño y fecha dentro de esa granularidad
//! sería invisible, así que el guion de recarga ejecuta siempre una copia por
//! encima, y el binario instalado cambia de tamaño al menos en el enlazado.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::host::ExternalPlugin;

/// Firma de un binario: tamaño y última modificación.
///
/// Se comparan los dos porque un reemplazo puede conservar la fecha dentro de
/// la misma granularidad del sistema de archivos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint {
    pub len: u64,
    pub modified: Option<SystemTime>,
}

impl Fingerprint {
    /// Lee la firma de un ejecutable. Un archivo que no existe no tiene firma.
    pub fn of(path: &PathBuf) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Self {
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }
}

/// Qué hacer con un plugin cuyo binario cambió.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reaction {
    /// El binario no cambió.
    Nothing,
    /// El proceso se relanzó con el binario nuevo.
    Reloaded,
    /// El binario cambió pero el proceso no volvió a levantar.
    Failed,
}

/// Vigila el binario de un plugin y lo recarga cuando cambia.
///
/// Se mantiene entre llamadas: guarda la firma vista la última vez, de modo
/// que solo reactiona a cambios reales.
pub struct Watcher {
    fingerprint: Option<Fingerprint>,
    /// Cada cuánto se mira. Suficiente para reaccionar antes de que se note,
    /// y tan barato que no importa ejecutarlo en cada fotograma.
    pub interval: Duration,
}

impl Watcher {
    pub fn new() -> Self {
        Self {
            fingerprint: None,
            // Un cuarto de segundo: por debajo de eso se nota el trabajo de
            // disco, y por encima la recarga tarda en verse.
            interval: Duration::from_millis(250),
        }
    }

    /// Mira el binario y recarga el plugin si hace falta.
    ///
    /// Nunca entra en pánico ni propaga el fallo del plugin: un plugin que no
    /// compila es un plugin que no está, no una terminal rota.
    pub fn poll(&mut self, plugin: &Arc<ExternalPlugin>) -> Reaction {
        let path = PathBuf::from(&plugin.manifest().executable);
        let current = Fingerprint::of(&path);

        match (self.fingerprint, current) {
            // Primera observación: se memoriza y no se recarga nada. Sin esto,
            // arrancar PORT reiniciaria todos los plugins sin motivo.
            (None, current) => {
                self.fingerprint = current;
                Reaction::Nothing
            }
            (previous, Some(current)) if previous == Some(current) => Reaction::Nothing,
            // El binario ya no esta: el plugin se dio de baja.
            (Some(_), None) => {
                self.fingerprint = None;
                Reaction::Failed
            }
            (previous, Some(current)) => {
                self.fingerprint = Some(current);
                match plugin.reload() {
                    Ok(()) => Reaction::Reloaded,
                    // Se deja la firma puesta: si el binario sigue cambiado,
                    // no se insiste cada 250 ms contra un plugin roto.
                    Err(_) => {
                        let _ = previous;
                        Reaction::Failed
                    }
                }
            }
        }
    }
}

impl Default for Watcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::ExternalPlugin;
    use crate::protocol::{Capability, PluginManifest};
    use std::path::Path;

    #[test]
    fn a_file_that_is_not_there_has_no_fingerprint() {
        assert!(Fingerprint::of(&PathBuf::from("/no/existe/nada")).is_none());
    }

    #[test]
    fn a_fingerprint_changes_when_the_content_changes() {
        let path = scratch("fp");
        std::fs::write(&path, b"uno").unwrap();
        let before = Fingerprint::of(&path).expect("deberia existir");
        assert_eq!(
            before,
            Fingerprint::of(&path).expect("no cambia sin tocarlo")
        );

        std::fs::write(&path, b"uno-mas-largo").unwrap();
        let after = Fingerprint::of(&path).expect("deberia existir");
        assert_ne!(before, after, "cambiar el contenido debe cambiar la firma");

        let _ = std::fs::remove_file(&path);
    }

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "port-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    /// Escribe un plugin de shell que responde con `version`.
    /// Reescribirlo es lo que simula una recompilacion.
    fn write_plugin(path: &Path, version: &str) {
        let script = format!(
            r##"#!/bin/sh
printf '{{"t":"ready","name":"probe","version":"{version}","capabilities":["appearance"]}}\n'
while IFS= read -r line; do
  case "$line" in
    *invoke*)
      seq=$(printf '%s' "$line" | sed -n 's/.*"seq":\([0-9]*\).*/\1/p')
      printf '{{"t":"result","seq":%s,"ok":true,"value":{{"v":"{version}"}}}}\n' "$seq"
      ;;
  esac
done
"##
        );
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, script).unwrap();
        std::fs::rename(&tmp, path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
        }
    }

    fn start_probe(path: &Path) -> Arc<ExternalPlugin> {
        ExternalPlugin::start(PluginManifest {
            id: "probe".into(),
            name: "Probe".into(),
            version: "0.1.0".into(),
            executable: path.to_string_lossy().to_string(),
            source: "test".into(),
            capabilities: vec![Capability::Appearance],
        })
        .expect("el plugin deberia arrancar")
    }

    #[test]
    fn starting_the_watcher_does_not_reload_anything() {
        // La primera observacion solo memoriza: sin esto, abrir PORT
        // reiniciaria todos los plugins sin que nadie recompilara nada.
        let path = scratch("watch-first");
        write_plugin(&path, "v1");
        let plugin = start_probe(&path);
        let mut watcher = Watcher::new();
        assert_eq!(watcher.poll(&plugin), Reaction::Nothing);
    }

    #[test]
    fn an_untouched_plugin_is_left_alone() {
        let path = scratch("watch-quiet");
        write_plugin(&path, "v1");
        let plugin = start_probe(&path);
        let mut watcher = Watcher::new();

        for _ in 0..5 {
            assert_eq!(watcher.poll(&plugin), Reaction::Nothing);
        }
        // Sigue siendo el proceso original, no uno relanzado.
        assert_eq!(plugin.invoke("ping").unwrap()["v"], "v1");
    }

    #[test]
    fn recompiling_the_plugin_reloads_it_without_being_asked() {
        let path = scratch("watch-reload");
        write_plugin(&path, "v1");
        let plugin = start_probe(&path);
        let mut watcher = Watcher::new();
        assert_eq!(watcher.poll(&plugin), Reaction::Nothing);
        assert_eq!(plugin.invoke("ping").unwrap()["v"], "v1");

        write_plugin(&path, "v2-recompilado");

        assert_eq!(watcher.poll(&plugin), Reaction::Reloaded);
        assert_eq!(
            plugin.invoke("ping").unwrap()["v"],
            "v2-recompilado",
            "tras recargar debe servir la version nueva"
        );

        // Y una vez recargado, se vuelve a quedar quieto.
        assert_eq!(watcher.poll(&plugin), Reaction::Nothing);
    }

    #[test]
    fn a_broken_rebuild_does_not_break_the_terminal() {
        let path = scratch("watch-broken");
        write_plugin(&path, "v1");
        let plugin = start_probe(&path);
        let mut watcher = Watcher::new();
        watcher.poll(&plugin);

        // Sustitucion por algo que no arranca.
        std::fs::write(
            &path,
            "#!/bin/sh
exit 1
",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
        }

        assert_eq!(watcher.poll(&plugin), Reaction::Failed);
        // Sigue siendo un fallo contenido: el watcher no lanza.
    }

    #[test]
    fn a_deleted_binary_is_reported_as_a_failure() {
        let path = scratch("watch-gone");
        write_plugin(&path, "v1");
        let plugin = start_probe(&path);
        let mut watcher = Watcher::new();
        watcher.poll(&plugin);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(watcher.poll(&plugin), Reaction::Failed);
    }
}
