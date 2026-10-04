//! Lado del plugin: el bucle que habla con el núcleo.
//!
//! Un plugin externo se escribe en veinte líneas. Se declara qué hace y qué
//! teclas quiere, y [`serve`] se encarga del bucle:
//!
//! ```no_run
//! use port_plugin_sdk::protocol::{Appearance, Binding, Capability};
//! use port_plugin_sdk::runtime::serve;
//!
//! struct MiPlugin;
//!
//! impl port_plugin_sdk::runtime::Plugin for MiPlugin {
//!     fn name(&self) -> &'static str {
//!         "mi-plugin"
//!     }
//!     fn version(&self) -> &'static str {
//!         "0.1.0"
//!     }
//!     fn capabilities(&self) -> Vec<Capability> {
//!         vec![Capability::Appearance]
//!     }
//!     fn appearance(&self) -> Appearance {
//!         Appearance { opacity: Some(0.92), ..Default::default() }
//!     }
//!     fn invoke(&self, action: &str, params: &serde_json::Value) -> Option<serde_json::Value> {
//!         match action {
//!             "saludar" => Some(serde_json::json!({"texto": "hola"})),
//!             _ => None,
//!         }
//!     }
//! }
//!
//! serve(MiPlugin);
//! ```
//!
//! El bloqueo de entrada y salida es intencionado: es el hilo del proceso del
//! plugin, no un hilo de la terminal. El núcleo siempre espera con plazo, así
//! que un bucle lento degrada a "el plugin no responde", nunca a "PORT se
//! congela".

use std::io::{BufRead, Write};

use serde::Serialize;
use serde_json::Value;

use crate::protocol::{Appearance, Binding, Capability, HostRequest, PluginReply};

/// Lo que un plugin declara y lo que sabe hacer.
///
/// Los métodos por defecto cubren el caso mínimo: un plugin sin apariencia ni
/// teclas solo necesita `name` y `version`.
pub trait Plugin {
    /// Nombre legible.
    fn name(&self) -> &'static str;

    /// Versión semántica.
    fn version(&self) -> &'static str;

    /// Qué puede hacer este plugin.
    fn capabilities(&self) -> Vec<Capability> {
        Vec::new()
    }

    /// Combinaciones que este plugin reclama.
    fn bindings(&self) -> Vec<Binding> {
        Vec::new()
    }

    /// Estado visual inicial.
    fn appearance(&self) -> Appearance {
        Appearance::default()
    }

    /// Ejecuta una acción.
    ///
    /// `None` significa "esta acción no es mía o no puedo ahora mismo", no
    /// un error.
    fn invoke(&self, action: &str, params: &Value) -> Option<Value> {
        let _ = (action, params);
        None
    }

    /// Recibe el bloque de configuración.
    fn configure(&mut self, _values: &std::collections::BTreeMap<String, String>) {}

    /// Saludo que se envía en `Ready`.
    fn ready(&self) -> PluginReply {
        PluginReply::Ready {
            name: self.name().to_string(),
            version: self.version().to_string(),
            capabilities: self.capabilities(),
            bindings: self.bindings(),
            appearance: self.appearance(),
        }
    }
}

/// Error de salida del bucle, para cuando el núcleo se desconecta.
pub struct Disconnected;

/// Ejecuta el bucle del plugin hasta que el núcleo cierra la conexión.
///
/// `plugin` se declara `mut` porque [`Plugin::configure`] puede cambiar su
/// estado; el resto del bucle solo necesita `&self`.
pub fn serve<P: Plugin>(mut plugin: P) {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    if run(
        &mut plugin,
        stdin.lock(),
        &mut std::io::BufWriter::new(stdout.lock()),
    )
    .is_err()
    {
        // El núcleo se fue. Terminar es lo correcto: un plugin huérfano
        // consumiendo CPU sin nadie que lo atienda no sirve de nada.
        std::process::exit(0);
    }
}

