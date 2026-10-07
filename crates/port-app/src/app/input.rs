//! Interceptor de teclado de la terminal.
//!
//! Responsabilidad única: decidir qué hacer con cada pulsación antes de que
//! GPUI la reparta. El diálogo de cierre es modal y captura el teclado entero;
//! el menú de plugins tiene su propio atajo y su propia entrada; los atajos de
//! copiar y pegar de PORT, los plugins (compilados y externos) y, por último,
//! el PTY resuelven el resto. El orden de las ramas es la política y no se
//! reordena.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{App, ClipboardItem, KeystrokeEvent, Window};
use port_plugin_api::KeyAction;

use crate::app::Handles;
use crate::keys;
use crate::view::ClosePromptState;

/// Procesa una pulsación en el interceptor global de teclado.
///
/// Reúne las ramas que el arranque registraba en línea: el diálogo de cierre
/// (modal), el atajo y la entrada del menú de plugins, el copiado y pegado de
/// PORT, el despacho a los plugins compilados y externos y, como último
/// recurso, la escritura de bytes al PTY. Cada rama que atiende la tecla
/// refresca la ventana y retorna; solo la ruta del PTY cae en el refresco final.
pub(crate) fn handle_key(
    ev: &KeystrokeEvent,
    window: &mut Window,
    cx: &mut App,
    close_prompt: &Rc<RefCell<ClosePromptState>>,
    handles: &Handles<'_>,
) {
    // Los manejadores siguen siendo de `run_terminal`: aquí solo se prestan y
    // se desestructuran para que el cuerpo los nombre igual que antes.
    let &Handles {
        sessions,
        plugins,
        menu_state,
        external_plugins,
        ..
    } = handles;
    if let Some(key) = keys::to_core_key(&ev.keystroke) {
        // El diálogo de cierre es modal: captura el teclado entero.
        if close_prompt.borrow().open {
            match key.key.as_str() {
                // Atajos directos de una tecla.
                "Escape" | "n" | "N" => crate::view::decline_close(close_prompt, window),
                "y" | "Y" => crate::view::accept_close(close_prompt, window),
                // Navegación entre los dos botones.
                "Left" | "Right" | "Up" | "Down" | "h" | "l" | "Tab" => {
                    {
                        let mut s = close_prompt.borrow_mut();
                        s.selected = 1 - s.selected;
                    }
                    // Sin este repintado el foco cambiaba de estado
                    // pero se seguía viendo el botón anterior.
                    window.refresh();
                }
                "Enter" | "Return" | " " => {
                    let confirm = close_prompt.borrow().selected == 1;
                    if confirm {
                        crate::view::accept_close(close_prompt, window);
                    } else {
                        crate::view::decline_close(close_prompt, window);
                    }
                }
                // Cualquier otra tecla no hace nada: el diálogo no se cierra solo.
                _ => {}
            }
            return;
        }

        // Comprobación de atajo de apertura/cierre del menú de plugins (Ctrl+Shift+L por defecto)
        let shortcut = crate::menu::MENU_SHORTCUT;
        if keys::matches_shortcut(&key, shortcut) {
            let mut s = menu_state.borrow_mut();
            if s.open {
                s.close_menu();
            } else {
                s.open_menu();
            }
            drop(s);
            window.refresh();
            return;
        }

        // Si el menú está abierto, delega la entrada al módulo de menú
        if menu_state.borrow().open {
            // Pegar no llega como tecla: hay que ir al portapapeles.
            // Sin esto, `Ctrl+V` caía en la rama que ignora las teclas
            // con modificador y no pasaba absolutamente nada.
            let pegando_url = {
                let s = menu_state.borrow();
                matches!(s.mode, crate::menu::MenuMode::Adding { .. })
            };
            if pegando_url && key.ctrl && !key.alt && key.key.eq_ignore_ascii_case("v") {
                if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    menu_state.borrow_mut().input_paste(&text);
                }
                window.refresh();
                return;
            }

            let (total_items, compiled_count) = {
                let s = menu_state.borrow();
                let c_count = plugins.borrow().len();
                (s.total_items_count(c_count), c_count)
            };
            let action = menu_state
                .borrow_mut()
                .handle_key(&key, total_items, compiled_count);
            crate::app::menu_actions::execute(action, handles, cx);
            window.refresh();
            return;
        }

        // Copiar y pegar de PORT: `Ctrl+Shift+C` y `Ctrl+Shift+V`. Van
        // antes de la ruta que manda bytes al PTY para que nunca lleguen
        // a la shell, y después de la rama del menú para no pisar su
        // propio `Ctrl+V` del campo de URL.
        if let Some(action) = keys::edit_action(&key) {
            match action {
                keys::EditAction::Copy => {
                    let text = sessions.borrow().active_session().selection_text();
                    if let Some(text) = text {
                        if !text.is_empty() {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                        }
                    }
                }
                keys::EditAction::Paste => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        sessions.borrow_mut().active_session_mut().paste(&text);
                    }
                }
            }
            window.refresh();
            return;
        }

        // Si un plugin consume la tecla (ej. atajo de tab o sidebar), no se envía al PTY
        if plugins.borrow().dispatch_key(&key) == KeyAction::Consume {
            window.refresh();
            return;
        }

        // Plugins externos: resolver combinaciones registradas
        if let Some((plugin, action)) = port_plugin_api::host::ExternalPlugin::resolve_action(
            &external_plugins.borrow(),
            &key.key,
            key.ctrl,
            key.alt,
            key.shift,
        ) {
            let _ = plugin.invoke(&action);
            window.refresh();
            return;
        }

        let mut session = sessions.borrow_mut();
        let mode = session.cursor_key_mode();
        if let Some(bytes) = port_term_core::input::encode(&key, mode) {
            let _ = session.write(&bytes);
        }
    }
    window.refresh();
}
