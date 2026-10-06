//! Salud de un plugin y política de watchdog sobre sus hooks.
//!
//! Es el único lugar donde se ejecuta un hook aislando su pánico y midiendo
//! cuánto tarda: al pasarse del presupuesto el plugin degrada de `Ok` a `Slow`
//! y de `Slow` a `Disabled`, con lo que deja de aportar. El registro que aplica
//! esta política vive en [`crate::registry`].

use std::cell::Cell;
use std::time::{Duration, Instant};

use crate::hooks::Plugin;

/// Presupuesto de tiempo que un hook puede gastar antes de contar como lento.
pub const DEFAULT_HOOK_BUDGET: Duration = Duration::from_millis(50);

/// Estado de salud de un plugin frente al presupuesto de hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PluginHealth {
    /// Todos sus hooks responden dentro del presupuesto.
    #[default]
    Ok,
    /// Se pasó del presupuesto una vez. Sigue invocándose, con aviso.
    Slow,
    /// Se pasó del presupuesto dos veces. Ya no se invoca nunca más.
    Disabled,
}

pub(crate) struct RegisteredPlugin {
    pub(crate) plugin: Box<dyn Plugin>,
    pub(crate) enabled: bool,
    pub(crate) health: Cell<PluginHealth>,
}

impl RegisteredPlugin {
    /// Ejecuta una llamada a un hook de plugin aislando pánicos y midiendo el tiempo de ejecución.
    ///
    /// Aplica la política de watchdog con dos strikes:
    /// - Primer strike: `Ok` pasa a `Slow`, se emite aviso a stderr y el resultado de la llamada se usa.
    /// - Segundo strike: `Slow` pasa a `Disabled`, se emite aviso a stderr y el resultado de la llamada se usa.
    /// - En `Disabled`: el plugin no se vuelve a invocar nunca más y se devuelve el valor de respaldo (`fallback`).
    pub(crate) fn invoke_hook<T>(
        &self,
        budget: Duration,
        fallback: T,
        call: impl FnOnce() -> T,
    ) -> T {
        if !self.enabled || self.health.get() == PluginHealth::Disabled {
            return fallback;
        }

        let start = Instant::now();
        let res = guarded(fallback, call);
        let elapsed = start.elapsed();

        if elapsed > budget {
            match self.health.get() {
                PluginHealth::Ok => {
                    self.health.set(PluginHealth::Slow);
                    eprintln!(
                        "aviso: el plugin '{}' tardó {:?} (presupuesto {:?}) y pasa a estado Slow",
                        self.plugin.id(),
                        elapsed,
                        budget
                    );
                }
                PluginHealth::Slow => {
                    self.health.set(PluginHealth::Disabled);
                    eprintln!(
                        "aviso: el plugin '{}' superó el presupuesto por segunda vez ({:?} > {:?}) y ha sido deshabilitado",
                        self.plugin.id(),
                        elapsed,
                        budget
                    );
                }
                PluginHealth::Disabled => {}
            }
        }

        res
    }
}

/// Ejecuta un hook de plugin aislando su pánico.
///
/// Un plugin es código de terceros dentro de la terminal. Si su hook entra en
/// pánico, el desenrollado sube por el manejador de teclado o por el render de
/// GPUI y se lleva la terminal por delante: el usuario pierde su sesión entera
/// por un fallo en una extensión.
///
/// Aquí el pánico se corta en la frontera y el plugin pasa a no aportar nada,
/// que es justo lo que significa estar deshabilitado. El resto de la terminal
/// sigue viva.
///
/// El hook de pánico global no se toca a propósito: silenciar el aviso
/// escondería el fallo de quien está desarrollando el plugin, y el aviso por
/// stderr es la única pista que le queda.
fn guarded<T>(fallback: T, call: impl FnOnce() -> T) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)) {
        Ok(value) => value,
        Err(_) => fallback,
    }
}
