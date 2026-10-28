//! Almacén de imágenes y de sus colocaciones en la rejilla.
//!
//! El núcleo no pinta: guarda los píxeles, recuerda dónde va cada imagen y
//! entrega una lista resuelta a filas de pantalla para que la UI la dibuje. La
//! imagen comprimida (PNG) viaja tal cual hasta la UI, que es quien tiene el
//! decodificador; el sixel y el RGBA crudo se guardan ya en píxeles.
//!
//! Anclaje: una colocación de la pantalla primaria se ancla a una línea
//! absoluta del flujo, de modo que se desplaza con el texto; en la pantalla
//! alterna se ancla a la fila de pantalla y queda fija, que es lo que hacen los
//! programas de pantalla completa. Las líneas absolutas se cuentan con el
//! crecimiento del historial: al llenarse el scrollback (10 000 líneas) deja de
//! crecer, y desde ahí las imágenes dejan de seguir el desplazamiento. La
//! limitación es del contador y está anotada en la spec.

use std::collections::HashMap;
use std::sync::Arc;

use super::kitty::{Command, Delete, Format, Payload, Transmission};
use super::Image;

/// Tope por imagen: más que eso se descarta en vez de guardarse.
pub const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;
/// Tope de memoria del almacén completo.
pub const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;
/// Colocaciones simultáneas.
pub const MAX_PLACEMENTS: usize = 256;
/// Tope al leer un archivo que un programa pide mostrar.
pub const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
/// Los sixel no traen identificador: se numeran desde aquí para no chocar con
/// los que el programa asigne por el protocolo de kitty.
const SIXEL_ID_BASE: u32 = 1 << 31;

/// Formato comprimido que el núcleo no decodifica.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodedFormat {
    Png,
}

/// Píxeles de una imagen, tal como los necesita la UI.
#[derive(Clone, Debug, PartialEq)]
pub enum Pixels {
    /// RGBA de 8 bits por canal, ya decodificado.
    Rgba { width: usize, height: usize, data: Vec<u8> },
    /// Imagen comprimida: la UI la decodifica al pintar.
    Encoded { format: EncodedFormat, bytes: Vec<u8> },
}

impl Pixels {
    /// Tamaño en píxeles, si se puede conocer sin decodificar la imagen.
    pub fn dimensions(&self) -> Option<(usize, usize)> {
        match self {
            Pixels::Rgba { width, height, .. } => Some((*width, *height)),
            Pixels::Encoded {
                format: EncodedFormat::Png,
                bytes,
            } => png_dimensions(bytes),
        }
    }

    /// Bytes que ocupa en memoria.
    pub fn bytes(&self) -> usize {
        match self {
            Pixels::Rgba { data, .. } => data.len(),
            Pixels::Encoded { bytes, .. } => bytes.len(),
        }
    }
}

/// Una imagen colocada en la rejilla, ya resuelta a filas de pantalla.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedImage {
    pub image: u32,
    pub placement: u32,
    /// Fila de pantalla donde empieza. Negativa si empieza por encima del borde
    /// superior: la UI recorta.
    pub row: i32,
    pub column: usize,
    pub columns: usize,
    pub rows: usize,
    pub z: i32,
    pub pixels: Arc<Pixels>,
}

/// Dónde está el cursor cuando llega la secuencia.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Origin {
    /// Línea absoluta del flujo (crece al desplazarse la pantalla).
    pub line: i64,
    /// Fila de pantalla del cursor, desde cero.
    pub row: usize,
    /// Columna del cursor, desde cero.
    pub column: usize,
    /// Si el programa está en la pantalla alterna.
    pub alternate: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Anchor {
    Line(i64),
    Screen(usize),
}

struct StoredPlacement {
    image: u32,
    placement: u32,
    anchor: Anchor,
    column: usize,
    columns: usize,
    rows: usize,
    z: i32,
}

/// Lo que una secuencia gráfica deja al núcleo.
pub struct Effect {
    /// Bytes que el programa pidió recibir de vuelta.
    pub reply: Option<Vec<u8>>,
    /// Filas que el cursor baja tras pintar.
    pub rows: usize,
}

impl Effect {
    fn nothing() -> Self {
        Self {
            reply: None,
            rows: 0,
        }
    }
}

