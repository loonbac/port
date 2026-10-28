# Debug overlay: OSC 556 layers in PORT

Velocitty ships a debug-only overlay that a program turns on with `OSC 556`
(`src/debug.zig`): flags for a cell grid, wrap markers, cursor, dirty regions and
a HUD. PORT has nothing like it, so diagnosing a layout bug means adding a
`println` and rebuilding. The capability is small and the vocabulary is worth
copying; the painting is not, because PORT's grid is drawn by GPUI and the
overlay costs one extra pass.

## Specs

S1 — Program-controlled. `ESC ] 556 ; <payload> ST` sets layers. Empty payload,
`on`, `all` or `1` turn everything on; `off` or `0` turn everything off;
`hud` and `grid` name individual layers; anything else turns on nothing. Without
a sequence there is no overlay and the frame carries no debug state.

S2 — The sequence never reaches the grid. It is an OSC consumed by the existing
scanner, exactly like OSC 7501, so no `556` text is ever painted.

S3 — The counters are measured, not invented: pending PTY bytes, bytes dropped
by the backlog cap, live placements, grid size, scrollback size and whether the
alternate screen is active. If a number cannot be measured it is not shown.

S4 — The core carries state, the UI paints. `Frame.debug` is the whole contract;
no GPUI type enters `term-core`.

## Tasks

- T1 (S1, S2, S4) — `overlay` module: flags, parsing, stats and state; OSC 556
  dispatch inside the existing scanner; `Frame.debug`.
- T2 (S3) — `Session::debug_flags` and `debug_state`, with the counters read from
  the PTY backlog, the image store and the grid.
- T3 (S1-S3) — `port-app`: grid lines and the HUD box, painted last.

## Log

- L1 — el usuario aprobó «(4) overlay de debug como plugin in-process» al revisar
  Velocitty.
- L2 — desvío deliberado: el overlay se implementa en el núcleo + UI, no como
  plugin. `LayoutHook` solo ofrece barras (arriba, izquierda, abajo) y un overlay
  flotante exigía cambiar el contrato publicado en `port-plugins`, que en este
  árbol está a medio trabajo (dependencia `port-plugin-mobile` sin commitear).
  El resultado es el mismo y verificable sin ventana.
