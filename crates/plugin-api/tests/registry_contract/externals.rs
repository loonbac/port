//! Apariencia de externos: el registro compone ambas fuentes.

use std::sync::Arc;

use port_plugin_api::host::ExternalPlugin;
use port_plugin_api::PluginRegistry;

use crate::{MockFontPlugin, MockTransparencyPlugin};

/// Arranca un plugin externo falso que declara `appearance_json` en su saludo.
///
/// Reutiliza el idioma del arnés de `crates/port-app/tests/external_plugin.rs`:
/// un script que habla el protocolo por stdio, sin compilar un crate entero
/// para un valor que ya viaja en el `ready`.
#[cfg(unix)]
fn external_with_appearance(id: &str, appearance_json: &str) -> Arc<ExternalPlugin> {
    use port_plugin_api::protocol::{Capability, PluginManifest};

    let dir = std::env::temp_dir().join(format!(
        "port-registry-ext-{id}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let path = dir.join(id);
    let tmp = dir.join(format!("{id}.tmp"));
    let script = format!(
        r#"#!/bin/sh
printf '{{"t":"ready","name":"{id}","version":"0.1.0","capabilities":["appearance"],"appearance":{appearance_json}}}\n'
while IFS= read -r line; do :; done
"#
    );
    std::fs::write(&tmp, script).unwrap();
    std::fs::rename(&tmp, &path).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();

    let manifest = PluginManifest {
        id: id.to_string(),
        name: id.to_string(),
        version: "0.1.0".to_string(),
        executable: path.to_string_lossy().to_string(),
        source: "test".to_string(),
        capabilities: vec![Capability::Appearance],
    };
    ExternalPlugin::start(manifest).expect("el externo falso debe arrancar")
}

/// Opacidad: el mínimo entre ambas fuentes, igual que ya hacía solo el registro.
#[cfg(unix)]
#[test]
fn la_opacidad_externa_se_combina_por_minimo() {
    // Solo el externo: aporta su valor.
    let mut registry = PluginRegistry::new();
    registry.set_externals(vec![external_with_appearance(
        "transparency",
        r#"{"opacity":0.85}"#,
    )]);
    assert_eq!(registry.effective_opacity(), 0.85);

    // In-process más opaco: gana el mínimo, que es el externo.
    registry.register(MockTransparencyPlugin { opacity: 0.95 });
    assert_eq!(registry.effective_opacity(), 0.85);

    // In-process más translúcido: gana el mínimo, que es el in-process.
    let mut registry = PluginRegistry::new();
    registry.set_externals(vec![external_with_appearance(
        "transparency",
        r#"{"opacity":0.85}"#,
    )]);
    registry.register(MockTransparencyPlugin { opacity: 0.60 });
    assert_eq!(registry.effective_opacity(), 0.60);
}

/// Un externo no puede sacar la opacidad de su rango válido.
#[cfg(unix)]
#[test]
fn la_opacidad_externa_se_recorta_al_rango_valido() {
    let mut registry = PluginRegistry::new();
    registry.set_externals(vec![external_with_appearance(
        "transparency",
        r#"{"opacity":1.7}"#,
    )]);
    assert_eq!(registry.effective_opacity(), 1.0);

    registry.set_externals(vec![external_with_appearance(
        "transparency",
        r#"{"opacity":-0.4}"#,
    )]);
    assert_eq!(registry.effective_opacity(), 0.0);
}

/// Familia y tamaño: primero el registro en proceso —es la configuración propia
/// del núcleo— y, si no se pronuncia, el primer externo no vacío.
#[cfg(unix)]
#[test]
fn la_familia_y_el_tamano_en_proceso_tienen_precedencia_sobre_los_externos() {
    let mut registry = PluginRegistry::new();
    registry.set_externals(vec![external_with_appearance(
        "font",
        r#"{"font_family":"External Mono","font_size":20.0}"#,
    )]);

    // Sin in-process, el externo sí cambia la fuente.
    assert_eq!(registry.effective_font_family("Fira Code"), "External Mono");
    assert_eq!(registry.effective_font_size(14.0), 20.0);

    // Con in-process, manda el núcleo.
    registry.register(MockFontPlugin {
        family: "Core Mono",
        size: 12.0,
    });
    assert_eq!(registry.effective_font_family("Fira Code"), "Core Mono");
    assert_eq!(registry.effective_font_size(14.0), 12.0);
}

/// Un valor vacío o ausente no se pronuncia: pasa al siguiente candidato.
#[cfg(unix)]
#[test]
fn un_valor_externo_vacio_o_ausente_cede_ante_el_siguiente() {
    // El primero declara una familia en blanco y sin tamaño; el segundo sí
    // aporta. Ninguno debe dejar la fuente a medias.
    let mut registry = PluginRegistry::new();
    registry.set_externals(vec![
        external_with_appearance("font-a", r#"{"font_family":"   ","font_size":null}"#),
        external_with_appearance(
            "font-b",
            r#"{"font_family":"Segunda Mono","font_size":16.0}"#,
        ),
    ]);
    assert_eq!(registry.effective_font_family("Fira Code"), "Segunda Mono");
    assert_eq!(registry.effective_font_size(14.0), 16.0);

    // Si ninguno aporta, queda el valor por defecto.
    registry.set_externals(vec![external_with_appearance(
        "font-a",
        r#"{"font_family":"","font_size":null}"#,
    )]);
    assert_eq!(registry.effective_font_family("Fira Code"), "Fira Code");
    assert_eq!(registry.effective_font_size(14.0), 14.0);
}

/// Un externo apagado desde el menú sale de la lista de vivos y no aporta nada.
#[cfg(unix)]
#[test]
fn un_externo_apagado_no_aporta_nada() {
    let mut registry = PluginRegistry::new();
    registry.set_externals(vec![external_with_appearance(
        "transparency",
        r#"{"opacity":0.85,"font_family":"External Mono","font_size":20.0}"#,
    )]);
    assert_eq!(registry.effective_opacity(), 0.85);
    assert_eq!(registry.effective_font_size(14.0), 20.0);

    // Así apaga el menú: se retira de la lista de externos vivos.
    registry.set_externals(Vec::new());
    assert_eq!(registry.effective_opacity(), 1.0);
    assert_eq!(registry.effective_font_family("Fira Code"), "Fira Code");
    assert_eq!(registry.effective_font_size(14.0), 14.0);
}

/// Las fallbacks no viajan en el protocolo `Appearance`, así que siguen siendo
/// competencia exclusiva del registro en proceso.
#[cfg(unix)]
#[test]
fn las_fallbacks_siguen_siendo_solo_del_registro_en_proceso() {
    let mut registry = PluginRegistry::new();
    registry.set_externals(vec![external_with_appearance(
        "font",
        r#"{"font_family":"External Mono","font_size":20.0}"#,
    )]);
    assert_eq!(
        registry.effective_font_fallbacks().len(),
        3,
        "un externo no puede aportar fallbacks: quedan las de fábrica"
    );

    registry.register(MockFontPlugin {
        family: "Core Mono",
        size: 12.0,
    });
    assert_eq!(
        registry.effective_font_fallbacks(),
        vec!["Custom Fallback".to_string()]
    );
}