fn run<P: Plugin + ?Sized, R: BufRead, W: Write>(
    plugin: &mut P,
    input: R,
    output: &mut W,
) -> Result<(), Disconnected> {
    let mut lines = input.lines();

    // El saludo lo inicia el nucleo con un `Hello`; el plugin no se anuncia
    // solo. Si se anunciara aqui, ese `Ready` sobrante quedaria en la cola y
    // el nucleo lo leeria como si fuera la respuesta a la siguiente peticion.
    while let Some(Ok(line)) = lines.next() {
        if line.trim().is_empty() {
            continue;
        }
        let request: HostRequest = match serde_json::from_str(&line) {
            Ok(request) => request,
            // Una peticion ilegible no es motivo para morir: el nucleo puede
            // haber enviado algo de una version posterior.
            Err(_) => continue,
        };

        match request {
            HostRequest::Hello { .. } => {
                write_line(output, &plugin.ready())?;
            }
            HostRequest::Configure { values } => {
                plugin.configure(&values);
                write_line(output, &PluginReply::Ack)?;
            }
            HostRequest::Invoke { seq, action } => {
                let params = serde_json::json!({});
                let outcome = match plugin.invoke(&action, &params) {
                    Some(value) => PluginReply::Result {
                        seq,
                        ok: true,
                        value,
                    },
                    None => PluginReply::Result {
                        seq,
                        ok: false,
                        value: Value::Null,
                    },
                };
                write_line(output, &outcome)?;
            }
            HostRequest::Shutdown => return Ok(()),
        }
    }
    Err(Disconnected)
}

