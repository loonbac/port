//! Contrato de las secciones nativas del archivo de configuración.
//!
//! PORT trae fuente y política de ratón de fábrica, sin plugins. Lo que se
//! verifica aquí es que el archivo sigue mandando, que un plugin sigue pudiendo
//! reemplazar el valor nativo, y que el archivo distribuido no puede separarse de
//! los defaults del núcleo.

use std::collections::BTreeMap;

use port_plugin_api::native::{self, Font};
use port_plugin_api::{
    AppearanceHook, ConfigFile, MouseHook, Plugin, PluginConfig, PluginRegistry,
};
use port_term_core::session::MousePolicy;

/// Plugin in-process que fija la tipografía, como haría un `font` instalado.
struct FontPlugin;

impl Plugin for FontPlugin {
    fn id(&self) -> &'static str {
        "font-de-prueba"
    }

    fn name(&self) -> &'static str {
        "Font de prueba"
    }

    fn appearance_hook(&self) -> Option<&dyn AppearanceHook> {
        Some(self)
    }
}

impl AppearanceHook for FontPlugin {
    fn font_family(&self) -> Option<String> {
        Some("Plugin Mono".to_string())
    }

    fn font_size(&self) -> Option<f32> {
        Some(21.0)
    }
}

/// Plugin in-process que fija la política de ratón.
struct MousePlugin;

impl Plugin for MousePlugin {
    fn id(&self) -> &'static str {
        "raton-de-prueba"
    }

    fn name(&self) -> &'static str {
        "Ratón de prueba"
    }

    fn mouse_hook(&self) -> Option<&dyn MouseHook> {
        Some(self)
    }
}

impl MouseHook for MousePlugin {
    fn mouse_policy(&self) -> MousePolicy {
        MousePolicy {
            copy_on_select: false,
            ..MousePolicy::default()
        }
    }
}

fn config_with(pairs: &[(&str, &str)]) -> PluginConfig {
    let mut cfg = PluginConfig::new();
    for (key, value) in pairs {
        cfg.set(*key, *value);
    }
    cfg
}

fn parse(text: &str) -> BTreeMap<String, PluginConfig> {
    ConfigFile::parse(text)
}

/// Sin ningún plugin registrado la terminal usa lo que dice su archivo.
#[test]
fn sin_plugins_manda_la_seccion_nativa() {
    let mut registro = PluginRegistry::new();
    let font = Font {
        family: Some("Mi Mono".to_string()),
        size: Some(17.0),
        ..Font::default()
    };
    registro.set_native_font(font);
    let policy = MousePolicy {
        copy_on_select: false,
        ..MousePolicy::default()
    };
    registro.set_native_mouse_policy(policy);

    assert_eq!(registro.effective_font_family("otra"), "Mi Mono");
    assert_eq!(registro.effective_font_size(12.0), 17.0);
    assert_eq!(registro.mouse_policy(), policy);
}

/// Un plugin sigue pudiendo reemplazar el valor nativo.
#[test]
fn un_plugin_gana_sobre_la_seccion_nativa() {
    let mut registro = PluginRegistry::new();
    registro.set_native_font(Font {
        family: Some("Mi Mono".to_string()),
        size: Some(17.0),
        ..Font::default()
    });
    registro.set_native_mouse_policy(MousePolicy::default());
    registro.register(FontPlugin);
    registro.register(MousePlugin);

    assert_eq!(registro.effective_font_family("otra"), "Plugin Mono");
    assert_eq!(registro.effective_font_size(12.0), 21.0);
    assert!(!registro.mouse_policy().copy_on_select);
}

/// Las claves son las mismas que tenían los plugins: el archivo viejo sirve.
#[test]
fn la_seccion_de_raton_se_lee_del_archivo() {
    let text = "```selection\nforward_clicks = false\ncopy_on_select = false\nhighlight_color = #102030\nhighlight_opacity = 1.5\nenabled = true\n```\n";
    let configs = parse(text);
    let policy = native::mouse_policy_from_config(&configs[native::SELECTION_SECTION]);

    assert!(!policy.forward_clicks);
    assert!(!policy.copy_on_select);
    assert_eq!(
        (policy.highlight.r, policy.highlight.g, policy.highlight.b),
        (0x10, 0x20, 0x30)
    );
    assert_eq!(
        policy.highlight_opacity, 1.0,
        "la opacidad se recorta a 0..=1"
    );
}

