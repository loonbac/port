//! Invocación de los hooks de los plugins registrados.
//!
//! Es la mitad del registro que ejecuta el código de los plugins: apariencia,
//! entrada, ratón, layout, espacios y ciclo de vida. Cada método recorre los
//! plugins registrados y delega en `RegisteredPlugin::invoke_hook`, con el
//! presupuesto de hook del registro y el valor de respaldo que le toca a ese
//! hook, para que la vista siga consultando un solo objeto.
//!
//! La contabilidad del registro —qué plugins hay, cuáles están habilitados, su
//! configuración, sus servicios y la composición con los externos vivos— vive
//! en el módulo padre, que es quien conoce esas dos fuentes.

use std::path::Path;

use gpui::AnyElement;
use port_term_core::frame::Rgb;
use port_term_core::input::Key;
use port_term_core::pty::{PtyConfig, RunningApp};
use port_term_core::session::MousePolicy;

use crate::health::PluginHealth;
use crate::hooks::{CloseDecision, KeyAction};

use super::PluginRegistry;

impl PluginRegistry {
    /// Atajo efectivo para abrir/cerrar el menú de plugins (por defecto "ctrl+shift+l").
    /// Calcula la opacidad efectiva combinando los plugins de apariencia activos.
    /// Si ningún plugin especifica opacidad, el valor por defecto es `1.0` (opaco).
    ///
    /// Fuentes: el registro in-process y los externos vivos (una lista que
    /// mantiene al día quien arranca y apaga los procesos). Regla: gana la
    /// opacidad **mínima** de todas, que es la que ya aplicaba solo entre
    /// plugins in-process; un externo solo puede volver la ventana más
    /// translúcida, nunca tapar el ajuste de otro. Un valor fuera de rango se
    /// recorta a `0.0..=1.0`, igual que el in-process.
    pub fn effective_opacity(&self) -> f32 {
        let mut min_opacity = 1.0f32;
        for entry in &self.plugins {
            if let Some(op) = entry.invoke_hook(self.hook_budget, None, || {
                entry.plugin.appearance_hook().and_then(|h| h.opacity())
            }) {
                min_opacity = min_opacity.min(op.clamp(0.0, 1.0));
            }
        }
        for external in &self.externals {
            if let Some(op) = external.appearance().opacity {
                min_opacity = min_opacity.min(op.clamp(0.0, 1.0));
            }
        }
        min_opacity
    }

    /// Calcula el color de fondo efectivo aplicando los tintes de los plugins activos en orden.
    pub fn effective_background(&self, mut base: Rgb) -> Rgb {
        for entry in &self.plugins {
            base = entry.invoke_hook(self.hook_budget, base, || {
                match entry.plugin.appearance_hook() {
                    Some(hook) => hook.background_tint(base),
                    None => base,
                }
            });
        }
        base
    }

    /// Obtiene la familia de fuente configurada por los plugins activos, o el valor por defecto.
    ///
    /// Orden de precedencia: primero el registro **in-process** y después los
    /// externos vivos, en el orden en que están instalados. El in-process es la
    /// configuración propia del núcleo (lo que PORT trae y controla); los
    /// externos son añadidos instalados, así que no pueden pisarla. Gana el
    /// primer valor no vacío, igual que antes; un externo sin familia cede ante
    /// el siguiente.
    pub fn effective_font_family(&self, default: &str) -> String {
        for entry in &self.plugins {
            if let Some(family) = entry.invoke_hook(self.hook_budget, None, || {
                entry.plugin.appearance_hook().and_then(|h| h.font_family())
            }) {
                if !family.trim().is_empty() {
                    return family;
                }
            }
        }
        for external in &self.externals {
            if let Some(family) = external.appearance().font_family {
                if !family.trim().is_empty() {
                    return family;
                }
            }
        }
        default.to_string()
    }

    /// Obtiene el tamaño de fuente configurado por los plugins activos, o el valor por defecto.
    ///
    /// Misma precedencia que [`PluginRegistry::effective_font_family`]: primero
    /// el registro in-process —la configuración propia del núcleo— y después
    /// los externos vivos, por orden de instalación. Gana el primer valor
    /// presente.
    pub fn effective_font_size(&self, default: f32) -> f32 {
        for entry in &self.plugins {
            if let Some(size) = entry.invoke_hook(self.hook_budget, None, || {
                entry.plugin.appearance_hook().and_then(|h| h.font_size())
            }) {
                return size;
            }
        }
        for external in &self.externals {
            if let Some(size) = external.appearance().font_size {
                return size;
            }
        }
        default
    }

