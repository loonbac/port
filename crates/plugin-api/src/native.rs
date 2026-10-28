//! Secciones del archivo de configuración que PORT trae de fábrica.
//!
//! La fuente y la política de ratón no son capacidades que un plugin reemplace:
//! son datos que la terminal necesita siempre. Vivían en dos plugins que solo
//! parseaban su propio bloque; ahora el bloque se parsea aquí, con los mismos
//! nombres y las mismas claves, así que un `config.md` existente sigue sirviendo
//! y un plugin que implemente [`crate::AppearanceHook`] o [`crate::MouseHook`]
//! sigue teniendo la última palabra.
//!
//! Parsear al cargar el archivo, nunca por cuadro, es lo que mantiene esto fuera
//! de la ruta caliente: los getters por cuadro solo leen el valor ya resuelto.

use port_term_core::frame::Rgb;
use port_term_core::session::MousePolicy;

use crate::config::PluginConfig;

/// Bloque que describe la tipografía.
pub const FONT_SECTION: &str = "font";
/// Bloque que describe la política de ratón y selección.
pub const SELECTION_SECTION: &str = "selection";
/// Familia tipográfica que PORT usa sin configuración.
pub const DEFAULT_FAMILY: &str = "FiraCode Nerd Font Mono";

/// Tipografía vigente, ya resuelta desde el archivo.
///
/// Los dos campos son opcionales a propósito: `None` significa «el archivo no
/// dijo nada», y entonces decide el llamador —la UI, con la fuente y el zoom que
/// ya tiene—. Así una terminal sin configuración se comporta como siempre y el
/// default del núcleo no queda secuestrado por esta capa.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Font {
    /// Familia principal, si el archivo la fija.
    pub family: Option<String>,
    /// Tamaño en puntos, si el archivo lo fija.
    pub size: Option<f32>,
    /// Familias de respaldo, en el orden en que se prueban.
    pub fallbacks: Option<Vec<String>>,
}

/// Lee la sección de fuente.
///
/// Una familia vacía o un tamaño que no es un número positivo conservan el
/// default: el archivo no puede dejar a la terminal sin tipografía. Con
/// `enabled = false` no se aplica nada de la sección.
pub fn font_from_config(config: &PluginConfig) -> Font {
    let mut font = Font::default();
    if !is_enabled(config) {
        return font;
    }
    if let Some(family) = config.get("family") {
        let family = family.trim();
        if !family.is_empty() {
            font.family = Some(family.to_string());
        }
    }
    if let Some(raw) = config.get("size") {
        match raw.trim().parse::<f32>() {
            Ok(size) if size > 0.0 && size.is_finite() => font.size = Some(size),
            _ => warn(FONT_SECTION, "size", raw, "se esperaba un número positivo"),
        }
    }
    // La lista vacía no se guarda: sin respaldos la apariencia usa los suyos.
    if let Some(raw) = config.get("fallbacks") {
        let fallbacks: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect();
        if !fallbacks.is_empty() {
            font.fallbacks = Some(fallbacks);
        }
    }
    font
}

/// Bloque de configuración con los valores por defecto de una tipografía.
pub fn font_default_config(font: &Font) -> PluginConfig {
    let mut cfg = PluginConfig::new();
    // El archivo distribuido documenta la familia del núcleo aunque `Font` la
    // deje en `None`: es el valor con el que arranca la terminal sin configurar.
    cfg.set("family", font.family.as_deref().unwrap_or(DEFAULT_FAMILY));
    if let Some(size) = font.size {
        cfg.set("size", size);
    }
    if let Some(fallbacks) = &font.fallbacks {
        cfg.set("fallbacks", fallbacks.join(", "));
    }
    cfg.set("enabled", true);
    cfg
}

/// Lee la sección de ratón y selección.
///
/// Se parte siempre de [`MousePolicy::default()`]: una clave ausente deja su
/// campo en el default y no en el valor de una carga anterior, así que el
/// archivo es la única fuente de verdad.
pub fn mouse_policy_from_config(config: &PluginConfig) -> MousePolicy {
    let mut policy = MousePolicy::default();
    if !is_enabled(config) {
        return policy;
    }
    apply_bool(config, "forward_clicks", &mut policy.forward_clicks);
    apply_bool(config, "forward_drag", &mut policy.forward_drag);
    apply_bool(config, "forward_motion", &mut policy.forward_motion);
    apply_bool(config, "shift_selects", &mut policy.shift_selects);
    apply_bool(config, "copy_on_select", &mut policy.copy_on_select);
    apply_bool(
        config,
        "word_on_double_click",
        &mut policy.word_on_double_click,
    );
    apply_bool(
        config,
        "line_on_triple_click",
        &mut policy.line_on_triple_click,
    );
    apply_color(config, "highlight_color", &mut policy.highlight);
    apply_opacity(config, "highlight_opacity", &mut policy.highlight_opacity);
    policy
}

