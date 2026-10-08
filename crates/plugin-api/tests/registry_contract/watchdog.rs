//! Vigilancia de hooks: aislamiento de pánicos y salud (slow/disabled).

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use gpui::{div, Element};
use port_plugin_api::{
    AppearanceHook, CloseDecision, InputHook, KeyAction, LayoutHook, LifecycleHook, MouseHook,
    Plugin, PluginHealth, PluginRegistry, SpaceHook, DEFAULT_HOOK_BUDGET,
};
use port_term_core::frame::Rgb;
use port_term_core::input::Key;
use port_term_core::session::MousePolicy;

use crate::{MockTintPlugin, MockTransparencyPlugin};

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

impl MouseHook for PanickingPlugin {
    fn mouse_policy(&self) -> MousePolicy {
        panic!("politica de raton fallo");
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

    fn mouse_hook(&self) -> Option<&dyn MouseHook> {
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

/// Plugin lento de prueba cuya invocación incrementa un contador y duerme una duración fija.
struct MockSlowPlugin {
    delay: Duration,
    counter: Arc<AtomicUsize>,
}

impl MockSlowPlugin {
    fn new(delay: Duration, counter: Arc<AtomicUsize>) -> Self {
        Self { delay, counter }
    }
}

impl AppearanceHook for MockSlowPlugin {
    fn opacity(&self) -> Option<f32> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        Some(0.42)
    }
}

impl Plugin for MockSlowPlugin {
    fn id(&self) -> &'static str {
        "slow"
    }

    fn name(&self) -> &'static str {
        "Mock Slow"
    }

    fn appearance_hook(&self) -> Option<&dyn AppearanceHook> {
        Some(self)
    }
}

/// Plugin de layout lento para probar que los hooks con AnyElement también son vigilados.
struct MockSlowLayoutPlugin {
    delay: Duration,
    counter: Arc<AtomicUsize>,
}

impl LayoutHook for MockSlowLayoutPlugin {
    fn top_bar(&self) -> Option<gpui::AnyElement> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        Some(div().into_any())
    }
}