/// Un valor ilegible no rompe: conserva el default de esa clave.
#[test]
fn un_valor_ilegible_conserva_el_default() {
    let cfg = config_with(&[
        ("forward_clicks", "quizá"),
        ("highlight_color", "azul"),
        ("highlight_opacity", "mucho"),
        ("copy_on_select", "false"),
    ]);
    let policy = native::mouse_policy_from_config(&cfg);
    let default = MousePolicy::default();

    assert_eq!(policy.forward_clicks, default.forward_clicks);
    assert_eq!(policy.highlight, default.highlight);
    assert_eq!(policy.highlight_opacity, default.highlight_opacity);
    assert!(!policy.copy_on_select, "la clave válida sí se aplica");
}

/// `enabled = false` en una sección nativa significa «no la apliques».
#[test]
fn una_seccion_apagada_se_ignora() {
    let text = "```selection\ncopy_on_select = false\nenabled = false\n```\n\n```font\nfamily = Otra\nenabled = false\n```\n";
    let configs = parse(text);
    let mut registro = PluginRegistry::new();
    registro.load_configs(&configs);

    assert_eq!(registro.mouse_policy(), MousePolicy::default());
    assert_eq!(
        registro.effective_font_family("x"),
        "x",
        "una sección apagada deja mandar al llamador"
    );
}

/// El archivo que PORT escribe trae las dos secciones y coincide con el núcleo.
#[test]
fn la_plantilla_trae_las_dos_secciones() {
    let registro = PluginRegistry::new();
    let defaults = registro.default_configs();

    assert!(defaults.contains_key(native::SELECTION_SECTION));
    assert!(defaults.contains_key(native::FONT_SECTION));

    let policy = native::mouse_policy_from_config(&defaults[native::SELECTION_SECTION]);
    assert_eq!(
        policy,
        MousePolicy::default(),
        "la plantilla no puede divergir"
    );
    assert_eq!(
        defaults[native::FONT_SECTION].get("family"),
        Some(native::DEFAULT_FAMILY),
        "el archivo documenta la familia del núcleo"
    );
}

/// La sección de fuente lee familia y tamaño, y un tamaño imposible no entra.
#[test]
fn la_seccion_de_fuente_se_lee_del_archivo() {
    let cfg = config_with(&[("family", "JetBrains Mono"), ("size", "15.5")]);
    let font = native::font_from_config(&cfg);
    assert_eq!(font.family.as_deref(), Some("JetBrains Mono"));
    assert_eq!(font.size, Some(15.5));

    let roto = native::font_from_config(&config_with(&[("family", "  "), ("size", "-3")]));
    assert_eq!(roto.family, None, "una familia vacía no vale");
    assert_eq!(roto.size, None, "un tamaño no positivo no vale");
}

/// Equivalencia con el plugin que esta sección reemplaza: las siete claves
/// booleanas se aplican una por una, y la opacidad se recorta por los dos lados.
///
/// Los casos son los del contrato de `selection` en `port-plugins`; si el parser
/// nativo se separa de aquel, esto lo dice.
#[test]
fn las_siete_claves_booleanas_y_los_dos_recortes() {
    let claves = [
        "forward_clicks",
        "forward_drag",
        "forward_motion",
        "shift_selects",
        "copy_on_select",
        "word_on_double_click",
        "line_on_triple_click",
    ];
    for clave in claves {
        let policy = native::mouse_policy_from_config(&config_with(&[(clave, "false")]));
        let (aplicado, resto) = match clave {
            "forward_clicks" => (!policy.forward_clicks, policy.forward_drag),
            "forward_drag" => (!policy.forward_drag, policy.forward_motion),
            "forward_motion" => (!policy.forward_motion, policy.shift_selects),
            "shift_selects" => (!policy.shift_selects, policy.copy_on_select),
            "copy_on_select" => (!policy.copy_on_select, policy.word_on_double_click),
            "word_on_double_click" => (!policy.word_on_double_click, policy.line_on_triple_click),
            _ => (!policy.line_on_triple_click, policy.forward_clicks),
        };
        assert!(aplicado, "la clave {clave} no se aplicó");
        assert!(resto, "la clave {clave} tocó un campo que no era el suyo");
    }

    let abajo = native::mouse_policy_from_config(&config_with(&[("highlight_opacity", "-1")]));
    assert_eq!(abajo.highlight_opacity, 0.0);
    let arriba = native::mouse_policy_from_config(&config_with(&[("highlight_opacity", "2")]));
    assert_eq!(arriba.highlight_opacity, 1.0);
}

