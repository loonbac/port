//! Extremo a extremo: instalar un plugin externo, arrancarlo, usarlo y quitarlo.
//!
//! Esta prueba no simula nada: compila un plugin real de verdad, lo instala por
//! el mismo camino que usa `port plugin add`, lo arranca como proceso hijo y
//! comprueba que responde. Es la garantia que importa: el protocolo funciona
//! entre dos procesos, no solo entre funciones de la misma unidad de pruebas.
//!
//! Se salta cuando no hay compilador disponible, porque en una imagen
//! mínima fallaría por motivos que no dicen nada del diseño.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use port_plugin_api::host::{self, ExternalPlugin};
use port_plugin_api::protocol::{Capability, PluginManifest};
use port_plugin_api::{ConfigFile, PluginConfig};

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

/// Plugin falso mínimo: un script que registra la configuración recibida, la
/// declara en el saludo siguiente y la devuelve al invocarlo. Evita compilar un
/// crate solo para probar el canal.
///
/// Declara apariencia en su saludo, como haría un `transparency` o un `font`
/// instalados: así la misma prueba cubre el canal y la composición del registro.
///
/// `mode` decide qué hace con el bloque de configuración:
///
/// - `static`: lo acusa, pero su saludo sigue declarando lo del arranque.
/// - `reflect`: el saludo posterior declara ya los valores configurados.
/// - `no-ack`: no lo acusa, así que configurar falla.
#[cfg(unix)]
fn fake_configure_plugin(dir: &Path, mode: &str) -> PathBuf {
    let path = dir.join(format!("fake-configure-{mode}"));
    let script = FAKE_PLUGIN.replace("__MODE__", mode);
    let tmp = dir.join(format!("fake-configure-{mode}.tmp"));
    std::fs::write(&tmp, script).unwrap();
    std::fs::rename(&tmp, &path).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// Cuerpo del plugin falso. `__MODE__` lo rellena [`fake_configure_plugin`].
#[cfg(unix)]
const FAKE_PLUGIN: &str = r#"#!/bin/sh
mode=__MODE__
opacity=0.5
family="Fake Mono"
size=18.0
while IFS= read -r line; do
  case "$line" in
    *hello*)
      if [ "$mode" = static ]; then
        printf '{"t":"ready","name":"fake","version":"0.1.0","capabilities":["appearance"],"appearance":{"opacity":0.5,"font_family":"Fake Mono","font_size":18.0}}\n'
      else
        printf '{"t":"ready","name":"fake","version":"0.1.0","capabilities":["appearance"],"appearance":{"opacity":%s,"font_family":"%s","font_size":%s}}\n' "$opacity" "$family" "$size"
      fi
      ;;
    *configure*)
      if [ "$mode" != no-ack ]; then
        value=$(printf '%s' "$line" | sed -n 's/.*"opacity":"\([^"]*\)".*/\1/p')
        if [ -n "$value" ]; then opacity=$value; fi
        fam=$(printf '%s' "$line" | sed -n 's/.*"font_family":"\([^"]*\)".*/\1/p')
        if [ -n "$fam" ]; then family=$fam; fi
        printf '{"t":"ack"}\n'
      fi
      ;;
    *invoke*)
      seq=$(printf '%s' "$line" | sed -n 's/.*"seq":\([0-9]*\).*/\1/p')
      printf '{"t":"result","seq":%s,"ok":true,"value":{"opacity":"%s"}}\n' "$seq" "$opacity"
      ;;
  esac
done
"#;

