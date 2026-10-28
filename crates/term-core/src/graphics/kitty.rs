//! Parser del protocolo Kitty Graphics (`ESC _ G ... ST`).
//!
//! Este módulo implementa la interpretación de secuencias APC con prefijo `'G'`
//! según la especificación del protocolo de gráficos de Kitty. Extrae comandos
//! de transmisión, visualización, eliminación y consulta sin interactuar con la UI
//! ni decodificar formatos de imagen como PNG (responsabilidad de capas superiores).

/// Formato de los datos de píxeles transmitidos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Rgba,
    Rgb,
    Png,
}

/// Medio o fuente de los datos de imagen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payload {
    Inline(Vec<u8>),
    File(String),
    TempFile(String),
}

/// Parámetros de colocación y disposición de una imagen en la rejilla de celdas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub image: u32,
    pub placement: Option<u32>,
    pub columns: Option<usize>,
    pub rows: Option<usize>,
    pub column_offset: usize,
    pub row_offset: usize,
    pub z: i32,
    pub no_cursor_move: bool,
}

/// Comando de transmisión de imagen a la memoria de la terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transmission {
    pub image: u32,
    pub format: Format,
    pub width: usize,
    pub height: usize,
    pub payload: Payload,
    pub more: bool,
    pub quiet: bool,
    pub display: Option<Placement>,
}

/// Criterio de eliminación de imágenes o colocaciones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delete {
    All,
    Image(u32),
    Placement { image: u32, placement: u32 },
}

/// Comandos reconocidos del protocolo Kitty Graphics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Transmit(Transmission),
    Display(Placement),
    Delete(Delete),
    Query { image: Option<u32> },
    Unsupported,
}

