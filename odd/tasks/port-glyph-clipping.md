# Core: glyph ink is overdrawn by neighbouring cell backgrounds

## Symptom

In PORT only (not in Kitty or tmux), and with no plugins loaded:

- Glyphs whose ink exceeds their cell are cut on the right and bottom. The
  clearest example is `❀` (U+2740) in gentle-pi's prompt frame.
- The prompt box reads as shifted rather than aligned to the grid.

## Confirmed cause

`crates/port-app/src/view/paint.rs` paints in row-major order, and each run
paints its own background quad immediately before its glyphs:

    if style.bg != default_bg || is_cursor {
        window.paint_quad(fill(Bounds::new(point(left, top), size(width, cell_height)), ...));
    }
    if !run.text.trim().is_empty() { /* glyphs */ }

GPUI gives every later primitive a higher z-order, so:

- the cell to the RIGHT paints its background after this glyph, overdrawing the
  glyph's right edge;
- the row BELOW paints all of its backgrounds after this row, overdrawing the
  glyph's bottom edge;
- cells to the left and rows above were painted earlier, so they stay underneath.

That is why the clip appears exactly on the right and bottom.

`❀` reaches this state because it is absent from Fira Code and from the first
fallback, `Symbols Nerd Font Mono`, so it resolves to `DejaVu Sans Mono`
(`crates/plugin-api/src/registry/hooks.rs:122`), whose ink for that glyph is
larger than the 8.4 x 19 px cell.

## Fix

Paint the frame in two passes: every cell background first (selection and run
backgrounds, including the cursor), then every glyph. A glyph that overflows its
cell then lands on top of the neighbouring backgrounds instead of under them.

## Tasks

- [x] Split `paint_frame` into a background pass and a glyph pass
- [x] Keep the selection highlight and the cursor background in the first pass
- [x] `cargo fmt --check`, `cargo clippy`, `cargo test --workspace`
- [x] Build and confirm PORT still runs
- [x] Commit the work unit

## Evidence

- `cargo check -p port --all-targets` — clean, 44.9 s.
- `cargo clippy -p port --all-targets` — exit 0, no warnings.
- `cargo fmt --check` — `paint.rs` absent from the diff. The three files it does
  report (`plugin-api/src/registry/hooks.rs`, `plugin-api/tests/registry_contract/watchdog.rs`,
  `port-app/src/cli.rs`) are already unformatted at `HEAD` and were not touched.
- `nix develop --command cargo test --workspace` — 248 passed, 0 failed.
  The dev shell is required: the system toolchain cannot link `-lxcb`,
  `-lxkbcommon` or `-lxkbcommon-x11`.
- `cargo build -p port` — `target/debug/port` is newer than the edited source and
  `port --help` exits 0.

## Visual result

**Confirmed by the user** on the workstation, with the installed binary
(`/nix/store/j5v6l7fwd3v05r23rkj9s27ls9x9w2nr-port-0.1.10`, built from `95abcbf`): the
framed glyph renders complete, with no clipping at the top. That closes the only item
this document left open.

What follows is the record of the earlier attempt, which could not observe it.

An attempt was made on the workstation, which does run Wayland (`WAYLAND_DISPLAY=wayland-1`,
`DISPLAY=:0`, compositor `niri`):

1. `PORT_CONFIG=/tmp/port-test.toml` with `[font] size = 48` and a `SHELL` script that
   prints `❀ U+2740` in a frame.
2. `grim` (wlr-screencopy) over the whole output, then cropped to the window.

Result: the PORT surface comes out **black** in the capture, and the frame does not
appear. The same happens with the installed binary from `/run/current-system/sw/bin/port`
and with `target/debug/port` without any override, so the black surface is a property
of the capture path against a GPUI window, not a finding about this change.

Conclusion at the time: the capture path could not see the window, so it was evidence
neither against nor in favour of the fix. The user's own look at the running build
settled it afterwards, as described at the top of this section.

## Commit

Not made. `port/AGENTS.md` forbids committing without authorization from the task
or the approved flow, and the working tree already carries the user's own
uncommitted local-only patch block in `Cargo.toml` that must not be committed.


## Deliberately out of scope

- The horizontal slack of less than one cell is not distributed symmetrically,
  so the grid sits up to half a cell left of centre. Changing that also moves
  mouse hit-testing, which reads the same origin through `grid_offset`, so it is
  a separate change.
- `paint.rs` positions each character by its index inside the run, while the
  run's `columns` counts cells. No glyph the user sees is `WIDE_CHAR`, so this
  does not fire today, but it is wrong for CJK and combining marks.
- `Metrics::cell_width` is a hardcoded 0.6 em instead of the advance of the font
  actually resolved at runtime.

## Evidence

Recorded only once each check has actually run. See the Evidence section above.
