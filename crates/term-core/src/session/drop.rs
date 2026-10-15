//! Ubicación de los elementos soltados sobre la terminal.
//!
//! Arrastrar un archivo desde el gestor de archivos y soltarlo sobre la rejilla
//! es entrada del usuario, igual que pegar: lo que se inserta es la ubicación
//! del elemento, nunca una orden. La conversión de rutas a texto es pura para
//! poder verificar el escapado sin PTY y sin ventana.
//!
//! El escapado es el del shell: lo que el usuario suelta debe llegar a la línea
//! de comandos como un solo argumento aunque la ruta traiga espacios, comillas
//! o cualquier carácter que el shell interpretaría.

use std::path::PathBuf;

/// `true` si el texto necesita comillas para llegar entero al shell.
///
/// El conjunto seguro es el habitual (`shlex`): letras, dígitos y los
/// separadores que no tienen significado especial en una palabra. Todo lo demás
/// obliga a entrecomillar, porque el shell lo partiría, lo expandiría o lo
/// interpretaría.
fn necesita_comillas(texto: &str) -> bool {
    texto.is_empty()
        || !texto
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c))
}

/// Texto que se pega al soltar rutas: la ubicación de cada elemento, escapada
/// para el shell.
///
/// Una ruta con espacios va entre comillas simples, y una comilla simple
/// interior se cierra y se reabre (`'a'\''b'`), que es la única forma de
/// llevarla dentro de un texto entrecomillado así. Las rutas se unen con un
/// espacio: soltar varios archivos pega una lista lista para usar.
///
/// La ruta se pega como la entrega el sistema operativo —la ubicación del
/// elemento, absoluta en los gestores de archivos— y se convierte de forma
/// tolerante: una ruta que no sea UTF-8 no pierde el pegado.
pub(crate) fn paths_as_text(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| entrecomillar(&path.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Entrecomilla un texto si el shell lo necesitaría.
fn entrecomillar(texto: &str) -> String {
    if !necesita_comillas(texto) {
        return texto.to_string();
    }
    let mut citado = String::with_capacity(texto.len() + 2);
    citado.push('\'');
    for c in texto.chars() {
        if c == '\'' {
            citado.push_str("'\\''");
        } else {
            citado.push(c);
        }
    }
    citado.push('\'');
    citado
}

#[cfg(test)]
mod tests {
    use super::paths_as_text;
    use std::path::PathBuf;

    fn ruta(texto: &str) -> PathBuf {
        PathBuf::from(texto)
    }

    /// Una ruta sin caracteres especiales se pega tal cual: las comillas de más
    /// ensuciarían la línea de comandos.
    #[test]
    fn una_ruta_simple_se_pega_sin_comillas() {
        assert_eq!(
            paths_as_text(&[ruta("/home/ana/foto.png")]),
            "/home/ana/foto.png"
        );
    }

    /// Una ruta con espacios va entre comillas simples: sin ellas el shell la
    /// partiría en dos argumentos.
    #[test]
    fn una_ruta_con_espacios_va_entre_comillas() {
        assert_eq!(
            paths_as_text(&[ruta("/home/ana/mis fotos/foto 1.png")]),
            "'/home/ana/mis fotos/foto 1.png'"
        );
    }

    /// Una comilla simple interior se cierra y se reabre: es la única forma de
    /// llevarla dentro de un texto entrecomillado con comillas simples.
    #[test]
    fn una_comilla_simple_interior_se_cierra_y_se_reabre() {
        assert_eq!(paths_as_text(&[ruta("/tmp/a'b.txt")]), "'/tmp/a'\\''b.txt'");
    }

    /// Varios elementos soltados a la vez se pegan separados por un espacio.
    #[test]
    fn varias_rutas_se_separan_con_un_espacio() {
        assert_eq!(
            paths_as_text(&[ruta("/tmp/uno.txt"), ruta("/tmp/dos tres.txt")]),
            "/tmp/uno.txt '/tmp/dos tres.txt'"
        );
    }

    /// Sin rutas no hay texto: `paste` ignora una cadena vacía y no se escribe
    /// nada al programa.
    #[test]
    fn sin_rutas_no_hay_texto() {
        assert_eq!(paths_as_text(&[]), "");
    }
}
