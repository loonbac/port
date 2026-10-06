//! Ratón y selección: traduce los eventos de gpui al protocolo de la terminal.
//!
//! Traduce el puntero a celdas, los botones y modificadores al reporte SGR, y
//! decide cuándo un gesto pertenece al programa o inicia una selección local.
//! Los helpers puros viven aquí para poder verificarlos sin ventana.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{App, ClipboardItem, Modifiers, MouseButton, Pixels, Point, ScrollDelta};
use port_term_core::session::{
    CellPos, MouseButton as TerminalMouseButton, MouseModifiers, MousePolicy, SelectionKind,
    SessionManager,
};

/// Cuántas líneas del scrollback mueve un evento de rueda.
///
/// `ScrollDelta::Lines` ya llega en líneas: en Linux cada muesca de rueda se
/// entrega como `Lines(3.0)` porque gpui multiplica el salto discreto por
/// `SCROLL_LINES = 3.0` en sus backends X11/Wayland. `ScrollDelta::Pixels` es
/// scroll suave y se convierte con la altura de línea de la rejilla. El
/// componente horizontal se ignora: la terminal solo se desplaza en vertical.
pub(crate) fn wheel_lines(delta: ScrollDelta, line_height: f32) -> i32 {
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

/// Traduce el recuento de clics al tipo de selección del núcleo, según la
/// política de la sesión.
///
/// Un clic selecciona celdas sueltas. El doble clic expande a la palabra bajo
/// el puntero solo con `word_on_double_click` activo, y el triple a la línea
/// entera solo con `line_on_triple_click` activo; cuando el switch apagado
/// corresponde a ese recuento, el gesto degrada a un clic simple. Cualquier
/// otro recuento (un cuarto clic o un valor inesperado) también es simple,
/// para que el gesto nunca quede sin definir.
pub(crate) fn selection_kind_for_click_count(
    click_count: usize,
    policy: MousePolicy,
) -> SelectionKind {
    match click_count {
        2 if policy.word_on_double_click => SelectionKind::Word,
        3 if policy.line_on_triple_click => SelectionKind::Line,
        _ => SelectionKind::Simple,
    }
}

/// `true` si al cerrar un arrastre corresponde copiar la selección.
///
/// La copia automática la gobierna `copy_on_select` y solo tiene sentido con
/// texto real: una selección vacía no debe pisar lo que el usuario tuviera
/// copiado. Con la copia apagada la selección queda disponible para
/// `Ctrl+Shift+C`, que la lee directamente de la sesión.
pub(crate) fn should_copy_on_select(policy: MousePolicy, has_selection: bool) -> bool {
    policy.copy_on_select && has_selection
}

/// Celda (1-based) bajo el puntero, para el reporte SGR del ratón.
///
/// El puntero llega en coordenadas de ventana y `origin` es la esquina del
/// área de rejilla ya con su margen interior descontado. Fuera de la rejilla se
/// recorta a la primera celda, como exige el reporte.
pub(crate) fn cell_under_pointer(
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

/// Celda (1-based) bajo una posición de ventana.
///
/// Es la traducción que comparten todos los eventos de ratón: la esquina de la
/// rejilla y el margen interior ya se descuentan aquí, y el recorte a la
/// primera celda lo hace `CellPos`.
pub(crate) fn cell_at_pointer(
    grid_origin: &Cell<(f32, f32)>,
    position: Point<Pixels>,
    cell_width: f32,
    cell_height: f32,
    padding: f32,
) -> CellPos {
    let (origin_x, origin_y) = grid_origin.get();
    cell_under_pointer(
        position.x.into(),
        position.y.into(),
        origin_x + padding,
        origin_y + padding,
        cell_width,
        cell_height,
    )
}

/// Traduce el botón de gpui al botón del protocolo de terminal.
///
/// El protocolo SGR no tiene los botones de navegación (atrás/adelante), así
/// que no se traducen. Los manejadores solo se registran para los tres botones
/// reales, de modo que ese camino nunca llega aquí.
pub(crate) fn terminal_button(button: MouseButton) -> Option<TerminalMouseButton> {
    match button {
        MouseButton::Left => Some(TerminalMouseButton::Left),
        MouseButton::Middle => Some(TerminalMouseButton::Middle),
        MouseButton::Right => Some(TerminalMouseButton::Right),
        MouseButton::Navigate(_) => None,
    }
}

/// Traduce los modificadores de gpui a los que viajan en el reporte SGR.
///
/// La tecla plataforma (Super) no existe en el protocolo: solo Shift, Alt y
/// Ctrl acompañan a un reporte de ratón.
pub(crate) fn mouse_modifiers(modifiers: Modifiers) -> MouseModifiers {
    MouseModifiers {
        shift: modifiers.shift,
        ctrl: modifiers.control,
        alt: modifiers.alt,
    }
}

/// `true` si el gesto debe convertirse en selección local de PORT.
///
/// El programa gana el gesto cuando pidió reportes de ratón y el núcleo le
/// reenvió la pulsación; la selección local solo nace del botón izquierdo que
/// el programa no consumió, nunca de los botones medio o derecho. Es pura para
/// poder verificarla sin ventana.
pub(crate) fn starts_local_selection(button: MouseButton, consumed_by_program: bool) -> bool {
    matches!(button, MouseButton::Left) && !consumed_by_program
}

/// Cierra un arrastre de selección: apaga la bandera y copia lo seleccionado.
///
/// Se ejecuta tanto al soltar dentro del contenedor como fuera de él. Solo
/// escribe en el portapapeles cuando hay texto real, para no pisar lo que el
/// usuario tuviera copiado con una selección vacía.
pub(crate) fn finish_drag(
    session: &Rc<RefCell<SessionManager>>,
    drag_selecting: &Rc<Cell<bool>>,
    cx: &mut App,
) {
    drag_selecting.set(false);
    // La política vive en la sesión: la vista no guarda una segunda copia. Con
    // `copy_on_select` apagado la selección no se toca y `Ctrl+Shift+C` la lee
    // de la sesión cuando el usuario la pida.
    let (text, policy) = {
        let manager = session.borrow();
        let active = manager.active_session();
        (active.selection_text(), active.mouse_policy())
    };
    let has_selection = text.as_deref().is_some_and(|text| !text.is_empty());
    if should_copy_on_select(policy, has_selection) {
        if let Some(text) = text {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px};
    use port_term_core::frame::Rgb;

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

    /// Un clic selecciona una celda, el doble la palabra y el triple la línea
    /// cuando la política conserva los switches de fábrica.
    #[test]
    fn cada_recuento_de_clics_pide_su_tipo_de_seleccion() {
        let policy = MousePolicy::default();
        assert_eq!(
            selection_kind_for_click_count(1, policy),
            SelectionKind::Simple
        );
        assert_eq!(
            selection_kind_for_click_count(2, policy),
            SelectionKind::Word
        );
        assert_eq!(
            selection_kind_for_click_count(3, policy),
            SelectionKind::Line
        );
    }

    /// Con `word_on_double_click` apagado el doble clic deja de expandir a la
    /// palabra y vuelve al clic simple.
    #[test]
    fn el_doble_clic_sin_palabra_se_comporta_como_simple() {
        let policy = MousePolicy {
            word_on_double_click: false,
            ..MousePolicy::default()
        };
        assert_eq!(
            selection_kind_for_click_count(2, policy),
            SelectionKind::Simple
        );
        // El triple clic sigue su propio switch, que aquí sigue activo.
        assert_eq!(
            selection_kind_for_click_count(3, policy),
            SelectionKind::Line
        );
    }

    /// Con `line_on_triple_click` apagado el triple clic deja de expandir a la
    /// línea entera y vuelve al clic simple.
    #[test]
    fn el_triple_clic_sin_linea_se_comporta_como_simple() {
        let policy = MousePolicy {
            line_on_triple_click: false,
            ..MousePolicy::default()
        };
        assert_eq!(
            selection_kind_for_click_count(3, policy),
            SelectionKind::Simple
        );
        // El doble clic sigue su propio switch, que aquí sigue activo.
        assert_eq!(
            selection_kind_for_click_count(2, policy),
            SelectionKind::Word
        );
    }

    /// Un cuarto clic (o cualquier recuento raro) vuelve al clic simple: el
    /// gesto nunca deja la selección sin definir, incluso con los switches
    /// encendidos.
    #[test]
    fn mas_de_tres_clics_se_comportan_como_uno() {
        let policy = MousePolicy::default();
        assert_eq!(
            selection_kind_for_click_count(4, policy),
            SelectionKind::Simple
        );
        assert_eq!(
            selection_kind_for_click_count(0, policy),
            SelectionKind::Simple
        );
    }

    /// La copia al soltar solo ocurre con `copy_on_select` activo y una
    /// selección con texto real; apagado, la selección queda para
    /// `Ctrl+Shift+C`.
    #[test]
    fn la_copia_al_soltar_respeta_copy_on_select() {
        let apagado = MousePolicy {
            copy_on_select: false,
            ..MousePolicy::default()
        };
        assert!(!should_copy_on_select(apagado, true));
        assert!(should_copy_on_select(MousePolicy::default(), true));
        assert!(!should_copy_on_select(MousePolicy::default(), false));
    }

    /// La política por defecto conserva el azul y la opacidad documentados: la
    /// vista ya no lleva constantes propias, así que este es el único lugar
    /// que fija el resaltado de fábrica.
    #[test]
    fn el_resaltado_por_defecto_conserva_color_y_opacidad() {
        let policy = MousePolicy::default();
        assert_eq!(policy.highlight, Rgb::new(0x58, 0xa6, 0xff));
        assert_eq!(policy.highlight_opacity, 0.35);
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

    /// La selección local solo nace del botón izquierdo que el programa no
    /// consumió: un gesto capturado por el programa (pi, vim, less) le
    /// pertenece, y los botones medio y derecho nunca seleccionan.
    #[test]
    fn la_seleccion_local_solo_nace_del_izquierdo_no_consumido() {
        assert!(starts_local_selection(MouseButton::Left, false));
        assert!(!starts_local_selection(MouseButton::Left, true));
        assert!(!starts_local_selection(MouseButton::Middle, false));
        assert!(!starts_local_selection(MouseButton::Right, false));
    }

    /// Los modificadores de gpui se traducen a los del reporte SGR; la tecla
    /// plataforma (Super) no viaja en el protocolo.
    #[test]
    fn los_modificadores_de_gpui_se_traducen_al_reporte() {
        let mods = mouse_modifiers(Modifiers {
            shift: true,
            control: true,
            alt: true,
            platform: true,
            function: true,
        });
        assert_eq!(
            mods,
            MouseModifiers {
                shift: true,
                ctrl: true,
                alt: true,
            }
        );
        assert_eq!(
            mouse_modifiers(Modifiers::default()),
            MouseModifiers::default()
        );
    }

    /// Los tres botones reales se traducen a los del protocolo y los de
    /// navegación (atrás/adelante) no: el protocolo SGR no los tiene y los
    /// manejadores solo se registran para los tres reales.
    #[test]
    fn los_botones_de_navegacion_no_se_traducen() {
        assert_eq!(
            terminal_button(MouseButton::Left),
            Some(TerminalMouseButton::Left)
        );
        assert_eq!(
            terminal_button(MouseButton::Middle),
            Some(TerminalMouseButton::Middle)
        );
        assert_eq!(
            terminal_button(MouseButton::Right),
            Some(TerminalMouseButton::Right)
        );
        assert_eq!(
            terminal_button(MouseButton::Navigate(gpui::NavigationDirection::Back)),
            None
        );
    }
}
