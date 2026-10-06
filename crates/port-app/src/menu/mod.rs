//! Menú gestor de plugins: estado, máquina de estados y gestión de entrada.
//!
//! Este módulo es la única fuente de verdad para el estado del menú de plugins
//! del núcleo: maneja los estados de navegación, la adición e instalación de
//! plugins por URL de Git y el despacho de pulsaciones. El pintado vive en el
//! submódulo `render`.

#![allow(dead_code)]

mod render;

use self::render::{render_adding, render_browsing, render_installing, render_result};
use gpui::prelude::*;
use gpui::{anchored, deferred, div, point, px, rgb, AnyElement, IntoElement};
use port_plugin_api::PluginInfo;
use port_term_core::input::Key;

/// Atajo que alterna la visibilidad del menú.
///
/// Vive en el núcleo, no en un plugin: el menú es funcionalidad de PORT, no una
/// extensión. Un atajo que dependa de un plugin tampoco quedaría disponible si
/// ese plugin falla al cargar.
pub const MENU_SHORTCUT: &str = "ctrl+shift+l";

/// Título del encabezado del menú.
pub const MENU_TITLE: &str = "Gestor de Plugins (PORT)";

/// Modos o estados en los que puede encontrarse el menú gestor de plugins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuMode {
    /// Vista de lista: permite activar/desactivar plugins y seleccionar acciones fijas.
    Browsing,
    /// Entrada de texto para la URL del repositorio Git a instalar.
    Adding { input: String },
    /// La instalación y compilación está ejecutándose en segundo plano.
    Installing { url: String },
    /// Muestra el resultado de la instalación (éxito o fallo), esperando pulsación para volver.
    Result { message: String, success: bool },
}

/// Acción que solicita el menú al llamador tras procesar una pulsación de tecla.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    /// No se requiere ninguna acción externa.
    None,
    /// El menú debe cerrarse.
    Close,
    /// Alternar el estado activo/inactivo del plugin compilado en el índice dado.
    ToggleCompiled(usize),
    /// Alternar el estado activo/inactivo del plugin externo en el índice dado.
    ToggleExternal(usize),
    /// Desinstalar el plugin externo en el índice dado (relativo a `external_plugins`).
    UninstallExternal(usize),
    /// Iniciar la instalación en segundo plano para la URL indicada.
    StartInstall(String),
    /// El menú cambió internamente y requiere redibujar la ventana.
    Refresh,
}

/// Metadatos de un plugin externo administrado por el menú.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalPluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub enabled: bool,
    pub running: bool,
}

/// Estado global del menú gestor de plugins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuState {
    /// Indica si la ventana modal del menú está abierta.
    pub open: bool,
    /// Índice del elemento enfocado actualmente.
    pub selected_index: usize,
    /// Modo o pantalla activa dentro del menú.
    pub mode: MenuMode,
    /// Plugins externos instalados o cargados en la sesión.
    pub external_plugins: Vec<ExternalPluginInfo>,
    /// Índice (relativo a `external_plugins`) cuya desinstalación está armada.
    ///
    /// El primer `x` solo arma; el segundo confirma. Se guarda el índice para
    /// comprobarlo al ejecutar: si el foco cambió de fila, la confirmación no vale.
    pub pending_uninstall: Option<usize>,
}

impl Default for MenuState {
    fn default() -> Self {
        Self {
            open: false,
            selected_index: 0,
            mode: MenuMode::Browsing,
            external_plugins: Vec::new(),
            pending_uninstall: None,
        }
    }
}

impl MenuState {
    /// Crea un estado inicial con el menú cerrado y en modo Browsing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Abre el menú en modo Browsing, reseteando la selección al primer elemento.
    pub fn open_menu(&mut self) {
        self.open = true;
        self.selected_index = 0;
        self.mode = MenuMode::Browsing;
        self.pending_uninstall = None;
    }

    /// Cierra el menú.
    pub fn close_menu(&mut self) {
        self.open = false;
        self.pending_uninstall = None;
    }

    /// Número total de elementos interactivos en la lista en modo Browsing.
    /// Incluye plugins compilados, plugins externos y la acción fija "+ Install plugin".
    pub fn total_items_count(&self, compiled_count: usize) -> usize {
        compiled_count + self.external_plugins.len() + 1
    }

    /// Desplaza la selección hacia arriba, haciendo clamp en 0.
    pub fn navigate_up(&mut self) {
        self.selected_index = self.selected_index.saturating_sub(1);
    }

    /// Desplaza la selección hacia abajo, haciendo clamp en `total_items - 1`.
    pub fn navigate_down(&mut self, total_items: usize) {
        if total_items > 0 {
            self.selected_index = (self.selected_index + 1).min(total_items - 1);
        }
    }

