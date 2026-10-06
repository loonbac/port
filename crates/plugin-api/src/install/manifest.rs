//! Lectura del `Cargo.toml` de un repositorio y descubrimiento de sus plugins.
//!
//! Esta pieza interpreta el manifiesto —los metadatos de registro, el binario y
//! la versión, incluida la herencia del workspace— y decide qué subdirectorios
//! del árbol son plugins instalables. No toca el destino ni compila: solo lee lo
//! que hay en el origen.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::protocol::Capability;

/// Tabla `[package.metadata]`: hoy solo la subsection `port` tiene sentido.
#[derive(Debug, Deserialize, Default)]
pub(crate) struct MetadataTable {
    #[serde(default)]
    pub(crate) port: Option<PluginMetadata>,
}

/// Datos opcionales de la sección `[package.metadata.port]`.
#[derive(Debug, Deserialize, Default)]
pub(crate) struct PluginMetadata {
    /// Identificador con el que se registra el plugin.
    pub(crate) id: Option<String>,
    /// Capacidades, si el repositorio quiere declararlas sin arrancar.
    pub(crate) capabilities: Option<Vec<Capability>>,
}

/// Lo que se lee del `Cargo.toml` del repositorio.
#[derive(Debug, Deserialize)]
pub(crate) struct CargoManifest {
    pub(crate) package: CargoPackage,
    #[serde(default)]
    pub(crate) bin: Vec<CargoBin>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CargoPackage {
    pub(crate) name: String,
    /// `version` puede ser literal o heredarse del workspace
    /// (`version.workspace = true`), así que se acepta como tabla.
    #[serde(default)]
    pub(crate) version: Option<WorkspaceVersion>,
    #[serde(default)]
    pub(crate) metadata: Option<MetadataTable>,
}

/// Valor de `package.version`: literal o declaración de herencia.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum WorkspaceVersion {
    Literal(String),
    Inherited {
        #[allow(dead_code)]
        workspace: bool,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct CargoBin {
    pub(crate) name: String,
}

/// Rutas donde puede quedar el ejecutable tras compilar.
pub(crate) fn candidate_paths(dir: &Path, name: &str) -> Vec<PathBuf> {
    vec![
        dir.join("target/release").join(name),
        dir.join("target/release").join(format!("{name}.exe")),
        dir.join("target/debug").join(name),
        dir.join("target/debug").join(format!("{name}.exe")),
    ]
}

/// Nombre del binario que hay que construir: el primer `[[bin]]` o, si no se
/// declara ninguno, el nombre del paquete (que es el bin por defecto de cargo).
pub(crate) fn binary_name(manifest: &CargoManifest) -> String {
    manifest
        .bin
        .first()
        .map(|b| b.name.clone())
        .unwrap_or_else(|| manifest.package.name.clone())
}

/// Resuelve `package.version`, incluida la herencia del workspace.
pub(crate) fn resolve_version(
    package_version: &Option<WorkspaceVersion>,
    plugin_dir: &Path,
    staging: &Path,
) -> String {
    match package_version {
        Some(WorkspaceVersion::Literal(value)) => value.clone(),
        Some(WorkspaceVersion::Inherited { .. }) | None => workspace_version(plugin_dir)
            .or_else(|| workspace_version(staging))
            .unwrap_or_else(|| "0.0.0".to_string()),
    }
}

/// `[workspace.package].version` de la raíz indicada, si existe.
fn workspace_version(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let value: toml::Value = toml::from_str(&text).ok()?;
    value
        .get("workspace")?
        .get("package")?
        .get("version")?
        .as_str()
        .map(str::to_string)
}

/// Si la raíz es un workspace virtual, devuelve los plugins disponibles.
///
/// Se apoya en que un `CargoManifest` falló al parsear: si además hay
/// `[workspace]` y no hay `[package]`, lo que falta no es un toml válido sino
/// elegir qué plugin instalar.
pub(crate) fn virtual_workspace_plugins(root: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let value: toml::Value = toml::from_str(&text).ok()?;
    if value.get("package").is_some() || value.get("workspace").is_none() {
        return None;
    }
    Some(plugin_directories(root))
}

/// Directorios del workspace que contienen un plugin.
///
/// Primero usa `members` del `Cargo.toml` raíz; si trae globs (por ejemplo
/// `plugins/*`) o no deja ninguno, recorre el árbol buscando `Cargo.toml` que
/// sean de un plugin. Los resultados son rutas relativas a `root`, listas para
/// pegar en `fuente#ruta`.
pub(crate) fn plugin_directories(root: &Path) -> Vec<String> {
    if let Some(members) = workspace_members(root) {
        if members.iter().all(|m| !m.contains('*') && !m.contains('?')) {
            let mut found = Vec::new();
            for member in members {
                let member = member.trim_end_matches('/').to_string();
                if member.is_empty() || !is_plugin_dir(&root.join(&member)) {
                    continue;
                }
                if !found.contains(&member) {
                    found.push(member);
                }
            }
            if !found.is_empty() {
                return found;
            }
        }
    }

    let mut found = Vec::new();
    scan_plugin_dirs(root, root, 0, &mut found);
    found.sort();
    found
}

/// Lista `members` del workspace sin expandir globs.
fn workspace_members(root: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let value: toml::Value = toml::from_str(&text).ok()?;
    value
        .get("workspace")?
        .get("members")?
        .as_array()
        .map(|members| {
            members
                .iter()
                .filter_map(|m| m.as_str().map(str::to_string))
                .collect()
        })
}

/// Recorre `dir` buscando directorios con un `Cargo.toml` de plugin.
fn scan_plugin_dirs(root: &Path, dir: &Path, depth: usize, found: &mut Vec<String>) {
    const MAX_DEPTH: usize = 3;
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "target" {
            continue;
        }
        if is_plugin_dir(&path) {
            if let Ok(relative) = path.strip_prefix(root) {
                let relative = relative.to_string_lossy().replace('\\', "/");
                if !found.contains(&relative) {
                    found.push(relative);
                }
            }
        }
        scan_plugin_dirs(root, &path, depth + 1, found);
    }
}

/// `true` si el directorio contiene un `Cargo.toml` con forma de plugin.
///
/// Se exige `[package.metadata.port]`, un `[[bin]]` o `src/main.rs` para no
/// sugerir crates de biblioteca que solo son miembros auxiliares del workspace.
fn is_plugin_dir(dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
        return false;
    };
    let Ok(value) = toml::from_str::<toml::Value>(&text) else {
        return false;
    };
    let Some(package) = value.get("package") else {
        return false;
    };
    let has_metadata = package
        .get("metadata")
        .and_then(|m| m.get("port"))
        .is_some();
    let has_bin = value.get("bin").is_some();
    let has_main = dir.join("src/main.rs").is_file();
    has_metadata || has_bin || has_main
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