impl Plugin for MockSlowLayoutPlugin {
    fn id(&self) -> &'static str {
        "slow-layout"
    }

    fn name(&self) -> &'static str {
        "Mock Slow Layout"
    }

    fn layout_hook(&self) -> Option<&dyn LayoutHook> {
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
    assert_eq!(registry.mouse_policy(), MousePolicy::default());
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

#[test]
fn healthy_plugin_never_changes_state() {
    let mut registry = PluginRegistry::new();
    registry.set_hook_budget(Duration::from_millis(50));
    registry.register(MockTransparencyPlugin { opacity: 0.85 });

    for _ in 0..10 {
        assert_eq!(registry.effective_opacity(), 0.85);
    }

    assert_eq!(
        registry.plugin_health("transparency"),
        Some(PluginHealth::Ok)
    );
    let plugins = registry.list_plugins();
    assert_eq!(plugins[0].health, PluginHealth::Ok);
}

#[test]
fn slow_plugin_transitions_to_slow_then_disabled_and_stops_executing() {
    let mut registry = PluginRegistry::new();
    registry.set_hook_budget(Duration::from_millis(2));
    let counter = Arc::new(AtomicUsize::new(0));
    let slow = MockSlowPlugin::new(Duration::from_millis(6), Arc::clone(&counter));
    registry.register(slow);

    assert_eq!(registry.plugin_health("slow"), Some(PluginHealth::Ok));
    assert_eq!(counter.load(Ordering::SeqCst), 0);

    // Primer disparo lento: pasa a Slow y su resultado sí se usa
    let op1 = registry.effective_opacity();
    assert_eq!(op1, 0.42);
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    assert_eq!(registry.plugin_health("slow"), Some(PluginHealth::Slow));

    // Segundo disparo lento: pasa a Disabled y su resultado se usa
    let op2 = registry.effective_opacity();
    assert_eq!(op2, 0.42);
    assert_eq!(counter.load(Ordering::SeqCst), 2);
    assert_eq!(registry.plugin_health("slow"), Some(PluginHealth::Disabled));

    // Tercer disparo: ya NO se ejecuta su código, se devuelve el valor neutro (1.0)
    let op3 = registry.effective_opacity();
    assert_eq!(op3, 1.0);
    assert_eq!(counter.load(Ordering::SeqCst), 2);

    // Cuarto disparo: sigue sin ejecutarse
    let op4 = registry.effective_opacity();
    assert_eq!(op4, 1.0);
    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

#[test]
fn configurable_budget_is_respected() {
    let mut registry = PluginRegistry::new();
    assert_eq!(registry.hook_budget(), DEFAULT_HOOK_BUDGET);

    // Con presupuesto grande (100ms), una llamada de 6ms no se considera lenta
    registry.set_hook_budget(Duration::from_millis(100));
    assert_eq!(registry.hook_budget(), Duration::from_millis(100));

    let counter = Arc::new(AtomicUsize::new(0));
    let slow = MockSlowPlugin::new(Duration::from_millis(6), Arc::clone(&counter));
    registry.register(slow);

    let _ = registry.effective_opacity();
    assert_eq!(registry.plugin_health("slow"), Some(PluginHealth::Ok));

    // Si reducimos el presupuesto a 2ms, la siguiente llamada de 6ms se detecta como lenta
    registry.set_hook_budget(Duration::from_millis(2));
    let _ = registry.effective_opacity();
    assert_eq!(registry.plugin_health("slow"), Some(PluginHealth::Slow));
}

#[test]
fn slow_plugin_does_not_prevent_other_plugins_from_contributing() {
    let mut registry = PluginRegistry::new();
    registry.set_hook_budget(Duration::from_millis(2));

    let counter = Arc::new(AtomicUsize::new(0));
    let slow = MockSlowPlugin::new(Duration::from_millis(6), Arc::clone(&counter));
    registry.register(slow);
    registry.register(MockTintPlugin);

    // La llamada ejecuta ambos; MockTintPlugin debe seguir aplicando su tinte
    let bg = registry.effective_background(Rgb::new(0, 0, 0));
    assert_eq!(bg, Rgb::new(20, 30, 40));

    // Desactivamos slow forzando el segundo rebase
    let _ = registry.effective_opacity();
    let _ = registry.effective_opacity();
    assert_eq!(registry.plugin_health("slow"), Some(PluginHealth::Disabled));

    // MockTintPlugin sigue aportando normalmente
    let bg2 = registry.effective_background(Rgb::new(0, 0, 0));
    assert_eq!(bg2, Rgb::new(20, 30, 40));
}

#[test]
fn disabled_plugin_cannot_revert_to_ok() {
    let mut registry = PluginRegistry::new();
    registry.set_hook_budget(Duration::from_millis(2));
    let counter = Arc::new(AtomicUsize::new(0));
    let slow = MockSlowPlugin::new(Duration::from_millis(6), Arc::clone(&counter));
    registry.register(slow);

    // Llevar a Disabled
    let _ = registry.effective_opacity();
    let _ = registry.effective_opacity();
    assert_eq!(registry.plugin_health("slow"), Some(PluginHealth::Disabled));

    // Intento de habilitar manualmente con set_enabled
    assert!(!registry.set_enabled("slow", true));
    assert_eq!(registry.plugin_health("slow"), Some(PluginHealth::Disabled));

    // Intento de alternar con toggle_enabled
    assert_eq!(registry.toggle_enabled("slow"), None);
    assert_eq!(registry.plugin_health("slow"), Some(PluginHealth::Disabled));

    // Intento con toggle_enabled_at
    assert_eq!(registry.toggle_enabled_at(0), None);
    assert_eq!(registry.plugin_health("slow"), Some(PluginHealth::Disabled));

    // Comprobamos que sigue deshabilitado en consultas
    assert!(!registry.is_enabled("slow"));
    assert_eq!(registry.plugin_health("slow"), Some(PluginHealth::Disabled));
}

#[test]
fn plugin_health_is_reflected_in_plugin_info() {
    let mut registry = PluginRegistry::new();
    registry.set_hook_budget(Duration::from_millis(2));
    let counter = Arc::new(AtomicUsize::new(0));
    let slow = MockSlowPlugin::new(Duration::from_millis(6), Arc::clone(&counter));
    registry.register(slow);

    let info_initial = registry.list_plugins();
    assert_eq!(info_initial[0].health, PluginHealth::Ok);

    // 1 strike
    let _ = registry.effective_opacity();
    let info_slow = registry.list_plugins();
    assert_eq!(info_slow[0].health, PluginHealth::Slow);

    // 2 strikes
    let _ = registry.effective_opacity();
    let info_disabled = registry.list_plugins();
    assert_eq!(info_disabled[0].health, PluginHealth::Disabled);
}

#[test]
fn slow_layout_hook_with_any_element_is_disabled_by_watchdog() {
    let mut registry = PluginRegistry::new();
    registry.set_hook_budget(Duration::from_millis(2));
    let counter = Arc::new(AtomicUsize::new(0));
    let slow_layout = MockSlowLayoutPlugin {
        delay: Duration::from_millis(6),
        counter: Arc::clone(&counter),
    };
    registry.register(slow_layout);

    // Strike 1
    let bars1 = registry.top_bars();
    assert_eq!(bars1.len(), 1);
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    assert_eq!(
        registry.plugin_health("slow-layout"),
        Some(PluginHealth::Slow)
    );

    // Strike 2
    let bars2 = registry.top_bars();
    assert_eq!(bars2.len(), 1);
    assert_eq!(counter.load(Ordering::SeqCst), 2);
    assert_eq!(
        registry.plugin_health("slow-layout"),
        Some(PluginHealth::Disabled)
    );

    // En Disabled no se ejecuta y devuelve lista vacía
    let bars3 = registry.top_bars();
    assert_eq!(bars3.len(), 0);
    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

/// Plugin con retardo dinámico para probar transiciones de presupuesto y decaimiento.
struct MockDynamicDelayPlugin {
    delay_ms: Arc<AtomicU64>,
    counter: Arc<AtomicUsize>,
}

impl MockDynamicDelayPlugin {
    fn new(delay_ms: Arc<AtomicU64>, counter: Arc<AtomicUsize>) -> Self {
        Self { delay_ms, counter }
    }
}

impl AppearanceHook for MockDynamicDelayPlugin {
    fn opacity(&self) -> Option<f32> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        let ms = self.delay_ms.load(Ordering::SeqCst);
        if ms > 0 {
            std::thread::sleep(Duration::from_millis(ms));
        }
        Some(0.42)
    }
}

impl Plugin for MockDynamicDelayPlugin {
    fn id(&self) -> &'static str {
        "dynamic-delay"
    }

    fn name(&self) -> &'static str {
        "Mock Dynamic Delay"
    }

    fn appearance_hook(&self) -> Option<&dyn AppearanceHook> {
        Some(self)
    }
}