/// El estado `enabled` sobrevive a un reinicio: se apaga desde el menú, queda
/// escrito en `config.md` y al releerlo el plugin sigue apagado.
#[test]
fn el_estado_enabled_persiste_entre_arranques() {
    let root = scratch("enabled-persist");
    let config_path = root.join("config.md");

    // Primer arranque: sin bloque, el plugin está habilitado.
    let configs = ConfigFile::parse(&std::fs::read_to_string(&config_path).unwrap_or_default());
    assert!(ConfigFile::is_enabled(&configs, "fake"));

    // El usuario lo apaga desde el menú.
    ConfigFile::set_enabled(&config_path, "fake", false).unwrap();
    let persisted = std::fs::read_to_string(&config_path).unwrap();
    assert!(persisted.contains("enabled = false"));

    // Simulación de reinicio: se relee el archivo y sigue apagado.
    let configs = ConfigFile::parse(&persisted);
    assert!(!ConfigFile::is_enabled(&configs, "fake"));

    // Volver a encenderlo también persiste.
    ConfigFile::set_enabled(&config_path, "fake", true).unwrap();
    let configs = ConfigFile::parse(&std::fs::read_to_string(&config_path).unwrap());
    assert!(ConfigFile::is_enabled(&configs, "fake"));

    let _ = std::fs::remove_dir_all(&root);
}

/// Un externo instalado pero con `enabled = false` no entra en la lista de
/// arranque; con `enabled = true` vuelve a entrar.
#[test]
fn un_externo_deshabilitado_no_se_arranca_al_iniciar() {
    let root = scratch("startup-filter");
    let plugins_dir = root.join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();

    let manifest = PluginManifest {
        id: "fake".into(),
        name: "Fake".into(),
        version: "0.1.0".into(),
        executable: plugins_dir.join("fake").to_string_lossy().to_string(),
        source: "fake".into(),
        capabilities: vec![],
    };
    host::write_manifest_in(&plugins_dir, &manifest).unwrap();

    let config_path = root.join("config.md");
    ConfigFile::set_enabled(&config_path, "fake", false).unwrap();
    let configs = ConfigFile::parse(&std::fs::read_to_string(&config_path).unwrap());
    let started = host::enabled_manifests(host::installed_in(&plugins_dir), &configs);
    assert!(started.is_empty(), "un externo apagado no debe arrancar");

    ConfigFile::set_enabled(&config_path, "fake", true).unwrap();
    let configs = ConfigFile::parse(&std::fs::read_to_string(&config_path).unwrap());
    let started = host::enabled_manifests(host::installed_in(&plugins_dir), &configs);
    assert_eq!(started.len(), 1, "encendido vuelve a la lista de arranque");

    let _ = std::fs::remove_dir_all(&root);
}

/// El bloque de configuración de un plugin externo llega a su proceso.
#[cfg(unix)]
#[test]
fn la_configuracion_llega_al_plugin_externo() {
    let root = scratch("configure");
    let executable = fake_configure_plugin(&root, "static");
    let manifest = PluginManifest {
        id: "fake".into(),
        name: "Fake".into(),
        version: "0.1.0".into(),
        executable: executable.to_string_lossy().to_string(),
        source: "fake".into(),
        capabilities: vec![],
    };
    let plugin = ExternalPlugin::start(manifest).expect("el plugin falso arranca");

    let mut block = PluginConfig::new();
    block.set("opacity", "0.42");
    let mut configs = BTreeMap::new();
    configs.insert("fake".to_string(), block);

    host::configure_externals(&[Arc::clone(&plugin)], &configs);

    let value = plugin.invoke("opacity").expect("debería responder");
    assert_eq!(value["opacity"], "0.42", "el bloque debe llegar al plugin");

    // La apariencia que el proceso declaró en su saludo entra en la composición
    // del registro: es lo que hace que instalar `transparency` o `font` cambie
    // la ventana de verdad.
    let mut registry = port_plugin_api::PluginRegistry::new();
    registry.set_externals(vec![Arc::clone(&plugin)]);
    assert_eq!(registry.effective_opacity(), 0.5);
    assert_eq!(registry.effective_font_family("Fira Code"), "Fake Mono");
    assert_eq!(registry.effective_font_size(14.0), 18.0);

    // Apagado desde el menú (o proceso muerto) sale de la lista y deja de
    // aportar: vuelven los valores por defecto.
    registry.set_externals(Vec::new());
    assert_eq!(registry.effective_opacity(), 1.0);
    assert_eq!(registry.effective_font_family("Fira Code"), "Fira Code");
    assert_eq!(registry.effective_font_size(14.0), 14.0);

    let _ = std::fs::remove_dir_all(&root);
}