    /// Ajusta la selección si quedó fuera del rango válido de elementos.
    pub fn clamp_selection(&mut self, total_items: usize) {
        if total_items == 0 {
            self.selected_index = 0;
        } else if self.selected_index >= total_items {
            self.selected_index = total_items - 1;
        }
    }

    /// Cambia el modo a `Adding` con un búfer de texto vacío.
    pub fn start_adding(&mut self) {
        self.mode = MenuMode::Adding {
            input: String::new(),
        };
    }

    /// Añade una porción de texto al búfer en modo `Adding`.
    pub fn input_append(&mut self, text: &str) {
        if let MenuMode::Adding { ref mut input } = self.mode {
            input.push_str(text);
        }
    }

    /// Pega texto desde el portapapeles en el campo de URL.
    ///
    /// Va aparte del pegado por teclado porque pegar NO llega como tecla:
    /// llega desde el portapapeles del sistema, y hay que ir a buscarlo.
    pub fn input_paste(&mut self, text: &str) {
        // Una URL copiada suele traer un salto de linea final; pegarla tal
        // cual meteria un caracter invisible que rompe la instalacion.
        let cleaned = text.trim_end_matches(['\r', '\n']);
        if cleaned.is_empty() {
            return;
        }
        if let MenuMode::Adding { ref mut input } = self.mode {
            input.push_str(cleaned);
        }
    }

    /// Añade un carácter al búfer en modo `Adding`.
    pub fn input_append_char(&mut self, c: char) {
        if let MenuMode::Adding { ref mut input } = self.mode {
            input.push(c);
        }
    }

    /// Elimina el último carácter del búfer en modo `Adding`.
    pub fn input_backspace(&mut self) {
        if let MenuMode::Adding { ref mut input } = self.mode {
            input.pop();
        }
    }

    /// Confirma la adición del plugin. Si la URL no está vacía, pasa a `Installing`
    /// y devuelve la URL a instalar.
    pub fn submit_add(&mut self) -> Option<String> {
        if let MenuMode::Adding { ref input } = self.mode {
            let trimmed = input.trim();
            if !trimmed.is_empty() {
                let url = trimmed.to_string();
                self.mode = MenuMode::Installing { url: url.clone() };
                return Some(url);
            }
        }
        None
    }

    /// Concluye la instalación y transiciona al estado `Result`.
    pub fn finish_install(&mut self, success: bool, message: impl Into<String>) {
        self.mode = MenuMode::Result {
            success,
            message: message.into(),
        };
    }

    /// Descarta la pantalla de resultado y retorna al listado en `Browsing`.
    pub fn dismiss_result(&mut self) {
        self.mode = MenuMode::Browsing;
    }

    /// Gestiona la pulsación de Escape según el estado activo del menú.
    pub fn handle_escape(&mut self) {
        // Desarmar una desinstalación consume el primer Escape en la lista:
        // cancelarla no debe cerrar el menú entero.
        if matches!(self.mode, MenuMode::Browsing) && self.pending_uninstall.take().is_some() {
            return;
        }
        match &self.mode {
            MenuMode::Browsing => {
                self.open = false;
            }
            MenuMode::Adding { .. } => {
                self.mode = MenuMode::Browsing;
            }
            MenuMode::Installing { .. } => {
                // Al presionar Escape durante la instalación, cerramos la ventana
                // pero la tarea continuará corriendo en segundo plano.
                self.open = false;
            }
            MenuMode::Result { .. } => {
                self.mode = MenuMode::Browsing;
            }
        }
    }

    /// Registra o actualiza la información de un plugin externo.
    pub fn add_or_update_external(&mut self, info: ExternalPluginInfo) {
        if let Some(existing) = self.external_plugins.iter_mut().find(|p| p.id == info.id) {
            *existing = info;
        } else {
            self.external_plugins.push(info);
        }
    }

