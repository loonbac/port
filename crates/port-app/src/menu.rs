//! Menú gestor de plugins: estado, renderizado y gestión de entrada.
//!
//! Este módulo es la única fuente de verdad para el menú de plugins del núcleo:
//! maneja los estados de navegación, la adición e instalación de plugins por URL de Git,
//! y el renderizado modal en la interfaz gráfica de GPUI.

#![allow(dead_code)]

use gpui::prelude::*;
use gpui::{anchored, deferred, div, point, px, rgb, AnyElement, FontWeight, IntoElement};
use port_plugin_api::{PluginHealth, PluginInfo};
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
}

impl Default for MenuState {
    fn default() -> Self {
        Self {
            open: false,
            selected_index: 0,
            mode: MenuMode::Browsing,
            external_plugins: Vec::new(),
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
    }

    /// Cierra el menú.
    pub fn close_menu(&mut self) {
        self.open = false;
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
                    "Enter" | "Return" | " " => {
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
                    "a" | "A" | "+" if !key.ctrl && !key.alt => {
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

/// Renderiza la pantalla de listado de plugins (`Browsing`).
fn render_browsing(
    state: &MenuState,
    compiled_plugins: &[PluginInfo],
    title: &str,
) -> impl IntoElement {
    let mut list = div().flex().flex_col().gap(px(6.0));
    let compiled_count = compiled_plugins.len();
    let external_count = state.external_plugins.len();

    // Si la lista está vacía, recomienda instalar plugins
    if compiled_count == 0 && external_count == 0 {
        list = list.child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .p(px(14.0))
                .rounded(px(6.0))
                .bg(rgb(0x161b22))
                .border_1()
                .border_color(rgb(0x30363d))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(4.0))
                                .bg(rgb(0x21262d))
                                .text_size(px(11.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(0xd29922))
                                .child("EMPTY"),
                        )
                        .child(
                            div()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(0xf0f6fc))
                                .child("No plugins installed"),
                        ),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(0x8b949e))
                        .child("PORT runs as a pure terminal core with 0 bundled plugins. Extend tabs, spaces, zoom, and appearance by installing plugins:"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .mt(px(4.0))
                        .text_size(px(11.0))
                        .text_color(rgb(0x58a6ff))
                        .child("• Official suite: https://github.com/loonbac/port-plugins")
                        .child("• Run in terminal: port plugin add <git-url-or-path>"),
                ),
        );
    }

    // 1. Plugins compilados del núcleo
    for (i, p) in compiled_plugins.iter().enumerate() {
        let is_selected = i == state.selected_index;
        list = list.child(render_plugin_row(
            p.name.clone(),
            p.id.clone(),
            p.version.clone(),
            p.enabled,
            p.health,
            false,
            is_selected,
        ));
    }

    // 2. Plugins externos cargados o instalados
    for (i, p) in state.external_plugins.iter().enumerate() {
        let global_idx = compiled_count + i;
        let is_selected = global_idx == state.selected_index;
        list = list.child(render_plugin_row(
            p.name.clone(),
            p.id.clone(),
            p.version.clone(),
            p.enabled && p.running,
            PluginHealth::Ok,
            true,
            is_selected,
        ));
    }

    // 3. Acción fija al final: Añadir / Instalar plugin
    let action_idx = compiled_count + external_count;
    let is_action_selected = action_idx == state.selected_index;
    let action_bg = if is_action_selected {
        rgb(0x21262d)
    } else {
        rgb(0x161b22)
    };
    let action_border = if is_action_selected {
        rgb(0x58a6ff)
    } else {
        rgb(0x30363d)
    };

    let action_row = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(px(10.0))
        .py(px(8.0))
        .rounded(px(6.0))
        .bg(action_bg)
        .border_1()
        .border_color(action_border)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(
                    div()
                        .px(px(6.0))
                        .py(px(2.0))
                        .rounded(px(4.0))
                        .bg(rgb(0x1f293d))
                        .border_1()
                        .border_color(rgb(0x388bfd))
                        .text_size(px(11.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(0x58a6ff))
                        .child("+"),
                )
                .child(
                    div()
                        .text_size(px(13.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(0xf0f6fc))
                        .child("Install plugin from git repository..."),
                ),
        )
        .child(
            div()
                .text_size(px(11.0))
                .text_color(rgb(0x8b949e))
                .child("Enter / Space"),
        );

    list = list.child(action_row);

    div()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(render_header("PLUGINS", title, "PORT Core"))
        .child(list)
}

/// Renderiza una fila individual para un plugin (compilado o externo).
fn render_plugin_row(
    name: String,
    id: String,
    version: String,
    enabled: bool,
    health: PluginHealth,
    is_external: bool,
    is_selected: bool,
) -> impl IntoElement {
    let (status_text, dot_color, badge_bg, badge_border) = match health {
        PluginHealth::Disabled => (
            "DISABLED (SLOW)",
            rgb(0xf85149),
            rgb(0x270e0e),
            rgb(0x862323),
        ),
        PluginHealth::Slow if enabled => ("SLOW", rgb(0xd29922), rgb(0x2b1d09), rgb(0x6e4a06)),
        _ => {
            if enabled {
                ("ACTIVE", rgb(0x3fb950), rgb(0x0e2717), rgb(0x238636))
            } else {
                ("OFF", rgb(0x8b949e), rgb(0x161b22), rgb(0x30363d))
            }
        }
    };

    let row_bg = if is_selected {
        rgb(0x21262d)
    } else {
        rgb(0x161b22)
    };
    let row_border = if is_selected {
        rgb(0x58a6ff)
    } else {
        rgb(0x30363d)
    };

    let mut left_col = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .px(px(8.0))
                .py(px(2.0))
                .rounded(px(4.0))
                .bg(badge_bg)
                .border_1()
                .border_color(badge_border)
                .child(div().w(px(6.0)).h(px(6.0)).rounded(px(3.0)).bg(dot_color))
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(dot_color)
                        .child(status_text),
                ),
        )
        .child(
            div()
                .text_size(px(13.0))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(0xf0f6fc))
                .child(name),
        )
        .child(
            div()
                .text_size(px(11.0))
                .text_color(rgb(0x8b949e))
                .child(format!("({id})")),
        );

    if is_external {
        left_col = left_col.child(
            div()
                .px(px(5.0))
                .py(px(1.0))
                .rounded(px(3.0))
                .bg(rgb(0x2d1f3d))
                .border_1()
                .border_color(rgb(0x8957e5))
                .text_size(px(10.0))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(0xd2a8ff))
                .child("EXTERNAL"),
        );
    }

    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(px(10.0))
        .py(px(8.0))
        .rounded(px(6.0))
        .bg(row_bg)
        .border_1()
        .border_color(row_border)
        .child(left_col)
        .child(
            div()
                .px(px(6.0))
                .py(px(2.0))
                .rounded(px(4.0))
                .bg(rgb(0x21262d))
                .text_size(px(11.0))
                .text_color(rgb(0x8b949e))
                .child(format!("v{version}")),
        )
}