    /// Fuentes de respaldo acumuladas de los plugins activos.
    ///
    /// Solo mira el registro in-process: el protocolo [`protocol::Appearance`]
    /// que declaran los externos no lleva fallbacks (solo opacidad, familia,
    /// tamaño y color de fondo), así que no hay nada que componer sin ampliar
    /// el protocolo. Un externo no puede aportar respaldos hoy.
    pub fn effective_font_fallbacks(&self) -> Vec<String> {
        let mut fallbacks = Vec::new();
        for entry in &self.plugins {
            if let Some(fbs) = entry.invoke_hook(self.hook_budget, None, || {
                entry
                    .plugin
                    .appearance_hook()
                    .and_then(|h| h.font_fallbacks())
            }) {
                fallbacks.extend(fbs);
            }
        }
        if fallbacks.is_empty() {
            fallbacks = vec![
                "Symbols Nerd Font Mono".to_string(),
                "DejaVu Sans Mono".to_string(),
                "FreeMono".to_string(),
            ];
        }
        fallbacks
    }

    /// Despacha una tecla a través de los hooks de entrada de los plugins activos.
    pub fn dispatch_key(&self, key: &Key) -> KeyAction {
        for entry in &self.plugins {
            let action = entry.invoke_hook(self.hook_budget, KeyAction::Pass, || {
                entry
                    .plugin
                    .input_hook()
                    .map(|h| h.on_key(key))
                    .unwrap_or(KeyAction::Pass)
            });
            if action == KeyAction::Consume {
                return KeyAction::Consume;
            }
        }
        KeyAction::Pass
    }

    /// Política de ratón y selección vigente, compuesta con los plugins activos.
    ///
    /// Regla: el **primer** plugin registrado que implemente el
    /// [`MouseHook`] es el dueño de la política; es la misma precedencia
    /// «gana el primero» que la familia de fuente. Un plugin deshabilitado o
    /// con el watchdog disparado no aporta —`invoke_hook` ya devuelve el valor
    /// de respaldo cuando corresponde— así que cede ante el siguiente, y si
    /// ninguno la aporta rige [`MousePolicy::default()`]. Leer la política pasa
    /// por el mismo aislamiento de pánicos y el mismo presupuesto de hook que
    /// el resto de hooks.
    pub fn mouse_policy(&self) -> MousePolicy {
        for entry in &self.plugins {
            if !entry.enabled || entry.health.get() == PluginHealth::Disabled {
                continue;
            }
            if entry.plugin.mouse_hook().is_some() {
                return entry.invoke_hook(self.hook_budget, MousePolicy::default(), || {
                    entry
                        .plugin
                        .mouse_hook()
                        .map(|hook| hook.mouse_policy())
                        .unwrap_or_default()
                });
            }
        }
        MousePolicy::default()
    }

    /// Altura acumulada de las barras superiores activas.
    pub fn top_bar_height(&self) -> f32 {
        let mut max_h = 0.0f32;
        for entry in &self.plugins {
            let h = entry.invoke_hook(self.hook_budget, 0.0, || {
                entry
                    .plugin
                    .layout_hook()
                    .map(|h| h.top_bar_height())
                    .unwrap_or(0.0)
            });
            max_h = max_h.max(h);
        }
        max_h
    }

    /// Ancho acumulado de las barras laterales izquierdas activas.
    pub fn left_sidebar_width(&self) -> f32 {
        let mut max_w = 0.0f32;
        for entry in &self.plugins {
            let w = entry.invoke_hook(self.hook_budget, 0.0, || {
                entry
                    .plugin
                    .layout_hook()
                    .map(|h| h.left_sidebar_width())
                    .unwrap_or(0.0)
            });
            max_w = max_w.max(w);
        }
        max_w
    }

    /// Altura acumulada de las barras inferiores activas.
    pub fn bottom_bar_height(&self) -> f32 {
        let mut max_h = 0.0f32;
        for entry in &self.plugins {
            let h = entry.invoke_hook(self.hook_budget, 0.0, || {
                entry
                    .plugin
                    .layout_hook()
                    .map(|h| h.bottom_bar_height())
                    .unwrap_or(0.0)
            });
            max_h = max_h.max(h);
        }
        max_h
    }

    /// Identificador de la sesión activa que debe mostrarse y recibir teclado.
    pub fn active_session_id(&self) -> usize {
        for entry in &self.plugins {
            if !entry.enabled || entry.health.get() == PluginHealth::Disabled {
                continue;
            }
            if entry.plugin.space_hook().is_some() {
                return entry.invoke_hook(self.hook_budget, 0, || {
                    entry
                        .plugin
                        .space_hook()
                        .map(|h| h.active_session())
                        .unwrap_or(0)
                });
            }
        }
        0
    }

    /// Índice del espacio activo según los plugins registrados.
    pub fn active_space_index(&self) -> usize {
        for entry in &self.plugins {
            if !entry.enabled || entry.health.get() == PluginHealth::Disabled {
                continue;
            }
            if entry.plugin.space_hook().is_some() {
                return entry.invoke_hook(self.hook_budget, 0, || {
                    entry
                        .plugin
                        .space_hook()
                        .map(|h| h.active_space())
                        .unwrap_or(0)
                });
            }
        }
        0
    }

    /// Comprueba si algún plugin activo solicita crear una nueva sesión de shell/espacio/pestaña.
    pub fn take_new_session_request(&self) -> bool {
        for entry in &self.plugins {
            let requested = entry.invoke_hook(self.hook_budget, false, || {
                entry
                    .plugin
                    .space_hook()
                    .map(|h| h.take_new_session_request())
                    .unwrap_or(false)
            });
            if requested {
                return true;
            }
        }
        false
    }

