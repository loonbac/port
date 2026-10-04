#[path = "../src/menu.rs"]
mod menu;

use menu::{ExternalPluginInfo, MenuAction, MenuMode, MenuState};
use port_term_core::input::Key;

#[test]
fn test_menu_navigation_and_clamping() {
    let mut state = MenuState {
        open: true,
        ..Default::default()
    };
    let total_items = 3;

    // Inicialmente en 0; hacia arriba no baja de 0 (clamp superior)
    assert_eq!(state.selected_index, 0);
    state.navigate_up();
    assert_eq!(state.selected_index, 0);

    // Navegar hacia abajo
    state.navigate_down(total_items);
    assert_eq!(state.selected_index, 1);
    state.navigate_down(total_items);
    assert_eq!(state.selected_index, 2);

    // Clamp inferior: no pasa de total_items - 1
    state.navigate_down(total_items);
    assert_eq!(state.selected_index, 2);

    // Navegar hacia arriba de vuelta
    state.navigate_up();
    assert_eq!(state.selected_index, 1);

    // Clamp manual de índice fuera de rango
    state.selected_index = 99;
    state.clamp_selection(total_items);
    assert_eq!(state.selected_index, 2);

    // Clamp con lista vacía
    state.clamp_selection(0);
    assert_eq!(state.selected_index, 0);
}

#[test]
fn test_transition_add_install_result() {
    let mut state = MenuState {
        open: true,
        ..Default::default()
    };
    assert_eq!(state.mode, MenuMode::Browsing);

    // Transición a Adding
    state.start_adding();
    assert_eq!(
        state.mode,
        MenuMode::Adding {
            input: String::new()
        }
    );

    // Intento de submit con entrada vacía no debe transicionar
    assert_eq!(state.submit_add(), None);
    assert_eq!(
        state.mode,
        MenuMode::Adding {
            input: String::new()
        }
    );

    // Entrada de texto, carácter individual y backspace
    state.input_append("https://github.com/test/plugin.gi");
    state.input_append_char('t');
    assert_eq!(
        state.mode,
        MenuMode::Adding {
            input: "https://github.com/test/plugin.git".to_string()
        }
    );
    for _ in 0..4 {
        state.input_backspace();
    }
    assert_eq!(
        state.mode,
        MenuMode::Adding {
            input: "https://github.com/test/plugin".to_string()
        }
    );

    // Confirmación pasa a Installing
    let url = state.submit_add();
    assert_eq!(url, Some("https://github.com/test/plugin".to_string()));
    assert_eq!(
        state.mode,
        MenuMode::Installing {
            url: "https://github.com/test/plugin".to_string()
        }
    );

    // Finalización exitosa pasa a Result
    state.finish_install(true, "Installed successfully");
    assert_eq!(
        state.mode,
        MenuMode::Result {
            message: "Installed successfully".to_string(),
            success: true
        }
    );

    // Descartar resultado vuelve a Browsing
    state.dismiss_result();
    assert_eq!(state.mode, MenuMode::Browsing);
}

#[test]
fn test_escape_behavior() {
    let mut state = MenuState {
        open: true,
        ..Default::default()
    };

    // En Adding: Escape vuelve a Browsing sin cerrar el menú
    state.start_adding();
    state.handle_escape();
    assert_eq!(state.mode, MenuMode::Browsing);
    assert!(
        state.open,
        "el menú debe seguir abierto tras cancelar Adding"
    );

    // En Result: Escape vuelve a Browsing sin cerrar el menú
    state.finish_install(false, "Fallo");
    state.handle_escape();
    assert_eq!(state.mode, MenuMode::Browsing);
    assert!(
        state.open,
        "el menú debe seguir abierto tras descartar Result"
    );

    // En Browsing: Escape cierra el menú
    state.handle_escape();
    assert!(!state.open, "Escape en Browsing debe cerrar el menú");

    // En Installing: Escape cierra la UI del menú (la tarea sigue en background)
    state.open = true;
    state.mode = MenuMode::Installing {
        url: "https://example.com".to_string(),
    };
    state.handle_escape();
    assert!(
        !state.open,
        "Escape en Installing debe cerrar la vista del menú"
    );
}