#[test]
fn strike_decays_when_call_is_within_budget() {
    let mut registry = PluginRegistry::new();
    registry.set_hook_budget(Duration::from_millis(5));
    let delay_ms = Arc::new(AtomicU64::new(15));
    let counter = Arc::new(AtomicUsize::new(0));
    let plugin = MockDynamicDelayPlugin::new(Arc::clone(&delay_ms), Arc::clone(&counter));
    registry.register(plugin);

    // Llamada 1 lenta: supera presupuesto -> pasa a Slow
    let _ = registry.effective_opacity();
    assert_eq!(
        registry.plugin_health("dynamic-delay"),
        Some(PluginHealth::Slow),
        "debe pasar a Slow tras la primera llamada lenta"
    );

    // Llamada 2 rápida: dentro del presupuesto -> debe decaer a Ok
    delay_ms.store(0, Ordering::SeqCst);
    let _ = registry.effective_opacity();
    assert_eq!(
        registry.plugin_health("dynamic-delay"),
        Some(PluginHealth::Ok),
        "una llamada dentro de presupuesto debe decaer el strike y volver a Ok"
    );

    // Llamada 3 lenta: al no ser consecutiva, debe volver a Slow, NO a Disabled
    delay_ms.store(15, Ordering::SeqCst);
    let _ = registry.effective_opacity();
    assert_eq!(
        registry.plugin_health("dynamic-delay"),
        Some(PluginHealth::Slow),
        "un strike aislado tras decaimiento no debe deshabilitar el plugin"
    );
}

