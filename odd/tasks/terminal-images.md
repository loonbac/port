# Terminal images: sixel and kitty graphics in PORT

PORT cannot show a single image today: no sixel, no kitty graphics, no path for
a program to hand pixels to the grid. Velocitty supports both protocols and that
is the capability this feature copies. The hard constraint is PORT's own doctrine:
the emulation core stays free of UI and codecs, so the core extracts and
positions, and the GPUI layer decodes and paints.

Facts that shape the design (verified in the pinned dependencies):

- `vte 0.15` puts APC (ESC _) into `SosPmApcString` and never dispatches it, so
  kitty graphics cannot be observed through `Processor`.
- `alacritty_terminal 0.26` implements no `hook`/`put`/`unhook`, so DCS is
  swallowed too.
- Therefore both protocols must be extracted from the raw byte stream before
  `Processor::advance`, exactly like `program_status.rs` already does for OSC 7501.

## Specs

S1 — Sixel. A program that emits `ESC P ... q ... ST` gets its image displayed at the
cursor; the image covers the cells it spans and the cursor ends below it, as the
protocol says.

S2 — Kitty graphics. A program that emits `ESC _ G ... ST` can transmit an image
(raw RGB/RGBA inline in base64, PNG/JPEG passthrough, or a file the terminal
reads), display it at the cursor or at cell offsets, query it (`a=q` must
answer `OK` on the PTY so `icat` works) and delete it (`a=d`).

S3 — Scrolling. Placements on the primary screen scroll with the text and are
dropped when fully scrolled out; on the alternate screen they stay fixed to the
screen.

S4 — The grid is unaffected. Image protocols never alter cell contents, and
every existing frame and session contract test passes unchanged.

S5 — No UI and no codec in the core. `port-term-core` extracts sequences, decodes
sixel (a wire format with no codec crate involved) and exposes placements plus
pixel data. Turning pixels into a GPU image belongs to `port-app`.

S6 — Bounded memory. The image store is capped (32 MiB per image, 64 MiB total):
beyond that the oldest image is evicted instead of growing without limit, and an
incomplete sequence is dropped once it passes `MAX_PENDING_SEQUENCE` bytes.

S7 — No unsolicited replies. Nothing is written to the PTY except the replies a
program asked for.

## Tasks

- T1 (S1, S5, S6) — `graphics` module: raw-byte scanner for sixel DCS and kitty APC,
  plus the sixel decoder to RGBA. Contract tests in
  `crates/term-core/tests/graphics_contract.rs`. Commit: pending.
- T2 (S2, S7) — kitty command parser: transmit (inline, file, temp), display,
  delete, query and the `OK` reply. Commit: pending.
- T3 (S1-S4, S6) — store, placements, scroll anchoring, `Frame` integration and
  `Session` wiring. Commit: pending.
- T4 (S1-S3, S5) — `port-app`: decode to `RenderImage`, cache per image id and paint
  with `Window::paint_image`. Commit: pending.
- T5 (S1-S7) — README feature list, module docs and the Velocitty comparison.
  Commit: pending.

## Log

- L1 — "quiero que revises el repo https://github.com/valvesky/velocitty para que
  podamos ver si podemos implementar algo para nuestra terminal"
- L2 — "dale hay que hacer todo eso, quiero poder ver imagenes tambien en port"
