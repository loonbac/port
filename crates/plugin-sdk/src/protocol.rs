//! Protocolo entre el núcleo de PORT y los plugins externos.
//!
//! # Por qué un proceso aparte
//!
//! Un plugin cargado como biblioteca dinámica comparte el heap, el runtime de
//! Rust y cada tipo de GPUI con el núcleo. Cualquier cambio de versión del
//! compilador o de GPUI convierte esa frontera en una fuente de comportamiento
//! indefinido que solo aparece en la máquina de otro usuario. Un proceso
//! separado elimina esa clase de problema entero: si el plugin no habla el
//! protocolo, no arranca.
//!
//! Un plugin es por tanto un ejecutable propio que lee peticiones por stdin y
//! escribe respuestas por stdout, una por línea (JSON Lines).
//!
//! # Por qué no se pregunta nada en cada tecla
//!
//! Un viaje de ida y vuelta por una tubería en cada pulsación metería en
//! la latencia de escritura, que es justo lo que el usuario nota al teclear. Por
//! eso el plugin declara en el saludo qué combinaciones quiere, y el núcleo las
//! resuelve **en local**. Solo cuando una combinación coincide se pregunta al
//! plugin dueño, y con un plazo máximo: si no contesta, la tecla pasa de largo.
//!
//! # Qué se cruza el límite
//!
//! Los hooks que devuelven valores (apariencia, claves, ciclo de vida) se
//! traducen a JSON sin pérdida. `LayoutHook` queda fuera de este protocolo: un
//! `AnyElement` de GPUI no se serializa. Los plugins externos declaran la
//! capacidad `layout` solo cuando exista una vista declarativa.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Versión del protocolo. Cambia cuando el contrato se rompe; el núcleo se
/// niega a hablar con un plugin que anuncie otra mayor.
pub const PROTOCOL_VERSION: u32 = 1;

/// Cuánto espera el núcleo una respuesta antes de darla por perdida.
pub const DEFAULT_TIMEOUT_MS: u64 = 250;

/// Capacidad que un plugin puede anunciar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Ajustar opacidad, color de fondo y tipografía.
    Appearance,
    /// Responder a combinaciones de teclas.
    Input,
    /// Crear o seleccionar sesiones y espacios.
    Session,
    /// Vetar el cierre de la ventana.
    Lifecycle,
}

/// Combinación de teclas que un plugin reclama en el saludo.
///
/// El núcleo las guarda y las resuelve sin preguntar, así que declarar una
/// combinación no cuesta nada en tiempo de tecleo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    /// Nombre de la tecla tal como la reporta el toolkit: `t`, `F5`, `Up`.
    pub key: String,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub shift: bool,
    /// Acción que el plugin ejecuta al pulsarse.
    pub action: String,
}

impl Binding {
    /// `true` si la tecla entrante coincide con esta combinación.
    ///
    /// `shift` se compara contra el estado real del teclado, no contra el
    /// carácter: en una terminal `Shift`+`1` llega como `!`, y si se comparara
    /// el carácter fallaría en todas las teclas de la fila superior.
    pub fn matches(&self, key: &str, ctrl: bool, alt: bool, shift: bool) -> bool {
        self.ctrl == ctrl
            && self.alt == alt
            && self.shift == shift
            && self.key.eq_ignore_ascii_case(key)
    }
}

/// Estado visual que un plugin puede pedir.
///
/// `#[serde(default)]` para que un campo ausente signifique "este plugin no se
/// pronuncia", no un error de deserializacion.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// Opacidad del fondo, de 0.0 a 1.0.
    pub opacity: Option<f32>,
    /// Familia tipográfica.
    pub font_family: Option<String>,
    /// Tamaño de fuente en puntos.
    pub font_size: Option<f32>,
    /// Color de fondo en `#RRGGBB`.
    pub background: Option<String>,
}

/// Petición del núcleo al plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum HostRequest {
    /// Saludo inicial. El núcleo responde con [`PluginReply::Ready`].
    Hello { api: u32 },
    /// Aplica el bloque de configuración del plugin.
    Configure { values: BTreeMap<String, String> },
    /// Ejecuta una acción. `seq` empareja la respuesta con su petición.
    Invoke { seq: u64, action: String },
    /// Avisa de que la ventana se está cerrando; el plugin puede vetarlo.
    Shutdown,
}

