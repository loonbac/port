use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use gpui::{div, Element};
use port_plugin_api::{
    AppearanceHook, CloseDecision, InputHook, KeyAction, LayoutHook, LifecycleHook, Plugin,
    PluginConfig, PluginRegistry, SpaceHook,
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

// ── Directorio de servicios: llamadas entre plugins ──────────────────────────

struct CountingService;

impl port_plugin_api::Service for CountingService {
    fn id(&self) -> &str {
        "counter"
    }
    fn name(&self) -> &str {
        "Counter"
    }
    fn actions(&self) -> Vec<&'static str> {
        vec!["add", "read"]
    }
    fn invoke(
        &self,
        action: &str,
        args: &[port_plugin_api::Arg],
    ) -> Option<std::result::Result<port_plugin_api::Ret, port_plugin_api::ServiceError>> {
        match action {
            "add" => {
                let amount = match args.first() {
                    Some(port_plugin_api::Arg::Num(n)) => *n,
                    _ => {
                        return Some(Err(port_plugin_api::ServiceError::Failed(
                            "add requiere un número".to_string(),
                        )))
                    }
                };
                Some(Ok(port_plugin_api::Ret::Num(100.0 + amount)))
            }
            "read" => Some(Ok(port_plugin_api::Ret::Num(100.0))),
            _ => None,
        }
    }
}

struct ServicePublishingPlugin;

impl Plugin for ServicePublishingPlugin {
    fn id(&self) -> &'static str {
        "counter"
    }
    fn name(&self) -> &'static str {
        "Counter Plugin"
    }
    fn services(&self) -> Vec<std::sync::Arc<dyn port_plugin_api::Service>> {
        vec![std::sync::Arc::new(CountingService)]
    }
}

#[test]
fn registering_a_plugin_publishes_its_services() {
    use port_plugin_api::{Arg, Ret, ServiceError};

    let mut registry = PluginRegistry::new();
    let services = registry.services();

    // Antes de registrar el plugin, el servicio no existe.
    assert!(!services.has("counter"));
    assert_eq!(
        services.call("counter", "read", &[]),
        Err(ServiceError::UnknownService("counter".to_string()))
    );

    registry.register(ServicePublishingPlugin);
    assert!(services.has("counter"));
    assert_eq!(services.published(), vec!["counter".to_string()]);

    // Invocación con argumentos y retorno de valor.
    assert_eq!(
        services.call("counter", "add", &[Arg::Num(5.0)]),
        Ok(Ret::Num(105.0))
    );

    // Acción inexistente: error explícito, no silencio.
    assert_eq!(
        services.call("counter", "nope", &[]),
        Err(ServiceError::UnknownAction {
            service: "counter".to_string(),
            action: "nope".to_string()
        })
    );
}

#[test]
fn a_service_action_can_fail_with_a_reason() {
    use port_plugin_api::{Arg, Ret, ServiceError};

    let mut registry = PluginRegistry::new();
    registry.register(ServicePublishingPlugin);
    let services = registry.services();

    // `add` sin argumento debe explicar por qué falló.
    assert_eq!(
        services.call("counter", "add", &[Arg::Unit]),
        Err(ServiceError::Failed("add requiere un número".to_string()))
    );
    // Y con el argumento correcto, funciona.
    assert_eq!(
        services.call("counter", "add", &[Arg::Num(1.0)]),
        Ok(Ret::Num(101.0))
    );
}

/// Plugin que entra en pánico en todos sus hooks.
///
/// Es el peor caso posible: código de terceros que revienta dentro de la
/// terminal. Sirve para comprobar que el núcleo sobrevive.
struct PanickingPlugin;

impl AppearanceHook for PanickingPlugin {
    fn opacity(&self) -> Option<f32> {
        panic!("hook de apariencia fallo");
    }

    fn background_tint(&self, _base: Rgb) -> Rgb {
        panic!("tinte fallo");
    }

    fn font_family(&self) -> Option<String> {
        panic!("familia fallo");
    }
}

