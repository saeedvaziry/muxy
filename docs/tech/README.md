# Muxy technical design

These documents lock the high-level technical design. They sit one level
below the [product model](../product/README.md) and one level above code.
Every design choice here was settled by a measured spike; the numbers are
kept in [benchmarks.md](./benchmarks.md) and the reasoning in
[decisions.md](./decisions.md).

## Reading order

1. [Decisions](./decisions.md) — what was decided, the numbers that decided
   it, and what was rejected.
2. [Architecture](./architecture.md) — the server, the wire, and the app as
   components with boundaries.
3. [Protocol](./protocol.md) — the wire contract between app and server.
4. [Constraints](./constraints.md) — facts about the chosen libraries and
   platforms that shape the implementation.
5. [Benchmarks](./benchmarks.md) — methodology, workloads, and the summary
   tables, so any regression can be re-measured.

## In one paragraph

The server is a Rust process that owns terminal sessions. Each session runs
on its own thread with a Ghostty terminal core, a PTY reader, and a
byte-budgeted, compressed history. Every 16 ms it turns changed rows into a
frame of style runs. Frames travel to the app as postcard messages with
streaming zstd over a channel-framed byte stream, with one merged pending
frame per session and control messages first. The app is Rust with GPUI; it
holds a run grid per attached session, renders it directly, and never
parses terminal output.