#[test]
fn two_consecutive_slow_calls_disable_the_plugin_and_stops_invocation() {
    let mut registry = PluginRegistry::new();
    registry.set_hook_budget(Duration::from_millis(5));
    let delay_ms = Arc::new(AtomicU64::new(15));
    let counter = Arc::new(AtomicUsize::new(0));
    let plugin = MockDynamicDelayPlugin::new(Arc::clone(&delay_ms), Arc::clone(&counter));
    registry.register(plugin);

    // Strike 1
    let _ = registry.effective_opacity();
    assert_eq!(
        registry.plugin_health("dynamic-delay"),
        Some(PluginHealth::Slow)
    );
    assert_eq!(counter.load(Ordering::SeqCst), 1);

    // Strike 2 consecutivo
    let _ = registry.effective_opacity();
    assert_eq!(
        registry.plugin_health("dynamic-delay"),
        Some(PluginHealth::Disabled),
        "dos llamadas lentas consecutivas deben deshabilitar el plugin"
    );
    assert_eq!(counter.load(Ordering::SeqCst), 2);

    // Mientras está deshabilitado y antes del enfriamiento, el cuerpo no se invoca
    let op = registry.effective_opacity();
    assert_eq!(op, 1.0, "debe devolver el valor neutro/fallback");
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "el cuerpo del hook no debe ser invocado mientras está deshabilitado"
    );
}

#[test]
fn after_cooldown_probe_runs_and_fast_call_restores_ok() {
    let mut registry = PluginRegistry::new();
    registry.set_hook_budget(Duration::from_millis(5));
    registry.set_probe_cooldown(Duration::from_millis(20));

    let delay_ms = Arc::new(AtomicU64::new(15));
    let counter = Arc::new(AtomicUsize::new(0));
    let plugin = MockDynamicDelayPlugin::new(Arc::clone(&delay_ms), Arc::clone(&counter));
    registry.register(plugin);

    // Deshabilitar con 2 strikes consecutivos
    let _ = registry.effective_opacity();
    let _ = registry.effective_opacity();
    assert_eq!(
        registry.plugin_health("dynamic-delay"),
        Some(PluginHealth::Disabled)
    );
    assert_eq!(counter.load(Ordering::SeqCst), 2);

    // Antes de enfriamiento: no se invoca
    let _ = registry.effective_opacity();
    assert_eq!(counter.load(Ordering::SeqCst), 2);

    // Esperar a que pase el enfriamiento
    std::thread::sleep(Duration::from_millis(30));

    // La llamada de prueba se configura como rápida
    delay_ms.store(0, Ordering::SeqCst);
    let op = registry.effective_opacity();
    assert_eq!(op, 0.42, "la llamada de prueba debe ejecutarse y aportar");
    assert_eq!(counter.load(Ordering::SeqCst), 3, "la prueba debe invocar el hook");
    assert_eq!(
        registry.plugin_health("dynamic-delay"),
        Some(PluginHealth::Ok),
        "la prueba exitosa debe restaurar el estado a Ok"
    );

    // Siguientes llamadas operan con normalidad en Ok
    let _ = registry.effective_opacity();
    assert_eq!(counter.load(Ordering::SeqCst), 4);
    assert_eq!(
        registry.plugin_health("dynamic-delay"),
        Some(PluginHealth::Ok)
    );
}

#[test]
fn after_cooldown_probe_runs_and_slow_call_stays_disabled() {
    let mut registry = PluginRegistry::new();
    registry.set_hook_budget(Duration::from_millis(5));
    registry.set_probe_cooldown(Duration::from_millis(20));

    let delay_ms = Arc::new(AtomicU64::new(15));
    let counter = Arc::new(AtomicUsize::new(0));
    let plugin = MockDynamicDelayPlugin::new(Arc::clone(&delay_ms), Arc::clone(&counter));
    registry.register(plugin);

    // Deshabilitar con 2 strikes consecutivos
    let _ = registry.effective_opacity();
    let _ = registry.effective_opacity();
    assert_eq!(
        registry.plugin_health("dynamic-delay"),
        Some(PluginHealth::Disabled)
    );
    assert_eq!(counter.load(Ordering::SeqCst), 2);

    // Esperar a que pase el enfriamiento
    std::thread::sleep(Duration::from_millis(30));

    // La prueba sigue siendo lenta
    delay_ms.store(15, Ordering::SeqCst);
    let _ = registry.effective_opacity();
    assert_eq!(counter.load(Ordering::SeqCst), 3, "la prueba debe haber corrido una vez");
    assert_eq!(
        registry.plugin_health("dynamic-delay"),
        Some(PluginHealth::Disabled),
        "si la prueba es lenta, el plugin debe permanecer en Disabled"
    );

    // Llamada inmediata posterior (nuevo enfriamiento no cumplido): no debe invocar el hook
    let _ = registry.effective_opacity();
    assert_eq!(
        counter.load(Ordering::SeqCst),
        3,
        "no debe volverse a invocar hasta el siguiente periodo de enfriamiento"
    );
}
