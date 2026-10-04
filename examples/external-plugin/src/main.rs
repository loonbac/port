//! Plugin de ejemplo de PORT, pensado para instalarse y compilar tal cual.
//!
//! No hace nada espectacular a propósito: sirve como plantilla mínima de un
//! plugin que corre como proceso independiente y que solo mueve valores.
//!
//! Esta misma lógica la ejercita la prueba de extremo a extremo
//! `crates/port-app/tests/external_plugin.rs`, que compila este directorio y lo
//! arranca como proceso hijo. Si el ejemplo se rompe, la suite lo detecta.

use std::collections::BTreeMap;

use port_plugin_sdk::protocol::{Appearance, Binding, Capability};
use port_plugin_sdk::runtime::{serve, Plugin};

struct Example {
    opacity: f32,
    triggers: u32,
}

impl Plugin for Example {
    fn name(&self) -> &'static str {
        "example"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Appearance, Capability::Input]
    }

    fn appearance(&self) -> Appearance {
        Appearance {
            opacity: Some(self.opacity),
            ..Default::default()
        }
    }

    fn bindings(&self) -> Vec<Binding> {
        // Declarar la combinación aquí permite que el nucleo la resuelva en
        // local: teclear no toca ninguna tuberia. Solo cuando coincide se
        // pregunta a este proceso.
        vec![Binding {
            key: "h".into(),
            ctrl: true,
            alt: true,
            shift: false,
            action: "toggle".into(),
        }]
    }

    fn invoke(
        &self,
        action: &str,
        _params: &serde_json::Value,
    ) -> Option<serde_json::Value> {
        // `None` significa "esta accion no es mia", no "algo fallo".
        (action == "toggle").then(|| serde_json::json!({ "triggers": self.triggers }))
    }

    fn configure(&mut self, values: &BTreeMap<String, String>) {
        if let Some(opacity) = values.get("opacity").and_then(|v| v.parse::<f32>().ok()) {
            self.opacity = opacity;
        }
    }
}

fn main() {
    serve(Example {
        opacity: 0.94,
        triggers: 0,
    });
}