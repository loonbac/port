//! Instalación de plugins externos a partir de un repositorio.
//!
//! # Por qué esto no es "cargo add"
//!
//! Un plugin es un ejecutable independiente, no una biblioteca. Instalarlo es
//! clonar, compilar una vez y dejar el binario junto a un manifiesto. A partir
//! de ahí cargarlo cuesta unos milisegundos: arrancar un proceso.
//!
//! Esa separación es la clave del diseño. El coste caro (compilar) ocurre una
//! sola vez, en la instalación; el arranque posterior es un `fork`/`exec`. Un
//! plugin cargado como `.so` ahorraba esos milisegundos a cambio de compartir
//! heap, runtime y tipos de GPUI con el núcleo, lo que convierte cualquier
//! diferencia de versión en comportamiento indefinido.
//!
//! # Por qué una copia, no un enlace al repositorio
//!
//! Si el ejecutable instalado queda apuntando al árbol de compilación, un
//! `cargo clean` deja la instalación inservible. Se copia a
//! `~/.local/share/port/plugins` y se guarda el origen en el manifiesto para
//! poder actualizar.
//!
//! # Qué se espera del repositorio
//!
//! Un `Cargo.toml` que produce un binario, y opcionalmente una sección
//! `[package.metadata.port]` con el identificador y la versión que se()
//! registrarán. Sin esa sección se deducen del nombre del paquete.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;

use crate::host;
use crate::protocol::{Capability, PluginManifest};

