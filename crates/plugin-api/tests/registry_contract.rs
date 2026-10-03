use gpui::{Element, div};
use port_plugin_api::{AppearanceHook, InputHook, KeyAction, LayoutHook, Plugin, PluginRegistry};
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

struct MockLayoutPlugin;

impl LayoutHook for MockLayoutPlugin {
    fn top_bar(&self) -> Option<gpui::AnyElement> {
        Some(div().into_any())
    }

    fn left_sidebar(&self) -> Option<gpui::AnyElement> {
        Some(div().into_any())
    }
}

impl Plugin for MockLayoutPlugin {
    fn id(&self) -> &'static str {
        "layout"
    }

    fn name(&self) -> &'static str {
        "Mock Layout"
    }

    fn layout_hook(&self) -> Option<&dyn LayoutHook> {
        Some(self)
    }
}

#[test]
fn empty_registry_has_default_values() {
    let registry = PluginRegistry::new();
    assert_eq!(registry.len(), 0);
    assert!(registry.is_empty());
    assert_eq!(registry.effective_opacity(), 1.0);
    assert_eq!(
        registry.effective_background(Rgb::new(10, 10, 10)),
        Rgb::new(10, 10, 10)
    );
    assert_eq!(registry.dispatch_key(&Key::new("a")), KeyAction::Pass);
    assert_eq!(registry.top_bars().len(), 0);
    assert_eq!(registry.left_sidebars().len(), 0);
}

#[test]
fn appearance_hook_calculates_effective_opacity_and_tint() {
    let mut registry = PluginRegistry::new();
    registry.register(MockTransparencyPlugin { opacity: 0.85 });
    registry.register(MockTintPlugin);

    assert_eq!(registry.len(), 2);
    assert_eq!(registry.effective_opacity(), 0.85);
    assert_eq!(
        registry.effective_background(Rgb::new(0, 0, 0)),
        Rgb::new(20, 30, 40)
    );
}

#[test]
fn appearance_hook_takes_minimum_opacity_when_multiple_present() {
    let mut registry = PluginRegistry::new();
    registry.register(MockTransparencyPlugin { opacity: 0.90 });
    registry.register(MockTransparencyPlugin { opacity: 0.75 });

    assert_eq!(registry.effective_opacity(), 0.75);
}

#[test]
fn input_hook_can_consume_or_pass_keys() {
    let mut registry = PluginRegistry::new();
    registry.register(MockShortcutPlugin);

    // Ctrl+T se consume
    let ctrl_t = Key::new("t").ctrl();
    assert_eq!(registry.dispatch_key(&ctrl_t), KeyAction::Consume);

    // Cualquier otra tecla pasa
    let plain_a = Key::new("a");
    assert_eq!(registry.dispatch_key(&plain_a), KeyAction::Pass);
}

#[test]
fn layout_hook_collects_slot_elements() {
    let mut registry = PluginRegistry::new();
    registry.register(MockLayoutPlugin);

    assert_eq!(registry.top_bars().len(), 1);
    assert_eq!(registry.left_sidebars().len(), 1);
    assert_eq!(registry.bottom_bars().len(), 0);
}
