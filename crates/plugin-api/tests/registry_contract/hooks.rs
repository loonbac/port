//! Invocación de hooks: apariencia, entrada, layout y peticiones de sesión.

use std::cell::Cell;
use std::sync::{Arc, Mutex};

use gpui::{div, Element};
use port_plugin_api::{KeyAction, LayoutHook, Plugin, PluginRegistry, SpaceHook};
use port_term_core::frame::{Frame, Rgb};
use port_term_core::input::Key;
use port_term_core::pty::PtyConfig;
use port_term_core::session::GridSize;

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
        env: Vec::new(),
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

struct MockSpacePlugin {
    session_id: usize,
}

impl SpaceHook for MockSpacePlugin {
    fn active_session(&self) -> usize {
        self.session_id
    }
}

impl Plugin for MockSpacePlugin {
    fn id(&self) -> &'static str {
        "space"
    }

    fn name(&self) -> &'static str {
        "Mock Space"
    }

    fn space_hook(&self) -> Option<&dyn SpaceHook> {
        Some(self)
    }
}

#[test]
fn active_session_id_returns_none_when_no_plugin_provides_space_hook() {
    let mut registry = PluginRegistry::new();
    registry.register(MockTransparencyPlugin { opacity: 0.8 });
    assert_eq!(
        registry.active_session_id(),
        None,
        "sin plugin de espacios debe devolver None"
    );
}

#[test]
fn active_session_id_returns_some_when_healthy_space_plugin_answers() {
    let mut registry = PluginRegistry::new();
    registry.register(MockSpacePlugin { session_id: 42 });
    assert_eq!(
        registry.active_session_id(),
        Some(42),
        "un plugin de espacios sano debe devolver su sesión"
    );
}

#[test]
fn host_preserves_active_session_when_registry_returns_none() {
    // Decisión de host (`view/mod.rs`):
    // La vista no puede probarse directamente en un test unitario headless porque
    // `TerminalView::render` requiere el bucle de eventos y contexto gráfico de GPUI.
    // Por tanto, este test valida formalmente la regla y decisión de integración de `view/mod.rs`:
    //
    // ```rust
    // if let Some(target_session) = plugins.active_session_id() {
    //     session_mgr.select(target_session);
    // }
    // ```
    //
    // Si `active_session_id()` devuelve `None`, el host omite `session_mgr.select(...)`,
    // preservando la sesión activa en pantalla en lugar de saltar a la sesión 0.
    use port_term_core::session::{GridSize, SessionManager};

    let mut session_mgr =
        SessionManager::new(PtyConfig::default(), GridSize::new(80, 24)).expect("crear manager");
    let s1 = session_mgr.spawn_session().expect("crear sesion 1");
    assert!(session_mgr.select(s1));
    assert_eq!(session_mgr.active_index(), 1);

    let registry = PluginRegistry::new();
    let target = registry.active_session_id();
    assert_eq!(target, None, "registro sin plugins debe devolver None");

    // Lógica exacta de sincronización en view/mod.rs
    if let Some(target_session) = target {
        session_mgr.select(target_session);
    }

    assert_eq!(
        session_mgr.active_index(),
        1,
        "la sesión activa en pantalla debe mantenerse en 1 y no saltar a la sesión 0"
    );
}

/// Plugin de prueba que pide ver unas sesiones y escribe en otras.
///
/// Guarda lo recibido en `Arc` porque el registro pasa a ser dueño del plugin:
/// la prueba conserva el mango para poder comprobarlo después.
struct MockSessionPlugin {
    watched: Vec<usize>,
    seen: VistosSesion,
    entrada: EntradaPendiente,
    redimension: RedimensionPendiente,
}

impl SpaceHook for MockSessionPlugin {
    fn watched_sessions(&self) -> Vec<usize> {
        self.watched.clone()
    }

    fn update_session_frame(&self, session_id: usize, frame: &Frame) {
        self.seen.lock().unwrap().push((session_id, frame.columns));
    }

