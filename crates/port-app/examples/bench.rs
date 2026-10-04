//! Mide lo que cuesta cargar un plugin y hablar con el.
//!
//! Se ejecuta contra un plugin ya instalado:
//!
//! ```sh
//! port plugin add <url-de-un-plugin>
//! cargo run --release --example bench -- ~/.local/share/port/plugins/<id>
//! ```
//!
//! Las cifras que importan son las de arranque e invocación. Un arranque en
//! torno a un milisegundo es lo que permite que recargar un plugin tras
//! recompilar sea percibido como instantáneo, y una invocación en torno a
//! diez microsegundos es lo que permite preguntar a un plugin en la tecla
//! pulsada sin que se note. Si estas cifras se degradan, el diseño necesita
//! revisar el transporte, no las reglas de negocio.

use std::time::Instant;

use port_plugin_api::host::ExternalPlugin;
use port_plugin_api::protocol::{Capability, PluginManifest};

fn main() {
    let Some(executable) = std::env::args().nth(1) else {
        eprintln!("uso: bench <ruta-al-ejecutable-del-plugin>");
        std::process::exit(2);
    };

    let manifest = PluginManifest {
        id: "bench".into(),
        name: "Bench".into(),
        version: "0.1.0".into(),
        executable,
        source: "bench".into(),
        capabilities: vec![Capability::Appearance],
    };

    let runs = 20;
    let started = Instant::now();
    for _ in 0..runs {
        let plugin = ExternalPlugin::start(manifest.clone()).expect("el plugin deberia arrancar");
        std::hint::black_box(&plugin);
    }
    let per_start = started.elapsed().as_secs_f64() * 1000.0 / runs as f64;
    println!("arranque: {per_start:.2} ms por plugin ({runs} arranques)");

    let plugin = ExternalPlugin::start(manifest).expect("el plugin deberia arrancar");
    let calls = 200;
    let started = Instant::now();
    for _ in 0..calls {
        std::hint::black_box(plugin.invoke("saludar"));
    }
    let per_call = started.elapsed().as_secs_f64() * 1000.0 / calls as f64;
    println!("invocación: {per_call:.3} ms por ida y vuelta");
}
