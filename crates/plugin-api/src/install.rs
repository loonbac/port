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
use std::sync::atomic::{AtomicU64, Ordering};

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
    /// La fuente es un workspace virtual: hay que elegir un plugin con `#`.
    WorkspaceWithoutSelection {
        source: String,
        plugins: Vec<String>,
    },
    /// El selector `#...` no apunta a ningún plugin del repositorio.
    UnknownSelection {
        source: String,
        selector: String,
        plugins: Vec<String>,
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
            Self::WorkspaceWithoutSelection { source, plugins } => {
                write!(
                    f,
                    "{source} es un workspace virtual (no tiene [package] propio): elige un plugin con el selector '#'.\nPlugins disponibles:"
                )?;
                write_available_plugins(f, source, plugins)
            }
            Self::UnknownSelection {
                source,
                selector,
                plugins,
            } => {
                write!(
                    f,
                    "{source} no tiene un plugin en el subdirectorio '{selector}': falta {selector}/Cargo.toml.\nPlugins disponibles:"
                )?;
                write_available_plugins(f, source, plugins)
            }
        }
    }
}

/// Escribe la lista de plugins disponibles, ya lista para copiar y pegar.
///
/// La comparten los dos errores que dejan elegir un plugin: el workspace
/// virtual sin selector y un selector que no apunta a ningún plugin.
fn write_available_plugins(
    f: &mut std::fmt::Formatter<'_>,
    source: &str,
    plugins: &[String],
) -> std::fmt::Result {
    if plugins.is_empty() {
        write!(f, " ninguno con un Cargo.toml de plugin")
    } else {
        for dir in plugins {
            write!(f, "\n  port plugin add {source}#{dir}")?;
        }
        Ok(())
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
    /// `version` puede ser literal o heredarse del workspace
    /// (`version.workspace = true`), así que se acepta como tabla.
    #[serde(default)]
    version: Option<WorkspaceVersion>,
    #[serde(default)]
    metadata: Option<MetadataTable>,
}

/// Valor de `package.version`: literal o declaración de herencia.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WorkspaceVersion {
    Literal(String),
    Inherited {
        #[allow(dead_code)]
        workspace: bool,
    },
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

/// Directorio de staging único por instalación.
///
/// Es único y no solo por PID: dos pruebas del mismo proceso corren en hilos y
/// compartirían la carpeta si el nombre dependiera únicamente del proceso.
fn staging_dir() -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("port-plugin-build-{}-{seq}", std::process::id()))
}

/// Limpia el staging al salir, incluso si la instalación falla a mitad.
///
/// Cada instalación usa una carpeta única, así que sin esto un error dejaría
/// un `target/` huérfano en `/tmp` en cada intento.
struct StagingGuard(PathBuf);

impl Drop for StagingGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Separa la fuente en su repositorio y, si procede, la subcarpeta del plugin.
///
/// Acepta dos formas de selector de subdirectorio:
///
/// - `fuente#subdirectorio`, la convención explícita. Vale igual para una URL
///   que para una ruta local.
/// - una URL que ya trae la ruta (`https://github.com/usuario/repo/plugins/x`),
///   donde se toman los dos primeros segmentos tras el host como repositorio y
///   el resto como subdirectorio. Es una heurística para GitHub/GitLab/Codeberg:
///   un host que sirva un repositorio bajo una ruta arbitraria se partiría mal,
///   así que en ese caso hay que usar `#`.
///
/// Una ruta local intacta (sin `#`) es exactamente el directorio indicado y no
/// se le recorta nada: en disco no existe la convención `host/owner/repo`.
fn split_source(source: &str) -> (String, Option<String>) {
    if let Some((base, sub)) = source.rsplit_once('#') {
        let sub = sub.trim_matches('/');
        if !base.is_empty() && !sub.is_empty() {
            return (base.to_string(), Some(sub.to_string()));
        }
    }
    match url_with_path(source) {
        Some((base, sub)) => (base, Some(sub)),
        None => (source.to_string(), None),
    }
}

