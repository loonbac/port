//! Salud de un plugin y política de watchdog sobre sus hooks.
//!
//! Es el único lugar donde se ejecuta un hook aislando su pánico y midiendo
//! cuánto tarda. Aplica una máquina de estados proporcional y recuperable:
//!
//! - `Ok`: Estado inicial. Los hooks se ejecutan con normalidad. Si una llamada
//!   supera el presupuesto ([`DEFAULT_HOOK_BUDGET`]), pasa a `Slow`.
//! - `Slow`: Avisado por un primer exceso de tiempo. Sigue ejecutándose:
//!   - Si la siguiente llamada finaliza dentro del presupuesto, el strike decae
//!     y regresa a `Ok` (un retraso aislado jamás deshabilita el plugin).
//!   - Si la siguiente llamada supera nuevamente el presupuesto (dos strikes
//!     consecutivos), pasa a `Disabled` y se anota el momento de fallo.
//! - `Disabled`: El plugin deja de ejecutarse para proteger la interfaz. Tras
//!   un periodo de enfriamiento ([`DEFAULT_PROBE_COOLDOWN`], ~10 s), se concede
//!   una única llamada de prueba (probe):
//!   - Si la prueba responde dentro del presupuesto, el plugin vuelve a `Ok`.
//!   - Si la prueba vuelve a superar el presupuesto, permanece en `Disabled`
//!     durante otro periodo de enfriamiento.
//! - Si el usuario desactiva manualmente el plugin (`enabled == false`),
//!   este nunca se invoca, independientemente de su estado de salud.
//!
//! El registro que aplica esta política vive en [`crate::registry`].

use std::cell::Cell;
use std::time::{Duration, Instant};

use crate::hooks::Plugin;

/// Presupuesto de tiempo que un hook puede gastar antes de contar como lento.
///
/// Un hook que construye elementos de UI o consulta estado legítimamente toma
/// unos pocos milisegundos; bajo un disco saturado por compilaciones o
/// reconstrucciones del sistema (`nixos-rebuild`), decenas de milisegundos.
/// 250 ms es el umbral a partir del cual el plugin realmente se está
/// comportando de forma anómala o bloqueando la experiencia interactiva.
pub const DEFAULT_HOOK_BUDGET: Duration = Duration::from_millis(250);

/// Periodo de enfriamiento por defecto tras el cual un plugin deshabilitado
/// recibe una llamada de prueba.
pub const DEFAULT_PROBE_COOLDOWN: Duration = Duration::from_secs(10);

/// Estado de salud de un plugin frente al presupuesto de hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PluginHealth {
    /// Todos sus hooks responden dentro del presupuesto.
    #[default]
    Ok,
    /// Se pasó del presupuesto una vez. Sigue invocándose, con aviso.
    Slow,
    /// Se pasó del presupuesto dos veces consecutivas. Deshabilitado temporalmente
    /// hasta que pase el periodo de enfriamiento y una prueba tenga éxito.
    Disabled,
}

pub(crate) struct RegisteredPlugin {
    pub(crate) plugin: Box<dyn Plugin>,
    pub(crate) enabled: bool,
    pub(crate) health: Cell<PluginHealth>,
    pub(crate) disabled_at: Cell<Option<Instant>>,
    pub(crate) cooldown: Cell<Duration>,
}

impl RegisteredPlugin {
    pub(crate) fn new(plugin: Box<dyn Plugin>, cooldown: Duration) -> Self {
        Self {
            plugin,
            enabled: true,
            health: Cell::new(PluginHealth::Ok),
            disabled_at: Cell::new(None),
            cooldown: Cell::new(cooldown),
        }
    }

    /// Comprueba si el periodo de enfriamiento ha transcurrido para una llamada de prueba.
    pub(crate) fn is_probe_ready(&self) -> bool {
        match self.disabled_at.get() {
            Some(at) => Instant::now().duration_since(at) >= self.cooldown.get(),
            None => false,
        }
    }

    /// Comprueba si el plugin puede ser invocado (habilitado y en Ok/Slow, o
    /// en Disabled habiendo cumplido el enfriamiento).
    pub(crate) fn can_invoke(&self) -> bool {
        if !self.enabled {
            return false;
        }
        if self.health.get() == PluginHealth::Disabled {
            return self.is_probe_ready();
        }
        true
    }

    /// Ejecuta una llamada a un hook de plugin aislando pánicos y midiendo el tiempo de ejecución.
    ///
    /// Aplica la política de watchdog con decaimiento de strikes y recuperación tras enfriamiento:
    /// - `enabled == false`: el plugin no se invoca y devuelve `fallback`.
    /// - Primer strike: de `Ok` pasa a `Slow`, emite aviso a stderr y usa el resultado.
    /// - Decaimiento: en `Slow`, si responde dentro de presupuesto regresa a `Ok`.
    /// - Segundo strike consecutivo: de `Slow` pasa a `Disabled`, anota el instante y usa el resultado.
    /// - En `Disabled`:
    ///   - Si no ha transcurrido el periodo de enfriamiento (`cooldown`), no se ejecuta y devuelve `fallback`.
    ///   - Si ya transcurrió el enfriamiento, se permite una llamada de prueba:
    ///     - Si la prueba responde a tiempo (<= `budget`): regresa a `Ok` y usa el resultado.
    ///     - Si la prueba supera el presupuesto (> `budget`): permanece en `Disabled` renovando el
    ///       periodo de enfriamiento y usa el resultado.
    pub(crate) fn invoke_hook<T>(
        &self,
        budget: Duration,
        fallback: T,
        call: impl FnOnce() -> T,
    ) -> T {
        if !self.enabled {
            return fallback;
        }

        if self.health.get() == PluginHealth::Disabled {
            let cooldown = self.cooldown.get();
            let should_probe = match self.disabled_at.get() {
                Some(at) => Instant::now().duration_since(at) >= cooldown,
                None => false,
            };

            if !should_probe {
                return fallback;
            }

            // Marcamos el instante inmediatamente para evitar que llamadas concurrentes
            // o reentrantes lancen múltiples pruebas simultáneas.
            self.disabled_at.set(Some(Instant::now()));

            let start = Instant::now();
            let res = guarded(fallback, call);
            let elapsed = start.elapsed();

            if elapsed <= budget {
                self.health.set(PluginHealth::Ok);
                self.disabled_at.set(None);
                eprintln!(
                    "info: el plugin '{}' respondió en {:?} (presupuesto {:?}) tras el periodo de enfriamiento y vuelve a estado Ok",
                    self.plugin.id(),
                    elapsed,
                    budget
                );
            } else {
                self.disabled_at.set(Some(Instant::now()));
                eprintln!(
                    "aviso: la prueba del plugin '{}' tras el periodo de enfriamiento superó el presupuesto ({:?} > {:?}) y continúa deshabilitado",
                    self.plugin.id(),
                    elapsed,
                    budget
                );
            }

            return res;
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
                    self.disabled_at.set(Some(Instant::now()));
                    eprintln!(
                        "aviso: el plugin '{}' superó el presupuesto por segunda vez ({:?} > {:?}) y ha sido deshabilitado",
                        self.plugin.id(),
                        elapsed,
                        budget
                    );
                }
                PluginHealth::Disabled => {}
            }
        } else if self.health.get() == PluginHealth::Slow {
            // Decaimiento del strike: si estaba en Slow y respondió dentro del
            // presupuesto, regresa a Ok.
            self.health.set(PluginHealth::Ok);
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
