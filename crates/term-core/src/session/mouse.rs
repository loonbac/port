//! Protocolo de ratón y su política.
//!
//! Codifica pulsación, liberación y movimiento como reportes SGR y decide,
//! según los modos que pidió el programa y la [`MousePolicy`], si el gesto sale
//! hacia el programa o queda para la selección local de PORT. Las funciones
//! puras permiten verificar los bytes exactos y el gating de modos sin PTY.

use alacritty_terminal::term::TermMode;

use crate::frame::Rgb;

use super::{CellPos, Session};

/// Botón del ratón reenviado al programa.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

impl MouseButton {
    /// Código base del botón en el protocolo SGR: 0, 1 y 2.
    fn code(self) -> u8 {
        match self {
            Self::Left => 0,
            Self::Middle => 1,
            Self::Right => 2,
        }
    }
}

/// Modificadores que acompañan a un evento de ratón.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// Política de ratón y selección de una sesión.
///
/// Los valores por defecto son el comportamiento de PORT tal cual: reenvío
/// completo de los modos que el programa pida y selección propia con Shift
/// como salida de emergencia.
///
/// Los campos de selección (`copy_on_select`, `word_on_double_click`,
/// `line_on_triple_click`, `highlight` y `highlight_opacity`) no cambian el
/// comportamiento del núcleo: este solo los guarda y los devuelve para que la
/// UI los lea.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MousePolicy {
    /// Reenvía las pulsaciones y liberaciones al programa que pidió reportes.
    pub forward_clicks: bool,
    /// Reenvía el arrastre (modo 1002) mientras hay un botón pulsado.
    pub forward_drag: bool,
    /// Reenvía todo movimiento (modo 1003), con o sin botón pulsado.
    pub forward_motion: bool,
    /// Shift devuelve el gesto a la selección local. Desactivado, Shift se
    /// reenvía como cualquier modificador.
    pub shift_selects: bool,
    /// Copia la selección al soltar. La UI lo lee.
    pub copy_on_select: bool,
    /// El doble clic selecciona la palabra bajo el puntero. La UI lo lee.
    pub word_on_double_click: bool,
    /// El triple clic selecciona la línea entera. La UI lo lee.
    pub line_on_triple_click: bool,
    /// Color del resaltado de selección. La UI lo lee.
    pub highlight: Rgb,
    /// Opacidad del resaltado de selección, de 0.0 a 1.0. La UI lo lee.
    pub highlight_opacity: f32,
}

impl Default for MousePolicy {
    fn default() -> Self {
        Self {
            forward_clicks: true,
            forward_drag: true,
            forward_motion: true,
            shift_selects: true,
            copy_on_select: true,
            word_on_double_click: true,
            line_on_triple_click: true,
            highlight: Rgb::new(0x58, 0xa6, 0xff),
            highlight_opacity: 0.35,
        }
    }
}

/// `true` si el programa pidió reportes de ratón en cualquiera de sus modos.
///
/// El modo 1000 (clics), el 1002 (arrastre) y el 1003 (todo movimiento) son
/// mutuamente excluyentes en la rejilla, pero los tres significan que el ratón
/// pertenece al programa. Es pura para poder verificar el gating de modos sin
/// PTY.
pub(crate) fn buttons_are_reported(mode: TermMode) -> bool {
    mode.intersects(TermMode::MOUSE_MODE)
}

/// Bits de modificador que se suman al código del botón en el reporte SGR:
/// Shift 4, Alt 8, Ctrl 16.
pub(crate) fn modifier_bits(mods: MouseModifiers) -> u8 {
    (u8::from(mods.shift) << 2) | (u8::from(mods.alt) << 3) | (u8::from(mods.ctrl) << 4)
}

/// Codifica un reporte SGR del ratón: `ESC[<código;columna;fila` más `M` si el
/// evento es una pulsación o un movimiento, y `m` si es una liberación.
///
/// La celda ya llega 1-based y recortada por [`CellPos`]: aquí no se vuelve a
/// recortar. Es pura para poder verificar los bytes exactos sin PTY.
pub(crate) fn sgr_mouse_report(code: u8, cell: CellPos, released: bool) -> String {
    let final_byte = if released { 'm' } else { 'M' };
    format!("\x1b[<{code};{};{}{final_byte}", cell.column, cell.row)
}

