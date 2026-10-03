//! Geometría de la celda de texto.
//!
//! Vive aparte de la vista porque es una cuenta, no un comportamiento de pintado:
//! el tamaño de la fuente decide cuántas columnas y filas caben, y esa respuesta
//! la necesita tanto el pintado como el `resize` del PTY.

use gpui::SharedString;
use port_term_core::session::GridSize;

/// Fuente monoespaciada y altura de línea.
///
/// Fira Code tiene un avance fijo de 0.6 em, que es justo lo que hace que un
/// ancho de celda constante capaz de alinear columnas sin medir cada glifo.
const FONT_FAMILY: &str = "Fira Code";
/// Avance horizontal en milésimas de em.
const ADVANCE: f32 = 600.0 / 1000.0;
/// Alto de celda respecto del tamaño de fuente.
const LINE_HEIGHT: f32 = 1.35;

#[derive(Clone, Debug)]
pub struct Metrics {
    #[allow(dead_code)]
    pub font_family: SharedString,
    pub font_size: f32,
    /// Ancho de una celda en píxeles lógicos.
    pub cell_width: f32,
    /// Alto de una celda en píxeles lógicos.
    pub cell_height: f32,
    /// Margen interior de la ventana, en píxeles lógicos.
    ///
    /// El texto no se pega al borde: sin este margen el primer carácter queda
    /// pegado a la barra de la ventana y la rejilla parece más ancha de lo que es.
    pub padding: f32,
}

impl Metrics {
    pub fn new(font_size: f32, padding: f32) -> Self {
        Self {
            font_family: FONT_FAMILY.into(),
            font_size,
            cell_width: font_size * ADVANCE,
            cell_height: (font_size * LINE_HEIGHT).round(),
            padding,
        }
    }

    /// Área útil: la ventana menos el margen de los cuatro bordes.
    ///
    /// Nunca baja de una celda: una ventana más pequeña que su propio margen
    /// debe seguir mostrando texto, no un área vacía.
    pub fn content_size(&self, width: f32, height: f32) -> (f32, f32) {
        let usable_width = (width - 2.0 * self.padding).max(self.cell_width);
        let usable_height = (height - 2.0 * self.padding).max(self.cell_height);
        (usable_width, usable_height)
    }

    /// Cuántas celdas caben en un área de píxeles lógicos.
    ///
    /// Nunca devuelve cero: una rejilla sin filas ni columnas no puede ni
    /// representar el shell que la usa.
    pub fn grid_for(&self, width: f32, height: f32) -> GridSize {
        let (usable_width, usable_height) = self.content_size(width, height);
        GridSize::new(
            ((usable_width / self.cell_width) as usize).max(1),
            ((usable_height / self.cell_height) as usize).max(1),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_width_follows_the_font_advance() {
        let metrics = Metrics::new(14.0, 0.0);
        assert!((metrics.cell_width - 8.4).abs() < 0.001);
        assert_eq!(metrics.cell_height, 19.0);
    }

    #[test]
    fn grid_counts_whole_cells_and_ignores_the_remainder() {
        let metrics = Metrics::new(14.0, 0.0);
        // 8.4 px de ancho y 19 px de alto por celda.
        let grid = metrics.grid_for(8.4 * 80.0 + 3.0, 19.0 * 24.0 + 5.0);
        assert_eq!(grid.columns, 80);
        assert_eq!(grid.rows, 24);
    }

    #[test]
    fn padding_takes_space_away_from_the_grid() {
        let without = Metrics::new(14.0, 0.0);
        let with = Metrics::new(14.0, 10.0);
        let (width, height) = (8.4 * 100.0, 19.0 * 40.0);

        // 840 - 20 = 820 px útiles, que son 97 celdas de 8.4 y no 98.
        assert_eq!(with.grid_for(width, height).columns, 97);
        assert_eq!(with.grid_for(width, height).rows, 38);
        assert!(
            with.grid_for(width, height).columns < without.grid_for(width, height).columns,
            "el margen debe reducir la rejilla, no solo dibujarse"
        );
    }

    #[test]
    fn content_size_never_goes_below_one_cell() {
        let metrics = Metrics::new(14.0, 10.0);
        let (width, height) = metrics.content_size(4.0, 4.0);
        assert_eq!(width, metrics.cell_width);
        assert_eq!(height, metrics.cell_height);
        assert_eq!(metrics.grid_for(4.0, 4.0).columns, 1);
    }

    #[test]
    fn a_window_too_small_still_gets_a_usable_grid() {
        let metrics = Metrics::new(14.0, 0.0);
        let grid = metrics.grid_for(1.0, 1.0);
        assert_eq!(grid.columns, 1);
        assert_eq!(grid.rows, 1);
    }
}
