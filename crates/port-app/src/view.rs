//! La vista: pinta un [`Frame`].
//!
//! No sabe nada de PTY, ni de `alacritty_terminal`, ni de celdas. Solo consume
//! el cuadro que le da el núcleo y lo dibuja.
//!
//! El texto se pinta directamente sobre un canvas con `shape_line`, como hace la
//! terminal de Zed. No se usa un `div` por carácter: el motor de layout (Taffy)
//! no garantiza que una caja de texto con altura fija conserve el glifo.
//!
//! Los elementos de bloque y los caracteres braille se dibujan como figuras
//! geométricas directas y suaves con `paint_quad`, evitando costosas búsquedas
//! en fuentes de respaldo en aplicaciones como `btop`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    anchored, canvas, deferred, div, fill, point, px, rgb, size, App, Bounds, ClipboardItem,
    Context, FocusHandle, Font, FontFallbacks, FontStyle, FontWeight, MouseButton, Pixels, Render,
    ScrollDelta, ScrollWheelEvent, TextRun, Window,
};
use port_plugin_api::PluginRegistry;
use port_term_core::frame::{Frame, Rgb, Run, Style};
use port_term_core::session::{CellPos, SelectionKind, SessionManager};

use crate::metrics::Metrics;

pub use crate::menu::MenuState;

pub struct TerminalView {
    session_manager: Rc<RefCell<SessionManager>>,
    metrics: Metrics,
    background: Rgb,
    focus_handle: FocusHandle,
    plugins: Rc<RefCell<PluginRegistry>>,
    menu_state: Rc<RefCell<MenuState>>,
    close_prompt: Rc<RefCell<ClosePromptState>>,
    /// Estado del arrastre de selección. Vive en la vista, no en el render: un
    /// `window.refresh()` reconstruye los manejadores y la bandera debe seguir
    /// encendida entre un movimiento del ratón y el siguiente.
    drag_selecting: Rc<Cell<bool>>,
}

