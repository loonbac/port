//! Decodificador de gráficos Sixel a mapa de píxeles RGBA.
//!
//! Implementa la decodificación pura del cuerpo de una secuencia Sixel según el
//! estándar DEC VT340 sin dependencias externas:
//! - Tabla de color inicial de 16 entradas VT340.
//! - Definición de colores con porcentajes RGB (`#n;2;r;g;b`).
//! - Selección de color activo (`#n`). Entradas no definidas pintan negro.
//! - Operador de repetición (`!n<byte>`).
//! - Salto de banda de 6 píxeles (`-`) y retorno de carro al inicio de la banda (`$`).
//! - Mapeo de bits de bytes de seis píxeles (0x3f..=0x7e, bit i = fila i de la banda).
//! - Tamaño acotado al ancho máximo pintado + 1 y al alto de la última banda con píxeles.

use super::{Image, MAX_PENDING_SEQUENCE};
use std::collections::HashMap;

/// Paleta por defecto de 16 colores según la especificación del terminal DEC VT340.
pub const VT340_PALETTE: [[u8; 4]; 16] = [
    [0, 0, 0, 255],       // 0: Negro (0%, 0%, 0%)
    [51, 51, 204, 255],   // 1: Azul (20%, 20%, 80%)
    [204, 33, 33, 255],   // 2: Rojo (80%, 13%, 13%)
    [51, 204, 51, 255],   // 3: Verde (20%, 80%, 20%)
    [204, 51, 204, 255],  // 4: Magenta (80%, 20%, 80%)
    [51, 204, 204, 255],  // 5: Cian (20%, 80%, 80%)
    [204, 204, 51, 255],  // 6: Amarillo (80%, 80%, 20%)
    [135, 135, 135, 255], // 7: Gris 53% (53%, 53%, 53%)
    [66, 66, 66, 255],    // 8: Gris 26% (26%, 26%, 26%)
    [84, 153, 255, 255],  // 9: Azul claro (33%, 60%, 100%)
    [255, 84, 84, 255],   // 10: Rojo claro (100%, 33%, 33%)
    [84, 255, 84, 255],   // 11: Verde claro (33%, 100%, 33%)
    [255, 84, 255, 255],  // 12: Magenta claro (100%, 33%, 100%)
    [84, 255, 255, 255],  // 13: Cian claro (33%, 100%, 100%)
    [255, 255, 84, 255],  // 14: Amarillo claro (100%, 100%, 33%)
    [255, 255, 255, 255], // 15: Blanco (100%, 100%, 100%)
];

/// Extrae un número decimal no negativo a partir de la posición actual del cursor.
fn parse_number(bytes: &[u8], idx: &mut usize) -> Option<usize> {
    let start = *idx;
    let mut val: usize = 0;
    while *idx < bytes.len() && bytes[*idx].is_ascii_digit() {
        val = val
            .saturating_mul(10)
            .saturating_add((bytes[*idx] - b'0') as usize);
        *idx += 1;
    }
    if *idx > start {
        Some(val)
    } else {
        None
    }
}

