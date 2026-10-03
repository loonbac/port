# PORT — Terminal Rust orientada a plugins

[English](README.md)

Un emulador de terminal escrito en Rust sobre
[GPUI](https://github.com/zed-industries/zed), construido alrededor de una idea:
**cualquier funcionalidad del núcleo puede ser reemplazada por un plugin, sin
tocar el núcleo de la terminal.**

```
┌─ port ───────────────────────────────┐
│ crates/term-core    PTY · rejilla VT · │
│                     codificación keys │
│ crates/plugin-api  contratos públicos │
│ crates/port-app    frontend GPUI      │
└──────────────────────────────────────┘
        ▲ todo lo de arriba es extensible
```

## Arquitectura

| Crate | Responsabilidad | Conoce |
|---|---|---|
| `port-term-core` | Ciclo del PTY, emulación VT, construcción de frames, codificación de teclas | Nada de la UI |
| `port-plugin-api` | Contratos públicos, archivo de configuración, registro de plugins | Solo `term-core` |
| `port-app` | Ventana GPUI, renderizado, enrutado de teclado | Ambos anteriores |

El núcleo de emulación está completamente libre de dependencias de interfaz. Toda
la interfaz y toda decisión de política vive detrás de un trait en
`port-plugin-api`, así que un plugin puede reemplazar comportamiento del núcleo
sin que el núcleo sepa que existe.

## Funcionalidades

- **Emulación de terminal** — parser VT de `alacritty_terminal`, scrollback,
  pantalla alternativa, atributos SGR, caracteres anchos.
- **Rendimiento real de renderizado** — las celdas se agrupan en runs, los
  caracteres Braille y de bloque se rasterizan como geometría de GPU en lugar de
  componerse como glifos, y los frames solo se reconstruyen cuando la rejilla
  cambia de verdad.
- **Multisesión** — cada espacio y cada pestaña tiene su propio PTY aislado; los
  procesos en segundo plano siguen corriendo al cambiar.
- **Sistema de plugins** — traits de Rust en el mismo proceso que cubren
  apariencia, entrada, layout, gestión de sesiones y ciclo de vida de la ventana.
- **Configuración legible** — un único archivo de texto plano con bloques de
  código Markdown por plugin, recargado al guardar.

## Atajos de teclado

| Atajo | Acción |
|---|---|
| `Ctrl` `Shift` `L` | Abrir el gestor de plugins |
| `Ctrl` `Shift` `T` | Nueva pestaña en el espacio actual |
| `Alt` `←` / `Alt` `→` | Cambiar de pestaña |
| `Ctrl` `W` | Cerrar la pestaña actual |
| `Ctrl` `Alt` `T` | Nuevo espacio (abre la barra lateral) |
| `Alt` `1`..`9` | Cambiar de espacio |
| `Ctrl` `+` / `Ctrl` `-` / `Ctrl` `0` | Aumentar / reducir / reiniciar fuente (vía `shortcuts`) |

## Configuración

`~/.config/port/config.md` se crea automáticamente en la primera ejecución. Cada
plugin tiene su bloque, editado como `clave = valor`:

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
incluyendo tamaño de fuente, opacidad y qué plugins están activos.

## Compilar

Requiere Nix, o un toolchain de Rust con las librerías de desarrollo de
Wayland/XCB/Vulkan.

```bash
nix-shell                      # entorno con librerías nativas enlazadas
cargo test                     # 65 tests unitarios y de contrato
cargo build --release -p port
./target/release/port
```

## Escribir un plugin

Añade el plugin como dependencia y regístralo:

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

### Llamar a un plugin desde otro

Un plugin puede publicar una capacidad con `Plugin::services()` y otro puede
invocarla por identificador, sin compartir tipos ni clonar internos:

```rust
// En el plugin que publica.
impl Plugin for MiPlugin {
    fn services(&self) -> Vec<Arc<dyn Service>> {
        vec![Arc::new(MiServicio)]
    }
}

// En el plugin que consume.
shortcuts.bind_service("ctrl+=", "mi-plugin", "haz_algo");
```

`bind_service` resuelve el destino al pulsar la tecla, así que el orden de
registro da igual.

## Anexo: plugins

Los plugins oficiales viven en **[loonbac/port-plugins](https://github.com/loonbac/port-plugins)**,
un repositorio separado para que la terminal y sus extensiones evolucionen de
forma independiente.

| Plugin | Hooks | Qué hace |
|---|---|---|
| `transparency` | `AppearanceHook` | Opacidad del fondo de la ventana |
| `font` | `AppearanceHook` | Familia, tamaño y fuentes de respaldo |
| `font-zoom` | `AppearanceHook` | Estado del tamaño de fuente y operaciones de zoom (sin atajos) |
| `shortcuts` | — | Asocia combinaciones de teclas a callbacks |
| `menu-customizer` | `PluginManagerHook` | Reestiliza o reemplaza el gestor de plugins |
| `herdr` | `AppearanceHook`, `InputHook`, `LayoutHook`, `SpaceHook` | Barra lateral de espacios, pestañas, detección de procesos en vivo, ancho ajustable y color acento del wallpaper |
| `close-guard` | `LifecycleHook` | Pide confirmación antes de cerrar si hay procesos corriendo |

Consulta el [repositorio de plugins](https://github.com/loonbac/port-plugins) para
la documentación detallada de cada uno.

## Licencia

MIT