/// Código SGR de una pulsación o liberación de botón, o `None` si el gesto no
/// pertenece al programa.
///
/// Devuelve `None` cuando el programa no pidió reportes de botón o cuando la
/// [`MousePolicy`] desactiva `forward_clicks`. Con `shift_selects` activo,
/// Shift devuelve el gesto a la selección local de PORT —la misma salida de
/// emergencia que en la rueda—; con `shift_selects` desactivado, Shift se
/// reenvía y su bit se suma al código. La liberación comparte el mismo gate:
/// el programa que pidió reportes espera el `m` que cierra su gesto.
pub(crate) fn press_code(
    policy: MousePolicy,
    mode: TermMode,
    button: MouseButton,
    mods: MouseModifiers,
) -> Option<u8> {
    if (policy.shift_selects && mods.shift) || !policy.forward_clicks || !buttons_are_reported(mode)
    {
        return None;
    }
    Some(button.code() | modifier_bits(mods))
}

/// Código base de un movimiento, o `None` si el movimiento no se reenvía.
///
/// Con 1003 (todos los movimientos) cualquier movimiento se reenvía mientras
/// `forward_motion` esté activo: el botón pulsado más 32, y 35 (32 + 3, «sin
/// botón») cuando no hay ninguno. Con solo 1002 el movimiento se reenvía
/// únicamente mientras un botón esté pulsado y `forward_drag` siga activo; un
/// mover suelto pertenece a PORT.
pub(crate) fn motion_base_code(
    policy: MousePolicy,
    mode: TermMode,
    held: Option<MouseButton>,
) -> Option<u8> {
    let all_motion = mode.contains(TermMode::MOUSE_MOTION);
    if all_motion {
        if !policy.forward_motion {
            return None;
        }
    } else if !mode.contains(TermMode::MOUSE_DRAG) || held.is_none() || !policy.forward_drag {
        return None;
    }
    Some(match held {
        Some(button) => button.code() + 32,
        None => 35,
    })
}

/// Código SGR completo de un movimiento, o `None` si el gesto es de PORT.
///
/// Aplica el mismo escape de Shift que la pulsación —gobernado por
/// `shift_selects`— y suma los bits de modificador al código base.
pub(crate) fn motion_code(
    policy: MousePolicy,
    mode: TermMode,
    held: Option<MouseButton>,
    mods: MouseModifiers,
) -> Option<u8> {
    if policy.shift_selects && mods.shift {
        return None;
    }
    motion_base_code(policy, mode, held).map(|base| base | modifier_bits(mods))
}

impl Session {
    /// Sustituye la política de ratón y selección de esta sesión.
    pub fn set_mouse_policy(&mut self, policy: MousePolicy) {
        self.mouse_policy = policy;
    }

    /// Política de ratón y selección vigente.
    ///
    /// El núcleo solo la guarda y la devuelve; los campos de selección los lee
    /// la UI, que es quien pinta el resaltado y decide copiar al seleccionar.
    pub fn mouse_policy(&self) -> MousePolicy {
        self.mouse_policy
    }

    /// Reenvía la pulsación de un botón al programa. `true` si la consumió.
    ///
    /// Contrato de terminal real, el mismo que ya sigue la rueda:
    ///
    /// - Si el programa pidió reportes de ratón (modos 1000/1002/1003) y el
    ///   usuario no mantiene Shift, la pulsación se le reenvía como reporte SGR
    ///   (`ESC[<código;x;yM`, con los bits de modificador ya sumados) y el gesto
    ///   pertenece al programa: PORT no inicia su propia selección.
    /// - Si el programa no pidió reportes, o si el usuario mantiene Shift como
    ///   salida de emergencia, devuelve `false` y el gesto queda para la
    ///   selección local.
    ///
    /// Se emite un solo reporte por evento y no se toca `dirty`: la rejilla no
    /// cambia, porque el reporte es entrada del programa y sale por el mismo
    /// camino que las respuestas de protocolo y la rueda.
    pub fn mouse_press(
        &mut self,
        button: MouseButton,
        cell: CellPos,
        mods: MouseModifiers,
    ) -> bool {
        let Some(code) = press_code(self.mouse_policy, *self.term.mode(), button, mods) else {
            return false;
        };
        let report = sgr_mouse_report(code, cell, false);
        let _ = self.pty.write(report.as_bytes());
        true
    }