/// R3-001: un color con bytes multibyte no puede partir el string en un límite
/// de carácter; conserva el default como cualquier otro valor ilegible.
#[test]
fn un_color_multibyte_conserva_el_default() {
    // Dos caracteres de tres bytes: seis bytes en total, y los cortes en 2 y 4
    // caen dentro de un carácter.
    let policy = native::mouse_policy_from_config(&config_with(&[("highlight_color", "€€")]));
    assert_eq!(policy.highlight, MousePolicy::default().highlight);
    assert_eq!(native::parse_hex_color("€€"), None);
}

/// R2-001: la sección nativa cubre las claves del plugin que reemplaza, así que
/// una lista de fallbacks del archivo llega a la apariencia.
#[test]
fn los_fallbacks_de_la_seccion_nativa_se_aplican() {
    let text = "```font\nfamily = Mi Mono\nfallbacks = Simbolos Mono, DejaVu Sans Mono\nenabled = true\n```\n";
    let configs = parse(text);
    let mut registro = PluginRegistry::new();
    registro.load_configs(&configs);
    assert_eq!(
        registro.effective_font_fallbacks(),
        vec!["Simbolos Mono".to_string(), "DejaVu Sans Mono".to_string()]
    );
}

/// S2 del slice de avisos: los espacios alrededor del color no dependen de la
/// posición del `#`.
#[test]
fn el_color_admite_espacios_alrededor() {
    let con_almohadilla = native::parse_hex_color("  #58a6ff  ");
    let sin_almohadilla = native::parse_hex_color(" 58a6ff ");
    assert_eq!(
        con_almohadilla,
        Some(port_term_core::frame::Rgb::new(0x58, 0xa6, 0xff))
    );
    assert_eq!(sin_almohadilla, con_almohadilla);
}

/// La misma verdad escrita en mayúsculas se aplica igual, tanto en las claves
/// como en `enabled`.
///
/// `PluginConfig::get_bool` ya normaliza la caja; esto lo deja fijado desde la
/// sección nativa, que es donde el contrato importa.
#[test]
fn un_booleano_en_mayusculas_se_aplica() {
    let default = MousePolicy::default();

    let text = "```selection\ncopy_on_select = False\nenabled = TRUE\n```\n";
    let configs = parse(text);
    let policy = native::mouse_policy_from_config(&configs[native::SELECTION_SECTION]);
    assert_eq!(
        policy.copy_on_select, !default.copy_on_select,
        "un False en mayúsculas apaga, y enabled = TRUE deja aplicar la sección"
    );

    let text = "```selection\ncopy_on_select = False\nenabled = FALSE\n```\n";
    let configs = parse(text);
    let policy = native::mouse_policy_from_config(&configs[native::SELECTION_SECTION]);
    assert_eq!(
        policy.copy_on_select, default.copy_on_select,
        "un enabled = FALSE en mayúsculas apaga la sección entera"
    );
}

/// S8 del slice de avisos: apagar la sección apaga también sus fallbacks.
#[test]
fn una_seccion_apagada_no_aporta_fallbacks() {
    let text = "```font\nfamily = Otra\nfallbacks = Solo Esta\nenabled = false\n```\n";
    let configs = parse(text);
    let mut registro = PluginRegistry::new();
    registro.load_configs(&configs);
    assert_eq!(
        registro.effective_font_fallbacks(),
        PluginRegistry::new().effective_font_fallbacks(),
        "una sección apagada deja los respaldos de fábrica, no los suyos"
    );
}