/// Extrae repositorio y subcarpeta de una URL que ya incluye la ruta.
///
/// Devuelve `None` cuando no hay subcarpeta que separar (dos segmentos o menos
/// tras el host) o cuando la fuente no es una URL con esquema.
fn url_with_path(source: &str) -> Option<(String, String)> {
    let scheme_end = source.find("://")?;
    let rest = &source[scheme_end + 3..];
    let slash = rest.find('/')?;
    let host = &rest[..slash];
    let path = rest[slash + 1..].trim_end_matches('/');
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() <= 2 {
        return None;
    }
    let subdir = segments[2..].join("/");
    if subdir.is_empty() {
        return None;
    }
    let repo = format!(
        "{}://{}/{}",
        &source[..scheme_end],
        host,
        segments[..2].join("/")
    );
    Some((repo, subdir))
}

/// Nombre del binario que hay que construir: el primer `[[bin]]` o, si no se
/// declara ninguno, el nombre del paquete (que es el bin por defecto de cargo).
fn binary_name(manifest: &CargoManifest) -> String {
    manifest
        .bin
        .first()
        .map(|b| b.name.clone())
        .unwrap_or_else(|| manifest.package.name.clone())
}

/// Resuelve `package.version`, incluida la herencia del workspace.
fn resolve_version(
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
fn virtual_workspace_plugins(root: &Path) -> Option<Vec<String>> {
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
fn plugin_directories(root: &Path) -> Vec<String> {
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

/// Clona un repositorio en `destino`.
///
/// Sin historial: la compilacion necesita el contenido, no la historia, y un
/// clon completo de un repositorio largo es una espera de sobra.
fn clone(url: &str, destino: &Path) -> Result<(), InstallError> {
    let clone = Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "--quiet",
            url,
            &destino.to_string_lossy(),
        ])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| InstallError::Clone {
            url: url.to_string(),
            source: e,
        })?;
    if clone.status.success() {
        Ok(())
    } else {
        Err(InstallError::Clone {
            url: url.to_string(),
            source: std::io::Error::other(String::from_utf8_lossy(&clone.stderr).trim()),
        })
    }
}

/// Copia un arbol de directorios, saltando lo que cargo ya genera.
///
/// `target/` no se copia: puede pesar mas que el propio plugin y siempre se
/// reconstruye en el siguiente paso.
fn copy_tree(from: &Path, to: &Path) -> Result<(), InstallError> {
    let entries = std::fs::read_dir(from).map_err(|e| InstallError::Clone {
        url: from.display().to_string(),
        source: e,
    })?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == "target" || name == ".git" {
            continue;
        }
        let from_path = entry.path();
        let to_path = to.join(&name);
        if from_path.is_dir() {
            std::fs::create_dir_all(&to_path).map_err(|e| InstallError::Clone {
                url: from_path.display().to_string(),
                source: e,
            })?;
            copy_tree(&from_path, &to_path)?;
        } else {
            std::fs::copy(&from_path, &to_path).map_err(|source_err| InstallError::Copy {
                source: from_path.clone(),
                source_err,
            })?;
        }
    }
    Ok(())
}

/// Reescribe las dependencias por ruta para que apunten al origen original.
///
/// Al copiar el plugin a un directorio temporal, una dependencia escrita como
/// `path = "../crates/algo"` dejaria de resolver. Convertirla a ruta absoluta
/// antes de compilar mantiene la copia sin perder la referencia.
///
/// Es una transformación deliberadamente simple: se reescribe cualquier
/// `path = "..."` de un `Cargo.toml`, sin interpretar el resto del manifiesto.
/// Un plugin publicado usa versiones de crates.io y no le afecta.
fn resolve_path_dependencies(from: &Path, to: &Path) -> Result<(), InstallError> {
    let manifest = to.join("Cargo.toml");
    let Ok(text) = std::fs::read_to_string(&manifest) else {
        return Ok(());
    };

    const MARKER: &str = "path = \"";
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();

    // Se recorre de ocurrencia en ocurrencia. Cortar el texto en trozos con
    // `split_inclusive` no sirve: dos `path = "` pueden caer en el mismo trozo y
    // la segunda se perderia.
    while let Some(at) = rest.find(MARKER) {
        let (before, after_marker) = rest.split_at(at);
        let after = &after_marker[MARKER.len()..];
        out.push_str(before);
        out.push_str(MARKER);

        match after.find('"') {
            Some(end) => {
                let (value, tail) = after.split_at(end);
                out.push_str(&rewrite_dependency_path(from, value));
                rest = tail;
            }
            // Comilla sin cerrar: el manifiesto esta roto y no es asunto
            // nuestro; se copia tal cual y que lo diga cargo.
            None => {
                rest = after;
            }
        }
    }
    out.push_str(rest);

    if out != text {
        std::fs::write(&manifest, out).map_err(|e| InstallError::Clone {
            url: from.display().to_string(),
            source: e,
        })?;
    }
    Ok(())
}