/// Errores de la instalación.
///
/// Todos son mensajes para una persona: el comando informa y no deja el sistema
/// a medias, porque `install` copia al final y hasta entonces no toca la
/// instalación existente.
#[derive(Debug)]
pub enum InstallError {
    Clone {
        url: String,
        source: std::io::Error,
    },
    MissingManifest {
        url: String,
    },
    InvalidManifest {
        url: String,
        source: toml::de::Error,
    },
    Build {
        output: String,
    },
    Copy {
        source: PathBuf,
        source_err: std::io::Error,
    },
    NoBinary {
        id: String,
        tried: Vec<PathBuf>,
    },
    Io(std::io::Error),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Clone { url, source } => write!(f, "no se pudo clonar {url}: {source}"),
            Self::MissingManifest { url } => {
                write!(f, "{url} no parece un plugin: falta Cargo.toml")
            }
            Self::InvalidManifest { url, source } => {
                write!(f, "el Cargo.toml de {url} no se pudo leer: {source}")
            }
            Self::Build { output } => write!(f, "el plugin no compiló:\n{output}"),
            Self::Copy { source, source_err } => {
                write!(f, "no se pudo copiar {source:?}: {source_err}")
            }
            Self::Io(e) => write!(f, "error de sistema: {e}"),
            Self::NoBinary { id, tried } => write!(
                f,
                "el plugin {id} no produjo ningún binario; se buscaron: {}",
                tried
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

impl From<std::io::Error> for InstallError {
    fn from(source: std::io::Error) -> Self {
        Self::Io(source)
    }
}

impl std::error::Error for InstallError {}

/// Tabla `[package.metadata]`: hoy solo la subsection `port` tiene sentido.
#[derive(Debug, Deserialize, Default)]
struct MetadataTable {
    #[serde(default)]
    port: Option<PluginMetadata>,
}

/// Datos opcionales de la sección `[package.metadata.port]`.
#[derive(Debug, Deserialize, Default)]
struct PluginMetadata {
    /// Identificador con el que se registra el plugin.
    id: Option<String>,
    /// Capacidades, si el repositorio quiere declararlas sin arrancar.
    capabilities: Option<Vec<Capability>>,
}

/// Lo que se lee del `Cargo.toml` del repositorio.
#[derive(Debug, Deserialize)]
struct CargoManifest {
    package: CargoPackage,
    #[serde(default)]
    bin: Vec<CargoBin>,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    name: String,
    version: String,
    #[serde(default)]
    metadata: Option<MetadataTable>,
}

#[derive(Debug, Deserialize)]
struct CargoBin {
    name: String,
}

/// Rutas donde puede quedar el ejecutable tras compilar.
fn candidate_paths(dir: &Path, name: &str) -> Vec<PathBuf> {
    vec![
        dir.join("target/release").join(name),
        dir.join("target/release").join(format!("{name}.exe")),
        dir.join("target/debug").join(name),
        dir.join("target/debug").join(format!("{name}.exe")),
    ]
}

/// Compila un repositorio ya clonado y devuelve el ejecutable producido.
///
/// No devuelve la ruta dentro de `target/`: se deja claro que quien llama tiene
/// que copiar el binario antes de dar por buena la instalación.
fn build(repo: &Path) -> Result<PathBuf, InstallError> {
    let output = Command::new("cargo")
        .current_dir(repo)
        .args(["build", "--release"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| InstallError::Build {
            output: format!("no se pudo lanzar cargo: {e}"),
        })?;

    if !output.status.success() {
        return Err(InstallError::Build {
            output: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }
    Ok(repo.to_path_buf())
}

/// Instala un plugin desde una URL de repositorio.
///
/// Devuelve el manifiesto registrado. Si el plugin ya estaba instalado, se
/// vuelve a construir y se reemplaza: actualizar y reinstalar son la misma
/// operación.
pub fn install(url: &str) -> Result<PluginManifest, InstallError> {
    let staging = std::env::temp_dir().join(format!("port-plugin-build-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);

    // Clone sin historial: el build necesita el contenido, no la historia, y
    // un clon completo de un repo con historia larga es una espera de sobra.
    let clone = Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "--quiet",
            url,
            &staging.to_string_lossy(),
        ])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| InstallError::Clone {
            url: url.to_string(),
            source: e,
        })?;
    if !clone.status.success() {
        return Err(InstallError::Clone {
            url: url.to_string(),
            source: std::io::Error::other(String::from_utf8_lossy(&clone.stderr).trim()),
        });
    }

    let cargo_path = staging.join("Cargo.toml");
    let text = std::fs::read_to_string(&cargo_path).map_err(|_| InstallError::MissingManifest {
        url: url.to_string(),
    })?;
    let manifest: CargoManifest =
        toml::from_str(&text).map_err(|e| InstallError::InvalidManifest {
            url: url.to_string(),
            source: e,
        })?;

    let id = manifest
        .package
        .metadata
        .as_ref()
        .and_then(|m| m.port.as_ref())
        .and_then(|p| p.id.clone())
        .unwrap_or_else(|| manifest.package.name.replace('-', "_"));

    build(&staging)?;

    // Se busca el binario por el nombre del paquete o por el primer `[[bin]]`.
    let names: Vec<String> = if manifest.bin.is_empty() {
        vec![manifest.package.name.clone()]
    } else {
        manifest.bin.iter().map(|b| b.name.clone()).collect()
    };

    let mut found = None;
    let mut tried = Vec::new();
    for name in &names {
        for candidate in candidate_paths(&staging, name) {
            if candidate.is_file() {
                found = Some(candidate);
                break;
            }
            tried.push(candidate);
        }
        if found.is_some() {
            break;
        }
    }
    let built = found.ok_or_else(|| InstallError::NoBinary {
        id: id.clone(),
        tried,
    })?;

    host::ensure_plugins_dir()?;
    let destination = host::plugins_dir().join(&id);
    std::fs::copy(&built, &destination).map_err(|source_err| InstallError::Copy {
        source: built.clone(),
        source_err,
    })?;
    set_executable(&destination)?;

    let manifest = PluginManifest {
        id,
        name: manifest.package.name.clone(),
        version: manifest.package.version.clone(),
        executable: destination.to_string_lossy().to_string(),
        source: url.to_string(),
        capabilities: manifest
            .package
            .metadata
            .and_then(|m| m.port)
            .and_then(|p| p.capabilities)
            .unwrap_or_default(),
    };
    host::write_manifest(&manifest)?;

    let _ = std::fs::remove_dir_all(&staging);
    Ok(manifest)
}

#[cfg(unix)]
fn set_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_without_metadata_still_parses() {
        // Lo minimo exigible: nombre y version. Los plugins pequenos no
        // deberian tener que declarar nada para instalarse.
        let m: CargoManifest = toml::from_str(
            r#"
[package]
name = "port-plugin-demo"
version = "0.1.0"
"#,
        )
        .unwrap();
        assert_eq!(m.package.name, "port-plugin-demo");
        assert!(m.bin.is_empty());
        assert!(m.package.metadata.is_none());
    }

    #[test]
    fn metadata_carries_the_registration_identity() {
        let m: CargoManifest = toml::from_str(
            r#"
[package]
name = "port-plugin-demo"
version = "0.2.0"

[package.metadata.port]
id = "demo"
capabilities = ["appearance", "input"]
"#,
        )
        .unwrap();
        let meta = m.package.metadata.unwrap().port.unwrap();
        assert_eq!(meta.id.as_deref(), Some("demo"));
        assert_eq!(meta.capabilities.unwrap().len(), 2);
    }

    #[test]
    fn a_repository_without_cargo_toml_is_not_a_plugin() {
        assert!(toml::from_str::<CargoManifest>("").is_err());
    }

    #[test]
    fn binary_paths_cover_release_and_debug_and_windows() {
        let paths = candidate_paths(Path::new("/repo"), "demo");
        let names: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        assert!(names.iter().any(|p| p.contains("release/demo")));
        assert!(names.iter().any(|p| p.contains("release/demo.exe")));
        assert!(names.iter().any(|p| p.contains("debug/demo")));
    }
}
