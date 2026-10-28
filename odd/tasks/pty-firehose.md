# PTY firehose: bound the ingestion instead of trusting it

PORT queues every byte the PTY produces in an unbounded channel and parses the
whole backlog in a single `pump()`. A program that floods the terminal therefore
turns into unbounded memory growth plus one giant synchronous parse on the UI
thread. Velocitty treats this as a design requirement; PORT has no policy at all.

## Specs

S1 — Bounded queue. Unread PTY output held in memory is capped at
`pty::MAX_PENDING_BYTES` (4 MiB). When the cap is reached the oldest queued bytes
are dropped and the reader thread keeps draining the PTY until EOF, so the shell
never blocks and memory never grows past the cap.

S2 — Per-frame budget. One `Session::pump()` parses at most
`Session::PUMP_BUDGET_BYTES` (1 MiB) of captured output. Anything left stays
queued for the next call.

S3 — Observable drops. `Session::dropped_output_bytes()` reports how many bytes
the cap discarded since spawn. It starts at zero and only grows.

S4 — No regression under the cap. While output stays below the cap every byte
reaches the grid, `pump()` still returns immediately when the shell is silent,
and the existing session contract tests pass unchanged.

S5 — Newest output wins. After a flood the grid shows the tail of the stream,
not its head: the last line written before EOF is the one painted.

## Tasks

- T1 (S1, S2, S3, S5) — `pty.rs` bounded queue, `session/mod.rs` pump budget and
  drop counter, contract tests in `tests/session_contract/pty.rs`. Route: inline.
  Commit: pending.
- T2 (S1–S5) — run the whole `port-term-core` suite plus the `port-app` build
  as the differential check. Route: inline.

## Log

- L1 — "quiero que revises el repo https://github.com/valvesky/velocitty para que
  podamos ver si podemos implementar algo para nuestra terminal"
- L2 — "dale hay que hacer todo eso, quiero poder ver imagenes tambien en port"