    /// Procesa una pulsación de tecla pura y devuelve la acción a ejecutar.
    pub fn handle_key(
        &mut self,
        key: &Key,
        total_items: usize,
        compiled_count: usize,
    ) -> MenuAction {
        match &self.mode {
            MenuMode::Browsing => {
                match key.key.as_str() {
                    "Escape" => {
                        // Un Escape armado solo cancela la confirmación; cerrar
                        // el menú requiere un segundo Escape.
                        let armado = self.pending_uninstall.is_some();
                        self.handle_escape();
                        if armado {
                            MenuAction::Refresh
                        } else {
                            MenuAction::Close
                        }
                    }
                    "Up" | "k" if !key.ctrl && !key.alt => {
                        // Moverse de fila desarma: la confirmación pertenece a la
                        // fila armada, no a la posición donde quedó el foco.
                        self.pending_uninstall = None;
                        self.navigate_up();
                        MenuAction::Refresh
                    }
                    "Down" | "j" if !key.ctrl && !key.alt => {
                        self.pending_uninstall = None;
                        self.navigate_down(total_items);
                        MenuAction::Refresh
                    }
                    "Enter" | "Return" | " " => {
                        // Cualquier acción distinta de `x` desarma la confirmación.
                        self.pending_uninstall = None;
                        let idx = self.selected_index;
                        if idx < compiled_count {
                            MenuAction::ToggleCompiled(idx)
                        } else if idx < compiled_count + self.external_plugins.len() {
                            let ext_idx = idx - compiled_count;
                            MenuAction::ToggleExternal(ext_idx)
                        } else {
                            // Elemento fijo: acción de añadir plugin
                            self.start_adding();
                            MenuAction::Refresh
                        }
                    }
                    "x" | "X" if !key.ctrl && !key.alt => {
                        let idx = self.selected_index;
                        if idx >= compiled_count
                            && idx < compiled_count + self.external_plugins.len()
                        {
                            let ext_idx = idx - compiled_count;
                            // El primer `x` arma; el segundo, sobre la misma fila,
                            // confirma. Se comprueba el índice: armar una fila y
                            // confirmar otra no vale.
                            if self.pending_uninstall == Some(ext_idx) {
                                self.pending_uninstall = None;
                                MenuAction::UninstallExternal(ext_idx)
                            } else {
                                self.pending_uninstall = Some(ext_idx);
                                MenuAction::Refresh
                            }
                        } else {
                            // Los plugins compilados (ej. herdr) y la acción de
                            // instalar no son desinstalables.
                            MenuAction::None
                        }
                    }
                    "a" | "A" | "+" if !key.ctrl && !key.alt => {
                        self.pending_uninstall = None;
                        self.start_adding();
                        MenuAction::Refresh
                    }
                    _ => MenuAction::None,
                }
            }
            MenuMode::Adding { .. } => match key.key.as_str() {
                "Escape" => {
                    self.handle_escape();
                    MenuAction::Refresh
                }
                "Backspace" => {
                    self.input_backspace();
                    MenuAction::Refresh
                }
                "Enter" | "Return" => {
                    if let Some(url) = self.submit_add() {
                        MenuAction::StartInstall(url)
                    } else {
                        MenuAction::Refresh
                    }
                }
                _ => {
                    if !key.ctrl && !key.alt {
                        if let Some(text) = &key.text {
                            self.input_append(text);
                            MenuAction::Refresh
                        } else if key.key.chars().count() == 1 {
                            self.input_append(&key.key);
                            MenuAction::Refresh
                        } else {
                            MenuAction::None
                        }
                    } else {
                        MenuAction::None
                    }
                }
            },
            MenuMode::Installing { .. } => {
                // Durante la instalación, la UI no se bloquea y la lista sigue siendo navegable
                match key.key.as_str() {
                    "Escape" => {
                        self.handle_escape();
                        MenuAction::Close
                    }
                    "Up" | "k" if !key.ctrl && !key.alt => {
                        self.navigate_up();
                        MenuAction::Refresh
                    }
                    "Down" | "j" if !key.ctrl && !key.alt => {
                        self.navigate_down(total_items);
                        MenuAction::Refresh
                    }
                    _ => MenuAction::None,
                }
            }
            MenuMode::Result { .. } => {
                // Cualquier tecla en la pantalla de resultado retorna al listado
                self.dismiss_result();
                MenuAction::Refresh
            }
        }
    }
}

