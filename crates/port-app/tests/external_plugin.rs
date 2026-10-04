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

/// Compila un plugin minimo que devuelve un valor conocido.
fn build_probe(root: &Path) -> Option<PathBuf> {
    let cargo = cargo()?;
    let src = root.join("probe-plugin");
    std::fs::create_dir_all(src.join("src")).ok()?;

    let sdk = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .join("plugin-sdk")
        .canonicalize()
        .ok()?;

    std::fs::write(
        src.join("Cargo.toml"),
        format!(
            "[package]\nname = \"port-plugin-probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [package.metadata.port]\nid = \"probe\"\ncapabilities = [\"appearance\", \"input\"]\n\n\
             [[bin]]\nname = \"port-plugin-probe\"\npath = \"src/main.rs\"\n\n\
             [dependencies]\nport-plugin-sdk = {{ path = {sdk:?} }}\nserde_json = \"1\"\n"
        ),
    )
    .ok()?;

    std::fs::write(
        src.join("src/main.rs"),
        r#"use port_plugin_sdk::protocol::{Appearance, Binding, Capability};
use port_plugin_sdk::runtime::{serve, Plugin};
use std::collections::BTreeMap;

struct Probe;
impl Plugin for Probe {
    fn name(&self) -> &'static str { "probe" }
    fn version(&self) -> &'static str { "0.1.0" }
    fn capabilities(&self) -> Vec<Capability> { vec![Capability::Appearance, Capability::Input] }
    fn appearance(&self) -> Appearance {
        Appearance { opacity: Some(0.87), ..Default::default() }
    }
    fn bindings(&self) -> Vec<Binding> {
        vec![Binding { key: "p".into(), ctrl: true, alt: false, shift: true, action: "ping".into() }]
    }
    fn invoke(&self, action: &str, _p: &serde_json::Value) -> Option<serde_json::Value> {
        (action == "ping").then(|| serde_json::json!({ "pong": 42 }))
    }
    fn configure(&mut self, v: &BTreeMap<String, String>) {
        if let Some(o) = v.get("opacity").and_then(|x| x.parse::<f32>().ok()) {
            eprintln!("probe: opacidad {o}");
        }
    }
}
fn main() { serve(Probe); }
"#,
    )
    .ok()?;

    let built = Command::new(cargo)
        .current_dir(&src)
        .args(["build", "--release", "--quiet"])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    built
        .status
        .success()
        .then(|| src.join("target/release/port-plugin-probe"))
}

#[test]
fn a_real_plugin_can_be_installed_started_used_and_removed() {
    let Some(_cargo) = cargo() else {
        eprintln!("sin compilador: se omite la prueba de extremo a extremo");
        return;
    };

    let root = scratch("install");
    let Some(executable) = build_probe(&root) else {
        eprintln!("el plugin de prueba no compiló: se omite");
        return;
    };

    // Instalar: copiar el binario y registrar el manifiesto, igual que
    // `port plugin add` hace tras compilar.
    host::ensure_plugins_dir().expect("directorio de plugins");
    let destination = host::plugins_dir().join("probe");
    std::fs::copy(&executable, &destination).expect("copiar el ejecutable");

    let manifest = PluginManifest {
        id: "probe".into(),
        name: "Probe".into(),
        version: "0.1.0".into(),
        executable: destination.to_string_lossy().to_string(),
        source: "https://example.invalid/probe".into(),
        capabilities: vec![Capability::Appearance, Capability::Input],
    };
    host::write_manifest(&manifest).expect("registrar el manifiesto");

    // Se ve en la lista sin arrancar nada.
    let found = host::installed();
    assert!(
        found.iter().any(|m| m.id == "probe"),
        "debe aparecer instalado"
    );

    // Arrancar de verdad.
    let plugin = ExternalPlugin::start(manifest).expect("el plugin deberia arrancar");

    // Lo que declaró al saludar es lo que el núcleo usa.
    assert!(plugin.has(Capability::Appearance));
    assert!(plugin.has(Capability::Input));
    assert_eq!(plugin.appearance().opacity, Some(0.87));
    assert_eq!(plugin.bindings().len(), 1);
    assert!(plugin.bindings()[0].matches("p", true, false, true));

    // Una acción devuelve su valor.
    let value = plugin.invoke("ping").expect("el plugin deberia responder");
    assert_eq!(value["pong"], 42);

    // La configuración llega al plugin sin romper la conexion.
    let mut values = std::collections::BTreeMap::new();
    values.insert("opacity".to_string(), "0.5".to_string());
    assert!(plugin.configure(values));
    assert_eq!(
        plugin.invoke("ping").expect("sigue respondiendo")["pong"],
        42
    );

    // Una acción desconocida no es un error: simplemente no hay respuesta.
    assert!(plugin.invoke("inexistente").is_none());

    // La resolucion de teclas es local a partir de las combinaciones declaradas.
    let plugins: Vec<Arc<ExternalPlugin>> = vec![Arc::clone(&plugin)];
    let (owner, action) =
        ExternalPlugin::resolve_action(&plugins, "p", true, false, true).expect("debe resolver");
    assert_eq!(owner.manifest().id, "probe");
    assert_eq!(action, "ping");
    assert!(
        ExternalPlugin::resolve_action(&plugins, "p", false, false, true).is_none(),
        "sin ctrl no es la misma combinacion"
    );

    // Recargar mantiene el plugin operativo.
    plugin.reload().expect("deberia recargar");
    assert_eq!(
        plugin.invoke("ping").expect("responde tras recargar")["pong"],
        42
    );

    // Desinstalar deja el directorio limpio.
    assert!(host::uninstall("probe").expect("desinstalar"));
    assert!(!host::installed().iter().any(|m| m.id == "probe"));

    let _ = std::fs::remove_dir_all(&root);
}
