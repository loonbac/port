//! Árbol de compilación temporal: staging, copia del origen y build.
//!
//! Prepara una copia desechable del repositorio, resuelve las dependencias por
//! ruta que escaparían de esa copia y compila el binario. No toca la
//! instalación existente: eso ocurre al final, en el módulo padre.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use super::InstallError;

/// Directorio de staging único por instalación.
///
/// Es único y no solo por PID: dos pruebas del mismo proceso corren en hilos y
/// compartirían la carpeta si el nombre dependiera únicamente del proceso.
pub(crate) fn staging_dir() -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("port-plugin-build-{}-{seq}", std::process::id()))
}

/// Limpia el staging al salir, incluso si la instalación falla a mitad.
///
/// Cada instalación usa una carpeta única, así que sin esto un error dejaría
/// un `target/` huérfano en `/tmp` en cada intento.
pub(crate) struct StagingGuard(pub(crate) PathBuf);

impl Drop for StagingGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Clona un repositorio en `destino`.
///
/// Sin historial: la compilacion necesita el contenido, no la historia, y un
/// clon completo de un repositorio largo es una espera de sobra.
pub(crate) fn clone(url: &str, destino: &Path) -> Result<(), InstallError> {
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
pub(crate) fn copy_tree(from: &Path, to: &Path) -> Result<(), InstallError> {
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
pub(crate) fn resolve_path_dependencies(from: &Path, to: &Path) -> Result<(), InstallError> {
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
pub(crate) fn build(
    manifest_path: &Path,
    bin: &str,
    plugin_dir: &Path,
) -> Result<(), InstallError> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
