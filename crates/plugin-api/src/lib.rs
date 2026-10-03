//! Contratos, capacidades y registro de plugins de PORT.
//!
//! Este crate define la API pública con la que interactúan los plugins.
//! Los plugins son componentes in-process desacoplados que extienden
//! la apariencia, la entrada o el layout de la terminal sin tocar el núcleo.

use gpui::AnyElement;
use port_term_core::frame::Rgb;
use port_term_core::input::Key;

/// Acción resultante tras procesar una pulsación en un hook de entrada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// La tecla continúa su curso normal hacia el PTY u otros plugins.
    Pass,
    /// El plugin consumió la tecla. No se envía al PTY.
    Consume,
}

/// Capacidades de personalización visual (fondo, opacidad, paleta).
pub trait AppearanceHook {
    /// Opacidad deseada para el fondo de la ventana (0.0 translúcido ..= 1.0 opaco).
    fn opacity(&self) -> Option<f32> {
        None
    }

    /// Tinte o modificación sobre el color de fondo base de la terminal.
    fn background_tint(&self, base: Rgb) -> Rgb {
        base
    }
}

/// Capacidades de intercepción y atajos de teclado.
pub trait InputHook {
    /// Permite a un plugin interceptar o consumir una tecla antes del PTY.
    fn on_key(&self, key: &Key) -> KeyAction {
        let _ = key;
        KeyAction::Pass
    }
}

/// Capacidades de inserción de cromo en el layout de la ventana (tabs, sidebar, status).
pub trait LayoutHook {
    /// Elemento que se inserta en la parte superior (ej. barra de pestañas).
    fn top_bar(&self) -> Option<AnyElement> {
        None
    }

    /// Elemento que se inserta a la izquierda (ej. barra de espacios/sidebar).
    fn left_sidebar(&self) -> Option<AnyElement> {
        None
    }

    /// Elemento que se inserta en la parte inferior (ej. barra de estado).
    fn bottom_bar(&self) -> Option<AnyElement> {
        None
    }
}

/// Interfaz base que todo plugin de PORT debe implementar.
pub trait Plugin: 'static {
    /// Identificador único del plugin en formato kebab-case (ej. "transparency").
    fn id(&self) -> &'static str;

    /// Nombre legible del plugin.
    fn name(&self) -> &'static str;

    /// Versión semántica del plugin.
    fn version(&self) -> &'static str {
        "0.1.0"
    }

    /// Hook de apariencia, si el plugin lo implementa.
    fn appearance_hook(&self) -> Option<&dyn AppearanceHook> {
        None
    }

    /// Hook de entrada, si el plugin lo implementa.
    fn input_hook(&self) -> Option<&dyn InputHook> {
        None
    }

    /// Hook de layout, si el plugin lo implementa.
    fn layout_hook(&self) -> Option<&dyn LayoutHook> {
        None
    }
}

/// Registro central de plugins activos de la terminal.
#[derive(Default)]
pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
}

impl PluginRegistry {
    /// Crea un registro de plugins vacío.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Registra un nuevo plugin en la terminal.
    pub fn register<P: Plugin>(&mut self, plugin: P) {
        self.plugins.push(Box::new(plugin));
    }

    /// Registra un plugin ya empaquetado en Box.
    pub fn register_boxed(&mut self, plugin: Box<dyn Plugin>) {
        self.plugins.push(plugin);
    }

    /// Devuelve el número de plugins registrados.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Devuelve `true` si no hay plugins registrados.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Calcula la opacidad efectiva combinando los plugins de apariencia.
    /// Si ningún plugin especifica opacidad, el valor por defecto es `1.0` (opaco).
    /// Si varios plugins especifican opacidad, se toma el valor mínimo (más transparente).
    pub fn effective_opacity(&self) -> f32 {
        let mut min_opacity = 1.0f32;
        for plugin in &self.plugins {
            if let Some(hook) = plugin.appearance_hook() {
                if let Some(op) = hook.opacity() {
                    min_opacity = min_opacity.min(op.clamp(0.0, 1.0));
                }
            }
        }
        min_opacity
    }

    /// Calcula el color de fondo efectivo aplicando los tintes de los plugins en orden.
    pub fn effective_background(&self, mut base: Rgb) -> Rgb {
        for plugin in &self.plugins {
            if let Some(hook) = plugin.appearance_hook() {
                base = hook.background_tint(base);
            }
        }
        base
    }

    /// Despacha una tecla a través de los hooks de entrada registrados.
    /// Se detiene en el primer plugin que consuma la tecla.
    pub fn dispatch_key(&self, key: &Key) -> KeyAction {
        for plugin in &self.plugins {
            if let Some(hook) = plugin.input_hook() {
                if hook.on_key(key) == KeyAction::Consume {
                    return KeyAction::Consume;
                }
            }
        }
        KeyAction::Pass
    }

    /// Recopila los elementos para la barra superior (top_bar).
    pub fn top_bars(&self) -> Vec<AnyElement> {
        let mut elements = Vec::new();
        for plugin in &self.plugins {
            if let Some(hook) = plugin.layout_hook() {
                if let Some(elem) = hook.top_bar() {
                    elements.push(elem);
                }
            }
        }
        elements
    }

    /// Recopila los elementos para la barra lateral izquierda (left_sidebar).
    pub fn left_sidebars(&self) -> Vec<AnyElement> {
        let mut elements = Vec::new();
        for plugin in &self.plugins {
            if let Some(hook) = plugin.layout_hook() {
                if let Some(elem) = hook.left_sidebar() {
                    elements.push(elem);
                }
            }
        }
        elements
    }

    /// Recopila los elementos para la barra inferior (bottom_bar).
    pub fn bottom_bars(&self) -> Vec<AnyElement> {
        let mut elements = Vec::new();
        for plugin in &self.plugins {
            if let Some(hook) = plugin.layout_hook() {
                if let Some(elem) = hook.bottom_bar() {
                    elements.push(elem);
                }
            }
        }
        elements
    }
}