#[test]
fn test_handle_key_browsing_and_adding() {
    let mut state = MenuState {
        open: true,
        external_plugins: vec![ExternalPluginInfo {
            id: "ext1".to_string(),
            name: "External 1".to_string(),
            version: "0.1.0".to_string(),
            enabled: true,
            running: true,
        }],
        ..Default::default()
    };

    let compiled_count = 2; // índices 0 y 1 son compilados
                            // índice 2 es ext1
                            // índice 3 es la acción fija "+ Install plugin"
    let total_items = state.total_items_count(compiled_count);
    assert_eq!(total_items, 4);

    // Enter en índice 0 -> ToggleCompiled(0)
    state.selected_index = 0;
    let action = state.handle_key(&Key::new("Enter"), total_items, compiled_count);
    assert_eq!(action, MenuAction::ToggleCompiled(0));

    // Enter en índice 2 -> ToggleExternal(0)
    state.selected_index = 2;
    let action = state.handle_key(&Key::new("Enter"), total_items, compiled_count);
    assert_eq!(action, MenuAction::ToggleExternal(0));

    // Enter en índice 3 (acción fija) -> entra a Adding
    state.selected_index = 3;
    let action = state.handle_key(&Key::new("Enter"), total_items, compiled_count);
    assert_eq!(action, MenuAction::Refresh);
    assert_eq!(
        state.mode,
        MenuMode::Adding {
            input: String::new()
        }
    );

    // Escribir texto en Adding
    let action = state.handle_key(&Key::new("h").with_text("h"), total_items, compiled_count);
    assert_eq!(action, MenuAction::Refresh);
    let action = state.handle_key(&Key::new("i").with_text("i"), total_items, compiled_count);
    assert_eq!(action, MenuAction::Refresh);
    assert_eq!(
        state.mode,
        MenuMode::Adding {
            input: "hi".to_string()
        }
    );

    // Backspace en Adding
    let action = state.handle_key(&Key::new("Backspace"), total_items, compiled_count);
    assert_eq!(action, MenuAction::Refresh);
    assert_eq!(
        state.mode,
        MenuMode::Adding {
            input: "h".to_string()
        }
    );

    // Enter en Adding -> StartInstall("h")
    let action = state.handle_key(&Key::new("Enter"), total_items, compiled_count);
    assert_eq!(action, MenuAction::StartInstall("h".to_string()));
    assert_eq!(
        state.mode,
        MenuMode::Installing {
            url: "h".to_string()
        }
    );

    // Mientras Installing, navegar Up y Down sigue funcionando
    state.selected_index = 1;
    let action = state.handle_key(&Key::new("Down"), total_items, compiled_count);
    assert_eq!(action, MenuAction::Refresh);
    assert_eq!(state.selected_index, 2);

    let action = state.handle_key(&Key::new("Up"), total_items, compiled_count);
    assert_eq!(action, MenuAction::Refresh);
    assert_eq!(state.selected_index, 1);
}

#[test]
fn test_open_close_menu_helpers() {
    let mut state = MenuState::new();
    assert!(!state.open);

    state.open_menu();
    assert!(state.open);
    assert_eq!(state.selected_index, 0);
    assert_eq!(state.mode, MenuMode::Browsing);

    state.selected_index = 5;
    state.close_menu();
    assert!(!state.open);
}

#[test]
fn test_add_or_update_external_plugin() {
    let mut state = MenuState::new();
    assert!(state.external_plugins.is_empty());

    state.add_or_update_external(ExternalPluginInfo {
        id: "my-plugin".to_string(),
        name: "My Plugin".to_string(),
        version: "0.1.0".to_string(),
        enabled: true,
        running: true,
    });
    assert_eq!(state.external_plugins.len(), 1);

    // Actualizar el mismo id
    state.add_or_update_external(ExternalPluginInfo {
        id: "my-plugin".to_string(),
        name: "My Plugin Updated".to_string(),
        version: "0.2.0".to_string(),
        enabled: true,
        running: true,
    });
    assert_eq!(state.external_plugins.len(), 1);
    assert_eq!(state.external_plugins[0].name, "My Plugin Updated");
    assert_eq!(state.external_plugins[0].version, "0.2.0");
}
