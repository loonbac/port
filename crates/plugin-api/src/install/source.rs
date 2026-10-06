//! Interpretación de la fuente que escribió la persona: repositorio y subcarpeta.
//!
//! Separa lo que se clona de lo que se instala. Su contrato es el de la
//! interfaz de línea de comandos: `fuente#subdirectorio` y la heurística de una
//! URL que ya trae la ruta.

/// Separa la fuente en su repositorio y, si procede, la subcarpeta del plugin.
///
/// Acepta dos formas de selector de subdirectorio:
///
/// - `fuente#subdirectorio`, la convención explícita. Vale igual para una URL
///   que para una ruta local.
/// - una URL que ya trae la ruta (`https://github.com/usuario/repo/plugins/x`),
///   donde se toman los dos primeros segmentos tras el host como repositorio y
///   el resto como subdirectorio. Es una heurística para GitHub/GitLab/Codeberg:
///   un host que sirva un repositorio bajo una ruta arbitraria se partiría mal,
///   así que en ese caso hay que usar `#`.
///
/// Una ruta local intacta (sin `#`) es exactamente el directorio indicado y no
/// se le recorta nada: en disco no existe la convención `host/owner/repo`.
pub(crate) fn split_source(source: &str) -> (String, Option<String>) {
    if let Some((base, sub)) = source.rsplit_once('#') {
        let sub = sub.trim_matches('/');
        if !base.is_empty() && !sub.is_empty() {
            return (base.to_string(), Some(sub.to_string()));
        }
    }
    match url_with_path(source) {
        Some((base, sub)) => (base, Some(sub)),
        None => (source.to_string(), None),
    }
}

/// Extrae repositorio y subcarpeta de una URL que ya incluye la ruta.
///
/// Devuelve `None` cuando no hay subcarpeta que separar (dos segmentos o menos
/// tras el host) o cuando la fuente no es una URL con esquema.
fn url_with_path(source: &str) -> Option<(String, String)> {
    let scheme_end = source.find("://")?;
    let rest = &source[scheme_end + 3..];
    let slash = rest.find('/')?;
    let host = &rest[..slash];
    let path = rest[slash + 1..].trim_end_matches('/');
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() <= 2 {
        return None;
    }
    let subdir = segments[2..].join("/");
    if subdir.is_empty() {
        return None;
    }
    let repo = format!(
        "{}://{}/{}",
        &source[..scheme_end],
        host,
        segments[..2].join("/")
    );
    Some((repo, subdir))
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Selector de subdirectorio en la fuente ---

    #[test]
    fn el_selector_con_almohadilla_separa_repositorio_y_subdirectorio() {
        assert_eq!(
            split_source("https://github.com/loonbac/port-plugins#plugins/font"),
            (
                "https://github.com/loonbac/port-plugins".to_string(),
                Some("plugins/font".to_string())
            )
        );
        assert_eq!(
            split_source("/tmp/monorepo#plugins/font"),
            (
                "/tmp/monorepo".to_string(),
                Some("plugins/font".to_string())
            )
        );
    }

    #[test]
    fn una_url_con_ruta_se_parte_con_la_heuristica_de_dos_segmentos() {
        assert_eq!(
            split_source("https://github.com/loonbac/port-plugins/plugins/font"),
            (
                "https://github.com/loonbac/port-plugins".to_string(),
                Some("plugins/font".to_string())
            )
        );
        assert_eq!(
            url_with_path("https://codeberg.org/loonbac/port-plugins/plugins/font"),
            Some((
                "https://codeberg.org/loonbac/port-plugins".to_string(),
                "plugins/font".to_string()
            ))
        );
        // La heurística también se aplica a hosts que no son un servicio git, y
        // ahí puede equivocarse al partir la ruta: por eso se documenta que en
        // esos casos conviene usar `#`.
        assert_eq!(
            split_source("https://git.interno.local/x/plugins/font").1,
            Some("font".to_string())
        );
    }

    #[test]
    fn una_url_sin_ruta_extra_no_lleva_subdirectorio() {
        assert_eq!(
            split_source("https://github.com/loonbac/port-plugins").1,
            None
        );
        assert_eq!(
            split_source("https://github.com/loonbac/port-plugins.git").1,
            None
        );
    }

    #[test]
    fn una_ruta_local_no_se_recorta_por_la_heuristica() {
        assert_eq!(split_source("/home/u/repos/mi-plugin").1, None);
        assert_eq!(split_source("./crates/plugin-demo").1, None);
    }

    #[test]
    fn el_selector_tiene_prioridad_sobre_la_heuristica() {
        assert_eq!(
            split_source("https://github.com/loonbac/port-plugins#plugins/font"),
            (
                "https://github.com/loonbac/port-plugins".to_string(),
                Some("plugins/font".to_string())
            )
        );
        // El `.git` del repo no es un subdirectorio.
        assert_eq!(
            split_source("https://github.com/loonbac/port-plugins.git"),
            (
                "https://github.com/loonbac/port-plugins.git".to_string(),
                None
            )
        );
    }
}