impl TerminalView {
    pub fn new(
        session_manager: Rc<RefCell<SessionManager>>,
        metrics: Metrics,
        focus_handle: FocusHandle,
        plugins: Rc<RefCell<PluginRegistry>>,
        menu_state: Rc<RefCell<MenuState>>,
        close_prompt: Rc<RefCell<ClosePromptState>>,
    ) -> Self {
        let background = session_manager.borrow().default_style().bg;
        Self {
            session_manager,
            metrics,
            background,
            focus_handle,
            plugins,
            menu_state,
            close_prompt,
            drag_selecting: Rc::new(Cell::new(false)),
        }
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let (width, height): (f32, f32) = (viewport.width.into(), viewport.height.into());

        let plugins = self.plugins.borrow();
        let effective_bg = plugins.effective_background(self.background);
        let opacity = plugins.effective_opacity();
        let root_bg = color(effective_bg).opacity(opacity);

        let font_family = plugins.effective_font_family("FiraCode Nerd Font Mono");
        let font_size = plugins.effective_font_size(self.metrics.font_size);
        let font_fallbacks = plugins.effective_font_fallbacks();

        let top_bars = plugins.top_bars();
        let left_sidebars = plugins.left_sidebars();
        let bottom_bars = plugins.bottom_bars();

        let left_inset = plugins.left_sidebar_width();
        let top_inset = plugins.top_bar_height();
        let bottom_inset = plugins.bottom_bar_height();

        let plugin_list = plugins.list_plugins();
        let menu_title = crate::menu::MENU_TITLE.to_string();

        let menu_state = self.menu_state.borrow();
        let menu_open = menu_state.open;
        drop(menu_state);

        let usable_w = (width - left_inset).max(100.0);
        let usable_h = (height - top_inset - bottom_inset).max(100.0);
        let metrics = Metrics::new(font_size, self.metrics.padding);
        let grid = metrics.grid_for(usable_w, usable_h);

        let mut session_mgr = self.session_manager.borrow_mut();

        // 1. Si algún plugin solicitó crear una nueva sesión (Ctrl+Shift+T o Ctrl+Alt+T)
        if plugins.take_new_session_request() {
            if let Ok(new_id) = session_mgr.spawn_session() {
                plugins.on_session_created(new_id);
            }
        }

        // 2. Si algún plugin solicitó una sesión que ejecute un programa propio
        if let Some(config) = plugins.take_spawn_session_request() {
            if let Ok(new_id) = session_mgr.spawn_session_with(config) {
                plugins.on_session_created(new_id);
            }
        }

        // 3. Si algún plugin solicitó cerrar una sesión específica
        if let Some(id) = plugins.take_close_session_request() {
            let _ = session_mgr.close(id);
        }

        // 4. Sincroniza la sesión activa del gestor con la sesión solicitada por los plugins
        let target_session = plugins.active_session_id();
        session_mgr.select(target_session);

        // 5. Notifica a los plugins el directorio de trabajo actual de cada sesión viva
        for (&id, session) in session_mgr.sessions().iter() {
            if let Some(cwd) = session.current_working_directory() {
                let folder = session
                    .current_folder_name()
                    .unwrap_or_else(|| "~".to_string());
                plugins.update_session_cwd(id, &cwd, &folder);
            }
        }

        // 6. Notifica a los plugins qué programa corre en primer plano en cada sesión
        for (&id, session) in session_mgr.sessions().iter() {
            let app = session.foreground_app();
            plugins.update_session_app(id, app.as_ref());
        }

        session_mgr.pump();

        // Retira las sesiones cuyo shell terminó (por ejemplo, tras `exit`) y
        // avisa a los plugins para quelimpien sus pestañas y espacios.
        let exited = session_mgr.reap_exited();
        for id in &exited {
            plugins.on_session_closed(*id);
        }

        // Si no queda ninguna sesión viva, la terminal ya no tiene nada que
        // mostrar: se cierra la ventana en lugar de quedarse pegada.
        if session_mgr.is_empty() {
            drop(session_mgr);
            drop(plugins);
            window.remove_window();
            return div().size_full();
        }

        if session_mgr.size() != grid {
            let _ = session_mgr.resize(grid);
        }

        let frame = session_mgr.frame();
        drop(session_mgr);
        drop(plugins);

        if !self.focus_handle.is_focused(window) {
            self.focus_handle.focus(window);
        }

        let focus = self.focus_handle.clone();

        let mut root = div()
            .flex()
            .flex_row()
            .size_full()
            .bg(root_bg)
            .track_focus(&self.focus_handle)
            // Clic en cualquier parte devuelve el foco a la rejilla.
            .on_mouse_down(MouseButton::Left, move |_event, window, _cx| {
                focus.focus(window);
            });

        // Esquina del área de rejilla en coordenadas de ventana, refrescada por
        // el canvas en cada cuadro. La necesita la rueda para saber qué celda
        // hay bajo el puntero cuando el programa pide reportes de ratón.
        let grid_origin: Rc<Cell<(f32, f32)>> = Rc::new(Cell::new((0.0, 0.0)));

        // Clon del estado de arrastre que vive en la vista: los manejadores lo
        // comparten y el clon apunta a la misma bandera entre repintados.
        let drag_selecting = Rc::clone(&self.drag_selecting);

        // 1. Barras laterales izquierdas (full height)
        for sidebar in left_sidebars {
            root = root.child(sidebar);
        }

        // 2. Columna principal (tabs arriba + canvas + barra inferior)
        let mut main_col = div().flex().flex_col().flex_1().h_full().overflow_hidden();
        for bar in top_bars {
            main_col = main_col.child(bar);
        }

        main_col = main_col.child(
            div()
                .flex_1()
                .w_full()
                // La rueda mueve el scrollback local del núcleo, o se reenvía al
                // programa si este pidió reportes de ratón (pi, vim, less). Esa
                // decisión vive en el núcleo: aquí solo se traduce el evento a
                // líneas y a la celda bajo el puntero. Se engancha al contenedor
                // de la terminal, no a la raíz, para que una rueda sobre una
                // barra lateral de un plugin no desplace la terminal.
                .on_scroll_wheel({
                    let wheel_session = Rc::clone(&self.session_manager);
                    let grid_origin = Rc::clone(&grid_origin);
                    let cell_width = metrics.cell_width;
                    let cell_height = metrics.cell_height;
                    let padding = metrics.padding;
                    move |event: &ScrollWheelEvent, window: &mut Window, _cx: &mut App| {
                        let lines = wheel_lines(event.delta, cell_height);
                        if lines == 0 {
                            return;
                        }
                        let (origin_x, origin_y) = grid_origin.get();
                        let cell = cell_under_pointer(
                            event.position.x.into(),
                            event.position.y.into(),
                            origin_x + padding,
                            origin_y + padding,
                            cell_width,
                            cell_height,
                        );
                        let consumed = wheel_session.borrow_mut().active_session_mut().wheel(
                            lines,
                            cell,
                            event.modifiers.shift,
                        );
                        // La rueda es entrada de usuario, no un flujo continuo:
                        // repintar cuando el evento se consume es barato.
                        if consumed {
                            window.refresh();
                        }
                    }
                })
                // Selección con el ratón: un clic ancla la celda, doble clic la
                // palabra y triple clic la línea. El arrastre extiende la
                // selección y al soltar se copia al portapapeles.
                .on_mouse_down(MouseButton::Left, {
                    let session = Rc::clone(&self.session_manager);
                    let grid_origin = Rc::clone(&grid_origin);
                    let drag_selecting = Rc::clone(&drag_selecting);
                    let cell_width = metrics.cell_width;
                    let cell_height = metrics.cell_height;
                    let padding = metrics.padding;
                    move |event, window, _cx| {
                        let (origin_x, origin_y) = grid_origin.get();
                        let cell = cell_under_pointer(
                            event.position.x.into(),
                            event.position.y.into(),
                            origin_x + padding,
                            origin_y + padding,
                            cell_width,
                            cell_height,
                        );
                        let kind = selection_kind_for_click_count(event.click_count);
                        session
                            .borrow_mut()
                            .active_session_mut()
                            .start_selection(cell, kind);
                        drag_selecting.set(true);
                        window.refresh();
                    }
                })
                .on_mouse_move({
                    let session = Rc::clone(&self.session_manager);
                    let grid_origin = Rc::clone(&grid_origin);
                    let drag_selecting = Rc::clone(&drag_selecting);
                    let cell_width = metrics.cell_width;
                    let cell_height = metrics.cell_height;
                    let padding = metrics.padding;
                    move |event, window, _cx| {
                        // Solo se extiende durante un arrastre: gpui entrega
                        // movimientos siempre que el puntero esté encima, y sin
                        // la bandera un simple pasar por encima movería la
                        // selección.
                        if !drag_selecting.get() {
                            return;
                        }
                        let (origin_x, origin_y) = grid_origin.get();
                        let cell = cell_under_pointer(
                            event.position.x.into(),
                            event.position.y.into(),
                            origin_x + padding,
                            origin_y + padding,
                            cell_width,
                            cell_height,
                        );
                        session
                            .borrow_mut()
                            .active_session_mut()
                            .extend_selection(cell);
                        window.refresh();
                    }
                })
                // Soltar dentro del contenedor y soltar fuera de él comparten
                // el cierre: si el arrastre termina en el borde, `on_mouse_up`
                // no dispara y la bandera quedaría encendida.
                .on_mouse_up(MouseButton::Left, {
                    let session = Rc::clone(&self.session_manager);
                    let drag_selecting = Rc::clone(&drag_selecting);
                    move |_event, window, cx| {
                        finish_drag(&session, &drag_selecting, cx);
                        window.refresh();
                    }
                })
                .on_mouse_up_out(MouseButton::Left, {
                    let session = Rc::clone(&self.session_manager);
                    let drag_selecting = Rc::clone(&drag_selecting);
                    move |_event, window, cx| {
                        finish_drag(&session, &drag_selecting, cx);
                        window.refresh();
                    }
                })
                .child(
                    canvas(
                        {
                            // El canvas deja lista su esquina en cada cuadro: es
                            // el único origen exacto del área de rejilla, sin
                            // repetir aquí la cuenta de barras y márgenes.
                            let grid_origin = Rc::clone(&grid_origin);
                            move |bounds: Bounds<Pixels>, _window: &mut Window, _cx: &mut App| {
                                grid_origin.set((bounds.origin.x.into(), bounds.origin.y.into()));
                            }
                        },
                        move |bounds, (), window, cx| {
                            paint_frame(
                                bounds,
                                &frame,
                                &metrics,
                                effective_bg,
                                &font_family,
                                font_size,
                                &font_fallbacks,
                                window,
                                cx,
                            );
                        },
                    )
                    .size_full(),
                ),
        );

        for bar in bottom_bars {
            main_col = main_col.child(bar);
        }

        root = root.child(main_col);

        // Si el menú de plugins está abierto, se proyecta como overlay flotante gestionado por el núcleo
        if menu_open {
            root = root.child(crate::menu::render_menu(
                &self.menu_state.borrow(),
                &plugin_list,
                &menu_title,
                width,
                height,
            ));
        }

        // Diálogo de confirmación de cierre, con el mismo tratamiento visual:
        // overlay flotante por encima de todo.
        let prompt = self.close_prompt.borrow();
        if prompt.open {
            root = root.child(render_close_prompt(
                &prompt.programs,
                Rc::clone(&self.close_prompt),
                width,
                height,
            ));
        }
        drop(prompt);

        root
    }
}