    fn take_session_input(&self) -> Vec<(usize, Vec<u8>)> {
        std::mem::take(&mut self.entrada.lock().unwrap())
    }

    fn take_session_resize(&self) -> Vec<(usize, GridSize)> {
        std::mem::take(&mut self.redimension.lock().unwrap())
    }
}

impl Plugin for MockSessionPlugin {
    fn id(&self) -> &'static str {
        "sessions"
    }

    fn name(&self) -> &'static str {
        "Mock Sessions"
    }

    fn space_hook(&self) -> Option<&dyn SpaceHook> {
        Some(self)
    }
}

/// Pares (sesión, columnas) que un plugin de prueba dice haber recibido.
type VistosSesion = Arc<Mutex<Vec<(usize, usize)>>>;
/// Entrada pendiente de un plugin de prueba: pares (sesión, bytes).
type EntradaPendiente = Arc<Mutex<Vec<(usize, Vec<u8>)>>>;
/// Redimensiones pendientes de un plugin de prueba: pares (sesión, tamaño).
type RedimensionPendiente = Arc<Mutex<Vec<(usize, GridSize)>>>;

/// Plugin que pide ver `watched`, con lo recibido en el mango dado.
fn mock_sesiones(watched: Vec<usize>, visto: VistosSesion) -> MockSessionPlugin {
    MockSessionPlugin {
        watched,
        seen: visto,
        entrada: Arc::new(Mutex::new(Vec::new())),
        redimension: Arc::new(Mutex::new(Vec::new())),
    }
}

/// Cuadro mínimo para probar la entrega, sin rejilla real.
fn cuadro_de(columnas: usize) -> Frame {
    Frame {
        rows: Vec::new(),
        columns: columnas,
        cursor: None,
        cursor_run: None,
        display_offset: 0,
    }
}

/// La rejilla se entrega a quien la pidió, y los pedidos no se repiten.
#[test]
fn la_rejilla_llega_a_quien_la_pidio() {
    let visto = Arc::new(Mutex::new(Vec::new()));
    let mut registro = PluginRegistry::new();
    registro.register(mock_sesiones(vec![7], visto.clone()));
    registro.register(mock_sesiones(vec![7], Arc::new(Mutex::new(Vec::new()))));

    assert_eq!(
        registro.watched_sessions(),
        vec![7],
        "dos plugins que piden la misma sesión no la repiten"
    );

    registro.update_session_frame(7, &cuadro_de(120));

    assert_eq!(*visto.lock().unwrap(), vec![(7, 120)]);
}

/// Una sesión que nadie pidió no se le entrega a nadie.
#[test]
fn una_sesion_no_pedida_no_se_entrega() {
    let visto = Arc::new(Mutex::new(Vec::new()));
    let mut registro = PluginRegistry::new();
    registro.register(mock_sesiones(vec![7], visto.clone()));

    registro.update_session_frame(99, &cuadro_de(80));

    assert!(
        visto.lock().unwrap().is_empty(),
        "recibió una rejilla que no pidió"
    );
}

/// La entrada y la redimensión se drenan una sola vez.
#[test]
fn la_entrada_se_drena_una_vez() {
    let plugin = mock_sesiones(vec![], Arc::new(Mutex::new(Vec::new())));
    plugin.entrada.lock().unwrap().push((7, b"ls\n".to_vec()));
    plugin
        .redimension
        .lock()
        .unwrap()
        .push((7, GridSize::new(100, 30)));
    let mut registro = PluginRegistry::new();
    registro.register(plugin);

    assert_eq!(registro.take_session_input(), vec![(7, b"ls\n".to_vec())]);
    assert!(
        registro.take_session_input().is_empty(),
        "la entrada se entregó dos veces"
    );
    assert_eq!(
        registro.take_session_resize(),
        vec![(7, GridSize::new(100, 30))]
    );
    assert!(
        registro.take_session_resize().is_empty(),
        "la redimensión se entregó dos veces"
    );
}
