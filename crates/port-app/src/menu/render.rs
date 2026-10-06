//! Pintado del menú gestor de plugins.
//!
//! Responsabilidad única: traducir el estado de `MenuState` a elementos de
//! GPUI. No decide estados ni procesa pulsaciones; solo dibuja.

use gpui::prelude::*;
use gpui::{div, px, rgb, FontWeight, IntoElement};
use port_plugin_api::{PluginHealth, PluginInfo};

use super::MenuState;

/// Renderiza la pantalla de listado de plugins (`Browsing`).
pub(crate) fn render_browsing(
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
            false,
        ));
    }

    // 2. Plugins externos cargados o instalados
    for (i, p) in state.external_plugins.iter().enumerate() {
        let global_idx = compiled_count + i;
        let is_selected = global_idx == state.selected_index;
        let confirm_uninstall = state.pending_uninstall == Some(i);
        list = list.child(render_plugin_row(
            p.name.clone(),
            p.id.clone(),
            p.version.clone(),
            p.enabled && p.running,
            PluginHealth::Ok,
            true,
            is_selected,
            confirm_uninstall,
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
///
/// `confirm_uninstall` marca la fila cuya desinstalación está armada: el texto
/// de la derecha pasa a pedir confirmación en lugar de mostrar la versión.
#[allow(clippy::too_many_arguments)]
fn render_plugin_row(
    name: String,
    id: String,
    version: String,
    enabled: bool,
    health: PluginHealth,
    is_external: bool,
    is_selected: bool,
    confirm_uninstall: bool,
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

    let (row_bg, row_border) = if confirm_uninstall {
        // La confirmación se pinta como alarma, no como foco.
        (rgb(0x2d1315), rgb(0xf85149))
    } else if is_selected {
        (rgb(0x21262d), rgb(0x58a6ff))
    } else {
        (rgb(0x161b22), rgb(0x30363d))
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
        .child(if confirm_uninstall {
            div()
                .px(px(6.0))
                .py(px(2.0))
                .rounded(px(4.0))
                .bg(rgb(0x270e0e))
                .text_size(px(11.0))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(0xf85149))
                .child("Uninstall? X=yes  Esc=no")
                .into_any_element()
        } else {
            div()
                .px(px(6.0))
                .py(px(2.0))
                .rounded(px(4.0))
                .bg(rgb(0x21262d))
                .text_size(px(11.0))
                .text_color(rgb(0x8b949e))
                .child(format!("v{version}"))
                .into_any_element()
        })
}

/// Renderiza la pantalla de captura de URL (`Adding`).
pub(crate) fn render_adding(input: &str) -> impl IntoElement {
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
pub(crate) fn render_installing(
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
            false,
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
            false,
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
pub(crate) fn render_result(success: bool, message: &str) -> impl IntoElement {
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