/// Cuántas líneas del scrollback mueve un evento de rueda.
///
/// `ScrollDelta::Lines` ya llega en líneas: en Linux cada muesca de rueda se
/// entrega como `Lines(3.0)` porque gpui multiplica el salto discreto por
/// `SCROLL_LINES = 3.0` en sus backends X11/Wayland. `ScrollDelta::Pixels` es
/// scroll suave y se convierte con la altura de línea de la rejilla. El
/// componente horizontal se ignora: la terminal solo se desplaza en vertical.
fn wheel_lines(delta: ScrollDelta, line_height: f32) -> i32 {
    let y = match delta {
        ScrollDelta::Lines(p) => p.y,
        ScrollDelta::Pixels(p) => {
            if line_height <= 0.0 {
                return 0;
            }
            f32::from(p.y) / line_height
        }
    };

    let rounded = y.round();
    if rounded != 0.0 {
        return rounded as i32;
    }
    // Una entrada no nula nunca se convierte en cero: el scroll suave por
    // píxeles debe mover al menos una línea, aunque el delta sea diminuto.
    if y > 0.0 {
        1
    } else if y < 0.0 {
        -1
    } else {
        0
    }
}

/// Traduce el recuento de clics al tipo de selección del núcleo.
///
/// Un clic selecciona celdas sueltas, el doble clic la palabra bajo el puntero
/// y el triple clic la línea entera. Cualquier otro recuento (un cuarto clic o
/// un valor inesperado) se comporta como un clic simple, para que el gesto
/// nunca quede sin definir.
fn selection_kind_for_click_count(click_count: usize) -> SelectionKind {
    match click_count {
        2 => SelectionKind::Word,
        3 => SelectionKind::Line,
        _ => SelectionKind::Simple,
    }
}