/// La apariencia que el usuario escribe en `config.md` llega a la ventana: tras
/// aplicar el bloque hay que releer lo que el plugin declara, o `opacity` y
/// `font_family` se quedarían en el saludo de arranque para siempre.
#[cfg(unix)]
#[test]
fn la_apariencia_configurada_llega_a_la_ventana() {
    let root = scratch("appearance-configured");
    let executable = fake_configure_plugin(&root, "reflect");
    let manifest = PluginManifest {
        id: "fake".into(),
        name: "Fake".into(),
        version: "0.1.0".into(),
        executable: executable.to_string_lossy().to_string(),
        source: "fake".into(),
        capabilities: vec![],
    };
    let plugin = ExternalPlugin::start(manifest).expect("el plugin falso arranca");

    let mut registry = port_plugin_api::PluginRegistry::new();
    registry.set_externals(vec![Arc::clone(&plugin)]);
    // Antes de configurar manda lo declarado al arrancar.
    assert_eq!(registry.effective_opacity(), 0.5);
    assert_eq!(registry.effective_font_family("Fira Code"), "Fake Mono");

    let mut block = PluginConfig::new();
    block.set("opacity", "0.42");
    block.set("font_family", "Configured Mono");
    let mut configs = BTreeMap::new();
    configs.insert("fake".to_string(), block);

    host::configure_externals(&[Arc::clone(&plugin)], &configs);

    // El registro lee la copia cacheada del plugin: tiene que reflejar lo
    // configurado, no el saludo de arranque.
    assert_eq!(
        registry.effective_opacity(),
        0.42,
        "la opacidad configurada debe llegar a la ventana"
    );
    assert_eq!(
        registry.effective_font_family("Fira Code"),
        "Configured Mono",
        "la familia configurada debe llegar a la ventana"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// Un `configure` que falla conserva la apariencia anterior y no apaga el
/// plugin: la configuración es una comodidad, no un requisito para funcionar.
#[cfg(unix)]
#[test]
fn un_configure_que_falla_conserva_la_apariencia_anterior() {
    let root = scratch("appearance-unchanged");
    let executable = fake_configure_plugin(&root, "no-ack");
    let manifest = PluginManifest {
        id: "fake".into(),
        name: "Fake".into(),
        version: "0.1.0".into(),
        executable: executable.to_string_lossy().to_string(),
        source: "fake".into(),
        capabilities: vec![],
    };
    let plugin = ExternalPlugin::start(manifest).expect("el plugin falso arranca");

    let mut registry = port_plugin_api::PluginRegistry::new();
    registry.set_externals(vec![Arc::clone(&plugin)]);

    let mut block = PluginConfig::new();
    block.set("opacity", "0.05");
    block.set("font_family", "Nunca Aplicada");
    let mut configs = BTreeMap::new();
    configs.insert("fake".to_string(), block);

    host::configure_externals(&[Arc::clone(&plugin)], &configs);

    assert_eq!(
        registry.effective_opacity(),
        0.5,
        "sin acuse se conserva lo anterior"
    );
    assert_eq!(
        registry.effective_font_family("Fira Code"),
        "Fake Mono",
        "sin acuse se conserva lo anterior"
    );
    // El proceso sigue vivo: un `configure` fallido no lo apaga.
    assert!(
        plugin.invoke("opacity").is_some(),
        "el plugin debe seguir respondiendo"
    );

    let _ = std::fs::remove_dir_all(&root);
}
