//! SDK ligero para escribir plugins externos de PORT.
//!
//! Deliberadamente mínimo: este crate depende solo de `serde`. Un plugin es un
//! proceso aparte, así que arrastrar la interfaz o el núcleo de terminal lo haría
//! tan pesado de compilar como el propio PORT, que es justo lo que el diseño
//! evita. El núcleo usa el mismo protocolo desde [`protocol`].

pub mod protocol;
pub mod runtime;

pub use protocol::{Capability, HostRequest, PluginManifest, PluginReply};
pub use runtime::{serve, Plugin};