/// Celda (1-based) bajo el puntero, para el reporte SGR del ratón.
///
/// El puntero llega en coordenadas de ventana y `origin` es la esquina del
/// área de rejilla ya con su margen interior descontado. Fuera de la rejilla se
/// recorta a la primera celda, como exige el reporte.
fn cell_under_pointer(
    pointer_x: f32,
    pointer_y: f32,
    origin_x: f32,
    origin_y: f32,
    cell_width: f32,
    cell_height: f32,
) -> CellPos {
    let column = ((pointer_x - origin_x) / cell_width).floor() as i32 + 1;
    let row = ((pointer_y - origin_y) / cell_height).floor() as i32 + 1;
    CellPos::new(column, row)
}

/// Cierra un arrastre de selección: apaga la bandera y copia lo seleccionado.
///
/// Se ejecuta tanto al soltar dentro del contenedor como fuera de él. Solo
/// escribe en el portapapeles cuando hay texto real, para no pisar lo que el
/// usuario tuviera copiado con una selección vacía.
fn finish_drag(
    session: &Rc<RefCell<SessionManager>>,
    drag_selecting: &Rc<Cell<bool>>,
    cx: &mut App,
) {
    drag_selecting.set(false);
    let text = session.borrow().active_session().selection_text();
    if let Some(text) = text {
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }
}

/// Estado del diálogo de confirmación de cierre.
#[derive(Debug, Default, Clone)]
pub struct ClosePromptState {
    pub open: bool,
    pub programs: Vec<String>,
    /// Botón enfocado: 0 = "No, seguir", 1 = "Si, cerrar".
    /// Por defecto se enfoca la opción segura, para que un Enter descuidado
    /// no termine matando la terminal.
    pub selected: usize,
}

/// Cierra la ventana de verdad, saltándose la confirmación.
pub fn accept_close(state: &Rc<RefCell<ClosePromptState>>, window: &mut Window) {
    {
        let mut s = state.borrow_mut();
        s.open = false;
        s.selected = 0;
    }
    window.refresh();
    window.remove_window();
}

/// Descarta la confirmación y vuelve a la terminal.
pub fn decline_close(state: &Rc<RefCell<ClosePromptState>>, window: &mut Window) {
    {
        let mut s = state.borrow_mut();
        s.open = false;
        s.selected = 0;
    }
    window.refresh();
}

