use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use gpui::{Element, div};
use port_plugin_api::{
    AppearanceHook, InputHook, KeyAction, LayoutHook, Plugin, PluginConfig, PluginRegistry,
};
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

struct MockConfigurablePlugin {
    font_size: Arc<AtomicUsize>,
}

impl Plugin for MockConfigurablePlugin {
    fn id(&self) -> &'static str {
        "font-zoom"
    }

    fn name(&self) -> &'static str {
        "Mock Configurable Font Zoom"
    }

    fn default_config(&self) -> Option<PluginConfig> {
        let mut cfg = PluginConfig::new();
        cfg.set("default_size", 14);
        Some(cfg)
    }

    fn load_config(&self, config: &PluginConfig) {
        if let Some(size) = config.get_u32("default_size") {
            self.font_size.store(size as usize, Ordering::SeqCst);
        }
    }

    fn save_config(&self) -> Option<PluginConfig> {
        let mut cfg = PluginConfig::new();
        cfg.set("default_size", self.font_size.load(Ordering::SeqCst));
        Some(cfg)
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
    assert_eq!(registry.effective_font_family("Fira Code"), "Fira Code");
    assert_eq!(registry.effective_font_size(14.0), 14.0);
    assert_eq!(registry.effective_font_fallbacks().len(), 3);
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

#[test]
fn registry_creates_and_loads_configuration_file() {
    let dir = std::env::temp_dir().join(format!("port-test-cfg-{}", std::process::id()));
    let path = dir.join("config.md");
    let _ = std::fs::remove_dir_all(&dir);

    let font_size = Arc::new(AtomicUsize::new(0));
    let mut registry = PluginRegistry::new();
    registry.register(MockConfigurablePlugin {
        font_size: Arc::clone(&font_size),
    });

    // 1. Archivo no existe: se crea con el bloque de font-zoom
    registry.load_or_create_config(&path).unwrap();
    assert!(path.exists());
    let file_text = std::fs::read_to_string(&path).unwrap();
    assert!(file_text.contains("```font-zoom\ndefault_size = 14\n```"));
    assert_eq!(font_size.load(Ordering::SeqCst), 14);

    // 2. Modificamos el archivo a mano (ej. default_size = 22)
    let modified = file_text.replace("default_size = 14", "default_size = 22");
    std::fs::write(&path, modified).unwrap();

    // 3. Volvemos a cargar y debe reflejar 22
    registry.load_or_create_config(&path).unwrap();
    assert_eq!(font_size.load(Ordering::SeqCst), 22);

    // 4. El plugin actualiza su tamaño y guarda
    font_size.store(28, Ordering::SeqCst);
    let saved = registry.save_plugin_config("font-zoom", &path).unwrap();
    assert!(saved);

    let updated_text = std::fs::read_to_string(&path).unwrap();
    assert!(updated_text.contains("```font-zoom\ndefault_size = 28\n```"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn registry_reloads_config_in_real_time_when_modified() {
    let dir = std::env::temp_dir().join(format!("port-test-hotreload-{}", std::process::id()));
    let path = dir.join("config.md");
    let _ = std::fs::remove_dir_all(&dir);

    let font_size = Arc::new(AtomicUsize::new(0));
    let mut registry = PluginRegistry::new();
    registry.register(MockConfigurablePlugin {
        font_size: Arc::clone(&font_size),
    });

    registry.load_or_create_config(&path).unwrap();
    assert_eq!(font_size.load(Ordering::SeqCst), 14);

    // Comprobación inicial: no ha habido modificaciones
    assert_eq!(registry.reload_if_modified().unwrap(), false);

    // Modificamos el archivo externamente
    std::thread::sleep(std::time::Duration::from_millis(50));
    let content = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, content.replace("default_size = 14", "default_size = 24")).unwrap();

    // Ahora reload_if_modified detecta el cambio y actualiza el plugin en caliente
    assert_eq!(registry.reload_if_modified().unwrap(), true);
    assert_eq!(font_size.load(Ordering::SeqCst), 24);

    // Comprobación posterior: ya está sincronizado
    assert_eq!(registry.reload_if_modified().unwrap(), false);

    let _ = std::fs::remove_dir_all(&dir);
}