/// Renderiza la pantalla de captura de URL (`Adding`).
fn render_adding(input: &str) -> impl IntoElement {
    let (display_text, is_placeholder) = if input.is_empty() {
        ("https://github.com/user/port-plugin-name.git", true)
    } else {
        (input, false)
    };

    let text_color = if is_placeholder {
        rgb(0x6e7681)
    } else {
        rgb(0xf0f6fc)
    };

    div()
        .flex()
        .flex_col()
        .gap(px(14.0))
        .child(render_header(
            "INSTALL",
            "Install Plugin from Git",
            "PORT Core",
        ))
        .child(div().text_size(px(12.0)).text_color(rgb(0x8b949e)).child(
            "Enter the Git repository URL. PORT will clone and compile it in the background:",
        ))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .px(px(12.0))
                .py(px(10.0))
                .rounded(px(6.0))
                .bg(rgb(0x161b22))
                .border_1()
                .border_color(rgb(0x388bfd))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(13.0))
                        .text_color(text_color)
                        .child(display_text.to_string()),
                )
                .child(div().w(px(2.0)).h(px(16.0)).bg(rgb(0x58a6ff))),
        )
}

/// Renderiza la pantalla mientras la compilación está en marcha (`Installing`).
fn render_installing(
    state: &MenuState,
    compiled_plugins: &[PluginInfo],
    url: &str,
) -> impl IntoElement {
    let banner = div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .p(px(12.0))
        .rounded(px(6.0))
        .bg(rgb(0x161b22))
        .border_1()
        .border_color(rgb(0x388bfd))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .px(px(6.0))
                        .py(px(2.0))
                        .rounded(px(4.0))
                        .bg(rgb(0x1f293d))
                        .text_size(px(11.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(0x58a6ff))
                        .child("BUILDING"),
                )
                .child(
                    div()
                        .text_size(px(13.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(0xf0f6fc))
                        .child("Compiling release binary in background..."),
                ),
        )
        .child(
            div()
                .text_size(px(11.0))
                .text_color(rgb(0x8b949e))
                .child(format!("Repository: {url}")),
        )
        .child(
            div()
                .text_size(px(11.0))
                .text_color(rgb(0x3fb950))
                .child("UI remains interactive. You may continue navigating below."),
        );

    let mut list = div().flex().flex_col().gap(px(6.0));
    let compiled_count = compiled_plugins.len();

    for (i, p) in compiled_plugins.iter().enumerate() {
        let is_selected = i == state.selected_index;
        list = list.child(render_plugin_row(
            p.name.clone(),
            p.id.clone(),
            p.version.clone(),
            p.enabled,
            p.health,
            false,
            is_selected,
        ));
    }

    for (i, p) in state.external_plugins.iter().enumerate() {
        let global_idx = compiled_count + i;
        let is_selected = global_idx == state.selected_index;
        list = list.child(render_plugin_row(
            p.name.clone(),
            p.id.clone(),
            p.version.clone(),
            p.enabled && p.running,
            PluginHealth::Ok,
            true,
            is_selected,
        ));
    }

    div()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(render_header(
            "INSTALLING",
            "Installing Plugin",
            "PORT Core",
        ))
        .child(banner)
        .child(list)
}