/// Modal de confirmación de cierre: avisa de qué programas se perderán.
fn render_close_prompt(
    programs: &[String],
    state: Rc<RefCell<ClosePromptState>>,
    window_w: f32,
    window_h: f32,
) -> impl IntoElement {
    // Centrado real en la ventana, no coordenadas fijas.
    let modal_w = 440.0f32.min(window_w - 40.0).max(240.0);
    let modal_h = 280.0f32;
    let left = ((window_w - modal_w) * 0.5).max(0.0);
    let top = ((window_h - modal_h) * 0.5).max(0.0);
    let selected = state.borrow().selected;
    let mut rows = div().flex().flex_col().gap(px(4.0));
    for bin in programs {
        rows = rows.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(8.0))
                .py(px(4.0))
                .rounded(px(4.0))
                .bg(rgb(0x1c1c2b))
                .child(
                    div()
                        .w(px(6.0))
                        .h(px(6.0))
                        .rounded(px(3.0))
                        .bg(rgb(0xf0883e)),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(0xf0f6fc))
                        .child(bin.clone()),
                ),
        );
    }

    // Ambos botones comparten estilo base: la diferencia entre el enfocado y
    // el resto es solo el foco, nunca "el que está en rojo".
    let mk_button = |label: &'static str,
                     focused: bool,
                     accent: gpui::Hsla,
                     state: Rc<RefCell<ClosePromptState>>| {
        let bg: gpui::Hsla = if focused {
            accent.opacity(0.22)
        } else {
            rgb(0x161b22).into()
        };
        let border: gpui::Hsla = if focused {
            accent
        } else {
            rgb(0x30363d).into()
        };
        let text: gpui::Hsla = if focused {
            rgb(0xffffff).into()
        } else {
            rgb(0x8b949e).into()
        };

        let mut inner = div().flex().flex_row().items_center().gap(px(8.0));
        // Punto de foco: aparece solo en la opción elegida.
        inner = if focused {
            inner.child(div().w(px(7.0)).h(px(7.0)).rounded(px(4.0)).bg(accent))
        } else {
            // Marcador de posición para que el texto no salte al aparecer.
            inner.child(div().w(px(7.0)).h(px(7.0)))
        };

        inner = inner.child(
            div()
                .text_size(px(13.0))
                .font_weight(FontWeight::BOLD)
                .text_color(text)
                .child(label),
        );

        div()
            .flex()
            .items_center()
            .justify_center()
            .px(px(18.0))
            .py(px(8.0))
            .rounded(px(6.0))
            .bg(bg)
            .border(if focused { px(2.0) } else { px(1.0) })
            .border_color(border)
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_event, window, _cx| {
                if focused {
                    accept_close(&state, window);
                } else {
                    decline_close(&state, window);
                }
            })
            .child(inner)
    };

    let cancel_btn = mk_button(
        "No, seguir",
        selected == 0,
        rgb(0x58a6ff).into(),
        Rc::clone(&state),
    );
    let confirm_btn = mk_button(
        "Si, cerrar",
        selected == 1,
        rgb(0xf85149).into(),
        Rc::clone(&state),
    );

    deferred(
        anchored().position(point(px(left), px(top))).child(
            div()
                .w(px(440.0))
                .p(px(16.0))
                .bg(rgb(0x0d1117))
                .border_1()
                .border_color(rgb(0xf0883e))
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(
                    div()
                        .text_size(px(14.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(0xf0883e))
                        .child("Hay procesos en ejecucion"),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(0x8b949e))
                        .child("Si cierras la terminal estos procesos se perderan:"),
                )
                .child(rows)
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(0x6e7681))
                        .child("[←/→] Elegir   [Enter] Confirmar   [Esc] Cancelar"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_center()
                        .gap(px(12.0))
                        .child(cancel_btn)
                        .child(confirm_btn),
                ),
        ),
    )
    .priority(200)
}

/// Fondo translúcido del resaltado de selección.
///
/// Reutiliza el azul acento que ya usa el diálogo de cierre de esta misma
/// vista en lugar de inventar una paleta nueva. Es un azul medio, no un gris:
/// un tono neutro se pierde igual sobre fondos oscuros que claros. Se pinta
/// por debajo de los glifos, así que el texto conserva su color y se lee sobre
/// el resaltado en cualquier tema sin recolorearse.
const SELECTION_BG: Rgb = Rgb::new(0x58, 0xa6, 0xff);
/// Opacidad del resaltado: suficiente para distinguir el rango, tenue para no
/// tapar el texto que va encima.
const SELECTION_BG_OPACITY: f32 = 0.35;

