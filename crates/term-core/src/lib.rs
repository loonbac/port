//! Superficie pública de `port-term-core`.
//!
//! Este crate es la terminal, sin ninguna dependencia de interfaz gráfica. La
//! aplicación (GPUI) consume únicamente lo que se reexporta aquí, de modo que un
//! cambio en GPUI o en `alacritty_terminal` no obliga a tocar la UI.
//!
//! Capacidades, cada una con su propia razón para cambiar:
//!
//! - [`input`]: traducción de una tecla a los bytes que espera el programa del
//!   otro lado del PTY. Es lógica pura y headless, así que se prueba sola.
//! - [`frame`]: los datos que la UI necesita para pintar (filas, runs y color).
//!   Aísla a la UI de los tipos de `alacritty_terminal`.
//! - [`pty`]: el proceso de shell y su tubería. Adaptador del sistema operativo.
//! - [`session`]: caso de uso que coordina PTY, rejilla y viewport.

pub mod frame;
pub mod input;
pub mod pty;
pub mod session;