/// Convierte una ruta de dependencia que escapa del plugin en una absoluta.
///
/// Solo se tocan las rutas relativas que empiezan por `..`. Un
/// `path = "src/main.rs"` de `[[bin]]` es interno: el archivo viaja con la
/// copia y su ruta sigue siendo correcta dentro del staging.
fn rewrite_dependency_path(from: &Path, value: &str) -> String {
    let path = Path::new(value);
    if !value.starts_with("..") || path.is_absolute() {
        return value.to_string();
    }
    from.join(path).display().to_string()
}

/// Compila el plugin con un manifiesto y un binario explícitos.
///
/// El manifiesto apunta al subdirectorio elegido, pero el staging conserva el
/// repositorio completo: así `version.workspace = true` y
/// `[workspace.dependencies]` siguen resolviendo. El `target/` resultante vive
/// en la raíz del workspace, no en el subdirectorio.
fn build(manifest_path: &Path, bin: &str, plugin_dir: &Path) -> Result<(), InstallError> {
    let output = Command::new("cargo")
        .current_dir(plugin_dir)
        .args(["build", "--release", "--manifest-path"])
        .arg(manifest_path)
        .args(["--bin", bin])
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
    Ok(())
}

/// Instala un plugin desde una URL o ruta, en el directorio de plugins habitual.
///
/// Devuelve el manifiesto registrado. Si el plugin ya estaba instalado, se
/// vuelve a construir y se reemplaza: actualizar y reinstalar son la misma
/// operación.
pub fn install(source: &str) -> Result<PluginManifest, InstallError> {
    install_into(source, &host::plugins_dir())
}