impl InputHook for PanickingPlugin {
    fn on_key(&self, _key: &Key) -> KeyAction {
        panic!("manejo de teclado fallo");
    }
}

impl LayoutHook for PanickingPlugin {
    fn left_sidebar_width(&self) -> f32 {
        panic!("sidebar fallo");
    }

    fn top_bar_height(&self) -> f32 {
        panic!("top bar fallo");
    }

    fn left_sidebar(&self) -> Option<gpui::AnyElement> {
        panic!("sidebar fallo");
    }
}

impl SpaceHook for PanickingPlugin {
    fn active_session(&self) -> usize {
        panic!("sesion activa fallo");
    }

    fn take_new_session_request(&self) -> bool {
        panic!("peticion de sesion fallo");
    }

    fn on_session_created(&self, _session_id: usize) {
        panic!("notificacion fallo");
    }
}

impl LifecycleHook for PanickingPlugin {
    fn on_close_request(&self) -> CloseDecision {
        panic!("decision de cierre fallo");
    }
}

impl Plugin for PanickingPlugin {
    fn id(&self) -> &'static str {
        "panicking"
    }

    fn name(&self) -> &'static str {
        "Panicking"
    }

    fn appearance_hook(&self) -> Option<&dyn AppearanceHook> {
        Some(self)
    }

    fn input_hook(&self) -> Option<&dyn InputHook> {
        Some(self)
    }

    fn layout_hook(&self) -> Option<&dyn LayoutHook> {
        Some(self)
    }

    fn space_hook(&self) -> Option<&dyn SpaceHook> {
        Some(self)
    }

    fn lifecycle_hook(&self) -> Option<&dyn LifecycleHook> {
        Some(self)
    }
}

/// Plugin sano que veta el cierre de la ventana.
struct MockCloseGuardPlugin;

impl LifecycleHook for MockCloseGuardPlugin {
    fn on_close_request(&self) -> CloseDecision {
        CloseDecision::Confirm
    }
}

impl Plugin for MockCloseGuardPlugin {
    fn id(&self) -> &'static str {
        "close-guard"
    }

    fn name(&self) -> &'static str {
        "Close Guard"
    }

    fn lifecycle_hook(&self) -> Option<&dyn LifecycleHook> {
        Some(self)
    }
}

#[test]
fn a_panicking_plugin_cannot_take_down_the_terminal() {
    let mut registry = PluginRegistry::new();
    registry.register(PanickingPlugin);

    // Cada llamada ejecuta código de plugin que entra en pánico. Si el
    // aislamiento fallara, ninguna llegaría aquí.
    assert_eq!(
        registry.effective_opacity(),
        1.0,
        "debe quedar opaco, no sin valor"
    );
    assert_eq!(registry.effective_font_family("Fira Code"), "Fira Code");
    assert_eq!(registry.left_sidebar_width(), 0.0);
    assert_eq!(registry.top_bar_height(), 0.0);
    assert_eq!(registry.dispatch_key(&Key::new("a")), KeyAction::Pass);
    registry.on_session_created(1);
    registry.on_session_closed(1);
    registry.take_new_session_request();
}

#[test]
fn a_panicking_plugin_does_not_stop_the_others_from_working() {
    let mut registry = PluginRegistry::new();
    // El que revienta va primero: si contaminara el estado, el segundo no
    // llegaría a ejecutarse.
    registry.register(PanickingPlugin);
    registry.register(MockTintPlugin);

    assert_eq!(
        registry.effective_background(Rgb::new(0, 0, 0)),
        Rgb::new(20, 30, 40),
        "el plugin sano debe seguir aportando lo suyo"
    );
}

#[test]
fn a_panicking_plugin_does_not_block_the_close_guard() {
    let mut registry = PluginRegistry::new();
    registry.register(PanickingPlugin);
    registry.register(MockCloseGuardPlugin);

    // El guard veta el cierre. Un pánico previo no debe convertir el veto en
    // "dejar cerrar": se perderían procesos en ejecución.
    assert_eq!(registry.close_decision(), CloseDecision::Confirm);
}
