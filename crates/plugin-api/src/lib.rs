//! Contratos, capacidades y registro de plugins de PORT.
//!
//! Este crate define la API pública con la que interactúan los plugins.
//! Los plugins son componentes in-process desacoplados que extienden
//! la apariencia, la entrada, el layout, el menú o la configuración de la terminal sin tocar el núcleo.
//!
//! Capacidades, cada una con su propia razón para cambiar:
//!
//! - [`services`]: directorio de servicios que un plugin publica y otro invoca
//!   por identificador estable, sin conocer su tipo concreto.
//! - [`hooks`]: los hooks que un plugin puede implementar (apariencia, entrada,
//!   ratón, layout, espacios y ciclo de vida) y el trait [`Plugin`] que los
//!   devuelve. Es el contrato, no la ejecución.
//! - [`registry`]: registro central que compone los plugins in-process con los
//!   externos vivos, invoca sus hooks bajo el watchdog de tiempo y persiste su
//!   configuración. La política de salud y watchdog vive en un submódulo
//!   privado del propio registro.

pub mod config;
pub mod hooks;
pub mod host;
pub mod install;
// La política de salud y watchdog del registro es interna: se reexporta desde
// `registry`, que es su único consumidor.
mod health;
pub mod registry;
pub mod services;
pub mod watch;
// El protocolo vive en el SDK para que un plugin no arrastre GPUI.
pub use port_plugin_sdk::protocol;

pub use config::{ConfigFile, PluginConfig};
pub use hooks::{
    AppearanceHook, CloseDecision, InputHook, KeyAction, LayoutHook, LifecycleHook, MouseHook,
    Plugin, SpaceHook,
};
pub use registry::{PluginHealth, PluginInfo, PluginRegistry, DEFAULT_HOOK_BUDGET};
pub use services::{Arg, Ret, Service, ServiceError, Services};