/// Igual que [`install`], pero escribiendo en `plugins_root`.
///
/// Existe para que las pruebas no dependan de `PORT_PLUGIN_DIR`, que es global
/// al proceso y se pisaría entre hilos.
fn install_into(source: &str, plugins_root: &Path) -> Result<PluginManifest, InstallError> {
    let (base, subdir) = split_source(source);
    let staging = staging_dir();
    let _ = std::fs::remove_dir_all(&staging);
    // El staging se limpia al salir, haya éxito o error.
    let _staging_guard = StagingGuard(staging.clone());

    // Se copia o clona SIEMPRE la raíz del repositorio, no solo la carpeta del
    // plugin: la herencia del workspace depende del `Cargo.toml` raíz y de sus
    // `members`, que no viajarían con una copia suelta.
    let local = Path::new(&base);
    if local.is_dir() {
        std::fs::create_dir_all(&staging).map_err(|e| InstallError::Clone {
            url: source.to_string(),
            source: e,
        })?;
        copy_tree(local, &staging)?;
        resolve_path_dependencies(local, &staging)?;
    } else {
        clone(&base, &staging)?;
    }

    // El manifiesto que identifica al plugin es el del subdirectorio elegido; la
    // raíz solo aporta el workspace. Sin selector, la raíz es el propio plugin.
    let plugin_dir = match &subdir {
        Some(relative) => staging.join(relative),
        None => staging.clone(),
    };
    let cargo_path = plugin_dir.join("Cargo.toml");
    let text = match std::fs::read_to_string(&cargo_path) {
        Ok(text) => text,
        // Con selector, que falte el `Cargo.toml` ya no significa "esto no es
        // un plugin": puede que el subdirectorio elegido no exista o no sea un
        // plugin. Nombrarlo y listar lo que sí hay es lo que permite corregir
        // el comando.
        Err(_) => {
            return Err(match &subdir {
                Some(relative) => InstallError::UnknownSelection {
                    source: source.to_string(),
                    selector: relative.clone(),
                    plugins: plugin_directories(&staging),
                },
                None => InstallError::MissingManifest {
                    url: source.to_string(),
                },
            });
        }
    };

    let manifest: CargoManifest = match toml::from_str(&text) {
        Ok(parsed) => parsed,
        Err(e) => {
            // Una raíz sin `[package]` puede ser un workspace virtual: lo que
            // falta no es un toml válido sino elegir qué plugin instalar.
            if subdir.is_none() {
                if let Some(plugins) = virtual_workspace_plugins(&staging) {
                    return Err(InstallError::WorkspaceWithoutSelection {
                        source: source.to_string(),
                        plugins,
                    });
                }
            }
            return Err(InstallError::InvalidManifest {
                url: source.to_string(),
                source: e,
            });
        }
    };

    let id = manifest
        .package
        .metadata
        .as_ref()
        .and_then(|m| m.port.as_ref())
        .and_then(|p| p.id.clone())
        .unwrap_or_else(|| manifest.package.name.replace('-', "_"));

    let bin = binary_name(&manifest);
    let version = resolve_version(&manifest.package.version, &plugin_dir, &staging);

    build(&cargo_path, &bin, &plugin_dir)?;

    // Se busca el binario por el nombre del paquete o por cualquier `[[bin]]`.
    // El `target/` puede estar en la raíz del workspace o junto al plugin si no
    // forma parte de uno.
    let names: Vec<String> = if manifest.bin.is_empty() {
        vec![manifest.package.name.clone()]
    } else {
        manifest.bin.iter().map(|b| b.name.clone()).collect()
    };

    let mut found = None;
    let mut tried = Vec::new();
    for name in &names {
        let candidates = candidate_paths(&staging, name)
            .into_iter()
            .chain(candidate_paths(&plugin_dir, name));
        for candidate in candidates {
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

    std::fs::create_dir_all(plugins_root)?;
    let destination = plugins_root.join(&id);
    std::fs::copy(&built, &destination).map_err(|source_err| InstallError::Copy {
        source: built.clone(),
        source_err,
    })?;
    set_executable(&destination)?;

    let manifest = PluginManifest {
        id,
        name: manifest.package.name.clone(),
        version,
        executable: destination.to_string_lossy().to_string(),
        source: source.to_string(),
        capabilities: manifest
            .package
            .metadata
            .and_then(|m| m.port)
            .and_then(|p| p.capabilities)
            .unwrap_or_default(),
    };
    host::write_manifest_in(plugins_root, &manifest)?;

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
    fn a_local_path_install_rewrites_escaping_dependencies() {
        // El caso que Rompio la documentacion: al copiar un plugin con una
        // dependencia `path = "../..."`, esa ruta deja de resolver en el
        // staging y hay que apuntarla al origen.
        let from = std::env::temp_dir().join(format!("port-from-{}", std::process::id()));
        let to = std::env::temp_dir().join(format!("port-to-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&from);
        let _ = std::fs::remove_dir_all(&to);
        std::fs::create_dir_all(&from).unwrap();
        std::fs::create_dir_all(&to).unwrap();
        std::fs::write(
            from.join("Cargo.toml"),
            "[[bin]]\nname = \"x\"\npath = \"src/main.rs\"\n\n             [dependencies]\nsdk = { path = \"../crates/sdk\" }\n",
        )
        .unwrap();

        copy_tree(&from, &to).unwrap();
        resolve_path_dependencies(&from, &to).unwrap();

        let staged = std::fs::read_to_string(to.join("Cargo.toml")).unwrap();
        assert!(
            staged.contains(&from.join("../crates/sdk").display().to_string()),
            "la dependencia que escapa debe volverse absoluta: {staged}"
        );
        assert!(
            staged.contains(r#"path = "src/main.rs""#),
            "el path de [[bin]] es interno y no debe tocarse: {staged}"
        );

        let _ = std::fs::remove_dir_all(&from);
        let _ = std::fs::remove_dir_all(&to);
    }

    #[test]
    fn copying_a_tree_skips_build_output() {
        let from = std::env::temp_dir().join(format!("port-skip-{}", std::process::id()));
        let to = std::env::temp_dir().join(format!("port-skip-to-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&from);
        std::fs::create_dir_all(from.join("src")).unwrap();
        std::fs::create_dir_all(from.join("target")).unwrap();
        std::fs::create_dir_all(to.clone()).unwrap();
        std::fs::write(from.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(from.join("target/big.bin"), "pesado").unwrap();

        copy_tree(&from, &to).unwrap();

        assert!(to.join("src/main.rs").is_file());
        assert!(
            !to.join("target").exists(),
            "target/ no debe copiarse: pesa mas que el plugin y se reconstruye"
        );

        let _ = std::fs::remove_dir_all(&from);
        let _ = std::fs::remove_dir_all(&to);
    }

    #[test]
    fn binary_paths_cover_release_and_debug_and_windows() {
        let paths = candidate_paths(Path::new("/repo"), "demo");
        let names: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        assert!(names.iter().any(|p| p.contains("release/demo")));
        assert!(names.iter().any(|p| p.contains("release/demo.exe")));
        assert!(names.iter().any(|p| p.contains("debug/demo")));
    }

    // --- Selector de subdirectorio en la fuente ---

    #[test]
    fn el_selector_con_almohadilla_separa_repositorio_y_subdirectorio() {
        assert_eq!(
            split_source("https://github.com/loonbac/port-plugins#plugins/font"),
            (
                "https://github.com/loonbac/port-plugins".to_string(),
                Some("plugins/font".to_string())
            )
        );
        assert_eq!(
            split_source("/tmp/monorepo#plugins/font"),
            (
                "/tmp/monorepo".to_string(),
                Some("plugins/font".to_string())
            )
        );
    }

    #[test]
    fn una_url_con_ruta_se_parte_con_la_heuristica_de_dos_segmentos() {
        assert_eq!(
            split_source("https://github.com/loonbac/port-plugins/plugins/font"),
            (
                "https://github.com/loonbac/port-plugins".to_string(),
                Some("plugins/font".to_string())
            )
        );
        assert_eq!(
            url_with_path("https://codeberg.org/loonbac/port-plugins/plugins/font"),
            Some((
                "https://codeberg.org/loonbac/port-plugins".to_string(),
                "plugins/font".to_string()
            ))
        );
        // La heurística también se aplica a hosts que no son un servicio git, y
        // ahí puede equivocarse al partir la ruta: por eso se documenta que en
        // esos casos conviene usar `#`.
        assert_eq!(
            split_source("https://git.interno.local/x/plugins/font").1,
            Some("font".to_string())
        );
    }

    #[test]
    fn una_url_sin_ruta_extra_no_lleva_subdirectorio() {
        assert_eq!(
            split_source("https://github.com/loonbac/port-plugins").1,
            None
        );
        assert_eq!(
            split_source("https://github.com/loonbac/port-plugins.git").1,
            None
        );
    }

    #[test]
    fn una_ruta_local_no_se_recorta_por_la_heuristica() {
        assert_eq!(split_source("/home/u/repos/mi-plugin").1, None);
        assert_eq!(split_source("./crates/plugin-demo").1, None);
    }

    #[test]
    fn el_selector_tiene_prioridad_sobre_la_heuristica() {
        assert_eq!(
            split_source("https://github.com/loonbac/port-plugins#plugins/font"),
            (
                "https://github.com/loonbac/port-plugins".to_string(),
                Some("plugins/font".to_string())
            )
        );
        // El `.git` del repo no es un subdirectorio.
        assert_eq!(
            split_source("https://github.com/loonbac/port-plugins.git"),
            (
                "https://github.com/loonbac/port-plugins.git".to_string(),
                None
            )
        );
    }

    // --- Instalación de un monorepo real ---

    /// Crea un workspace temporal con un plugin mínimo que hereda `version`.
    fn fixture_workspace(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("port-monorepo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("plugins/demo/src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nresolver = \"2\"\nmembers = [\"plugins/demo\"]\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(
            root.join("plugins/demo/Cargo.toml"),
            "[package]\nname = \"port-plugin-demo\"\nversion.workspace = true\nedition.workspace = true\n\n[package.metadata.port]\nid = \"demo\"\n\n[[bin]]\nname = \"demo\"\npath = \"src/main.rs\"\n",
        )
        .unwrap();
        std::fs::write(root.join("plugins/demo/src/main.rs"), "fn main() {}\n").unwrap();
        root
    }

    /// Comprueba si hay un compilador con el que construir el fixture.
    fn cargo_disponible() -> bool {
        Command::new("cargo")
            .arg("--version")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn instalar_desde_un_monorepo_local_hereda_la_version_del_workspace() {
        if !cargo_disponible() {
            eprintln!("sin compilador: se omite la instalación del monorepo");
            return;
        }
        let root = fixture_workspace("install");
        let dest = std::env::temp_dir().join(format!("port-monorepo-dest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dest);

        let source = format!("{}#plugins/demo", root.display());
        let manifest =
            install_into(&source, &dest).expect("el plugin del monorepo debería instalarse");

        assert_eq!(manifest.id, "demo");
        assert_eq!(manifest.name, "port-plugin-demo");
        assert_eq!(
            manifest.version, "0.1.0",
            "version.workspace = true debe resolverse desde la raíz"
        );
        assert!(
            dest.join("demo").is_file(),
            "el binario debe quedar instalado"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn la_instalacion_guarda_el_origen_y_supervive_la_lectura() {
        // El origen es lo único que permite actualizar después: si no queda
        // escrito en el manifiesto, `port plugin update` no tendría de dónde
        // tirar. La instalación ya lo persiste; esta prueba fija esa ida y
        // vuelta para que un cambio futuro no la rompa en silencio.
        if !cargo_disponible() {
            eprintln!("sin compilador: se omite la comprobación del origen");
            return;
        }
        let root = fixture_workspace("source");
        let dest =
            std::env::temp_dir().join(format!("port-monorepo-sourcedest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dest);

        let source = format!("{}#plugins/demo", root.display());
        let installed = install_into(&source, &dest).expect("el plugin debería instalarse");
        assert_eq!(installed.source, source, "el origen debe quedar tal cual");

        let read_back = host::installed_in(&dest)
            .into_iter()
            .find(|m| m.id == "demo")
            .expect("el manifiesto debería leerse de vuelta");
        assert_eq!(
            read_back.source, source,
            "el origen debe sobrevivir la ida y vuelta por el manifiesto"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn un_workspace_virtual_sin_selector_lista_los_plugins_disponibles() {
        let root = fixture_workspace("list");
        let dest = std::env::temp_dir().join(format!("port-monorepo-list-{}", std::process::id()));

        let error = install_into(&root.to_string_lossy(), &dest)
            .expect_err("un workspace virtual sin selector debe fallar");
        match &error {
            InstallError::WorkspaceWithoutSelection { plugins, .. } => {
                assert!(
                    plugins.iter().any(|p| p == "plugins/demo"),
                    "debe listar el plugin disponible: {plugins:?}"
                );
            }
            other => panic!("se esperaba el error de workspace, llegó {other:?}"),
        }
        let message = error.to_string();
        assert!(
            message.contains("port plugin add") && message.contains("#plugins/demo"),
            "el mensaje debe ser copiable y pegable: {message}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn un_selector_inexistente_nombra_el_subdirectorio_y_lista_los_plugins() {
        let root = fixture_workspace("selector-malo");
        let dest =
            std::env::temp_dir().join(format!("port-monorepo-badsel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dest);

        // `plugins/font` no existe: el error genérico de "falta Cargo.toml"
        // no diría qué subdirectorio falta ni qué había disponible.
        let source = format!("{}#plugins/font", root.display());
        let error = install_into(&source, &dest).expect_err("un selector inexistente debe fallar");
        match &error {
            InstallError::UnknownSelection {
                selector, plugins, ..
            } => {
                assert_eq!(selector, "plugins/font");
                assert!(
                    plugins.iter().any(|p| p == "plugins/demo"),
                    "debe listar el plugin disponible: {plugins:?}"
                );
            }
            other => panic!("se esperaba el error de selector, llegó {other:?}"),
        }
        let message = error.to_string();
        assert!(
            message.contains("plugins/font") && message.contains("plugins/demo"),
            "el mensaje debe nombrar el selector y un plugin disponible: {message}"
        );
        assert!(
            message.contains("port plugin add"),
            "el mensaje debe dejar pegar el comando corregido: {message}"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn un_selector_sobre_un_directorio_sin_cargo_toml_falla_igual() {
        let root = fixture_workspace("selector-sin-manifiesto");
        let dest =
            std::env::temp_dir().join(format!("port-monorepo-nomanifest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dest);

        // `plugins` existe pero no es un plugin: no tiene `Cargo.toml` propio.
        let source = format!("{}#plugins", root.display());
        let error =
            install_into(&source, &dest).expect_err("un selector sin manifiesto debe fallar");
        assert!(
            matches!(error, InstallError::UnknownSelection { .. }),
            "se esperaba el error de selector, llegó {error:?}"
        );
        let message = error.to_string();
        assert!(
            message.contains("plugins") && message.contains("plugins/demo"),
            "mismo estilo de error, con lo disponible: {message}"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&dest);
    }
}
