//! Política de ratón y selección: la del plugin más prioritario, o la de PORT.

use port_plugin_api::{MouseHook, Plugin, PluginRegistry};
use port_term_core::frame::Rgb;
use port_term_core::session::MousePolicy;

use crate::MockTransparencyPlugin;

/// Plugin de prueba que aporta una política de ratón y selección concreta.
struct MockMousePlugin {
    policy: MousePolicy,
}

impl MouseHook for MockMousePlugin {
    fn mouse_policy(&self) -> MousePolicy {
        self.policy
    }
}

impl Plugin for MockMousePlugin {
    fn id(&self) -> &'static str {
        "mouse"
    }

    fn name(&self) -> &'static str {
        "Mock Mouse"
    }

    fn mouse_hook(&self) -> Option<&dyn MouseHook> {
        Some(self)
    }
}

/// Sin ningún plugin que implemente el hook de ratón, rige la política de PORT.
#[test]
fn sin_plugin_de_raton_la_politica_es_la_de_port() {
    let mut registry = PluginRegistry::new();
    registry.register(MockTransparencyPlugin { opacity: 0.9 });
    assert_eq!(registry.mouse_policy(), MousePolicy::default());
}

/// Un plugin que implementa el hook es el dueño de la política vigente.
#[test]
fn un_plugin_de_raton_aporta_su_politica() {
    let policy = MousePolicy {
        forward_clicks: false,
        highlight: Rgb::new(1, 2, 3),
        ..MousePolicy::default()
    };
    let mut registry = PluginRegistry::new();
    registry.register(MockMousePlugin { policy });
    assert_eq!(registry.mouse_policy(), policy);
}

/// Un plugin de ratón deshabilitado no aporta: vuelve la política de PORT.
#[test]
fn un_plugin_de_raton_deshabilitado_no_aporta() {
    let policy = MousePolicy {
        forward_clicks: false,
        ..MousePolicy::default()
    };
    let mut registry = PluginRegistry::new();
    registry.register(MockMousePlugin { policy });
    assert!(registry.set_enabled("mouse", false));
    assert_eq!(registry.mouse_policy(), MousePolicy::default());
}

/// Con varios plugins, gana el primero registrado que implemente el hook, de
/// forma determinista: es la misma precedencia «gana el primero» de la fuente.
#[test]
fn con_dos_plugins_de_raton_gana_el_primero() {
    let primera = MousePolicy {
        forward_drag: false,
        ..MousePolicy::default()
    };
    let segunda = MousePolicy {
        forward_motion: false,
        ..MousePolicy::default()
    };
    let mut registry = PluginRegistry::new();
    registry.register(MockMousePlugin { policy: primera });
    registry.register(MockMousePlugin { policy: segunda });
    assert_eq!(registry.mouse_policy(), primera);
}

/// Un plugin de ratón deshabilitado cede ante el siguiente que sí aporte; solo
/// si ninguno de los que implementan el hook aporta vuelve la política de PORT.
#[test]
fn un_plugin_de_raton_deshabilitado_cede_ante_el_siguiente() {
    let primera = MousePolicy {
        forward_drag: false,
        ..MousePolicy::default()
    };
    let segunda = MousePolicy {
        forward_motion: false,
        ..MousePolicy::default()
    };
    let mut registry = PluginRegistry::new();
    registry.register(MockMousePlugin { policy: primera });
    registry.register(MockMousePlugin { policy: segunda });
    assert!(registry.set_enabled("mouse", false));
    assert_eq!(registry.mouse_policy(), segunda);
}