fn write_line<W: Write, T: Serialize>(out: &mut W, message: &T) -> Result<(), Disconnected> {
    let text = serde_json::to_string(message).map_err(|_| Disconnected)?;
    writeln!(out, "{text}").map_err(|_| Disconnected)?;
    out.flush().map_err(|_| Disconnected)
}

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::Cursor;

    /// Plugin minimo: una tecla y una accion.
    struct Demo;

    impl Plugin for Demo {
        fn name(&self) -> &'static str {
            "demo"
        }
        fn version(&self) -> &'static str {
            "0.1.0"
        }
        fn capabilities(&self) -> Vec<Capability> {
            vec![Capability::Input]
        }
        fn bindings(&self) -> Vec<Binding> {
            vec![Binding {
                key: "t".into(),
                ctrl: true,
                alt: false,
                shift: true,
                action: "nuevo".into(),
            }]
        }
        fn invoke(&self, action: &str, _params: &Value) -> Option<Value> {
            (action == "nuevo").then(|| serde_json::json!({"ok": true}))
        }
    }

    /// Plugin que guarda lo que recibe, para comprobar que `configure` llega.
    struct Configurable {
        opacity: Option<f32>,
    }

    impl Plugin for Configurable {
        fn name(&self) -> &'static str {
            "configurable"
        }
        fn version(&self) -> &'static str {
            "0.1.0"
        }
        fn configure(&mut self, values: &BTreeMap<String, String>) {
            self.opacity = values.get("opacity").and_then(|v| v.parse().ok());
        }
    }

    /// Corre el bucle con una entrada fija y devuelve lo que escribio.
    fn drive(plugin: &mut dyn Plugin, input: &str) -> Vec<PluginReply> {
        let mut out = Vec::new();
        let _ = run(plugin, Cursor::new(input), &mut out);
        String::from_utf8(out)
            .expect("la salida debe ser utf-8")
            .lines()
            .map(|l| serde_json::from_str(l).expect("cada linea debe ser json"))
            .collect()
    }

    #[test]
    fn ready_describes_the_plugin() {
        match Demo.ready() {
            PluginReply::Ready {
                name,
                version,
                capabilities,
                bindings,
                ..
            } => {
                assert_eq!(name, "demo");
                assert_eq!(version, "0.1.0");
                assert_eq!(capabilities, vec![Capability::Input]);
                assert_eq!(bindings.len(), 1);
                assert!(bindings[0].matches("t", true, false, true));
            }
            other => panic!("mensaje equivocado: {other:?}"),
        }
    }

    #[test]
    fn invoke_reports_unknown_actions_as_unsupported() {
        assert!(Demo.invoke("nuevo", &Value::Null).is_some());
        assert!(Demo.invoke("otro", &Value::Null).is_none());
    }

    #[test]
    fn the_plugin_answers_only_when_greeted() {
        // Sin `Hello` no dice nada: el nucleo dirige la conversacion.
        assert!(drive(&mut Demo, "").is_empty());

        let replies = drive(&mut Demo, "{\"t\":\"hello\",\"api\":1}\n");
        assert_eq!(replies.len(), 1);
        assert!(matches!(replies[0], PluginReply::Ready { .. }));
    }

    #[test]
    fn a_greeting_is_not_left_sitting_in_the_queue() {
        // El fallo que justifico el greeting dirigido por el nucleo: si el
        // plugin se anunciara al conectar, ese Ready se leeria como respuesta
        // a la peticion siguiente y la primera accion se perderia.
        let replies = drive(
            &mut Demo,
            "{\"t\":\"hello\",\"api\":1}\n{\"t\":\"invoke\",\"seq\":9,\"action\":\"nuevo\"}\n",
        );
        assert_eq!(replies.len(), 2);
        assert!(matches!(replies[0], PluginReply::Ready { .. }));
        match &replies[1] {
            PluginReply::Result { seq, .. } => assert_eq!(*seq, 9),
            other => panic!("la primera accion perdio su turno: {other:?}"),
        }
    }

    #[test]
    fn an_invoke_gets_an_answer_carrying_the_same_sequence() {
        let replies = drive(
            &mut Demo,
            "{\"t\":\"hello\",\"api\":1}\n{\"t\":\"invoke\",\"seq\":7,\"action\":\"nuevo\"}\n",
        );
        assert_eq!(replies.len(), 2);
        match &replies[1] {
            PluginReply::Result { seq, ok, value } => {
                assert_eq!(*seq, 7, "la respuesta debe traer el seq de la peticion");
                assert!(ok);
                assert_eq!(value["ok"], true);
            }
            other => panic!("mensaje equivocado: {other:?}"),
        }
    }

    #[test]
    fn an_unknown_action_answers_unsupported_instead_of_ignoring() {
        let replies = drive(
            &mut Demo,
            "{\"t\":\"hello\",\"api\":1}\n{\"t\":\"invoke\",\"seq\":1,\"action\":\"nada\"}\n",
        );
        match &replies[1] {
            PluginReply::Result { ok, .. } => assert!(!ok),
            other => panic!("mensaje equivocado: {other:?}"),
        }
    }

    #[test]
    fn a_garbled_line_does_not_kill_the_loop() {
        let replies = drive(
            &mut Demo,
            "esto no es json\n{\"t\":\"hello\",\"api\":1}\n\
             {\"t\":\"invoke\",\"seq\":3,\"action\":\"nuevo\"}\n",
        );
        assert_eq!(
            replies.len(),
            2,
            "saludo y respuesta, sin morir por la basura"
        );
        match &replies[1] {
            PluginReply::Result { seq, .. } => assert_eq!(*seq, 3),
            other => panic!("mensaje equivocado: {other:?}"),
        }
    }

    #[test]
    fn blank_lines_are_skipped() {
        let replies = drive(&mut Demo, "\n\n{\"t\":\"shutdown\"}\n");
        assert!(replies.is_empty(), "un cierre sin saludo no genera trafico");
    }

    #[test]
    fn configure_is_acknowledged_and_reaches_the_plugin() {
        let mut plugin = Configurable { opacity: None };
        let replies = drive(
            &mut plugin,
            "{\"t\":\"hello\",\"api\":1}\n\
             {\"t\":\"configure\",\"values\":{\"opacity\":\"0.5\"}}\n",
        );
        assert_eq!(plugin.opacity, Some(0.5), "el valor debe llegar al plugin");
        assert!(matches!(replies[1], PluginReply::Ack));
    }

    #[test]
    fn shutdown_ends_the_loop_cleanly() {
        let mut out = Vec::new();
        let result = run(&mut Demo, Cursor::new("{\"t\":\"shutdown\"}\n"), &mut out);
        assert!(result.is_ok(), "shutdown es una salida limpia");
    }

    #[test]
    fn a_closed_pipe_ends_the_loop_instead_of_looping_forever() {
        let mut out = Vec::new();
        // Sin entrada y sin shutdown: el bucle debe notar el fin de la
        // transmision y salir, no quedarse esperando para siempre.
        let result = run(&mut Demo, Cursor::new(""), &mut out);
        assert!(result.is_err(), "un pipe cerrado no es una salida limpia");
    }
}
