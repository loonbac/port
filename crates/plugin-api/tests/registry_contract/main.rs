//! Contrato del registro de plugins: ciclo de vida, hooks, servicios y externos.
//!
//! Cada módulo cubre un área del registro. Aquí viven las piezas de prueba que
//! comparten varias áreas; las que solo usa un área viven en su módulo.

#[cfg(unix)]
mod externals;
mod hooks;
mod mouse;
mod registration;
mod services;
mod watchdog;

use port_plugin_api::{AppearanceHook, InputHook, KeyAction, Plugin};
use port_term_core::frame::Rgb;
use port_term_core::input::Key;

struct MockTransparencyPlugin {
    opacity: f32,
}

impl AppearanceHook for MockTransparencyPlugin {
    fn opacity(&self) -> Option<f32> {
        Some(self.opacity)
    }
}

impl Plugin for MockTransparencyPlugin {
    fn id(&self) -> &'static str {
        "transparency"
    }

    fn name(&self) -> &'static str {
        "Mock Transparency"
    }

    fn appearance_hook(&self) -> Option<&dyn AppearanceHook> {
        Some(self)
    }
}

struct MockFontPlugin {
    family: &'static str,
    size: f32,
}

impl AppearanceHook for MockFontPlugin {
    fn font_family(&self) -> Option<String> {
        Some(self.family.to_string())
    }

    fn font_size(&self) -> Option<f32> {
        Some(self.size)
    }

    fn font_fallbacks(&self) -> Option<Vec<String>> {
        Some(vec!["Custom Fallback".to_string()])
    }
}

impl Plugin for MockFontPlugin {
    fn id(&self) -> &'static str {
        "font"
    }

    fn name(&self) -> &'static str {
        "Mock Font"
    }

    fn appearance_hook(&self) -> Option<&dyn AppearanceHook> {
        Some(self)
    }
}

struct MockTintPlugin;

impl AppearanceHook for MockTintPlugin {
    fn background_tint(&self, _base: Rgb) -> Rgb {
        Rgb::new(20, 30, 40)
    }
}

impl Plugin for MockTintPlugin {
    fn id(&self) -> &'static str {
        "tint"
    }

    fn name(&self) -> &'static str {
        "Mock Tint"
    }

    fn appearance_hook(&self) -> Option<&dyn AppearanceHook> {
        Some(self)
    }
}

struct MockShortcutPlugin;

impl InputHook for MockShortcutPlugin {
    fn on_key(&self, key: &Key) -> KeyAction {
        if key.ctrl && key.key == "t" {
            KeyAction::Consume
        } else {
            KeyAction::Pass
        }
    }
}

impl Plugin for MockShortcutPlugin {
    fn id(&self) -> &'static str {
        "shortcut"
    }

    fn name(&self) -> &'static str {
        "Mock Shortcut"
    }

    fn input_hook(&self) -> Option<&dyn InputHook> {
        Some(self)
    }
}
