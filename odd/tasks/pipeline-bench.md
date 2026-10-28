# Pipeline bench: measure the terminal instead of guessing

The repository had no way to state how fast the core ingests output: the only
benchmark measured plugin startup. Velocitty treats a pipeline bench as part of
the design (per-stage p50/p95 over a real PTY); PORT now has one too.

## Specs

S1 — Real PTY. The `session` stage runs an actual PTY with `cat` streaming a
generated workload, so the number includes the operating system, not just the
parser.

S2 — Per-stage percentiles. Every workload reports p50 and p95 for `parse`
(`Processor::advance` per 1 MiB chunk), `frame` (grid → rows → runs) and `session`.

S3 — Honest accounting. Discarded bytes are reported, and the `session` rate
counts what was actually parsed (input minus drops, plus the CR the line
discipline adds), never what was written to the file.

S4 — Comparable runs. Each run writes `bench/<epoch>-<commit>.txt` and `bench/last.txt`,
both gitignored, and prints the profile so a debug run is never compared with a
release one.

S5 — Not a gate. The bench is a reporting tool: it is not part of `cargo test`
and no CI job asserts an absolute throughput.

## Tasks

- T1 (S1-S5) — `crates/term-core/examples/pipeline_bench.rs`, `/bench` in `.gitignore`.
  Route: inline. Commit: `pending`.

## Baseline

2 MiB per workload, 10 samples, release, commit `83dc303e4d17`:

| workload | parse MiB/s | session MiB/s | session p95 |
| --- | --- | --- | --- |
| ascii | 70.5 | 71.7 | 11.3 ms |
| sgr | 88.4 | 59.0 | 14.4 ms |
| csi | 91.1 | 78.4 | 12.2 ms |
| esc | 2.2 | 2.3 | 438 ms |
| unicode | 129.1 | 77.4 | 10.2 ms |

The `esc` workload (a redraw loop with `\x1b[2J` on every line) is two orders of
magnitude slower than the rest: full-screen clears dominate, not the parser.

## Log

- L1 — "quiero que revises el repo https://github.com/valvesky/velocitty para que
  podamos ver si podemos implementar algo para nuestra terminal"
- L2 — "dale hay que hacer todo eso, quiero poder ver imagenes tambien en port"
