# Native font and mouse sections

`font` and `selection` are plugins today, but neither replaces anything: `font`
returns three values, and `selection` parses a config block into `MousePolicy`, a
struct that already lives in `term-core`. PORT's doctrine is a minimal, fast core
with plugins for what can be replaced, so these two are configuration wearing a
plugin costume.

Measured before the change: `selection` is 405 lines, of which the whole body is
config parsing plus a `MousePolicy` copy; `font` is 384 lines for a family, an
optional size and an unused fallback list. Nesting herdr (~10k lines, four hooks)
and `shortcuts` (key policy) is the opposite case and stays as plugins.

## Specs

S1 — Same names, no migration. PORT reads the ``` font``` and ``` selection``` sections
from the same file, with the same keys the plugins used. An existing
`config.md` keeps working unchanged, including `enabled = false`.

S2 — Plugins still win. An in-process `AppearanceHook` or `MouseHook` overrides the
native value, and an installed external appearance plugin overrides native font.
Without any plugin the native values are what the terminal uses.

S3 — The template cannot drift. The generated file carries both sections derived
from `Font::default()` and `MousePolicy::default()`, which is the one good idea
the plugins had: the distributed file and the core defaults cannot disagree.

S4 — Bad input never breaks. An unreadable value keeps the default and is
reported on stderr; `enabled = false` means «ignore this section».

S5 — Nothing new on the hot path. Parsing runs at load and reload only; the
per-frame getters gain one field read and no extra plugin invocation.

S6 — Minimum surface. No new trait and no new capability: `selection` leaves the
app dependency list, `font` never enters it, and the public additions are the
native sections plus the two setters the registry needs.

## Tasks

- T1 (S1, S3, S4, S6) — `port-plugin-api::native`: sections, `Font`, parsing and
  default blocks, with unit tests.
- T2 (S2, S5) — registry: native slots, precedence, `default_configs` and
  `load_configs`; contract tests with and without plugins.
- T3 (S6) — `port-app`: drop the `port-plugin-selection` dependency and its
  registration; the native section takes over the same policy.
- T4 (S1-S4) — docs: README (both), `docs/plugin-runtime.md`, and the port-plugins
  table.
- T5 (S1-S6) — verify: full suite, clippy, fmt, and a differential check that the
  old config file produces the same policy.

## Log

- L1 — «dale que selection y font vaya al nucleo, aunque si me gustaria que los
  atajos sea un plugin».
- L2 — «recuerda que la filosofia de este proyecto es que port sea una terminal
  velocisima con lo minimo, igual que pi, pero personalizable con plugins»:
  motiva S5 y S6, y deja `shortcuts` fuera del cambio.

## Evidence

- T1-T2 RED, observado antes de implementar: con los ocho tests de contrato
  escritos y `native.rs` aún ausente, `cargo test -p port-plugin-api` fallaba en
  compilación con `E0432: unresolved import port_plugin_api::native` y dos
  `E0599: no method named set_native_font` / `set_native_mouse_policy`.
- T1-T2 GREEN: `cargo test -p port-plugin-api` verde — 44 tests de la lib, 8 de
  `native_sections_contract`, 45 de `registry_contract`.
- Suite completa: `cargo test --workspace` verde, 16 binarios, 353 tests, 0
  fallos. `cargo clippy --workspace --all-targets` sin avisos propios y
  `rustfmt --check` limpio sobre los archivos tocados.
- T5 diferencial: los tests de `registry_contract` que ya existían pasan sin
  tocarlos, que es la garantía de que un registro sin configurar sigue honrando la
  familia del llamador; y `las_siete_claves_booleanas_y_los_dos_recortes` replica
  las siete claves y los dos recortes del contrato del plugin borrado.
- Árbol commiteado sin el WIP local de mobile: `cargo check -p port --all-targets`
  sobre un worktree detached del commit del slice termina en `Finished`.
- Limitación del entorno, no del slice: correr `cargo clippy` y `cargo test` en la
  misma sesión de cargo hace timeoutear el probe de los plugins externos de prueba
  (`host::process`, `watch`); la suite corrida sola pasa entera.

## Revisión nativa (RDD)

Rama `feat/firehose-bench-images`, candidato `3c96b4b` sobre la base `27d16cc`, tier
`medium`, lentes `risk`, `readability` y `reliability`. Lineage
`review-be4af47c65b3c3ed`.

El refuter devolvió `correction_required` con dos hallazgos CRITICAL causados por el
candidato, y el plan de corrección se congeló en 80 líneas de diff antes de tocar el
código (53 aplicadas):

- **R2-001** — el parser nativo leía `family` y `size` pero no `fallbacks`, que el
  plugin borrado sí exponía (`font_fallbacks()`), así que una sección `font` con esa
  clave se descartaba en silencio. `Font` gana `fallbacks`, `font_from_config` lo
  parsea como lista separada por comas y `effective_font_fallbacks` lo consulta antes
  de caer en las tres familias por defecto.
- **R3-001** — `parse_hex_color` medía en bytes y cortaba en índices fijos, así que un
  valor de seis bytes con un carácter multibyte partía el `&str` en una frontera
  inválida y abortaba la carga de configuración. Ahora exige ASCII antes de cortar.

Ambos se reprodujeron primero (panic `byte index 2 is not a char boundary`, y
fallbacks que no llegaban al registro) y luego se corrigieron con un test por regla.
El validador dirigido aprobó el candidato corregido y la autoridad quedó quemada con
`gentle-ai.review-acknowledged/v1`.

Siete avisos `WARNING` quedaron como trabajo posterior, sin bloquear ni reabrir la
revisión: R1-001 (native.rs:138-140), R2-002 (registry/mod.rs:277-284), R2-003
(native.rs:134-140), R2-004 (native.rs:169-175), R3-002 (native.rs:174), R3-003
(native.rs:146) y R3-004 (native_sections_contract.rs:157-161).