/// Pinta el cuadro completo: quads de fondo, glifos geométricos y texto.
#[allow(clippy::too_many_arguments)]
fn paint_frame(
    bounds: Bounds<Pixels>,
    frame: &Frame,
    metrics: &Metrics,
    default_bg: Rgb,
    font_family: &str,
    font_size_f32: f32,
    font_fallbacks: &[String],
    window: &mut Window,
    cx: &mut App,
) {
    let padding = px(metrics.padding);
    let cell_width = px(metrics.cell_width);
    let cell_height = px(metrics.cell_height);
    let font_size = px(font_size_f32);

    let fallbacks = Some(FontFallbacks::from_fonts(font_fallbacks.to_vec()));

    let regular = Font {
        family: font_family.to_string().into(),
        features: Default::default(),
        fallbacks: fallbacks.clone(),
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    };
    let bold = Font {
        weight: FontWeight::BOLD,
        ..regular.clone()
    };
    let italic = Font {
        style: FontStyle::Italic,
        ..regular.clone()
    };
    let bold_italic = Font {
        weight: FontWeight::BOLD,
        style: FontStyle::Italic,
        ..regular.clone()
    };

    let cw_f32 = metrics.cell_width;
    let ch_f32 = metrics.cell_height;

    let origin_x = bounds.origin.x + padding;
    let origin_y = bounds.origin.y + padding;

    for (row_index, row) in frame.rows.iter().enumerate() {
        let top = origin_y + cell_height * row_index as f32;
        let mut left = origin_x;

        // El resaltado va antes que los runs de la fila: el fondo y los glifos
        // se pintan encima y el texto sigue legible sobre el azul translúcido.
        if let Some(span) = row.selection {
            let selected_x = origin_x + cell_width * span.start as f32;
            let selected_width = cell_width * (span.end.saturating_sub(span.start) + 1) as f32;
            window.paint_quad(fill(
                Bounds::new(point(selected_x, top), size(selected_width, cell_height)),
                color(SELECTION_BG).opacity(SELECTION_BG_OPACITY),
            ));
        }

        for (run_index, run) in row.runs.iter().enumerate() {
            let is_cursor = frame
                .cursor_run
                .is_some_and(|cursor| cursor.row == row_index && cursor.run_index == run_index);
            let style = style_of(run, is_cursor);
            let width = cell_width * run.columns as f32;

            // Si el color coincide con el fondo por defecto y no es el cursor,
            // dejamos que se vea el fondo del contenedor principal (con su opacidad).
            if style.bg != default_bg || is_cursor {
                window.paint_quad(fill(
                    Bounds::new(point(left, top), size(width, cell_height)),
                    color(style.bg),
                ));
            }

            if !run.text.trim().is_empty() {
                let fg_color = color(style.fg);
                let font = match (style.bold, style.italic) {
                    (true, true) => &bold_italic,
                    (true, false) => &bold,
                    (false, true) => &italic,
                    (false, false) => &regular,
                };

                let mut text_buf = String::with_capacity(run.text.len());
                let mut text_start_col = 0usize;

                for (col_offset, ch) in run.text.chars().enumerate() {
                    let char_x: f32 = (left + cell_width * col_offset as f32).into();
                    let char_y: f32 = top.into();

                    // Intentamos pintar como elemento de bloque o braille geométrico suave
                    if paint_block_element(ch, char_x, char_y, cw_f32, ch_f32, fg_color, window)
                        || paint_braille(ch, char_x, char_y, cw_f32, ch_f32, fg_color, window)
                    {
                        // Si había texto regular acumulado previo, lo pintamos antes
                        if !text_buf.is_empty() {
                            let text_run = TextRun {
                                len: text_buf.len(),
                                font: font.clone(),
                                color: fg_color,
                                background_color: None,
                                underline: style.underline.then(|| gpui::UnderlineStyle {
                                    color: Some(fg_color),
                                    thickness: px(1.0),
                                    wavy: false,
                                }),
                                strikethrough: None,
                            };
                            let text_x = left + cell_width * text_start_col as f32;
                            window
                                .text_system()
                                .shape_line(
                                    text_buf.clone().into(),
                                    font_size,
                                    &[text_run],
                                    Some(cell_width),
                                )
                                .paint(point(text_x, top), cell_height, window, cx)
                                .ok();
                            text_buf.clear();
                        }
                    } else {
                        if text_buf.is_empty() {
                            text_start_col = col_offset;
                        }
                        text_buf.push(ch);
                    }
                }

                // Pintamos cualquier texto restante al final del run
                if !text_buf.is_empty() {
                    let text_run = TextRun {
                        len: text_buf.len(),
                        font: font.clone(),
                        color: fg_color,
                        background_color: None,
                        underline: style.underline.then(|| gpui::UnderlineStyle {
                            color: Some(fg_color),
                            thickness: px(1.0),
                            wavy: false,
                        }),
                        strikethrough: None,
                    };
                    let text_x = left + cell_width * text_start_col as f32;
                    window
                        .text_system()
                        .shape_line(text_buf.into(), font_size, &[text_run], Some(cell_width))
                        .paint(point(text_x, top), cell_height, window, cx)
                        .ok();
                }
            }

            left += width;
        }
    }
}

/// Dibuja caracteres braille (U+2800..=U+28FF) como puntos circulares suaves y anti-aliased.
fn paint_braille(
    ch: char,
    x: f32,
    y: f32,
    cell_w: f32,
    cell_h: f32,
    color: gpui::Hsla,
    window: &mut Window,
) -> bool {
    let cp = ch as u32;
    if !(0x2800..=0x28FF).contains(&cp) {
        return false;
    }
    let bits = (cp - 0x2800) as u8;
    if bits == 0 {
        return true;
    }

    // Matriz de 2 columnas x 4 filas con puntos redondos en vez de cuadrados
    let dot_size = (cell_w * 0.32).max(2.0).round();
    let col_step = cell_w * 0.45;
    let row_step = cell_h * 0.22;
    let offset_x = (cell_w - (col_step + dot_size)) * 0.5;
    let offset_y = (cell_h - (row_step * 3.0 + dot_size)) * 0.5;
    let radius = px(dot_size * 0.5);

    let dot_map: [(u8, f32, f32); 8] = [
        (0x01, 0.0, 0.0),
        (0x02, 0.0, 1.0),
        (0x04, 0.0, 2.0),
        (0x08, 1.0, 0.0),
        (0x10, 1.0, 1.0),
        (0x20, 1.0, 2.0),
        (0x40, 0.0, 3.0),
        (0x80, 1.0, 3.0),
    ];

    for (bit, col, row) in dot_map {
        if bits & bit != 0 {
            let dot_x = x + offset_x + col * col_step;
            let dot_y = y + offset_y + row * row_step;
            window.paint_quad(gpui::quad(
                Bounds::new(
                    point(px(dot_x), px(dot_y)),
                    size(px(dot_size), px(dot_size)),
                ),
                radius,
                color,
                gpui::Edges::default(),
                gpui::transparent_black(),
                gpui::BorderStyle::default(),
            ));
        }
    }
    true
}

