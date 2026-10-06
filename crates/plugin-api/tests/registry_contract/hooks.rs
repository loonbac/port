//! Invocación de hooks: apariencia, entrada, layout y peticiones de sesión.

use std::cell::Cell;

use gpui::{div, Element};
use port_plugin_api::{KeyAction, LayoutHook, Plugin, PluginRegistry, SpaceHook};
use port_term_core::frame::Rgb;
use port_term_core::input::Key;
use port_term_core::pty::PtyConfig;

use crate::{MockFontPlugin, MockShortcutPlugin, MockTintPlugin, MockTransparencyPlugin};

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

/// Plugin de prueba con una petición de sesión que ejecuta un programa propio.
///
/// Guarda la petición pendiente en una celda para comprobar que el consumo la
/// agota: la primera consulta la entrega, la siguiente ya no ve nada.
struct MockSpawnPlugin {
    pending: Cell<Option<PtyConfig>>,
}

impl SpaceHook for MockSpawnPlugin {
    fn take_spawn_session_request(&self) -> Option<PtyConfig> {
        self.pending.take()
    }
}

impl Plugin for MockSpawnPlugin {
    fn id(&self) -> &'static str {
        "spawn"
    }

    fn name(&self) -> &'static str {
        "Mock Spawn"
    }

    fn space_hook(&self) -> Option<&dyn SpaceHook> {
        Some(self)
    }
}

fn echo_config(marker: &str) -> PtyConfig {
    PtyConfig {
        command: "/bin/echo".to_string(),
        args: vec![marker.to_string()],
        cwd: None,
    }
}

#[test]
fn a_pending_spawn_request_is_returned_once_and_then_exhausted() {
    let mut registry = PluginRegistry::new();
    registry.register(MockSpawnPlugin {
        pending: Cell::new(Some(echo_config("visor"))),
    });

    let config = registry
        .take_spawn_session_request()
        .expect("la petición pendiente debe entregarse");
    assert_eq!(config.command, "/bin/echo");
    assert_eq!(config.args, vec!["visor".to_string()]);
    assert!(
        registry.take_spawn_session_request().is_none(),
        "la petición se consume una sola vez"
    );
}

#[test]
fn a_registry_without_spawn_requests_returns_none() {
    let mut registry = PluginRegistry::new();
    // Plugin con hook de espacios, pero sin petición pendiente.
    registry.register(MockSpawnPlugin {
        pending: Cell::new(None),
    });
    // Plugins que ni siquiera implementan el hook de espacios.
    registry.register(MockTransparencyPlugin { opacity: 0.9 });
    registry.register(MockShortcutPlugin);

    assert!(
        registry.take_spawn_session_request().is_none(),
        "sin petición pendiente no hay nada que crear"
    );
}

#[test]
fn a_later_plugin_still_wins_when_an_earlier_one_has_no_request() {
    let mut registry = PluginRegistry::new();
    registry.register(MockSpawnPlugin {
        pending: Cell::new(None),
    });
    registry.register(MockSpawnPlugin {
        pending: Cell::new(Some(echo_config("segundo"))),
    });

    let config = registry
        .take_spawn_session_request()
        .expect("la primera petición disponible debe entregarse");
    assert_eq!(config.args, vec!["segundo".to_string()]);
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
fn appearance_hook_configures_custom_font() {
    let mut registry = PluginRegistry::new();
    registry.register(MockFontPlugin {
        family: "JetBrainsMono Nerd Font Mono",
        size: 15.0,
    });

    assert_eq!(
        registry.effective_font_family("Fira Code"),
        "JetBrainsMono Nerd Font Mono"
    );
    assert_eq!(registry.effective_font_size(14.0), 15.0);
    assert_eq!(
        registry.effective_font_fallbacks(),
        vec!["Custom Fallback".to_string()]
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

    // Si se desactiva el plugin, la tecla pasa al PTY
    registry.set_enabled("shortcut", false);
    assert_eq!(registry.dispatch_key(&ctrl_t), KeyAction::Pass);

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