/// Almacén de imágenes y colocaciones.
pub struct Store {
    /// Bytes en tránsito de una transmisión partida, por identificador.
    chunked: HashMap<u32, Vec<u8>>,
    images: HashMap<u32, Arc<Pixels>>,
    /// Orden de llegada, para desalojar lo más viejo primero.
    order: Vec<u32>,
    total: usize,
    placements: Vec<StoredPlacement>,
    next_placement: u32,
    next_sixel: u32,
    cell_width: usize,
    cell_height: usize,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            chunked: HashMap::new(),
            images: HashMap::new(),
            order: Vec::new(),
            total: 0,
            placements: Vec::new(),
            next_placement: 1,
            next_sixel: 0,
            // Valores de arranque: la UI los corrige con su tamaño real de celda.
            cell_width: 8,
            cell_height: 18,
        }
    }
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    /// Tamaño real de una celda en píxeles, que fija la UI.
    pub fn set_cell_size(&mut self, width: usize, height: usize) {
        self.cell_width = width.max(1);
        self.cell_height = height.max(1);
    }

    /// Coloca un sixel en el cursor y devuelve las filas que ocupa.
    pub fn add_sixel(&mut self, image: Image, at: Origin) -> usize {
        self.next_sixel = self.next_sixel.wrapping_add(1);
        let id = SIXEL_ID_BASE.wrapping_add(self.next_sixel);
        let width = image.width;
        let height = image.height;
        let rows = self.ceil_rows(height).max(1);
        let columns = self.ceil_columns(width).max(1);
        let pixels = Pixels::Rgba {
            width,
            height,
            data: image.rgba,
        };
        self.insert_image(id, Arc::new(pixels));
        self.place(id, None, at, 0, 0, 0, Some(columns), Some(rows));
        rows
    }

    /// Aplica un comando de kitty graphics.
    pub fn apply(&mut self, command: Command, at: Origin) -> Effect {
        match command {
            Command::Transmit(transmission) => self.transmit(transmission, at),
            Command::Display(geometry) => {
                let mut effect = Effect::nothing();
                let Some(pixels) = self.images.get(&geometry.image).cloned() else {
                    return effect;
                };
                let (_, height) = pixels.dimensions().unwrap_or((0, 0));
                let rows = geometry.rows.unwrap_or_else(|| self.ceil_rows(height));
                self.place(
                    geometry.image,
                    geometry.placement,
                    at,
                    geometry.column_offset,
                    geometry.row_offset,
                    geometry.z,
                    geometry.columns,
                    Some(rows),
                );
                if !geometry.no_cursor_move {
                    effect.rows = rows;
                }
                effect
            }
            Command::Delete(delete) => {
                match delete {
                    Delete::All => {
                        self.placements.clear();
                        self.images.clear();
                        self.order.clear();
                        self.chunked.clear();
                        self.total = 0;
                    }
                    Delete::Image(image) => self.remove_image(image),
                    Delete::Placement { image, placement } => {
                        self.placements
                            .retain(|item| !(item.image == image && item.placement == placement));
                    }
                }
                Effect::nothing()
            }
            Command::Query { image } => Effect {
                reply: Command::Query { image }.reply(),
                rows: 0,
            },
            Command::Unsupported => Effect::nothing(),
        }
    }

    /// Colocaciones visibles en una pantalla que empieza en top_line.
    pub fn visible(&self, top_line: i64, screen_rows: usize) -> Vec<PlacedImage> {
        let mut out = Vec::new();
        for placement in &self.placements {
            let row = match placement.anchor {
                Anchor::Line(line) => line - top_line,
                Anchor::Screen(row) => row as i64,
            };
            if row + placement.rows as i64 <= 0 {
                continue;
            }
            if row >= screen_rows as i64 {
                continue;
            }
            let Some(pixels) = self.images.get(&placement.image) else {
                continue;
            };
            out.push(PlacedImage {
                image: placement.image,
                placement: placement.placement,
                row: row as i32,
                column: placement.column,
                columns: placement.columns,
                rows: placement.rows,
                z: placement.z,
                pixels: Arc::clone(pixels),
            });
        }
        out
    }

    /// Si hay alguna imagen colocada: la UI lo usa para no tocar su caché.
    pub fn has_placements(&self) -> bool {
        !self.placements.is_empty()
    }


    fn transmit(&mut self, transmission: Transmission, at: Origin) -> Effect {
        let mut effect = Effect::nothing();

        if transmission.more {
            // Llega partido: se acumula hasta el último trozo.
            let entry = self.chunked.entry(transmission.image).or_default();
            if let Payload::Inline(bytes) = &transmission.payload {
                entry.extend_from_slice(bytes);
            }
            if entry.len() > MAX_FILE_BYTES {
                self.chunked.remove(&transmission.image);
            }
            return effect;
        }

        let payload = match (&transmission.payload, self.chunked.remove(&transmission.image)) {
            (Payload::Inline(bytes), Some(mut pending)) => {
                pending.extend_from_slice(bytes);
                Payload::Inline(pending)
            }
            (other, _) => other.clone(),
        };

        let Some(pixels) = decode(
            transmission.format,
            payload,
            transmission.width,
            transmission.height,
        ) else {
            return effect;
        };
        if pixels.bytes() > MAX_IMAGE_BYTES {
            return effect;
        }

        let (_, height) = pixels.dimensions().unwrap_or((0, 0));
        self.insert_image(transmission.image, Arc::new(pixels));

        if let Some(geometry) = transmission.display {
            self.place(
                transmission.image,
                geometry.placement,
                at,
                geometry.column_offset,
                geometry.row_offset,
                geometry.z,
                geometry.columns,
                geometry
                    .rows
                    .or_else(|| (height > 0).then(|| self.ceil_rows(height))),
            );
            if !geometry.no_cursor_move {
                effect.rows = geometry.rows.unwrap_or_else(|| self.ceil_rows(height));
            }
        }
        if !transmission.quiet {
            effect.reply = Some(ok_reply(transmission.image));
        }
        effect
    }

    fn insert_image(&mut self, id: u32, pixels: Arc<Pixels>) {
        let bytes = pixels.bytes();
        if let Some(previous) = self.images.insert(id, pixels) {
            self.total = self.total.saturating_sub(previous.bytes());
        } else {
            self.order.push(id);
        }
        self.total += bytes;
        while self.total > MAX_TOTAL_BYTES && !self.order.is_empty() {
            let oldest = self.order.remove(0);
            self.remove_image(oldest);
        }
    }

    fn remove_image(&mut self, id: u32) {
        if let Some(pixels) = self.images.remove(&id) {
            self.total = self.total.saturating_sub(pixels.bytes());
        }
        self.order.retain(|item| *item != id);
        self.placements.retain(|item| item.image != id);
    }

    #[allow(clippy::too_many_arguments)]
    fn place(
        &mut self,
        image: u32,
        placement: Option<u32>,
        at: Origin,
        column_offset: usize,
        row_offset: usize,
        z: i32,
        columns: Option<usize>,
        rows: Option<usize>,
    ) {
        let Some(pixels) = self.images.get(&image).cloned() else {
            return;
        };
        if self.placements.len() >= MAX_PLACEMENTS {
            self.placements.remove(0);
        }
        let (pixel_width, pixel_height) = pixels.dimensions().unwrap_or((0, 0));
        let columns = columns
            .unwrap_or_else(|| self.ceil_columns(pixel_width))
            .max(1);
        let rows = rows
            .unwrap_or_else(|| self.ceil_rows(pixel_height))
            .max(1);
        let anchor = if at.alternate {
            Anchor::Screen(at.row.saturating_add(row_offset))
        } else {
            Anchor::Line(at.line + row_offset as i64)
        };
        let placement_id = match placement {
            Some(id) => id,
            None => {
                let id = self.next_placement;
                self.next_placement = self.next_placement.wrapping_add(1).max(1);
                id
            }
        };
        self.placements
            .retain(|item| !(item.image == image && item.placement == placement_id));
        self.placements.push(StoredPlacement {
            image,
            placement: placement_id,
            anchor,
            column: at.column.saturating_add(column_offset),
            columns,
            rows,
            z,
        });
    }

    fn ceil_columns(&self, pixels: usize) -> usize {
        pixels.div_ceil(self.cell_width)
    }

    fn ceil_rows(&self, pixels: usize) -> usize {
        pixels.div_ceil(self.cell_height)
    }
}