/// Dibuja elementos de bloque geométricos (U+2580..=U+259F y U+25A0).
fn paint_block_element(
    ch: char,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: gpui::Hsla,
    window: &mut Window,
) -> bool {
    let cp = ch as u32;
    let (bx, by, bw, bh) = match cp {
        // ▀ mitad superior
        0x2580 => (0.0, 0.0, 1.0, 0.5),
        // ▁▂▃▄▅▆▇█ bloques inferiores de 1..=8 octavos
        0x2581..=0x2588 => {
            let eighths = (cp - 0x2580) as f32 / 8.0;
            (0.0, 1.0 - eighths, 1.0, eighths)
        }
        // ▉▊▋▌▍▎▏ bloques izquierdos de 7..=1 octavos
        0x2589..=0x258F => {
            let eighths = (0x2590 - cp) as f32 / 8.0;
            (0.0, 0.0, eighths, 1.0)
        }
        // ▐ mitad derecha
        0x2590 => (0.5, 0.0, 0.5, 1.0),
        // ▔ octavo superior
        0x2594 => (0.0, 0.0, 1.0, 0.125),
        // ▕ octavo derecho
        0x2595 => (0.875, 0.0, 0.125, 1.0),
        // ■ cuadrado negro con bordes sutilmente redondeados
        0x25A0 => {
            let sq_w = (w * 0.65).round();
            let sq_h = (h * 0.55).round();
            let sq_x = x + (w - sq_w) * 0.5;
            let sq_y = y + (h - sq_h) * 0.5;
            window.paint_quad(gpui::quad(
                Bounds::new(point(px(sq_x), px(sq_y)), size(px(sq_w), px(sq_h))),
                px(2.0),
                color,
                gpui::Edges::default(),
                gpui::transparent_black(),
                gpui::BorderStyle::default(),
            ));
            return true;
        }
        // ░ sombra suave
        0x2591 => {
            window.paint_quad(fill(
                Bounds::new(point(px(x), px(y)), size(px(w), px(h))),
                color.opacity(0.25),
            ));
            return true;
        }
        // ▒ sombra media
        0x2592 => {
            window.paint_quad(fill(
                Bounds::new(point(px(x), px(y)), size(px(w), px(h))),
                color.opacity(0.50),
            ));
            return true;
        }
        // ▓ sombra densa
        0x2593 => {
            window.paint_quad(fill(
                Bounds::new(point(px(x), px(y)), size(px(w), px(h))),
                color.opacity(0.75),
            ));
            return true;
        }
        _ => return false,
    };

    window.paint_quad(fill(
        Bounds::new(
            point(px(x + bx * w), px(y + by * h)),
            size(px(bw * w), px(bh * h)),
        ),
        color,
    ));
    true
}

/// Estilo con el que se pinta un run, invirtiendo el del cursor.
///
/// El cursor es un bloque: se invierte el par de colores de esa celda.
fn style_of(run: &Run, is_cursor: bool) -> Style {
    if is_cursor {
        Style {
            fg: run.style.bg,
            bg: run.style.fg,
            ..run.style
        }
    } else {
        run.style
    }
}