    /// Reenvía la liberación de un botón al programa. `true` si la consumió.
    ///
    /// Comparte el gate de la pulsación (los mismos modos y el mismo escape de
    /// Shift) y solo cambia la letra final del reporte: `ESC[<código;x;y m`. Un
    /// programa que pidió reportes espera también el `m` que cierra el gesto,
    /// así que sin esa liberación su arrastre quedaría colgado.
    pub fn mouse_release(
        &mut self,
        button: MouseButton,
        cell: CellPos,
        mods: MouseModifiers,
    ) -> bool {
        let Some(code) = press_code(self.mouse_policy, *self.term.mode(), button, mods) else {
            return false;
        };
        let report = sgr_mouse_report(code, cell, true);
        let _ = self.pty.write(report.as_bytes());
        true
    }

    /// Reenvía un movimiento del ratón al programa. `true` si lo consumió.
    ///
    /// `held` es el botón pulsado, si lo hay. El movimiento se reenvía cuando:
    ///
    /// - el programa activó 1003 (todos los movimientos), con o sin botón
    ///   pulsado —es lo que necesitan el arrastre del scrollbar y el hover de
    ///   una TUI—;
    /// - o activó 1002 (arrastre) y hay un botón pulsado. Un mover suelto queda
    ///   para PORT.
    ///
    /// En cualquier otro caso devuelve `false`, y Shift sigue siendo la salida
    /// de emergencia que conserva el movimiento para la selección local.
    ///
    /// Un reporte de movimiento no cambia la rejilla: no se toca `dirty`.
    pub fn mouse_motion(
        &mut self,
        cell: CellPos,
        held: Option<MouseButton>,
        mods: MouseModifiers,
    ) -> bool {
        let Some(code) = motion_code(self.mouse_policy, *self.term.mode(), held, mods) else {
            return false;
        };
        let report = sgr_mouse_report(code, cell, false);
        let _ = self.pty.write(report.as_bytes());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{
        buttons_are_reported, modifier_bits, motion_base_code, motion_code, press_code,
        sgr_mouse_report, MouseButton, MouseModifiers, MousePolicy,
    };
    use crate::frame::Rgb;
    use crate::session::CellPos;
    use alacritty_terminal::term::TermMode;

    /// Una pulsación es `ESC[<código;col;filaM` con la letra `M` final. La celda
    /// ya llega 1-based y recortada por `CellPos`: el codificador no la toca.
    #[test]
    fn la_pulsacion_se_codifica_como_reporte_sgr() {
        assert_eq!(
            sgr_mouse_report(0, CellPos::new(5, 3), false),
            "\x1b[<0;5;3M"
        );
        assert_eq!(
            sgr_mouse_report(2, CellPos::new(1, 1), false),
            "\x1b[<2;1;1M"
        );
    }

    /// La liberación cambia solo la letra final: `m` en minúscula.
    #[test]
    fn la_liberacion_se_codifica_con_m_minuscula() {
        assert_eq!(
            sgr_mouse_report(0, CellPos::new(5, 3), true),
            "\x1b[<0;5;3m"
        );
    }

    /// Los botones ocupan los códigos base 0, 1 y 2 (izquierdo, medio, derecho),
    /// y con reportes de ratón activos la pulsación se reenvía tal cual.
    #[test]
    fn cada_boton_se_codifica_con_su_codigo_base() {
        let modo = TermMode::MOUSE_REPORT_CLICK;
        let sin_mods = MouseModifiers::default();
        let policy = MousePolicy::default();
        assert_eq!(
            press_code(policy, modo, MouseButton::Left, sin_mods),
            Some(0)
        );
        assert_eq!(
            press_code(policy, modo, MouseButton::Middle, sin_mods),
            Some(1)
        );
        assert_eq!(
            press_code(policy, modo, MouseButton::Right, sin_mods),
            Some(2)
        );
    }

    /// Los modificadores se suman al código con los bits del protocolo: Shift 4,
    /// Alt 8, Ctrl 16. Shift solo, y el par Ctrl+Alt, son los dos casos que
    /// ejercitan bits sueltos y combinados.
    #[test]
    fn los_modificadores_suman_sus_bits_al_codigo() {
        let solo_shift = MouseModifiers {
            shift: true,
            ..Default::default()
        };
        let ctrl_alt = MouseModifiers {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        assert_eq!(modifier_bits(solo_shift), 4);
        assert_eq!(modifier_bits(ctrl_alt), 24);
        assert_eq!(modifier_bits(MouseModifiers::default()), 0);

        // El mismo código compuesto que viaja en la pulsación del botón derecho
        // con Ctrl y Alt: 2 + 16 + 8.
        assert_eq!(
            press_code(
                MousePolicy::default(),
                TermMode::MOUSE_REPORT_CLICK,
                MouseButton::Right,
                ctrl_alt
            ),
            Some(26)
        );
        assert_eq!(
            sgr_mouse_report(26, CellPos::new(5, 3), false),
            "\x1b[<26;5;3M"
        );
    }

    /// Shift es la salida de emergencia, igual que en la rueda: aunque el
    /// programa pida reportes, pulsación y movimiento vuelven a PORT.
    #[test]
    fn shift_devuelve_el_gesto_a_la_seleccion_local() {
        let modo = TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION;
        let con_shift = MouseModifiers {
            shift: true,
            ..Default::default()
        };
        let policy = MousePolicy::default();
        assert_eq!(press_code(policy, modo, MouseButton::Left, con_shift), None);
        assert_eq!(
            motion_code(policy, modo, Some(MouseButton::Left), con_shift),
            None
        );
    }

    /// El movimiento se codifica con el botón pulsado más 32, y usa 35 (32 + 3,
    /// "sin botón") cuando no hay ninguno: es lo que define el protocolo.
    #[test]
    fn el_movimiento_usa_el_boton_pulsado_mas_32() {
        let modo = TermMode::MOUSE_MOTION;
        let policy = MousePolicy::default();
        assert_eq!(
            motion_base_code(policy, modo, Some(MouseButton::Left)),
            Some(32)
        );
        assert_eq!(
            motion_base_code(policy, modo, Some(MouseButton::Middle)),
            Some(33)
        );
        assert_eq!(
            motion_base_code(policy, modo, Some(MouseButton::Right)),
            Some(34)
        );
        assert_eq!(motion_base_code(policy, modo, None), Some(35));
        assert_eq!(
            motion_code(policy, modo, None, MouseModifiers::default()),
            Some(35)
        );
        assert_eq!(
            sgr_mouse_report(35, CellPos::new(2, 4), false),
            "\x1b[<35;2;4M"
        );
    }

    /// Con solo 1000 el programa pidió clics: el movimiento no se reenvía, ni
    /// con botón pulsado ni sin él.
    #[test]
    fn solo_con_clics_el_movimiento_no_se_reenvia() {
        let modo = TermMode::MOUSE_REPORT_CLICK;
        let policy = MousePolicy::default();
        assert_eq!(
            press_code(policy, modo, MouseButton::Left, MouseModifiers::default()),
            Some(0)
        );
        assert_eq!(
            motion_base_code(policy, modo, Some(MouseButton::Left)),
            None
        );
        assert_eq!(motion_base_code(policy, modo, None), None);
    }

    /// Con 1002 el arrastre pertenece al programa solo mientras un botón esté
    /// pulsado: un mover suelto vuelve a PORT.
    #[test]
    fn con_arrastre_el_movimiento_necesita_un_boton_pulsado() {
        let modo = TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG;
        let policy = MousePolicy::default();
        assert_eq!(
            motion_base_code(policy, modo, Some(MouseButton::Left)),
            Some(32)
        );
        assert_eq!(motion_base_code(policy, modo, None), None);
    }

    /// Al añadir 1003 (todos los movimientos) también se reenvía el mover suelto:
    /// es lo que habilita el arrastre del scrollbar de pi.
    #[test]
    fn con_todos_los_movimientos_el_mover_suelto_se_reenvia() {
        let modo = TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION;
        let policy = MousePolicy::default();
        assert_eq!(motion_base_code(policy, modo, None), Some(35));
        assert_eq!(
            motion_base_code(policy, modo, Some(MouseButton::Right)),
            Some(34)
        );
    }

    /// Sin ningún modo de reporte el ratón pertenece a PORT: ni la pulsación ni
    /// el movimiento salen hacia el programa.
    #[test]
    fn sin_reportes_de_raton_el_gesto_no_sale() {
        let modo = TermMode::default();
        let policy = MousePolicy::default();
        assert!(!buttons_are_reported(modo));
        assert_eq!(
            press_code(policy, modo, MouseButton::Left, MouseModifiers::default()),
            None
        );
        assert_eq!(
            motion_base_code(policy, modo, Some(MouseButton::Left)),
            None
        );
        assert_eq!(
            motion_code(policy, modo, None, MouseModifiers::default()),
            None
        );
    }

    /// La política por defecto reproduce el comportamiento de PORT tal cual:
    /// todos los reenvíos activos, Shift como salida de emergencia, copia al
    /// seleccionar con palabra y línea expandidas, y el resaltado azul.
    #[test]
    fn la_politica_por_defecto_es_el_comportamiento_de_port() {
        let policy = MousePolicy::default();
        assert!(policy.forward_clicks);
        assert!(policy.forward_drag);
        assert!(policy.forward_motion);
        assert!(policy.shift_selects);
        assert!(policy.copy_on_select);
        assert!(policy.word_on_double_click);
        assert!(policy.line_on_triple_click);
        assert_eq!(policy.highlight, Rgb::new(0x58, 0xa6, 0xff));
        assert_eq!(policy.highlight_opacity, 0.35);
    }

    /// Con `forward_clicks` desactivado la pulsación y su liberación dejan de
    /// salir al programa, pero el arrastre conserva su propio switch: apagar los
    /// clics no apaga el movimiento, que sigue la política de `forward_drag`.
    #[test]
    fn con_clics_desactivados_la_pulsacion_no_sale_pero_el_arrastre_sigue_su_switch() {
        let policy = MousePolicy {
            forward_clicks: false,
            forward_drag: true,
            ..MousePolicy::default()
        };
        let modo = TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG;
        let sin_mods = MouseModifiers::default();
        assert_eq!(press_code(policy, modo, MouseButton::Left, sin_mods), None);
        assert_eq!(
            motion_base_code(policy, modo, Some(MouseButton::Left)),
            Some(32)
        );
    }

    /// `forward_drag` solo afecta al caso 1002 con un botón pulsado: sin él, el
    /// arrastre no se reenvía aunque el programa lo haya pedido.
    #[test]
    fn con_arrastre_desactivado_el_movimiento_con_boton_no_se_reenvia() {
        let policy = MousePolicy {
            forward_drag: false,
            ..MousePolicy::default()
        };
        let modo = TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG;
        assert_eq!(
            motion_base_code(policy, modo, Some(MouseButton::Left)),
            None
        );
    }

    /// `forward_motion` gobierna el modo 1003 (todos los movimientos), con o sin
    /// botón pulsado.
    #[test]
    fn con_movimiento_desactivado_el_modo_1003_no_se_reenvia() {
        let policy = MousePolicy {
            forward_motion: false,
            ..MousePolicy::default()
        };
        let modo = TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION;
        assert_eq!(
            motion_base_code(policy, modo, Some(MouseButton::Left)),
            None
        );
        assert_eq!(motion_base_code(policy, modo, None), None);
    }

    /// Con `shift_selects` activo (el valor por defecto), Shift sigue siendo la
    /// salida de emergencia que devuelve el gesto a la selección local.
    #[test]
    fn con_shift_selects_activo_el_shift_no_se_reenvia() {
        let policy = MousePolicy::default();
        let modo = TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION;
        let con_shift = MouseModifiers {
            shift: true,
            ..Default::default()
        };
        assert!(policy.shift_selects);
        assert_eq!(press_code(policy, modo, MouseButton::Left, con_shift), None);
        assert_eq!(
            motion_code(policy, modo, Some(MouseButton::Left), con_shift),
            None
        );
    }

    /// Sin `shift_selects` Shift deja de ser escape: el reporte se reenvía y el
    /// bit de Shift (4) viaja sumado al código, como cualquier modificador.
    #[test]
    fn sin_shift_selects_el_shift_se_reenvia_con_su_bit() {
        let policy = MousePolicy {
            shift_selects: false,
            ..MousePolicy::default()
        };
        let modo = TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG;
        let con_shift = MouseModifiers {
            shift: true,
            ..Default::default()
        };
        assert_eq!(
            press_code(policy, modo, MouseButton::Left, con_shift),
            Some(4)
        );
        assert_eq!(
            motion_code(policy, modo, Some(MouseButton::Left), con_shift),
            Some(36)
        );
    }
}