/// Respuesta del plugin al núcleo.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum PluginReply {
    /// Saludo. Declara identidad, capacidades, combinaciones y apariencia.
    Ready {
        name: String,
        version: String,
        #[serde(default)]
        capabilities: Vec<Capability>,
        #[serde(default)]
        bindings: Vec<Binding>,
        #[serde(default)]
        appearance: Appearance,
    },
    /// Acuse de una petición.
    Ack,
    /// Respuesta a un `Invoke`.
    Result {
        seq: u64,
        ok: bool,
        value: serde_json::Value,
    },
    /// Evento espontáneo: el plugin pide algo al núcleo.
    Event {
        name: String,
        value: serde_json::Value,
    },
}

/// Estructura en disco de un plugin instalado, junto a su ejecutable.
///
/// Es un archivo de texto por plugin, no una base de datos: se puede editar a
/// mano, versionar y borrar sin herramienta de por medio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Identificador con el que el plugin se registra.
    pub id: String,
    /// Nombre legible.
    pub name: String,
    /// Versión semántica.
    pub version: String,
    /// Ruta absoluta del ejecutable.
    pub executable: String,
    /// Repositorio del que vino.
    pub source: String,
    /// Capabilities declaradas en el último saludo.
    #[serde(default)]
    pub capabilities: Vec<Capability>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_matches_ignores_key_case() {
        let b = Binding {
            key: "T".to_string(),
            ctrl: true,
            alt: false,
            shift: true,
            action: "new_tab".to_string(),
        };
        assert!(b.matches("t", true, false, true));
        assert!(!b.matches("t", true, false, false));
        assert!(!b.matches("k", true, false, true));
    }

    #[test]
    fn hello_round_trips_as_json_lines() {
        let req = HostRequest::Hello {
            api: PROTOCOL_VERSION,
        };
        let line = serde_json::to_string(&req).unwrap();
        assert_eq!(line, r#"{"t":"hello","api":1}"#);
        let back: HostRequest = serde_json::from_str(&line).unwrap();
        match back {
            HostRequest::Hello { api } => assert_eq!(api, PROTOCOL_VERSION),
            other => panic!("mensaje equivocado: {other:?}"),
        }
    }

    #[test]
    fn ready_declares_capabilities_and_bindings() {
        let reply = PluginReply::Ready {
            name: "statusline".to_string(),
            version: "0.1.0".to_string(),
            capabilities: vec![Capability::Appearance, Capability::Input],
            bindings: vec![Binding {
                key: "t".to_string(),
                ctrl: true,
                alt: false,
                shift: true,
                action: "new_tab".to_string(),
            }],
            appearance: Appearance {
                opacity: Some(0.9),
                ..Default::default()
            },
        };
        let line = serde_json::to_string(&reply).unwrap();
        let back: PluginReply = serde_json::from_str(&line).unwrap();
        match back {
            PluginReply::Ready {
                name,
                capabilities,
                bindings,
                appearance,
                ..
            } => {
                assert_eq!(name, "statusline");
                assert_eq!(capabilities.len(), 2);
                assert_eq!(bindings.len(), 1);
                assert_eq!(appearance.opacity, Some(0.9));
            }
            other => panic!("mensaje equivocado: {other:?}"),
        }
    }

    #[test]
    fn appearance_defaults_to_no_contribution() {
        let a = Appearance::default();
        assert!(a.opacity.is_none());
        assert!(a.font_family.is_none());
    }

    #[test]
    fn manifest_survives_a_round_trip() {
        let m = PluginManifest {
            id: "statusline".into(),
            name: "Statusline".into(),
            version: "0.1.0".into(),
            executable: "/home/u/.local/lib/port/plugins/statusline".into(),
            source: "https://github.com/u/statusline".into(),
            capabilities: vec![Capability::Appearance],
        };
        let back: PluginManifest =
            serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back, m);
    }
}