/// Decodifica el cuerpo de una secuencia Sixel en una [`Image`] con píxeles RGBA.
///
/// Si el cuerpo es inválido, está vacío, o no pinta ningún píxel, devuelve `None`.
/// Nunca entra en pánico ni excede [`MAX_PENDING_SEQUENCE`] bytes de memoria rasterizada.
pub fn decode(body: &[u8]) -> Option<Image> {
    if body.is_empty() {
        return None;
    }

    // Pase 1: Determinar dimensiones exactas del resultado sin asignaciones.
    let mut col: usize = 0;
    let mut band: usize = 0;
    let mut max_painted_col: Option<usize> = None;
    let mut max_painted_band: Option<usize> = None;

    let mut idx = 0;
    while idx < body.len() {
        let b = body[idx];
        match b {
            b'#' => {
                idx += 1;
                // Saltar identificador y parámetros de color en el pase 1
                while idx < body.len() && (body[idx].is_ascii_digit() || body[idx] == b';') {
                    idx += 1;
                }
            }
            b'!' => {
                idx += 1;
                let count = parse_number(body, &mut idx).unwrap_or(0);
                if idx < body.len() {
                    let sixel_b = body[idx];
                    idx += 1;
                    if (0x3f..=0x7e).contains(&sixel_b) {
                        let val = sixel_b - 0x3f;
                        if val != 0 && count > 0 {
                            max_painted_band =
                                Some(max_painted_band.map_or(band, |m| m.max(band)));
                            let rightmost = col.saturating_add(count.saturating_sub(1));
                            max_painted_col =
                                Some(max_painted_col.map_or(rightmost, |m| m.max(rightmost)));
                        }
                        col = col.saturating_add(count);
                    }
                }
            }
            b'-' => {
                band = band.saturating_add(1);
                col = 0;
                idx += 1;
            }
            b'$' => {
                col = 0;
                idx += 1;
            }
            b'"' => {
                idx += 1;
                // Atributos de ráster: ignorados según la especificación
                while idx < body.len() && (body[idx].is_ascii_digit() || body[idx] == b';') {
                    idx += 1;
                }
            }
            sixel_b if (0x3f..=0x7e).contains(&sixel_b) => {
                let val = sixel_b - 0x3f;
                if val != 0 {
                    max_painted_band = Some(max_painted_band.map_or(band, |m| m.max(band)));
                    max_painted_col = Some(max_painted_col.map_or(col, |m| m.max(col)));
                }
                col = col.saturating_add(1);
                idx += 1;
            }
            _ => {
                idx += 1;
            }
        }
    }

    let rightmost_col = max_painted_col?;
    let last_band = max_painted_band?;

    let width = rightmost_col.checked_add(1)?;
    let height = last_band.checked_add(1)?.checked_mul(6)?;

    let total_bytes = width.checked_mul(height)?.checked_mul(4)?;
    if total_bytes > MAX_PENDING_SEQUENCE {
        return None;
    }

    let mut rgba = vec![0u8; total_bytes];
    // Fondo completamente opaco: píxeles no pintados quedan en negro ([0, 0, 0, 255])
    for chunk in rgba.chunks_exact_mut(4) {
        chunk[3] = 255;
    }

    // Pase 2: Renderizado de píxeles con la paleta activa y mapeo de bandas.
    let mut palette: HashMap<usize, [u8; 4]> = HashMap::new();
    let mut active_color: usize = 0;
    col = 0;
    band = 0;
    idx = 0;

    let get_color = |pal: &HashMap<usize, [u8; 4]>, color_idx: usize| -> [u8; 4] {
        if let Some(&c) = pal.get(&color_idx) {
            c
        } else if color_idx < 16 {
            VT340_PALETTE[color_idx]
        } else {
            [0, 0, 0, 255]
        }
    };

    while idx < body.len() {
        let b = body[idx];
        match b {
            b'#' => {
                idx += 1;
                if let Some(n) = parse_number(body, &mut idx) {
                    let mut params = Vec::new();
                    while idx < body.len() && body[idx] == b';' {
                        idx += 1;
                        if let Some(p) = parse_number(body, &mut idx) {
                            params.push(p);
                        }
                    }
                    if params.len() == 4 && params[0] == 2 {
                        let r = params[1].min(100);
                        let g = params[2].min(100);
                        let b_val = params[3].min(100);
                        let r_8 = ((r as u32 * 255 + 50) / 100) as u8;
                        let g_8 = ((g as u32 * 255 + 50) / 100) as u8;
                        let b_8 = ((b_val as u32 * 255 + 50) / 100) as u8;
                        palette.insert(n, [r_8, g_8, b_8, 255]);
                    }
                    active_color = n;
                }
            }
            b'!' => {
                idx += 1;
                let count = parse_number(body, &mut idx).unwrap_or(0);
                if idx < body.len() {
                    let sixel_b = body[idx];
                    idx += 1;
                    if (0x3f..=0x7e).contains(&sixel_b) {
                        let val = sixel_b - 0x3f;
                        let color = get_color(&palette, active_color);
                        for _ in 0..count {
                            if val != 0 {
                                for bit in 0..6 {
                                    if (val >> bit) & 1 == 1 {
                                        let y = band * 6 + bit;
                                        let x = col;
                                        if x < width && y < height {
                                            let offset = (y * width + x) * 4;
                                            rgba[offset..offset + 4].copy_from_slice(&color);
                                        }
                                    }
                                }
                            }
                            col = col.saturating_add(1);
                        }
                    }
                }
            }
            b'-' => {
                band = band.saturating_add(1);
                col = 0;
                idx += 1;
            }
            b'$' => {
                col = 0;
                idx += 1;
            }
            b'"' => {
                idx += 1;
                while idx < body.len() && (body[idx].is_ascii_digit() || body[idx] == b';') {
                    idx += 1;
                }
            }
            sixel_b if (0x3f..=0x7e).contains(&sixel_b) => {
                let val = sixel_b - 0x3f;
                let color = get_color(&palette, active_color);
                if val != 0 {
                    for bit in 0..6 {
                        if (val >> bit) & 1 == 1 {
                            let y = band * 6 + bit;
                            let x = col;
                            if x < width && y < height {
                                let offset = (y * width + x) * 4;
                                rgba[offset..offset + 4].copy_from_slice(&color);
                            }
                        }
                    }
                }
                col = col.saturating_add(1);
                idx += 1;
            }
            _ => {
                idx += 1;
            }
        }
    }

    Some(Image {
        width,
        height,
        rgba,
    })
}