impl Command {
    /// Parsea la carga útil de una secuencia APC de Kitty (comenzando con `'G'`).
    ///
    /// Cualquier secuencia mal formada, con acción desconocida o que carezca de
    /// campos obligatorios retorna [`Command::Unsupported`] y nunca entra en pánico.
    pub fn parse(payload: &[u8]) -> Command {
        // La secuencia APC de Kitty debe comenzar obligatoriamente con el byte 'G'
        if payload.first() != Some(&b'G') {
            return Command::Unsupported;
        }

        let after_g = &payload[1..];

        // Los datos de control van desde 'G' hasta el primer ';'.
        // Todo lo que sigue al primer ';' es la carga útil de datos.
        let (control_bytes, data_bytes) = match after_g.iter().position(|&b| b == b';') {
            Some(pos) => (&after_g[..pos], Some(&after_g[pos + 1..])),
            None => (after_g, None),
        };

        // El bloque de control debe ser UTF-8 válido (claves y valores ASCII)
        let control_str = match std::str::from_utf8(control_bytes) {
            Ok(s) => s,
            Err(_) => return Command::Unsupported,
        };

        let mut action: Option<&str> = None;
        let mut format_val: Option<&str> = None;
        let mut medium: Option<&str> = None;
        let mut image_id: Option<u32> = None;
        let mut placement_id: Option<u32> = None;
        let mut pixel_width: Option<usize> = None;
        let mut pixel_height: Option<usize> = None;
        let mut columns: Option<usize> = None;
        let mut rows: Option<usize> = None;
        let mut column_offset: usize = 0;
        let mut row_offset: usize = 0;
        let mut no_cursor_move = false;
        let mut z_index: i32 = 0;
        let mut more = false;
        let mut quiet = false;
        let mut delete_val: Option<&str> = None;

        if !control_str.is_empty() {
            for pair in control_str.split(',') {
                if pair.is_empty() {
                    continue;
                }
                let (key, val) = match pair.split_once('=') {
                    Some(kv) => kv,
                    None => return Command::Unsupported,
                };

                match key {
                    "a" => action = Some(val),
                    "f" => format_val = Some(val),
                    "t" => medium = Some(val),
                    "i" => match val.parse::<u32>() {
                        Ok(id) => image_id = Some(id),
                        Err(_) => return Command::Unsupported,
                    },
                    "p" => match val.parse::<u32>() {
                        Ok(p) => placement_id = Some(p),
                        Err(_) => return Command::Unsupported,
                    },
                    "s" => match val.parse::<usize>() {
                        Ok(w) => pixel_width = Some(w),
                        Err(_) => return Command::Unsupported,
                    },
                    "v" => match val.parse::<usize>() {
                        Ok(h) => pixel_height = Some(h),
                        Err(_) => return Command::Unsupported,
                    },
                    "c" => match val.parse::<usize>() {
                        Ok(c) => columns = Some(c),
                        Err(_) => return Command::Unsupported,
                    },
                    "r" => match val.parse::<usize>() {
                        Ok(r) => rows = Some(r),
                        Err(_) => return Command::Unsupported,
                    },
                    "C" => match val.parse::<u8>() {
                        Ok(1) => no_cursor_move = true,
                        Ok(0) => no_cursor_move = false,
                        _ => return Command::Unsupported,
                    },
                    "z" => match val.parse::<i32>() {
                        Ok(z) => z_index = z,
                        Err(_) => return Command::Unsupported,
                    },
                    "m" => match val.parse::<u8>() {
                        Ok(1) => more = true,
                        Ok(0) => more = false,
                        _ => return Command::Unsupported,
                    },
                    "q" => match val.parse::<u8>() {
                        Ok(2) => quiet = true,
                        Ok(_) => quiet = false,
                        Err(_) => return Command::Unsupported,
                    },
                    "d" => delete_val = Some(val),
                    "X" => match val.parse::<usize>() {
                        Ok(x) => column_offset = x,
                        Err(_) => return Command::Unsupported,
                    },
                    "Y" => match val.parse::<usize>() {
                        Ok(y) => row_offset = y,
                        Err(_) => return Command::Unsupported,
                    },
                    "x" | "y" => {
                        // Desplazamientos en píxeles: ignorables en esta capa
                    }
                    _ => {
                        // Claves no reconocidas se ignoran para mantener compatibilidad futura
                    }
                }
            }
        }

        // Resolución de la acción:
        // - 'a=T' es el valor por defecto cuando 'a' está ausente pero hay datos (hay ';' con datos no vacíos).
        // - Si 'a' está ausente y no hay datos, no es un comando ejecutable válido.
        let effective_action = match (action, data_bytes) {
            (Some(a), _) => a,
            (None, Some(d)) if !d.is_empty() => "T",
            (None, _) => return Command::Unsupported,
        };

        match effective_action {
            "t" | "T" => {
                let format = match format_val {
                    None | Some("32") => Format::Rgba,
                    Some("24") => Format::Rgb,
                    Some("100") => Format::Png,
                    Some(_) => return Command::Unsupported,
                };

                let raw_data = match data_bytes {
                    Some(d) if !d.is_empty() => d,
                    _ => return Command::Unsupported,
                };

                let payload_medium = match medium {
                    None | Some("d") => match decode_base64(raw_data) {
                        Some(bytes) => Payload::Inline(bytes),
                        None => return Command::Unsupported,
                    },
                    Some("f") => Payload::File(String::from_utf8_lossy(raw_data).into_owned()),
                    Some("t") => Payload::TempFile(String::from_utf8_lossy(raw_data).into_owned()),
                    Some(_) => return Command::Unsupported,
                };

                let image = image_id.unwrap_or(0);
                let display = if effective_action == "T" {
                    Some(Placement {
                        image,
                        placement: placement_id,
                        columns,
                        rows,
                        column_offset,
                        row_offset,
                        z: z_index,
                        no_cursor_move,
                    })
                } else {
                    None
                };

                Command::Transmit(Transmission {
                    image,
                    format,
                    width: pixel_width.unwrap_or(0),
                    height: pixel_height.unwrap_or(0),
                    payload: payload_medium,
                    more,
                    quiet,
                    display,
                })
            }
            "p" => {
                // a=p produce Display con image tomado obligatoriamente de 'i'
                let image = match image_id {
                    Some(id) => id,
                    None => return Command::Unsupported,
                };

                Command::Display(Placement {
                    image,
                    placement: placement_id,
                    columns,
                    rows,
                    column_offset,
                    row_offset,
                    z: z_index,
                    no_cursor_move,
                })
            }
            "d" => {
                // a=d produce Delete: por defecto (ausente o 'd=a') es All;
                // 'd=i' es Image(i); 'd=p' es Placement { image, placement }.
                let delete = match delete_val {
                    None | Some("a") => Delete::All,
                    Some("i") => match image_id {
                        Some(id) => Delete::Image(id),
                        None => return Command::Unsupported,
                    },
                    Some("p") => match (image_id, placement_id) {
                        (Some(img), Some(plc)) => Delete::Placement {
                            image: img,
                            placement: plc,
                        },
                        _ => return Command::Unsupported,
                    },
                    Some(_) => return Command::Unsupported,
                };

                Command::Delete(delete)
            }
            "q" => {
                // a=q produce Query { image: Option<u32> }
                Command::Query { image: image_id }
            }
            _ => Command::Unsupported,
        }
    }

