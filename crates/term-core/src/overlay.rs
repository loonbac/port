//! Capa de depuración que el programa enciende por OSC 556.
//!
//! La terminal no adivina nada: sin secuencia no hay capa, y es el programa
//! quien decide qué quiere ver. El vocabulario es el de Velocitty (`grid`,
//! `hud`, `on`/`all`, `off`) para que el mismo script de depuración sirva en
//! ambos terminales.

/// Capas de depuración encendidas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    /// Caja con los contadores del cuadro en la esquina superior.
    pub hud: bool,
    /// Líneas finas en cada borde de celda.
    pub grid: bool,
}

impl Flags {
    /// Ninguna capa: el estado normal.
    pub const NONE: Self = Self {
        hud: false,
        grid: false,
    };

    /// Todas las capas.
    pub const ALL: Self = Self {
        hud: true,
        grid: true,
    };

    /// `true` si hay algo que pintar, para no recorrer el cuadro por gusto.
    pub fn any(self) -> bool {
        self.hud || self.grid
    }

    /// Interpreta el payload de OSC 556.
    ///
    /// Vacío, `on`, `all` o `1` encienden todo; `off` o `0` lo apagan; cualquier
    /// otra cosa es una lista de capas separada por comas, punto y coma o
    /// espacios. Una capa desconocida no enciende nada: un terminal no inventa
    /// comportamiento a partir de texto que no entiende.
    pub fn parse(payload: &str) -> Self {
        let trimmed = payload.trim();
        if trimmed.is_empty() {
            return Self::ALL;
        }
        let lower = trimmed.to_ascii_lowercase();
        match lower.as_str() {
            "on" | "all" | "1" => return Self::ALL,
            "off" | "0" => return Self::NONE,
            _ => {}
        }
        let mut flags = Self::NONE;
        for token in lower.split([',', ';', ' ', '\t']) {
            match token {
                "hud" => flags.hud = true,
                "grid" => flags.grid = true,
                "on" | "all" => return Self::ALL,
                "off" => return Self::NONE,
                _ => {}
            }
        }
        flags
    }
}

/// Contadores del cuadro que muestra el HUD.
///
/// Son hechos medidos, no estimaciones: lo que la cola del PTY tiene pendiente,
/// lo que el tope descartó, cuántas imágenes siguen colocadas y en qué parte del
/// scrollback está la vista.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub columns: usize,
    pub rows: usize,
    pub pending_bytes: usize,
    pub dropped_bytes: u64,
    pub images: usize,
    pub scrollback: usize,
    pub alternate: bool,
}

/// Estado de depuración de un cuadro completo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub flags: Flags,
    pub stats: Stats,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_payload_turns_every_layer_on() {
        assert_eq!(Flags::parse(""), Flags::ALL);
        assert_eq!(Flags::parse("  "), Flags::ALL);
    }

    #[test]
    fn off_is_off_whatever_the_case() {
        assert_eq!(Flags::parse("OFF"), Flags::NONE);
        assert_eq!(Flags::parse(" 0 "), Flags::NONE);
    }

    #[test]
    fn a_list_turns_on_only_the_named_layers() {
        let flags = Flags::parse("grid, hud");
        assert!(flags.grid && flags.hud);
        let only_hud = Flags::parse("hud");
        assert!(only_hud.hud && !only_hud.grid);
    }

    #[test]
    fn an_unknown_layer_turns_on_nothing() {
        assert_eq!(Flags::parse("lasers"), Flags::NONE);
    }
}