    /// Sesión nueva pedida por un plugin con un programa concreto, si la hay.
    pub fn take_spawn_session_request(&self) -> Option<PtyConfig> {
        for entry in &self.plugins {
            let request = entry.invoke_hook(self.hook_budget, None, || {
                entry
                    .plugin
                    .space_hook()
                    .and_then(|h| h.take_spawn_session_request())
            });
            if request.is_some() {
                return request;
            }
        }
        None
    }

    /// Notifica a los plugins el ID de la sesión recién creada.
    pub fn on_session_created(&self, session_id: usize) {
        for entry in &self.plugins {
            entry.invoke_hook(self.hook_budget, (), || {
                if let Some(hook) = entry.plugin.space_hook() {
                    hook.on_session_created(session_id);
                }
            });
        }
    }

    /// Notifica a los plugins el directorio de trabajo actual y nombre de carpeta de una sesión.
    pub fn update_session_cwd(&self, session_id: usize, cwd: &Path, folder_name: &str) {
        for entry in &self.plugins {
            entry.invoke_hook(self.hook_budget, (), || {
                if let Some(hook) = entry.plugin.space_hook() {
                    hook.update_session_cwd(session_id, cwd, folder_name);
                }
            });
        }
    }

    /// Notifica a los plugins el programa en primer plano de una sesión.
    pub fn update_session_app(&self, session_id: usize, app: Option<&RunningApp>) {
        for entry in &self.plugins {
            entry.invoke_hook(self.hook_budget, (), || {
                if let Some(hook) = entry.plugin.space_hook() {
                    hook.update_session_app(session_id, app);
                }
            });
        }
    }

    /// Comprueba si algún plugin activo solicita cerrar una sesión específica.
    pub fn take_close_session_request(&self) -> Option<usize> {
        for entry in &self.plugins {
            let request = entry.invoke_hook(self.hook_budget, None, || {
                entry
                    .plugin
                    .space_hook()
                    .and_then(|h| h.take_close_session_request())
            });
            if request.is_some() {
                return request;
            }
        }
        None
    }

    /// Consulta a los plugins si el cierre de la ventana necesita confirmación.
    /// Basta con que uno pida confirmar para bloquearlo.
    pub fn close_decision(&self) -> CloseDecision {
        for entry in &self.plugins {
            let decision = entry.invoke_hook(self.hook_budget, CloseDecision::Allow, || {
                entry
                    .plugin
                    .lifecycle_hook()
                    .map(|h| h.on_close_request())
                    .unwrap_or(CloseDecision::Allow)
            });
            if decision == CloseDecision::Confirm {
                return CloseDecision::Confirm;
            }
        }
        CloseDecision::Allow
    }

    /// Notifica a los plugins de que una sesión ya no existe.
    pub fn on_session_closed(&self, session_id: usize) {
        for entry in &self.plugins {
            entry.invoke_hook(self.hook_budget, (), || {
                if let Some(hook) = entry.plugin.space_hook() {
                    hook.on_session_closed(session_id);
                }
            });
        }
    }

    /// Notifica a los plugins que el cierre fue aceptado.
    pub fn on_close_confirmed(&self) {
        for entry in &self.plugins {
            entry.invoke_hook(self.hook_budget, (), || {
                if let Some(hook) = entry.plugin.lifecycle_hook() {
                    hook.on_close_confirmed();
                }
            });
        }
    }

    /// Compatibilidad previa.
    pub fn take_new_space_request(&self) -> bool {
        self.take_new_session_request()
    }

    /// Compatibilidad previa.
    pub fn take_close_space_request(&self) -> Option<usize> {
        self.take_close_session_request()
    }

    /// Recopila los elementos para la barra superior (top_bar) de los plugins activos.
    pub fn top_bars(&self) -> Vec<AnyElement> {
        let mut elements = Vec::new();
        for entry in &self.plugins {
            if let Some(elem) = entry.invoke_hook(self.hook_budget, None, || {
                entry.plugin.layout_hook().and_then(|h| h.top_bar())
            }) {
                elements.push(elem);
            }
        }
        elements
    }

    /// Recopila los elementos para la barra lateral izquierda de los plugins activos.
    pub fn left_sidebars(&self) -> Vec<AnyElement> {
        let mut elements = Vec::new();
        for entry in &self.plugins {
            if let Some(elem) = entry.invoke_hook(self.hook_budget, None, || {
                entry.plugin.layout_hook().and_then(|h| h.left_sidebar())
            }) {
                elements.push(elem);
            }
        }
        elements
    }

    /// Recopila los elementos para la barra inferior de los plugins activos.
    pub fn bottom_bars(&self) -> Vec<AnyElement> {
        let mut elements = Vec::new();
        for entry in &self.plugins {
            if let Some(elem) = entry.invoke_hook(self.hook_budget, None, || {
                entry.plugin.layout_hook().and_then(|h| h.bottom_bar())
            }) {
                elements.push(elem);
            }
        }
        elements
    }
}