/// Convierte la carga recibida en píxeles o en una imagen comprimida.
///
/// Para los formatos crudos el protocolo declara el ancho y el alto; si el
/// bloque no mide exactamente lo declarado, no hay imagen: adivinar sería pintar
/// basura.
fn decode(format: Format, payload: Payload, width: usize, height: usize) -> Option<Pixels> {
    let bytes = match payload {
        Payload::Inline(bytes) => bytes,
        Payload::File(path) | Payload::TempFile(path) => {
            let metadata = std::fs::metadata(&path).ok()?;
            if metadata.len() > MAX_FILE_BYTES as u64 {
                return None;
            }
            std::fs::read(&path).ok()?
        }
    };
    if bytes.len() > MAX_FILE_BYTES {
        return None;
    }
    match format {
        Format::Png => Some(Pixels::Encoded {
            format: EncodedFormat::Png,
            bytes,
        }),
        Format::Rgba => {
            if width == 0 || height == 0 || bytes.len() != width * height * 4 {
                return None;
            }
            Some(Pixels::Rgba {
                width,
                height,
                data: bytes,
            })
        }
        Format::Rgb => {
            if width == 0 || height == 0 || bytes.len() != width * height * 3 {
                return None;
            }
            let mut data = Vec::with_capacity(width * height * 4);
            for pixel in bytes.chunks_exact(3) {
                data.extend_from_slice(pixel);
                data.push(255);
            }
            Some(Pixels::Rgba {
                width,
                height,
                data,
            })
        }
    }
}

/// Respuesta de aceptación que kitty espera tras una transmisión.
fn ok_reply(image: u32) -> Vec<u8> {
    let mut reply = Vec::new();
    reply.extend_from_slice(b"\x1b_Gi=");
    reply.extend_from_slice(image.to_string().as_bytes());
    reply.extend_from_slice(b";OK\x1b");
    reply.push(b'\\');
    reply
}

/// Lee ancho y alto de la cabecera IHDR de un PNG.
///
/// Solo la cabecera: decodificar los píxeles es trabajo de la UI, pero la
/// geometría hace falta aquí para saber cuántas celdas ocupa.
fn png_dimensions(bytes: &[u8]) -> Option<(usize, usize)> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || &bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?) as usize;
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?) as usize;
    if width == 0 || height == 0 {
        return None;
    }
    Some((width, height))
}