/// Bloque de configuración derivado de una política.
///
/// Se deriva en lugar de repetir los números: es lo que impide que el archivo
/// distribuido y los defaults del núcleo se separen con el tiempo.
pub fn mouse_policy_default_config(policy: &MousePolicy) -> PluginConfig {
    let mut cfg = PluginConfig::new();
    cfg.set("forward_clicks", policy.forward_clicks);
    cfg.set("forward_drag", policy.forward_drag);
    cfg.set("forward_motion", policy.forward_motion);
    cfg.set("shift_selects", policy.shift_selects);
    cfg.set("copy_on_select", policy.copy_on_select);
    cfg.set("word_on_double_click", policy.word_on_double_click);
    cfg.set("line_on_triple_click", policy.line_on_triple_click);
    cfg.set(
        "highlight_color",
        format!(
            "#{:02x}{:02x}{:02x}",
            policy.highlight.r, policy.highlight.g, policy.highlight.b
        ),
    );
    cfg.set("highlight_opacity", policy.highlight_opacity);
    cfg.set("enabled", true);
    cfg
}

/// Parsea un color `#rrggbb` o `rrggbb`.
pub fn parse_hex_color(hex: &str) -> Option<Rgb> {
    let clean = hex.strip_prefix('#').unwrap_or(hex).trim();
    // Los cortes de abajo son por byte: sin ASCII puro el índice puede caer
    // dentro de un carácter y partir el `&str` en un límite inválido.
    if clean.len() != 6 || !clean.is_ascii() {
        return None;
    }
    let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
    let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
    let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
    Some(Rgb::new(r, g, b))
}

/// Sin la clave la sección se aplica; `enabled = false` la ignora entera.
fn is_enabled(config: &PluginConfig) -> bool {
    config.get_bool("enabled").unwrap_or(true)
}

fn apply_bool(config: &PluginConfig, key: &str, target: &mut bool) {
    let Some(raw) = config.get(key) else {
        return;
    };
    match config.get_bool(key) {
        Some(value) => *target = value,
        None => warn(SELECTION_SECTION, key, raw, "se esperaba true o false"),
    }
}

fn apply_color(config: &PluginConfig, key: &str, target: &mut Rgb) {
    let Some(raw) = config.get(key) else {
        return;
    };
    match parse_hex_color(raw) {
        Some(color) => *target = color,
        None => warn(SELECTION_SECTION, key, raw, "se esperaba un color #rrggbb"),
    }
}

fn apply_opacity(config: &PluginConfig, key: &str, target: &mut f32) {
    let Some(raw) = config.get(key) else {
        return;
    };
    match raw.trim().parse::<f32>() {
        Ok(value) => *target = value.clamp(0.0, 1.0),
        Err(_) => warn(
            SELECTION_SECTION,
            key,
            raw,
            "se esperaba un número de 0 a 1",
        ),
    }
}

/// Un valor que no se puede leer conserva el default y se avisa por stderr.
///
/// Falla en voz alta a propósito: una errata que apaga un ajuste en silencio es
/// más cara de encontrar que un aviso en la terminal.
fn warn(section: &str, key: &str, raw: &str, expected: &str) {
    eprintln!("aviso: la sección '{section}' ignoró `{key} = {raw}`: {expected}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_bloque_por_defecto_es_estable() {
        // El archivo distribuido nombra la familia del núcleo, así que al leerlo
        // queda fijada —con el mismo valor— y volver a escribirlo no cambia nada.
        let once = font_from_config(&font_default_config(&Font::default()));
        assert_eq!(once.family.as_deref(), Some(DEFAULT_FAMILY));
        let twice = font_from_config(&font_default_config(&once));
        assert_eq!(once, twice);

        let policy =
            mouse_policy_from_config(&mouse_policy_default_config(&MousePolicy::default()));
        assert_eq!(policy, MousePolicy::default());
    }

    #[test]
    fn los_colores_admiten_las_dos_formas() {
        assert_eq!(parse_hex_color("#0a0b0c"), Some(Rgb::new(10, 11, 12)));
        assert_eq!(parse_hex_color("0A0B0C"), Some(Rgb::new(10, 11, 12)));
        assert_eq!(parse_hex_color("#0a0b"), None);
        assert_eq!(parse_hex_color("#zzzzzz"), None);
    }
}
