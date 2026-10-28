# Avisos de la revisión nativa de las secciones nativas

Slice de seguimiento del candidato `3c96b4b` (`odd/tasks/native-font-selection.md`):
la revisión nativa aprobó con siete hallazgos `WARNING` no bloqueantes.

## Specs

S1 — La revisión solo conservó `id`, lente, ubicación y severidad de cada aviso; su
texto no es recuperable por la vía pública (`inspect-candidate` devuelve el diff
congelado, no los resultados). Cada arreglo se deriva de leer la línea señalada y se
registra aquí con su criterio.

S2 — `highlight_color` acepta el valor con espacios alrededor, tenga o no `#` delante:
`parse_hex_color("  #58a6ff  ")` y `parse_hex_color(" 58a6ff ")` dan el mismo color.

S3 — Un valor con bytes multibyte sigue devolviendo `None` sin paniquear (regresión de
R3-001 del slice anterior).

S4 — Las secciones nativas se resuelven en una función con su propio contrato, no
inline en `load_configs`, y el doc de `load_configs` describe las dos mitades.

S5 — Las funciones auxiliares que avisan por stderr reciben la sección a la que
pertenece la clave; ninguna lleva una sección fija escondida.

S6 — Un booleano escrito en mayúsculas (`True`, `False`) se aplica igual que en
minúsculas; un valor ilegible conserva el default.

S7 — `mouse_policy_default_config` documenta que el bloque generado fija los valores
del núcleo y que un archivo existente no se reescribe al cambiar esos valores.

S8 — `enabled = false` sigue apagando la sección entera, incluidos `fallbacks`.

S9 — Nada nuevo en la ruta caliente: los getters por cuadro no cambian.

## Tasks

- T1 (S2, S3) — `parse_hex_color`: recortar antes de quitar `#`, un solo comentario
  correcto, y tests de las dos formas y del caso multibyte.
- T2 (S4) — `PluginRegistry::apply_native_sections` con doc propio; `load_configs` la
  llama y su doc describe las dos mitades.
- T3 (S5) — `apply_bool`, `apply_color`, `apply_opacity` reciben `section`.
- T4 (S6) — `read_bool` local acepta la misma verdad en cualquier caja.
- T5 (S7) — doc de `mouse_policy_default_config`.
- T6 (S8) — extender `una_seccion_apagada_se_ignora` con `fallbacks`.
- T7 (S1-S9) — suite, clippy, fmt y commit del slice.

## Log

- L1 — «todavia no toques port mobile, separalo eso, arregla lo de losavisos a ver que
  pasa».
- L2 — El WIP de `port-mobile` quedó fuera de todos los commits de esta rama y no se
  tocó.

## Revisión nativa del slice de avisos

Candidato `f6d9ee7` sobre `396155d`, lineage `review-824dbc76369335fd`, tier
`medium`, lentes `risk`, `readability` y `reliability`.

**Aprobada en la primera pasada**: los tres revisores admitidos, sin refuter ni
corrección, y la autoridad quemada con `gentle-ai.review-acknowledged/v1`.

Ocho avisos informativos quedan como trabajo posterior, sin bloquear ni reabrir:
R2-001 (native.rs:175-178), R2-002 (native_sections_contract.rs:307-310), R2-003
(native_sections_contract.rs:294-297), R2-004 (registry/mod.rs:98-106), R3-001
(native.rs:179-180), R3-002 (native_sections_contract.rs:294-297), R3-003
(native_sections_contract.rs:307-310) y R3-004 (native_sections_contract.rs:273-280).

Dos de los avisos del linaje anterior no eran defectos y quedaron fijados con test:
`PluginConfig::get_bool` ya normaliza la caja del booleano, y `enabled = false` ya
cortaba la sección entera antes de mirar `fallbacks`.