fn color(rgb_value: Rgb) -> gpui::Hsla {
    let packed = (rgb_value.r as u32) << 16 | (rgb_value.g as u32) << 8 | (rgb_value.b as u32);
    rgb(packed).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Altura de línea de una fuente de 14 px, la misma que produce `Metrics`.
    const LINE_HEIGHT: f32 = 19.0;

    #[test]
    fn pixels_se_convierten_a_lineas_con_la_altura_de_linea() {
        // 57 px sobre una línea de 19 px son 3 líneas exactas.
        let delta = ScrollDelta::Pixels(point(px(0.0), px(3.0 * LINE_HEIGHT)));
        assert_eq!(wheel_lines(delta, LINE_HEIGHT), 3);

        // El redondeo se lleva al entero más cercano, no trunca.
        let delta = ScrollDelta::Pixels(point(px(0.0), px(1.6 * LINE_HEIGHT)));
        assert_eq!(wheel_lines(delta, LINE_HEIGHT), 2);
    }

    #[test]
    fn lines_ya_son_lineas_y_no_se_reescalan() {
        // Una muesca de rueda en Linux llega como `Lines(3.0)`: tres líneas.
        let delta = ScrollDelta::Lines(point(0.0, 3.0));
        assert_eq!(wheel_lines(delta, LINE_HEIGHT), 3);

        let delta = ScrollDelta::Lines(point(0.0, 6.0));
        assert_eq!(wheel_lines(delta, LINE_HEIGHT), 6);
    }

    #[test]
    fn la_entrada_nula_no_mueve_nada() {
        assert_eq!(
            wheel_lines(ScrollDelta::Lines(point(0.0, 0.0)), LINE_HEIGHT),
            0
        );
        assert_eq!(
            wheel_lines(ScrollDelta::Pixels(point(px(0.0), px(0.0))), LINE_HEIGHT),
            0
        );
    }

    #[test]
    fn rueda_hacia_arriba_es_positivo_y_hacia_abajo_negativo() {
        // En gpui un delta `y` positivo es rueda hacia arriba, que en el
        // núcleo aleja la vista del final mostrando líneas más viejas.
        let arriba = ScrollDelta::Pixels(point(px(0.0), px(LINE_HEIGHT)));
        let abajo = ScrollDelta::Pixels(point(px(0.0), px(-LINE_HEIGHT)));
        assert_eq!(wheel_lines(arriba, LINE_HEIGHT), 1);
        assert_eq!(wheel_lines(abajo, LINE_HEIGHT), -1);
        assert_eq!(
            wheel_lines(ScrollDelta::Lines(point(0.0, 3.0)), LINE_HEIGHT),
            3
        );
        assert_eq!(
            wheel_lines(ScrollDelta::Lines(point(0.0, -3.0)), LINE_HEIGHT),
            -3
        );
    }

    #[test]
    fn un_pixel_suelto_mueve_al_menos_una_linea() {
        // Scroll suave diminuto: redondearía a cero, así que se fuerza ±1.
        let delta = ScrollDelta::Pixels(point(px(0.0), px(2.0)));
        assert_eq!(wheel_lines(delta, LINE_HEIGHT), 1);
        let delta = ScrollDelta::Pixels(point(px(0.0), px(-2.0)));
        assert_eq!(wheel_lines(delta, LINE_HEIGHT), -1);
    }

    #[test]
    fn el_componente_horizontal_se_ignora() {
        let delta = ScrollDelta::Pixels(point(px(40.0), px(0.0)));
        assert_eq!(wheel_lines(delta, LINE_HEIGHT), 0);
        let delta = ScrollDelta::Lines(point(9.0, 0.0));
        assert_eq!(wheel_lines(delta, LINE_HEIGHT), 0);
    }

    /// Un clic selecciona una celda, el doble la palabra y el triple la línea.
    #[test]
    fn cada_recuento_de_clics_pide_su_tipo_de_seleccion() {
        assert_eq!(selection_kind_for_click_count(1), SelectionKind::Simple);
        assert_eq!(selection_kind_for_click_count(2), SelectionKind::Word);
        assert_eq!(selection_kind_for_click_count(3), SelectionKind::Line);
    }

    /// Un cuarto clic (o cualquier recuento raro) vuelve al clic simple: el
    /// gesto nunca deja la selección sin definir.
    #[test]
    fn mas_de_tres_clics_se_comportan_como_uno() {
        assert_eq!(selection_kind_for_click_count(4), SelectionKind::Simple);
        assert_eq!(selection_kind_for_click_count(0), SelectionKind::Simple);
    }

    /// La rejilla empieza en el origen del área de pintado: la esquina superior
    /// izquierda es la celda 1,1 y cada celda avanza un ancho y una altura.
    #[test]
    fn el_puntero_se_traduce_a_celdas_1_based() {
        let (origen_x, origen_y) = (20.0, 10.0);
        let (ancho, alto) = (8.0, 19.0);

        assert_eq!(
            cell_under_pointer(origen_x, origen_y, origen_x, origen_y, ancho, alto),
            CellPos::new(1, 1),
            "la esquina superior izquierda es la celda 1,1"
        );

        // Cuatro celdas y dos filas más allá, con 3 px dentro de la celda.
        let pointer_x = origen_x + ancho * 4.0 + 3.0;
        let pointer_y = origen_y + alto * 2.0 + 1.0;
        assert_eq!(
            cell_under_pointer(pointer_x, pointer_y, origen_x, origen_y, ancho, alto),
            CellPos::new(5, 3),
            "la celda se cuenta desde uno, no desde cero"
        );
    }

    /// Un puntero sobre el margen o por encima de la rejilla no puede producir
    /// una celda cero: el reporte SGR no la admite.
    #[test]
    fn el_puntero_fuera_de_la_rejilla_se_recorta_a_la_primera_celda() {
        assert_eq!(
            cell_under_pointer(0.0, 0.0, 20.0, 10.0, 8.0, 19.0),
            CellPos::new(1, 1)
        );
        assert_eq!(
            cell_under_pointer(24.0, 5.0, 20.0, 10.0, 8.0, 19.0),
            CellPos::new(1, 1),
            "por encima del origen también se recorta a la primera fila"
        );
    }
}
