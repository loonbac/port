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
- **Sistema de plugins** — dos tipos, un mismo contrato. Traits de Rust en el
  proceso para lo que necesita dibujar con GPUI, y procesos independientes para
  los plugins que instalas sin recompilar PORT.
- **Instalar plugins desde la URL de un repositorio** — `Ctrl` `Shift` `L`, la
  entrada de instalar, pegas la URL. Clona, compila y carga el plugin sin
  reiniciar la terminal.
- **Un plugin no puede tumbar la terminal** — un hook que entra en pánico queda
  aislado, y uno que excede su presupuesto de tiempo se desactiva en vez de
  congelar la sesión para siempre.
- **Configuración legible** — un único archivo de texto plano con bloques de
  código Markdown por plugin, recargado al guardar.

## Atajos de teclado

| Atajo | Acción |
|---|---|
| `Ctrl` `Shift` `L` | Abrir el gestor de plugins (listar, activar, instalar) |
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

## Instalación

### Bundle portable (recomendado)

Sin Nix y sin instalar paquetes. Descarga el directorio
`port-<version>-linux-x86_64` de la release, descomprímelo donde quieras y
ejecuta `bin/port`:

```bash
tar xf port-*.tar.gz
./port-*/bin/port
```

Para instalarlo en `~/.local`:

```bash
./port-*/INSTALL.sh
```

El bundle incluye las librerías de X11/xkb contra las que GPUI enlaza, y carga
Wayland y Vulkan en tiempo de ejecución, todo dentro de `lib/`. **FreeType no
se empaqueta**: GPUI la abre con `dlopen` en tiempo de ejecución, así que se
resuelve desde el sistema anfitrión igual que el driver de Vulkan. Lo
único que no puede empaquetar es tu driver de GPU: Vulkan tiene que encontrar el
ICD que corresponde a tu tarjeta, y eso es una propiedad de la máquina, no de la
terminal.

Solo el driver tiene que estar presente. Lo demás viaja con la aplicación.

### Paquetes nativos

```bash
# Debian / Ubuntu
sudo apt install ./port.deb

# Fedora / RHEL
sudo dnf install ./port.rpm
```

### Nix (opcional)

```bash
nix profile install github:loonbac/port
```

Útil en NixOS y para compilaciones reproducibles. **No** es obligatorio: un
binario compilado con Nix lleva dentro el cargador dinámico del store, así que
funciona en NixOS y en ningún otro sitio. El bundle portable se compila dentro de
Debian 12 contra glibc 2.36, y por eso arranca en Ubuntu 22.04 o posterior,
Fedora 36 o posterior y RHEL/Rocky 9.

### Requisitos

Una sesión Wayland o X11 y un driver de Vulkan que funcione.

## Compilar

Requiere Nix, o un toolchain de Rust con las librerías de desarrollo de
Wayland/XCB/Vulkan.

```bash
nix-shell                      # entorno con librerías nativas enlazadas
cargo test                     # 67 tests unitarios y de contrato
cargo build --release -p port
./target/release/port
```

## Instalar un plugin

Los plugins que no necesitan dibujar con GPUI se instalan como procesos
independientes. Dale a PORT la URL del repositorio y lo clona, compila y carga:

```sh
port plugin add https://github.com/tu/port-plugin-ejemplo
port plugin list
port plugin remove ejemplo
```

Lo mismo funciona desde el teclado: `Ctrl` `Shift` `L`, eliges la entrada de
instalar, pegas la URL y pulsas `Enter`. La compilación corre en segundo plano
—unos seis segundos para un plugin pequeño— y el menú sigue usable mientras
ocurre.

Un repositorio es un plugin cuando su `Cargo.toml` produce un binario y,
opcionalmente, dice cómo registrarse:

```toml
[package.metadata.port]
id = "ejemplo"
capabilities = ["appearance", "input"]
```

Sin esa sección, el identificador se deduce del nombre del paquete.

Hay un ejemplo completo y compilable en
[`examples/external-plugin/`](examples/external-plugin). La prueba de extremo a
extremo compila y ejecuta ese mismo ejemplo, así que no puede quedarse obsoleto.

Cargar un plugin cuesta 0,71 ms y cada invocación 12 µs, medido en esta máquina;
el diseño y las cifras están en
[`docs/plugin-runtime.md`](docs/plugin-runtime.md).

## Escribir un plugin

### En el proceso, cuando necesitas dibujar

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
`SpaceHook` y `LifecycleHook`.

### Independiente, cuando no lo necesitas

Un plugin que solo ajusta valores, responde a atajos o veta el cierre no
necesita vivir en el proceso. Se distribuye como su propio binario que habla
JSON por stdin/stdout, y nunca compila contra GPUI:

```rust
use port_plugin_sdk::protocol::{Appearance, Binding, Capability};
use port_plugin_sdk::runtime::{serve, Plugin};

struct MiPlugin;

impl Plugin for MiPlugin {
    fn name(&self) -> &'static str { "mi-plugin" }
    fn version(&self) -> &'static str { "0.1.0" }
    fn capabilities(&self) -> Vec<Capability> { vec![Capability::Appearance] }
    fn appearance(&self) -> Appearance {
        Appearance { opacity: Some(0.90), ..Default::default() }
    }
    fn bindings(&self) -> Vec<Binding> {
        vec![Binding { key: "k".into(), ctrl: true, alt: false,
                       shift: false, action: "alternar".into() }]
    }
    fn invoke(&self, action: &str, _p: &serde_json::Value) -> Option<serde_json::Value> {
        (action == "alternar").then(|| serde_json::json!({ "ok": true }))
    }
}

fn main() { serve(MiPlugin); }
```

[`docs/plugin-runtime.md`](docs/plugin-runtime.md) tiene el protocolo, el
razonamiento detrás de él y los compromisos que implica.

### ¿Cuál de los dos debería escribir?

Elige en el proceso cuando el plugin dibuja: una barra lateral, una barra de
estado, cualquier cosa que componga elementos de GPUI. Es el único camino que
puede, y también es el más rápido.

Elige un proceso independiente cuando el plugin solo mueve valores. Gana un
`kill` duro que el camino en el proceso no puede ofrecer, y no puede enlazarse
contra los internos de PORT. Cuesta un proceso en reposo: alrededor de 1 MB y
0 % de CPU.

Ninguno de los dos es una versión recortada. La división existe para que los
plugins con interfaz y los plugins de desconocidos funcionen ambos sin
renunciar a nada.

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
| `herdr` | `AppearanceHook`, `InputHook`, `LayoutHook`, `SpaceHook` | Barra lateral de espacios, pestañas, detección de procesos en vivo, ancho ajustable y color acento del wallpaper |
| `close-guard` | `LifecycleHook` | Pide confirmación antes de cerrar si hay procesos corriendo |

Consulta el [repositorio de plugins](https://github.com/loonbac/port-plugins) para
la documentación detallada de cada uno.

## Licencia

MIT
