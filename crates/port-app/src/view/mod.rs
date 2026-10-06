//! La vista: composición del árbol de elementos y sus manejadores.
//!
//! No sabe nada de PTY, ni de `alacritty_terminal`, ni de celdas. Solo consume
//! el cuadro que le da el núcleo y lo dibuja.
//!
//! Este módulo compone la interfaz —el canvas de la terminal, la rueda, el
//! ratón, el menú de plugins y el diálogo de cierre— y delega cada
//! responsabilidad con contrato propio: el pintado en [`paint`], el ratón y la
//! selección en [`mouse`] y el diálogo en [`close_prompt`].

mod close_prompt;
mod mouse;
mod paint;

pub use crate::menu::MenuState;
pub use close_prompt::{accept_close, decline_close, ClosePromptState};

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    canvas, div, App, Bounds, Context, FocusHandle, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Render, ScrollWheelEvent, Window,
};
use port_plugin_api::PluginRegistry;
use port_term_core::frame::Rgb;
use port_term_core::session::{MouseButton as TerminalMouseButton, SessionManager};

use crate::metrics::Metrics;

use close_prompt::render_close_prompt;
use mouse::{
    cell_at_pointer, cell_under_pointer, finish_drag, mouse_modifiers,
    selection_kind_for_click_count, starts_local_selection, terminal_button, wheel_lines,
};
use paint::{color, paint_frame};

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
    /// Botón que capturó el programa, si el gesto es suyo: `Some` significa
    /// «el programa pidió reportes y este gesto le pertenece», y guarda además
    /// cuál está pulsado para reportar el movimiento y la liberación. Vive aquí
    /// por la misma razón que la bandera de arrastre.
    program_drag: Rc<Cell<Option<TerminalMouseButton>>>,
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
            program_drag: Rc::new(Cell::new(None)),
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
        // La sesión es la única fuente de verdad de la política: la vista la
        // lee aquí, con el préstamo que ya tiene, y no guarda copia propia.
        let policy = session_mgr.active_session().mouse_policy();
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

        // Clones del estado del ratón que vive en la vista: los manejadores lo
        // comparten y el clon apunta al mismo estado entre repintados.
        let drag_selecting = Rc::clone(&self.drag_selecting);
        let program_drag = Rc::clone(&self.program_drag);
        let mouse_session = Rc::clone(&self.session_manager);
        let (cell_width, cell_height, padding) =
            (metrics.cell_width, metrics.cell_height, metrics.padding);

        // gpui filtra el botón antes de invocar al manejador, así que cada
        // manejador nace sabiendo cuál atiende y su cuerpo no vuelve a
        // comprobarlo. La decisión es del núcleo: si el programa capturó el
        // gesto, es suyo; PORT solo selecciona lo que el programa no quiso.
        let mouse_down = |button: MouseButton| {
            let session = Rc::clone(&mouse_session);
            let grid_origin = Rc::clone(&grid_origin);
            let drag_selecting = Rc::clone(&drag_selecting);
            let program_drag = Rc::clone(&program_drag);
            move |event: &MouseDownEvent, window: &mut Window, _cx: &mut App| {
                let cell = cell_at_pointer(
                    &grid_origin,
                    event.position,
                    cell_width,
                    cell_height,
                    padding,
                );
                let Some(terminal) = terminal_button(button) else {
                    return;
                };
                let consumed = session.borrow_mut().active_session_mut().mouse_press(
                    terminal,
                    cell,
                    mouse_modifiers(event.modifiers),
                );
                if starts_local_selection(button, consumed) {
                    // El programa no pidió este gesto: se selecciona según la
                    // política de la sesión (clic, doble clic o triple clic).
                    let kind = selection_kind_for_click_count(
                        event.click_count,
                        session.borrow().active_session().mouse_policy(),
                    );
                    session
                        .borrow_mut()
                        .active_session_mut()
                        .start_selection(cell, kind);
                    drag_selecting.set(true);
                    window.refresh();
                } else if consumed {
                    // El gesto es del programa: no se inicia selección local.
                    program_drag.set(Some(terminal));
                }
            }
        };

        // Un soltar cierra el gesto que le corresponde: si el programa lo
        // capturó, le reenvía la liberación; si era un arrastre local de
        // PORT, lo termina y copia. Los botones medio y derecho nunca
        // cierran una selección local.
        let mouse_up = |button: MouseButton| {
            let session = Rc::clone(&mouse_session);
            let grid_origin = Rc::clone(&grid_origin);
            let drag_selecting = Rc::clone(&drag_selecting);
            let program_drag = Rc::clone(&program_drag);
            move |event: &MouseUpEvent, window: &mut Window, cx: &mut App| {
                let Some(terminal) = terminal_button(button) else {
                    return;
                };
                if program_drag.get() == Some(terminal) {
                    let cell = cell_at_pointer(
                        &grid_origin,
                        event.position,
                        cell_width,
                        cell_height,
                        padding,
                    );
                    session.borrow_mut().active_session_mut().mouse_release(
                        terminal,
                        cell,
                        mouse_modifiers(event.modifiers),
                    );
                    program_drag.set(None);
                    return;
                }
                if terminal != TerminalMouseButton::Left {
                    return;
                }
                finish_drag(&session, &drag_selecting, cx);
                window.refresh();
            }
        };

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
                // Ratón: el programa que pide reportes (pi, vim, less) se queda
                // con el gesto —pulsación, arrastre y liberación— y PORT no
                // inicia selección. Sin reportes, o con Shift como salida de
                // emergencia, el gesto es de PORT: un clic ancla la celda, doble
                // clic la palabra y triple clic la línea; el arrastre extiende y
                // al soltar se copia.
                .on_mouse_down(MouseButton::Left, mouse_down(MouseButton::Left))
                .on_mouse_down(MouseButton::Middle, mouse_down(MouseButton::Middle))
                .on_mouse_down(MouseButton::Right, mouse_down(MouseButton::Right))
                .on_mouse_move({
                    let session = Rc::clone(&mouse_session);
                    let grid_origin = Rc::clone(&grid_origin);
                    let drag_selecting = Rc::clone(&drag_selecting);
                    let program_drag = Rc::clone(&program_drag);
                    move |event: &MouseMoveEvent, window: &mut Window, _cx: &mut App| {
                        // Mientras el programa capture el gesto, el movimiento
                        // es suyo. Se reenvía sin repintar: un reporte no cambia
                        // la rejilla, y pi habilita todos los movimientos (1003).
                        if let Some(held) = program_drag.get() {
                            let cell = cell_at_pointer(
                                &grid_origin,
                                event.position,
                                cell_width,
                                cell_height,
                                padding,
                            );
                            session.borrow_mut().active_session_mut().mouse_motion(
                                cell,
                                Some(held),
                                mouse_modifiers(event.modifiers),
                            );
                            return;
                        }
                        // Solo se extiende durante un arrastre: gpui entrega
                        // movimientos siempre que el puntero esté encima, y sin
                        // la bandera un simple pasar por encima movería la
                        // selección.
                        if !drag_selecting.get() {
                            return;
                        }
                        let cell = cell_at_pointer(
                            &grid_origin,
                            event.position,
                            cell_width,
                            cell_height,
                            padding,
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
                .on_mouse_up(MouseButton::Left, mouse_up(MouseButton::Left))
                .on_mouse_up(MouseButton::Middle, mouse_up(MouseButton::Middle))
                .on_mouse_up(MouseButton::Right, mouse_up(MouseButton::Right))
                .on_mouse_up_out(MouseButton::Left, mouse_up(MouseButton::Left))
                .on_mouse_up_out(MouseButton::Middle, mouse_up(MouseButton::Middle))
                .on_mouse_up_out(MouseButton::Right, mouse_up(MouseButton::Right))
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
                                policy,
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