/// Renderiza la pantalla de resultado (`Result`).
fn render_result(success: bool, message: &str) -> impl IntoElement {
    let (tag, tag_bg, tag_border, tag_text_color, border_color) = if success {
        (
            "SUCCESS",
            rgb(0x0e2717),
            rgb(0x238636),
            rgb(0x3fb950),
            rgb(0x238636),
        )
    } else {
        (
            "FAILED",
            rgb(0x270e0e),
            rgb(0x862323),
            rgb(0xf85149),
            rgb(0xda3633),
        )
    };

    div()
        .flex()
        .flex_col()
        .gap(px(14.0))
        .child(render_header("RESULT", "Installation Result", "PORT Core"))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .p(px(12.0))
                .rounded(px(6.0))
                .bg(rgb(0x161b22))
                .border_1()
                .border_color(border_color)
                .child(
                    div().flex().flex_row().items_center().gap(px(8.0)).child(
                        div()
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .bg(tag_bg)
                            .border_1()
                            .border_color(tag_border)
                            .text_size(px(11.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(tag_text_color)
                            .child(tag),
                    ),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(0xf0f6fc))
                        .child(message.to_string()),
                ),
        )
}

/// Barra de encabezado del modal.
fn render_header(badge: &'static str, title: &str, subtitle: &'static str) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .px(px(6.0))
                        .py(px(2.0))
                        .rounded(px(4.0))
                        .bg(rgb(0x1f293d))
                        .border_1()
                        .border_color(rgb(0x388bfd))
                        .text_size(px(10.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(0x58a6ff))
                        .child(badge),
                )
                .child(
                    div()
                        .text_size(px(14.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(0xf0f6fc))
                        .child(title.to_string()),
                ),
        )
        .child(
            div()
                .px(px(6.0))
                .py(px(2.0))
                .rounded(px(4.0))
                .bg(rgb(0x161b22))
                .border_1()
                .border_color(rgb(0x30363d))
                .text_size(px(11.0))
                .text_color(rgb(0x8b949e))
                .child(subtitle),
        )
}
