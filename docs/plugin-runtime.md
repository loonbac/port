# Sistema de carga de plugins

## Resumen

Un plugin es un **proceso aparte** que habla JSON por `stdin`/`stdout`. Se
compila una vez al instalarlo; cargarlo cuesta arrancar un proceso.

```sh
port plugin add https://github.com/mi/usuario/mi-plugin
```

Eso da tres cosas:

| Medida | Valor |
| --- | --- |
| Arranque de un plugin | 0,71 ms |
| Invocación (ida y vuelta) | 0,012 ms |
| Recarga tras recompilar | automática, sin reiniciar PORT |

## Por qué un proceso y no una biblioteca

Un plugin cargado como `.so` comparte heap, runtime de Rust y cada tipo de
GPUI con el núcleo. Cualquier diferencia de versión del compilador o de GPUI
convierte esa frontera en comportamiento indefinido, y el síntoma aparece en la
máquina de otro usuario, no en la tuya. Con un proceso separado esa clase de
problema desaparece: si el plugin no habla el protocolo, no arranca.

El precio son unos milisegundos de arranque. Es aceptable porque el coste caro,
la compilación, ocurre una sola vez en la instalación.

## Por qué no se pregunta nada en cada tecla

Un viaje de ida y vuelta por tubería en cada pulsación metería latencia
justo donde el usuario la nota. Por eso funciona al revés:

1. El plugin **declara** en el saludo qué combinaciones quiere.
2. El núcleo las guarda y las resuelve **en local**
   (`ExternalPlugin::resolve_action`).
3. Solo cuando hay coincidencia se pregunta al plugin dueño, **con plazo**.

Con 12 µs por invocación, preguntar en la tecla pulsada sería aceptable, pero
no hace falta: así la ruta normal de teclear no toca ninguna tubería.

## Nada puede congelar la terminal

El terminal no se bloquea nunca por un plugin. Cada espera lleva plazo
(`DEFAULT_TIMEOUT_MS`, 250 ms) y un plugin que no contesta degrada a "no aporta
nada". Un plugin roto no es una terminal rota.

Las respuestas que llegan sin que nadie las pidiera se guardan en una cola de
pendientes en vez de descartarse. Descartarlas hacía que el acuse de `configure`
se leyera como respuesta a la acción siguiente: el tipo de bug que no aparece en
pruebas unitarias y sí en uso real.

## Recarga en caliente

`port-plugin-api::watch::Watcher` vigila el **binario instalado**, no el código
fuente. El código cambia en cada guardado y casi nunca produce un binario
distinto; el binario es el punto donde la recompilación ocurrió de verdad.

La firma es tamaño más fecha de modificación. Solo con la fecha se pierden
cambios: dos escrituras dentro del mismo tick dejan la misma fecha.

Ante un binario roto el watcher reporta `Failed` y no insiste: recargar cada
250 ms un plugin que no compila sería peor que no recargarlo.

## Cómo se mide

```sh
port plugin add <url-de-un-plugin>
cargo run --release --example bench -- ~/.local/share/port/plugins/<id>
```

Cifras de esta máquina (Linux, plugin Rust mínimo):

```text
arranque: 0.71 ms por plugin (20 arranques)
invocación: 0.012 ms por ida y vuelta
```

Si estas cifras se degradan, el problema está en el transporte, no en las reglas
de negocio.

## Escribir un plugin

```rust
use port_plugin_sdk::protocol::{Appearance, Capability};
use port_plugin_sdk::runtime::{serve, Plugin};

struct MiPlugin;

impl Plugin for MiPlugin {
    fn name(&self) -> &'static str { "mi-plugin" }
    fn version(&self) -> &'static str { "0.1.0" }
    fn capabilities(&self) -> Vec<Capability> { vec![Capability::Appearance] }
    fn appearance(&self) -> Appearance {
        Appearance { opacity: Some(0.92), ..Default::default() }
    }
    fn invoke(&self, action: &str, _p: &serde_json::Value) -> Option<serde_json::Value> {
        (action == "saludar").then(|| serde_json::json!({ "ok": true }))
    }
}

fn main() { serve(MiPlugin); }
```

El SDK (`port-plugin-sdk`) depende solo de `serde`. A propósito **no** arrastra
GPUI: si lo hiciera, instalar un plugin sería tan pesado como recompilar PORT,
que es justo lo que este diseño evita.

## Cuándo elegir cada uno

La decisión se tomaba una vez por plugin y volvía a discutirse. Queda escrita:

**Proceso independiente** cuando el plugin solo mueve valores: opacidad,
tipografía, un atajo, un veto de cierre. Gana un `kill` duro y no puede
enlazarse contra los internos de PORT.

**En el proceso** cuando el plugin dibuja: una barra lateral, una barra de
estado, cualquier cosa que componga elementos de GPUI. Es el único camino que
puede, así que mover estos plugins al otro lado **rompería la posibilidad de
dibujar**, que es justamente la visión del proyecto.

Ninguno es una versión recortada.

### Por qué no se mueven todos a proceso independiente

Porque se perdería el dibujo. El motivo habitual a favor es el aislamiento, y
esa parte ya está cubierta en el camino en el proceso:

- Un hook que entra en pánico queda aislado por `catch_unwind`.
- Un hook que excede su presupuesto de 50 ms se desactiva y deja de invocarse.

El `kill` duro solo importa frente a un plugin que cuelga un hilo sin bloquear
la interfaz, cosa que no hace ninguno de los plugins que hay aquí.

### El coste real de un proceso independiente

Medido en esta máquina: **1 MB de RSS y 0,0 % de CPU** en reposo, con
**0,71 ms** de arranque. El número de procesos es el que la gente cuenta, pero
no es el que consume. Una instalación por defecto no tiene ninguno: son
opt-in.

Si aun así preocupara, el límite razonable no es un tope de procesos sino
mantener en el proceso lo que dibuja y aceptar el resto. Un contador de
procesos solo convertiría un problema perceptible en un problema invisible.

## Límites actuales

- `LayoutHook` **no** cruza el límite del proceso: un `AnyElement` de GPUI no
  se serializa. Los plugins con UI rica (`herdr`) siguen siendo in-process.
- El canal es JSON sobre texto. Es legible y basta, pero un canal binario con el
  mismo contrato reduciría el coste de invocación si alguna vez hace falta.
