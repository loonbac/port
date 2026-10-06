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

mod manifest;
mod source;
mod staging;

use std::path::{Path, PathBuf};

use crate::host;
use crate::protocol::PluginManifest;

pub(crate) use self::manifest::{
    binary_name, candidate_paths, plugin_directories, resolve_version, virtual_workspace_plugins,
    CargoManifest,
};
pub(crate) use self::source::split_source;
pub(crate) use self::staging::{
    build, clone, copy_tree, resolve_path_dependencies, staging_dir, StagingGuard,
};

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
    use std::process::Command;

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
