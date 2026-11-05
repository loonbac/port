//! Ciclo de vida y registro: valores por defecto, activación y configuración.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use port_plugin_api::{KeyAction, Plugin, PluginConfig, PluginRegistry};
use port_term_core::frame::Rgb;
use port_term_core::input::Key;
use port_term_core::session::MousePolicy;

use crate::MockTransparencyPlugin;

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
    assert_eq!(registry.mouse_policy(), MousePolicy::default());
}

#[test]
fn disabling_plugin_omits_its_effects() {
    let mut registry = PluginRegistry::new();
    registry.register(MockTransparencyPlugin { opacity: 0.70 });
    assert_eq!(registry.effective_opacity(), 0.70);

    // Desactivamos el plugin de transparencia
    assert!(registry.set_enabled("transparency", false));
    assert!(!registry.is_enabled("transparency"));

    // La opacidad vuelve al 1.0 por defecto
    assert_eq!(registry.effective_opacity(), 1.0);

    // Al alternar vuelve a estar activo
    assert_eq!(registry.toggle_enabled("transparency"), Some(true));
    assert!(registry.is_enabled("transparency"));
    assert_eq!(registry.effective_opacity(), 0.70);
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

    // 1. Archivo no existe: se crea con el bloque de font-zoom incluyendo enabled = true
    registry.load_or_create_config(&path).unwrap();
    assert!(path.exists());
    let file_text = std::fs::read_to_string(&path).unwrap();
    assert!(file_text.contains("default_size = 14"));
    assert!(file_text.contains("enabled = true"));
    assert_eq!(font_size.load(Ordering::SeqCst), 14);

    // 2. Modificamos el archivo a mano (ej. default_size = 22 y enabled = false)
    let modified = file_text
        .replace("default_size = 14", "default_size = 22")
        .replace("enabled = true", "enabled = false");
    std::fs::write(&path, modified).unwrap();

    // 3. Volvemos a cargar y debe reflejar 22 e inactivo
    registry.load_or_create_config(&path).unwrap();
    assert_eq!(font_size.load(Ordering::SeqCst), 22);
    assert!(!registry.is_enabled("font-zoom"));

    // 4. El plugin actualiza su tamaño y guarda
    font_size.store(28, Ordering::SeqCst);
    let saved = registry.save_plugin_config("font-zoom", &path).unwrap();
    assert!(saved);

    let updated_text = std::fs::read_to_string(&path).unwrap();
    assert!(updated_text.contains("default_size = 28"));
    assert!(updated_text.contains("enabled = false"));

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
    assert!(!registry.reload_if_modified().unwrap());

    // Modificamos el archivo externamente
    std::thread::sleep(std::time::Duration::from_millis(50));
    let content = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        content.replace("default_size = 14", "default_size = 24"),
    )
    .unwrap();

    // Ahora reload_if_modified detecta el cambio y actualiza el plugin en caliente
    assert!(registry.reload_if_modified().unwrap());
    assert_eq!(font_size.load(Ordering::SeqCst), 24);

    // Comprobación posterior: ya está sincronizado
    assert!(!registry.reload_if_modified().unwrap());

    let _ = std::fs::remove_dir_all(&dir);
}

/// Un plugin que solo cuenta cuántas veces le llega su bloque.
struct CountingPlugin {
    id: &'static str,
    loads: Arc<AtomicUsize>,
}

impl Plugin for CountingPlugin {
    fn id(&self) -> &'static str {
        self.id
    }

    fn name(&self) -> &'static str {
        "Counting Plugin"
    }

    fn default_config(&self) -> Option<PluginConfig> {
        let mut cfg = PluginConfig::new();
        cfg.set("value", 1);
        Some(cfg)
    }

    fn load_config(&self, _config: &PluginConfig) {
        self.loads.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn reloading_the_file_only_notifies_the_plugin_whose_block_changed() {
    let dir = std::env::temp_dir().join(format!(
        "port-test-reload-isolation-{}",
        std::process::id()
    ));
    let path = dir.join("config.md");
    let _ = std::fs::remove_dir_all(&dir);

    let herdr_loads = Arc::new(AtomicUsize::new(0));
    let zoom_loads = Arc::new(AtomicUsize::new(0));
    let mut registry = PluginRegistry::new();
    registry.register(CountingPlugin {
        id: "herdr",
        loads: Arc::clone(&herdr_loads),
    });
    registry.register(CountingPlugin {
        id: "font-zoom",
        loads: Arc::clone(&zoom_loads),
    });

    registry.load_or_create_config(&path).unwrap();
    assert_eq!(herdr_loads.load(Ordering::SeqCst), 1);
    assert_eq!(zoom_loads.load(Ordering::SeqCst), 1);

    // Se edita SOLO la sección nativa de la fuente. Ningún plugin tiene por qué
    // enterarse: reenviarle su bloque intacto autoriza a uno que reconstruye
    // estado desde su configuración —herdr y sus spaces— a destruirlo sin que
    // nadie lo haya pedido.
    std::thread::sleep(std::time::Duration::from_millis(50));
    let content = std::fs::read_to_string(&path).unwrap();
    let edited = content
        .lines()
        .map(|line| {
            if line.starts_with("family = ") {
                "family = Obel Mono"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_ne!(edited, content, "el archivo trae la sección font");
    std::fs::write(&path, edited).unwrap();

    assert!(registry.reload_if_modified().unwrap());
    assert_eq!(
        herdr_loads.load(Ordering::SeqCst),
        1,
        "un bloque ajeno intacto no avisa al plugin"
    );
    assert_eq!(zoom_loads.load(Ordering::SeqCst), 1);

    // Y el cambio propio sí llega: el filtro no puede silenciar al interesado.
    std::thread::sleep(std::time::Duration::from_millis(50));
    let content = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, content.replace("value = 1", "value = 2")).unwrap();
    assert!(registry.reload_if_modified().unwrap());
    assert_eq!(herdr_loads.load(Ordering::SeqCst), 2);
    assert_eq!(zoom_loads.load(Ordering::SeqCst), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

