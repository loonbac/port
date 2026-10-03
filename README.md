# PORT — Plugin-Oriented Rust Terminal

Una terminal construida en Rust y GPUI, diseñada con estricta separación modular:
- `port-term-core`: Núcleo desacoplado de UI (PTY con hilo no bloqueante, emulación VT vía `alacritty_terminal`, traducción a `Frame` y codificación de teclado).
- `port-app`: Frontend GPUI ligero con pintado directo sobre canvas (`shape_line`) y captura global de teclado.

## Desarrollo

Requiere Nix o toolchain de Rust con dependencias de Wayland/Vulkan:

```bash
# Entrar al entorno con librerías nativas configuradas
nix-shell

# Ejecutar suite de pruebas
cargo test

# Compilar y ejecutar
cargo run -p port
```