/// Renderiza la vista modal del menú según su estado y plugins disponibles.
pub fn render_menu(
    state: &MenuState,
    compiled_plugins: &[PluginInfo],
    title: &str,
    window_width: f32,
    window_height: f32,
) -> impl IntoElement {
    let modal_w = 560.0f32.min(window_width - 40.0);
    let left = ((window_width - modal_w) * 0.5).max(10.0);
    let top = (window_height * 0.10).max(20.0);

    let modal = div()
        .w(px(modal_w))
        .p(px(16.0))
        .rounded(px(8.0))
        .bg(rgb(0x0d1117))
        .border_1()
        .border_color(rgb(0x30363d))
        .flex()
        .flex_col()
        .gap(px(12.0));

    let content: AnyElement = match &state.mode {
        MenuMode::Browsing => render_browsing(state, compiled_plugins, title).into_any_element(),
        MenuMode::Adding { input } => render_adding(input).into_any_element(),
        MenuMode::Installing { url } => {
            render_installing(state, compiled_plugins, url).into_any_element()
        }
        MenuMode::Result { message, success } => {
            render_result(*success, message).into_any_element()
        }
    };

    deferred(
        anchored()
            .position(point(px(left), px(top)))
            .child(modal.child(content)),
    )
    .priority(100)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un compilado de relleno (índice 0, no desinstalable), dos externos
    /// (índices 1 y 2) y la acción fija de instalar (índice 3).
    fn estado_con_dos_externos() -> MenuState {
        MenuState {
            open: true,
            external_plugins: vec![
                ExternalPluginInfo {
                    id: "uno".to_string(),
                    name: "Uno".to_string(),
                    version: "0.1.0".to_string(),
                    enabled: true,
                    running: true,
                },
                ExternalPluginInfo {
                    id: "dos".to_string(),
                    name: "Dos".to_string(),
                    version: "0.2.0".to_string(),
                    enabled: false,
                    running: false,
                },
            ],
            ..Default::default()
        }
    }

    /// Número de plugins compilados en las pruebas.
    const COMPILADOS: usize = 1;

    #[test]
    fn x_arma_la_confirmacion_sin_desinstalar() {
        let mut state = estado_con_dos_externos();
        let total = state.total_items_count(COMPILADOS);
        state.selected_index = COMPILADOS; // primer externo

        let action = state.handle_key(&Key::new("x"), total, COMPILADOS);
        assert_eq!(action, MenuAction::Refresh, "el primer x solo arma");
        assert_eq!(state.pending_uninstall, Some(0));
        assert_eq!(
            state.external_plugins.len(),
            2,
            "armar no debe quitar la fila de la lista"
        );
    }

    #[test]
    fn el_segundo_x_confirma_la_desinstalacion() {
        let mut state = estado_con_dos_externos();
        let total = state.total_items_count(COMPILADOS);
        state.selected_index = COMPILADOS + 1; // segundo externo

        assert_eq!(
            state.handle_key(&Key::new("x"), total, COMPILADOS),
            MenuAction::Refresh
        );
        // La segunda pulsación en la misma fila ya confirma; `X` mayúscula
        // también vale.
        assert_eq!(
            state.handle_key(&Key::new("X"), total, COMPILADOS),
            MenuAction::UninstallExternal(1)
        );
        assert_eq!(state.pending_uninstall, None, "tras confirmar se desarma");
    }

    #[test]
    fn x_en_otra_fila_no_confirma_la_anterior() {
        let mut state = estado_con_dos_externos();
        let total = state.total_items_count(COMPILADOS);
        state.selected_index = COMPILADOS; // arma el externo 0
        assert_eq!(
            state.handle_key(&Key::new("x"), total, COMPILADOS),
            MenuAction::Refresh
        );

        // Sin cancelar por flecha: el foco se mueve directamente a otra fila.
        state.selected_index = COMPILADOS + 1;
        let action = state.handle_key(&Key::new("x"), total, COMPILADOS);
        assert_ne!(
            action,
            MenuAction::UninstallExternal(0),
            "no se puede confirmar una fila distinta de la armada"
        );
        assert_eq!(state.pending_uninstall, Some(1), "arma la fila nueva");
    }

    #[test]
    fn las_flechas_cancelan_la_confirmacion() {
        let mut state = estado_con_dos_externos();
        let total = state.total_items_count(COMPILADOS);
        state.selected_index = COMPILADOS;
        assert_eq!(
            state.handle_key(&Key::new("x"), total, COMPILADOS),
            MenuAction::Refresh
        );

        assert_eq!(
            state.handle_key(&Key::new("Down"), total, COMPILADOS),
            MenuAction::Refresh
        );
        assert_eq!(state.pending_uninstall, None, "moverse desarma");
    }

    #[test]
    fn escape_cancela_la_confirmacion_sin_cerrar_el_menu() {
        let mut state = estado_con_dos_externos();
        let total = state.total_items_count(COMPILADOS);
        state.selected_index = COMPILADOS;
        assert_eq!(
            state.handle_key(&Key::new("x"), total, COMPILADOS),
            MenuAction::Refresh
        );

        let action = state.handle_key(&Key::new("Escape"), total, COMPILADOS);
        assert_ne!(action, MenuAction::Close, "cancela sin cerrar el menú");
        assert!(state.open, "el menú sigue abierto");
        assert_eq!(state.pending_uninstall, None);
    }

    #[test]
    fn x_en_una_fila_compilada_no_hace_nada() {
        let mut state = estado_con_dos_externos();
        let total = state.total_items_count(COMPILADOS);
        state.selected_index = 0; // compilado (ej. herdr)

        let action = state.handle_key(&Key::new("x"), total, COMPILADOS);
        assert_eq!(action, MenuAction::None);
        assert_eq!(state.pending_uninstall, None, "no se arma sobre compilados");
    }

    #[test]
    fn x_sobre_la_accion_de_instalar_no_hace_nada() {
        let mut state = estado_con_dos_externos();
        let total = state.total_items_count(COMPILADOS);
        state.selected_index = total - 1; // "+ Install plugin"

        let action = state.handle_key(&Key::new("x"), total, COMPILADOS);
        assert_eq!(action, MenuAction::None);
        assert_eq!(state.pending_uninstall, None);
    }
}
