//! Caché de imágenes listas para pintar.
//!
//! Convertir píxeles a una imagen de GPUI cuesta una copia y, en PNG, una
//! decodificación. El cuadro se reconstruye en cada refresco, así que la
//! conversión se hace una vez por imagen y se reutiliza mientras el búfer de
//! píxeles sea el mismo: la identidad se toma del puntero del Arc que entrega el
//! núcleo, de modo que una imagen reemplazada se vuelve a construir sola.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::RenderImage;
use port_term_core::frame::{Frame, PlacedImage};
use port_term_core::graphics::store::{EncodedFormat, Pixels};

/// Identidad del búfer de píxeles que entrega el núcleo.
type SourceId = usize;

/// Imágenes ya convertidas, por identificador de imagen del protocolo.
#[derive(Default)]
pub(crate) struct ImageCache {
    entries: HashMap<u32, (SourceId, Arc<RenderImage>)>,
}

impl ImageCache {
    /// Imagen de GPUI para esta colocación, construida la primera vez.
    pub fn render(&mut self, placed: &PlacedImage) -> Option<Arc<RenderImage>> {
        let source = Arc::as_ptr(&placed.pixels) as SourceId;
        if let Some((cached, render)) = self.entries.get(&placed.image) {
            if *cached == source {
                return Some(Arc::clone(render));
            }
        }
        let render = build(placed.pixels.as_ref())?;
        self.entries
            .insert(placed.image, (source, Arc::clone(&render)));
        Some(render)
    }

    /// Suelta lo que ya no está en el cuadro.
    pub fn retain(&mut self, frame: &Frame) {
        if frame.images.is_empty() {
            self.entries.clear();
            return;
        }
        let live: HashSet<u32> = frame.images.iter().map(|placed| placed.image).collect();
        self.entries.retain(|id, _| live.contains(id));
    }
}

/// Convierte los píxeles del núcleo en la imagen que espera GPUI.
fn build(pixels: &Pixels) -> Option<Arc<RenderImage>> {
    match pixels {
        Pixels::Encoded {
            format: EncodedFormat::Png,
            bytes,
        } => {
            let decoded =
                image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?;
            let buffer = to_bgra(&decoded.to_rgba8());
            Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
        }
        Pixels::Rgba {
            width,
            height,
            data,
        } => {
            let source = image::RgbaImage::from_raw(*width as u32, *height as u32, data.to_vec())?;
            let buffer = to_bgra(&source);
            Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
        }
    }
}

/// Reordena RGBA a BGRA y premultiplica el alfa.
///
/// GPUI pinta con mezcla premultiplicada, así que una imagen con transparencia
/// (un PNG de kitty graphics, por ejemplo) necesita el alfa aplicado al color;
/// para una imagen opaca la cuenta es la identidad. El intercambio de canales es
/// porque el núcleo entrega RGBA y GPUI espera BGRA.
fn to_bgra(source: &image::RgbaImage) -> image::RgbaImage {
    let (width, height) = (source.width(), source.height());
    let mut out = Vec::with_capacity(source.as_raw().len());
    for pixel in source.as_raw().as_chunks::<4>().0 {
        let (r, g, b, a) = (
            pixel[0] as u32,
            pixel[1] as u32,
            pixel[2] as u32,
            pixel[3] as u32,
        );
        out.extend_from_slice(&[
            ((b * a) / 255) as u8,
            ((g * a) / 255) as u8,
            ((r * a) / 255) as u8,
            a as u8,
        ]);
    }
    image::RgbaImage::from_raw(width, height, out)
        .unwrap_or_else(|| image::RgbaImage::new(width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_pixels_become_premultiplied_bgra() {
        let source = image::RgbaImage::from_raw(2, 1, vec![10, 20, 30, 255, 200, 100, 50, 128])
            .expect("búfer 2x1");

        let converted = to_bgra(&source);

        assert_eq!(converted.dimensions(), (2, 1));
        // Opaco: solo se intercambian los canales de color.
        assert_eq!(&converted.as_raw()[0..4], &[30, 20, 10, 255]);
        // Semitransparente: el alfa se aplica al color.
        assert_eq!(&converted.as_raw()[4..8], &[25, 50, 100, 128]);
    }

    #[test]
    fn an_empty_buffer_converts_without_panicking() {
        let converted = to_bgra(&image::RgbaImage::new(0, 0));
        assert!(converted.as_raw().is_empty());
    }
}
