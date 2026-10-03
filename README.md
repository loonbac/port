# PORT — Plugin-Oriented Rust Terminal

A terminal emulator written in Rust on [GPUI](https://github.com/zed-industries/zed), built around one idea: **every core behaviour can be replaced by a plugin, without touching the terminal core.**

```
┌─ port ───────────────────────────────┐
│ crates/term-core    PTY · VT grid ·   │
│                     input encoding   │
│ crates/plugin-api  public contracts  │
│ crates/port-app    GPUI frontend     │
└──────────────────────────────────────┘
        ▲ everything above is extensible
```

---

## English

### What it is

PORT is a from-scratch terminal emulator with a plugin-first architecture. The
emulation core is isolated and UI-free; every part of the interface and every
policy decision lives behind a trait in `port-plugin-api`.

| Crate | Responsibility | Knows about |
|---|---|---|
| `port-term-core` | PTY lifecycle, VT emulation, frame building, key encoding | Nothing from the UI |
| `port-plugin-api` | Public contracts, configuration file, plugin registry | `term-core` only |
| `port-app` | GPUI window, renderer, key routing | Both of the above |

### Features

- **Terminal emulation** — `alacritty_terminal` VT parser, scrollback, alternate
  screen, SGR attributes, wide characters.
- **True rendering performance** — cells are batched into runs, Braille and block
  elements are rasterised as GPU geometry instead of being shaped as glyphs, and
  frames are only rebuilt when the grid actually changes.
- **Multi-session** — every space and every tab owns an isolated PTY; background
  jobs keep running when you switch away.
- **Plugin system** — in-process Rust traits. Appearance, input, layout, session
  management, window lifecycle.
- **Human-readable configuration** — one plain-text file with Markdown code
  blocks per plugin, hot-reloaded as you save.

### Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl` `Shift` `L` | Open the plugin manager |
| `Ctrl` `Shift` `T` | New tab in the current space |
| `Alt` `←` / `Alt` `→` | Switch tab |
| `Ctrl` `W` | Close current tab |
| `Ctrl` `Alt` `T` | New space (opens the sidebar) |
| `Alt` `1`..`9` | Switch space |
| `Ctrl` `+` / `Ctrl` `-` / `Ctrl` `0` | Zoom font out / in / reset |

### Configuration

`~/.config/port/config.md` is created automatically. Each plugin owns one code
block, edited as `key = value`:

````markdown
```font-zoom
default_size = 14
step = 1
min_size = 6
max_size = 72
enabled = true
```

```transparency
opacity = 0.85
enabled = true
```
````

The file is watched: saving it applies changes immediately, including font size,
opacity and enabled plugins.

### Building

Requires Nix, or a Rust toolchain plus the Wayland/XCB/Vulkan development
libraries.

```bash
nix-shell          # environment with native libraries linked
cargo test         # 63 unit and contract tests
cargo build --release -p port
./target/release/port
```

### Writing a plugin

```rust
use port_plugin_api::{Plugin, PluginConfig, AppearanceHook};

struct MyPlugin;

impl AppearanceHook for MyPlugin {
    fn opacity(&self) -> Option<f32> { Some(0.90) }
}

impl Plugin for MyPlugin {
    fn id(&self) -> &'static str { "my-plugin" }
    fn name(&self) -> &'static str { "My Plugin" }
    fn appearance_hook(&self) -> Option<&dyn AppearanceHook> { Some(self) }
    fn default_config(&self) -> Option<PluginConfig> {
        let mut cfg = PluginConfig::new();
        cfg.set("enabled", true);
        Some(cfg)
    }
}
```

Available hooks: `AppearanceHook`, `InputHook`, `LayoutHook`, `SpaceHook`,
`PluginManagerHook` and `LifecycleHook`.

---

## Español

### Qué es

PORT es un emulador de terminal escrito desde cero en Rust sobre
[GPUI](https://github.com/zed-industries/zed), construido alrededor de una idea:
**cualquier funcionalidad del núcleo puede ser reemplazada por un plugin, sin
tocar el núcleo de la terminal.**

| Crate | Responsabilidad | Conoce |
|---|---|---|
| `port-term-core` | Ciclo del PTY, emulación VT, construcción de frames, codificación de teclas | Nada de la UI |
| `port-plugin-api` | Contratos públicos, archivo de configuración, registro de plugins | Solo `term-core` |
| `port-app` | Ventana GPUI, renderizado, enrutado de teclado | Ambos anteriores |

### Funcionalidades

- **Emulación de terminal** — parser VT de `alacritty_terminal`, scrollback,
  pantalla alternativa, atributos SGR, caracteres anchos.
- **Rendimiento real de renderizado** — las celdas se agrupan en runs, los
  caracteres Braille y de bloque se rasterizan como geometría de GPU en lugar de
  componerse como glifos, y los frames solo se reconstruyen cuando la rejilla
  cambia de verdad.
- **Multisesión** — cada espacio y cada pestaña tiene su propio PTY aislado; los
  procesos en segundo plano siguen corriendo al cambiar.
- **Sistema de plugins** — traits de Rust en el mismo proceso. Apariencia,
  entrada, layout, gestión de sesiones y ciclo de vida de la ventana.
- **Configuración legible** — un único archivo de texto plano con bloques de
  código Markdown por plugin, recargado al guardar.

### Atajos de teclado

| Atajo | Acción |
|---|---|
| `Ctrl` `Shift` `L` | Abrir el gestor de plugins |
| `Ctrl` `Shift` `T` | Nueva pestaña en el espacio actual |
| `Alt` `←` / `Alt` `→` | Cambiar de pestaña |
| `Ctrl` `W` | Cerrar la pestaña actual |
| `Ctrl` `Alt` `T` | Nuevo espacio (abre la barra lateral) |
| `Alt` `1`..`9` | Cambiar de espacio |
| `Ctrl` `+` / `Ctrl` `-` / `Ctrl` `0` | Reducir / aumentar / reiniciar fuente |

### Configuración

`~/.config/port/config.md` se crea automáticamente. Cada plugin tiene su bloque,
editado como `clave = valor`:

````markdown
```font-zoom
default_size = 14
step = 1
min_size = 6
max_size = 72
enabled = true
```

```transparency
opacity = 0.85
enabled = true
```
````

El archivo se vigila: al guardarlo los cambios se aplican al instante,
incluyendo tamaño de fuente, opacidad y plugins activados.

### Compilar

Requiere Nix, o un toolchain de Rust con las librerías de desarrollo de
Wayland/XCB/Vulkan.

```bash
nix-shell          # entorno con librerías nativas enlazadas
cargo test         # 63 tests unitarios y de contrato
cargo build --release -p port
./target/release/port
```

### Escribir un plugin

```rust
use port_plugin_api::{Plugin, PluginConfig, AppearanceHook};

struct MiPlugin;

impl AppearanceHook for MiPlugin {
    fn opacity(&self) -> Option<f32> { Some(0.90) }
}

impl Plugin for MiPlugin {
    fn id(&self) -> &'static str { "mi-plugin" }
    fn name(&self) -> &'static str { "Mi Plugin" }
    fn appearance_hook(&self) -> Option<&dyn AppearanceHook> { Some(self) }
    fn default_config(&self) -> Option<PluginConfig> {
        let mut cfg = PluginConfig::new();
        cfg.set("enabled", true);
        Some(cfg)
    }
}
```

Hooks disponibles: `AppearanceHook`, `InputHook`, `LayoutHook`, `SpaceHook`,
`PluginManagerHook` y `LifecycleHook`.

---

## Appendix: plugins · Anexo: plugins

Official plugins live in **[loonbac/port-plugins](https://github.com/loonbac/port-plugins)** — a separate repository so the terminal and its extensions evolve independently.

| Plugin | Hooks | What it does |
|---|---|---|
| `transparency` | `AppearanceHook` | Window background opacity |
| `font` | `AppearanceHook` | Font family, size and fallbacks |
| `font-zoom` | `AppearanceHook`, `InputHook` | Interactive font zoom with `Ctrl` `+` / `-` / `0` |
| `shortcuts` | — | Bind custom key combinations to callbacks |
| `menu-customizer` | `PluginManagerHook` | Restyle or fully replace the plugin manager |
| `herdr` | `AppearanceHook`, `InputHook`, `LayoutHook`, `SpaceHook` | Spaces sidebar, tabs, live process detection, resizable layout, wallpaper accent colour |
| `close-guard` | `LifecycleHook` | Asks for confirmation before closing with running processes |

Los plugins oficiales viven en **[loonbac/port-plugins](https://github.com/loonbac/port-plugins)**, un repositorio separado para que la terminal y sus extensiones evolucionen de forma independiente.

| Plugin | Hooks | Qué hace |
|---|---|---|
| `transparency` | `AppearanceHook` | Opacidad del fondo de la ventana |
| `font` | `AppearanceHook` | Familia, tamaño y fuentes de respaldo |
| `font-zoom` | `AppearanceHook`, `InputHook` | Zoom interactivo con `Ctrl` `+` / `-` / `0` |
| `shortcuts` | — | Asocia combinaciones de teclas a callbacks |
| `menu-customizer` | `PluginManagerHook` | Reestiliza o reemplaza el gestor de plugins |
| `herdr` | `AppearanceHook`, `InputHook`, `LayoutHook`, `SpaceHook` | Barra lateral de espacios, pestañas, detección de procesos en vivo, ancho ajustable y color acento del wallpaper |
| `close-guard` | `LifecycleHook` | Pide confirmación antes de cerrar si hay procesos corriendo |

## License · Licencia

MIT
