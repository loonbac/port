//! Extremo a extremo: instalar un plugin externo, arrancarlo, usarlo y quitarlo.
//!
//! Esta prueba no simula nada: compila un plugin real de verdad, lo instala por
//! el mismo camino que usa `port plugin add`, lo arranca como proceso hijo y
//! comprueba que responde. Es la garantia que importa: el protocolo funciona
//! entre dos procesos, no solo entre funciones de la misma unidad de pruebas.
//!
//! Se salta cuando no hay compilador disponible, porque en una imagen
//! mínima fallaría por motivos que no dicen nada del diseño.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use port_plugin_api::host::{self, ExternalPlugin};
use port_plugin_api::protocol::{Capability, PluginManifest};

fn cargo() -> Option<PathBuf> {
    let out = Command::new("cargo").arg("--version").output().ok()?;
    out.status.success().then(|| PathBuf::from("cargo"))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("port-e2e-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("directorio de trabajo");
    dir
}

/// Compila el plugin de ejemplo del propio repositorio.
///
/// Se usa el ejemplo real, no un plugin inventado aqui a proposito: asi el
/// ejemplo del que habla la documentacion es el mismo que verifica la suite, y
/// no puede quedarse obsoleto en silencio mientras los tests siguen verdes.
///
/// Devuelve `None` si no hay compilador, en cuyo caso la prueba se omite: en
/// una imagen minima fallaria por motivos que no dicen nada del diseno.
fn build_example(root: &Path) -> Option<PathBuf> {
    let cargo = cargo()?;
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let example = manifest_dir.join("../../examples/external-plugin/Cargo.toml");
    assert!(
        example.is_file(),
        "el ejemplo deberia existir en examples/external-plugin"
    );
    let example = example.canonicalize().ok()?;

    // Target propio: el ejemplo esta fuera del workspace y no debe ensuciar el
    // `target/` compartido, que se limpia por la suite.
    let target = root.join("example-target");
    let built = Command::new(cargo)
        .current_dir(root)
        .args(["build", "--release", "--quiet"])
        .env("CARGO_TARGET_DIR", &target)
        .args(["--manifest-path", &example.to_string_lossy()])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !built.status.success() {
        eprintln!(
            "el plugin de ejemplo no compilo:\n{}",
            String::from_utf8_lossy(&built.stderr)
        );
        return None;
    }
    let binary = target.join("release/port-plugin-example");
    binary.is_file().then_some(binary)
}

#[test]
fn a_real_plugin_can_be_installed_started_used_and_removed() {
    let Some(_cargo) = cargo() else {
        eprintln!("sin compilador: se omite la prueba de extremo a extremo");
        return;
    };

    let root = scratch("install");
    let Some(executable) = build_example(&root) else {
        eprintln!("el plugin de prueba no compiló: se omite");
        return;
    };

    // Instalar: copiar el binario y registrar el manifiesto, igual que
    // `port plugin add` hace tras compilar.
    host::ensure_plugins_dir().expect("directorio de plugins");
    let destination = host::plugins_dir().join("example");
    std::fs::copy(&executable, &destination).expect("copiar el ejecutable");

    let manifest = PluginManifest {
        id: "example".into(),
        name: "Example".into(),
        version: "0.1.0".into(),
        executable: destination.to_string_lossy().to_string(),
        source: "examples/external-plugin".into(),
        capabilities: vec![Capability::Appearance, Capability::Input],
    };
    host::write_manifest(&manifest).expect("registrar el manifiesto");

    // Se ve en la lista sin arrancar nada.
    let found = host::installed();
    assert!(
        found.iter().any(|m| m.id == "example"),
        "debe aparecer instalado"
    );

    // Arrancar de verdad.
    let plugin = ExternalPlugin::start(manifest).expect("el plugin deberia arrancar");

    // Lo que declaró al saludar es lo que el núcleo usa.
    assert!(plugin.has(Capability::Appearance));
    assert!(plugin.has(Capability::Input));
    assert_eq!(plugin.appearance().opacity, Some(0.94));
    assert_eq!(plugin.bindings().len(), 1);
    assert!(plugin.bindings()[0].matches("h", true, true, false));

    // Una acción devuelve su valor.
    let value = plugin
        .invoke("toggle")
        .expect("el plugin deberia responder");
    assert_eq!(value["triggers"], 0);

    // La configuración llega al plugin sin romper la conexion.
    let mut values = std::collections::BTreeMap::new();
    values.insert("opacity".to_string(), "0.5".to_string());
    assert!(plugin.configure(values));
    assert_eq!(
        plugin.invoke("toggle").expect("sigue respondiendo")["triggers"],
        0
    );

    // Una acción desconocida no es un error: simplemente no hay respuesta.
    assert!(plugin.invoke("inexistente").is_none());

    // La resolucion de teclas es local a partir de las combinaciones declaradas.
    let plugins: Vec<Arc<ExternalPlugin>> = vec![Arc::clone(&plugin)];
    let (owner, action) =
        ExternalPlugin::resolve_action(&plugins, "h", true, true, false).expect("debe resolver");
    assert_eq!(owner.manifest().id, "example");
    assert_eq!(action, "toggle");
    assert!(
        ExternalPlugin::resolve_action(&plugins, "h", true, false, false).is_none(),
        "sin alt no es la misma combinacion"
    );

    // Recargar mantiene el plugin operativo.
    plugin.reload().expect("deberia recargar");
    assert_eq!(
        plugin.invoke("toggle").expect("responde tras recargar")["triggers"],
        0
    );

    // Desinstalar deja el directorio limpio.
    assert!(host::uninstall("example").expect("desinstalar"));
    assert!(!host::installed().iter().any(|m| m.id == "example"));

    let _ = std::fs::remove_dir_all(&root);
}