    /// Genera la respuesta sincrónica solicitada por el emisor (por ejemplo para `a=q`).
    ///
    /// Todo comando excepto [`Command::Query`] devuelve `None` en cumplimiento
    /// del principio S7 (sin respuestas no solicitadas hacia el PTY).
    pub fn reply(&self) -> Option<Vec<u8>> {
        match self {
            Command::Query { image } => {
                let id = image.unwrap_or(0);
                Some(format!("\x1b_Gi={id};OK\x1b\\").into_bytes())
            }
            _ => None,
        }
    }
}

/// Decodificador Base64 manual tolerante a espacios y variantes con o sin relleno.
///
/// Soporta:
/// - Espacios en blanco intercalados (`\n`, `\r`, `\t`, `' '`).
/// - Entradas con relleno completo (`=`, `==`) y sin relleno (longitud 4k+2 o 4k+3).
/// - Rechaza caracteres no pertenecientes al alfabeto Base64 estándar sin entrar en pánico.
fn decode_base64(input: &[u8]) -> Option<Vec<u8>> {
    let mut sextets = Vec::with_capacity(input.len());
    let mut padding_count = 0;
    let mut saw_padding = false;

    for &b in input {
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'=' {
            saw_padding = true;
            padding_count += 1;
            if padding_count > 2 {
                return None;
            }
            continue;
        }
        // Si ya apareció '=', no puede haber más caracteres de datos después
        if saw_padding {
            return None;
        }

        let sextet = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        sextets.push(sextet);
    }

    let len = sextets.len();
    if padding_count == 2 {
        if len % 4 != 2 {
            return None;
        }
    } else if padding_count == 1 {
        if len % 4 != 3 {
            return None;
        }
    } else if len % 4 == 1 {
        // Un único sexteto contiene solo 6 bits: insuficiente para reconstruir un byte
        return None;
    }

    let num_quads = len / 4;
    let remainder = len % 4;
    let est_capacity = num_quads * 3
        + match remainder {
            2 => 1,
            3 => 2,
            _ => 0,
        };
    let mut out = Vec::with_capacity(est_capacity);

    for chunk in sextets[..num_quads * 4].as_chunks::<4>().0 {
        let (s0, s1, s2, s3) = (chunk[0], chunk[1], chunk[2], chunk[3]);
        out.push((s0 << 2) | (s1 >> 4));
        out.push(((s1 & 0x0f) << 4) | (s2 >> 2));
        out.push(((s2 & 0x03) << 6) | s3);
    }

    let rem_sextets = &sextets[num_quads * 4..];
    match remainder {
        2 => {
            let (s0, s1) = (rem_sextets[0], rem_sextets[1]);
            out.push((s0 << 2) | (s1 >> 4));
        }
        3 => {
            let (s0, s1, s2) = (rem_sextets[0], rem_sextets[1], rem_sextets[2]);
            out.push((s0 << 2) | (s1 >> 4));
            out.push(((s1 & 0x0f) << 4) | (s2 >> 2));
        }
        _ => {}
    }

    Some(out)
}
