//! Modelo de selección de la sesión.
//!
//! Traduce las celdas del viewport a coordenadas absolutas del buffer de la
//! rejilla ([`cell_to_point`]) y expone el ciclo de vida de la selección:
//! anclar, extender, borrar y extraer el texto. La conversión es pura para
//! poder verificarla sin PTY.

use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::viewport_to_point;

use super::{CellPos, Session};

/// Tipo de selección pedido por el gesto del ratón.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionKind {
    /// Celda a celda, sin expansión.
    Simple,
    /// Se expande a la palabra completa bajo el puntero.
    Word,
    /// Selecciona líneas enteras.
    Line,
}

/// Convierte una celda del viewport (1-based) en la coordenada absoluta del
/// buffer de la rejilla.
///
/// La celda se recorta a la rejilla visible antes de convertir, de modo que un
/// arrastre más allá del borde nunca produce un punto fuera de rango. La
/// coordenada resultante es absoluta: `viewport_to_point` le resta el
/// desplazamiento del viewport, así que sobrevive al scroll.
///
/// Es pura para poder verificar la conversión sin PTY.
pub(crate) fn cell_to_point(
    display_offset: usize,
    columns: usize,
    rows: usize,
    cell: CellPos,
) -> Point {
    let column = usize::from(cell.column.saturating_sub(1)).min(columns.saturating_sub(1));
    let row = usize::from(cell.row.saturating_sub(1)).min(rows.saturating_sub(1));
    viewport_to_point(display_offset, Point::new(row, Column(column)))
}

impl Session {
    /// Convierte una celda del viewport a la coordenada absoluta de la rejilla.
    ///
    /// Se apoya en [`cell_to_point`] con el desplazamiento actual del viewport y
    /// el tamaño de la sesión.
    fn point_at(&self, cell: CellPos) -> Point {
        let display_offset = self.term.grid().display_offset();
        cell_to_point(display_offset, self.size.columns, self.size.rows, cell)
    }

    /// Inicia una selección anclada en la celda dada.
    ///
    /// El ancla usa el lado izquierdo y cada extensión el lado derecho, que es
    /// como alacritty delimita las celdas cubiertas.
    pub fn start_selection(&mut self, cell: CellPos, kind: SelectionKind) {
        let point = self.point_at(cell);
        let ty = match kind {
            SelectionKind::Simple => SelectionType::Simple,
            SelectionKind::Word => SelectionType::Semantic,
            SelectionKind::Line => SelectionType::Lines,
        };
        self.term.selection = Some(Selection::new(ty, point, Side::Left));
        self.dirty.set(true);
    }

    /// Extiende la selección activa hasta la celda dada.
    ///
    /// Sin selección activa no hace nada: una extensión suelta no crea una
    /// selección desde cero.
    pub fn extend_selection(&mut self, cell: CellPos) {
        let point = self.point_at(cell);
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, Side::Right);
            self.dirty.set(true);
        }
    }

    /// Borra la selección activa.
    pub fn clear_selection(&mut self) {
        if self.term.selection.take().is_some() {
            self.dirty.set(true);
        }
    }

    /// `true` si hay una selección activa y no vacía.
    pub fn has_selection(&self) -> bool {
        self.term
            .selection
            .as_ref()
            .is_some_and(|selection| !selection.is_empty())
    }

    /// Texto de la selección activa, si la hay.
    ///
    /// El recorte, los saltos de línea y los cuatro tipos los resuelve
    /// `alacritty_terminal`; aquí no se reimplementa la extracción.
    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{cell_to_point, CellPos};
    use alacritty_terminal::index::{Column, Line, Point};

    /// Sin desplazamiento del viewport, la fila 1 de pantalla es la línea 0 del
    /// buffer y la columna 1 es la columna 0: la conversión es 1-based → 0-based.
    #[test]
    fn la_conversion_sin_desplazamiento_mapea_el_viewport_al_buffer() {
        assert_eq!(
            cell_to_point(0, 80, 24, CellPos::new(1, 1)),
            Point::new(Line(0), Column(0))
        );
        assert_eq!(
            cell_to_point(0, 80, 24, CellPos::new(5, 3)),
            Point::new(Line(2), Column(4))
        );
    }

    /// Con el viewport desplazado sobre el historial, la misma fila de pantalla
    /// apunta a una línea anterior del buffer: la conversión es absoluta.
    #[test]
    fn la_conversion_con_desplazamiento_mueve_la_linea_del_buffer() {
        assert_eq!(
            cell_to_point(3, 80, 24, CellPos::new(1, 1)),
            Point::new(Line(-3), Column(0))
        );
    }

    /// Una celda fuera de la rejilla se recorta antes de convertir, para que un
    /// arrastre más allá del borde no genere un punto fuera del buffer.
    #[test]
    fn la_conversion_recorta_las_celdas_fuera_de_la_rejilla() {
        assert_eq!(
            cell_to_point(0, 10, 4, CellPos::new(99, 99)),
            Point::new(Line(3), Column(9))
        );
    }
}
