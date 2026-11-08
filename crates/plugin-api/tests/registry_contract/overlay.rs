//! Overlay por cuadro: el primer plugin que devuelve Some manda, sin estado
//! pegado entre cuadros, y needs_animation consulta también a este hook.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, Ordering};

use port_plugin_api::{
    CellRect, CursorState, FrameView, Overlay, OverlayHook, OverlayQuad, Plugin, PluginRegistry,
};
use port_term_core::frame::{CursorShape, Rgb};

use crate::MockTransparencyPlugin;

fn vista() -> FrameView {
    FrameView {
        cursor: Some(CursorState {
            rect: CellRect {
                column: 3.0,
                row: 1.0,
                width: 1.0,
                height: 1.0,
            },
            shape: CursorShape::Block,
            color: Rgb::DEFAULT_FG,
        }),
        now_ms: 100,
        focused: true,
    }
}

fn quad_con_color(r: u8, g: u8, b: u8) -> OverlayQuad {
    OverlayQuad {
        rect: CellRect {
            column: 3.5,
            row: 1.0,
            width: 1.0,
            height: 1.0,
        },
        color: Rgb::new(r, g, b),
        opacity: 0.8,
        corner_radius_px: 2.0,
    }
}

fn overlay_con_color(r: u8, g: u8, b: u8) -> Overlay {
    Overlay {
        quads: vec![quad_con_color(r, g, b)],
        suppress_native_cursor: true,
    }
}

/// Plugin de prueba que aporta un overlay fijo y un tick configurable.
struct MockOverlayPlugin {
    id: &'static str,
    overlay: RefCell<Option<Overlay>>,
    animation: AtomicBool,
}

impl MockOverlayPlugin {
    fn fijo(id: &'static str, overlay: Overlay) -> Self {
        Self {
            id,
            overlay: RefCell::new(Some(overlay)),
            animation: AtomicBool::new(false),
        }
    }
}

impl OverlayHook for MockOverlayPlugin {
    fn overlay(&self, _view: &FrameView) -> Option<Overlay> {
        self.overlay.borrow().clone()
    }

    fn needs_animation(&self) -> bool {
        self.animation.load(Ordering::SeqCst)
    }
}

impl Plugin for MockOverlayPlugin {
    fn id(&self) -> &'static str {
        self.id
    }

    fn name(&self) -> &'static str {
        "Mock Overlay"
    }

    fn overlay_hook(&self) -> Option<&dyn OverlayHook> {
        Some(self)
    }
}

/// Sin ningún plugin de overlay rige el passthrough: nadie aporta nada.
#[test]
fn sin_plugin_de_overlay_no_hay_overlay() {
    let mut registry = PluginRegistry::new();
    registry.register(MockTransparencyPlugin { opacity: 0.9 });
    assert_eq!(registry.overlay(&vista()), None);
}

/// Un plugin de overlay aporta su overlay tal cual, con supresión incluida.
#[test]
fn un_plugin_de_overlay_aporta_su_overlay() {
    let esperado = overlay_con_color(1, 2, 3);
    let mut registry = PluginRegistry::new();
    registry.register(MockOverlayPlugin::fijo("overlay", esperado.clone()));
    assert_eq!(registry.overlay(&vista()), Some(esperado));
}

/// Con dos plugins de overlay gana el primero que devuelve Some.
#[test]
fn con_dos_plugins_de_overlay_gana_el_primero() {
    let primero = overlay_con_color(10, 0, 0);
    let segundo = overlay_con_color(0, 0, 90);
    let mut registry = PluginRegistry::new();
    registry.register(MockOverlayPlugin::fijo("primero", primero.clone()));
    registry.register(MockOverlayPlugin::fijo("segundo", segundo));
    assert_eq!(registry.overlay(&vista()), Some(primero));
}

/// El None del primero cede ante el siguiente: solo el primer Some gana.
#[test]
fn el_none_del_primero_cede_ante_el_siguiente() {
    let segundo = overlay_con_color(0, 0, 90);
    let passthrough = MockOverlayPlugin {
        id: "passthrough",
        overlay: RefCell::new(None),
        animation: AtomicBool::new(false),
    };
    let mut registry = PluginRegistry::new();
    registry.register(passthrough);
    registry.register(MockOverlayPlugin::fijo("segundo", segundo.clone()));
    assert_eq!(registry.overlay(&vista()), Some(segundo));
}

/// Un plugin de overlay deshabilitado no aporta: vuelve el passthrough.
#[test]
fn un_plugin_de_overlay_deshabilitado_no_aporta() {
    let mut registry = PluginRegistry::new();
    registry.register(MockOverlayPlugin::fijo(
        "overlay",
        overlay_con_color(1, 2, 3),
    ));
    assert!(registry.set_enabled("overlay", false));
    assert_eq!(registry.overlay(&vista()), None);
}

/// El registro no guarda nada entre cuadros: un None tras un Some quita la
/// supresión en ese mismo cuadro, que es la garantía del cuadro de transición.
#[test]
fn la_supresion_no_tiene_estado_pegado() {
    struct Intermitente {
        llamadas: Cell<usize>,
    }
    impl OverlayHook for Intermitente {
        fn overlay(&self, _view: &FrameView) -> Option<Overlay> {
            let n = self.llamadas.get();
            self.llamadas.set(n + 1);
            if n == 0 {
                Some(overlay_con_color(1, 2, 3))
            } else {
                None
            }
        }
    }
    impl Plugin for Intermitente {
        fn id(&self) -> &'static str {
            "intermitente"
        }
        fn name(&self) -> &'static str {
            "Mock Flaky Overlay"
        }
        fn overlay_hook(&self) -> Option<&dyn OverlayHook> {
            Some(self)
        }
    }
    let mut registry = PluginRegistry::new();
    registry.register(Intermitente {
        llamadas: Cell::new(0),
    });
    assert_eq!(registry.overlay(&vista()), Some(overlay_con_color(1, 2, 3)));
    assert_eq!(registry.overlay(&vista()), None);
}

/// needs_animation consulta también a los hooks de overlay, no solo a layout.
#[test]
fn needs_animation_consulta_tambien_al_overlay() {
    let mut animando = PluginRegistry::new();
    let plugin = MockOverlayPlugin {
        id: "overlay",
        overlay: RefCell::new(None),
        animation: AtomicBool::new(true),
    };
    animando.register(plugin);
    assert!(animando.needs_animation());

    let mut quieto = PluginRegistry::new();
    quieto.register(MockOverlayPlugin::fijo(
        "overlay",
        overlay_con_color(1, 2, 3),
    ));
    assert!(!quieto.needs_animation());
}
