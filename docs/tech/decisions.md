# Decisions

Each decision names the question, the choice, the measurements that settled
it, and what was rejected. Machine for all numbers: Apple M3, 24 GB, macOS,
Rust 1.92, release builds, medians of repeated runs on recorded workloads.
Full tables are in [benchmarks.md](./benchmarks.md).

## D1. The server's terminal core is libghostty-vt

| | Ghostty, compressed | Alacritty | WezTerm core | vt100 |
| --- | ---: | ---: | ---: | ---: |
| Memory per 10K history rows | 1.7 MB | 51 MB | 3.3 MB | 71 MB |
| Build-log parse speed | 317 MB/s | 123 MB/s | 49 MB/s | 85 MB/s |
| Peak memory reflowing 100K rows | 29 MB | 1,300 MB | 68 MB | 690 MB |
| Server footprint, 100 sessions, 30 busy | 135 MB | 2,060 MB | not run | not run |

All engines produced identical screens on every workload, including a
recorded vim session and reflow after resize, except vt100 on combining
marks. Rejected: Alacritty for the server because of its per-row cost and
reflow peak; WezTerm's core for speed; vt100 for memory and correctness; a
custom grid because Ghostty already beats what it could reach.

Alacritty remains a valid engine for an optional app-side full-emulator
surface, where no history is held. See D9.

## D2. The server owns the grid and history; the app renders and never parses

Four placements were built as real processes over a Unix socket.

| Placement | Server memory under floods | Client memory, 100 sessions | Wire on floods | Attach |
| --- | --- | ---: | ---: | ---: |
| Server grid, client renders rows | bounded by the engine | 9 MB | under 2 percent of output | 1 ms |
| Server grid, VT re-stream to a client emulator | bounded by the engine | 561 MB | under 3 percent | 1 ms |
| Raw bytes, client emulates | balloons to 389 MB per session | 2,340 MB | 100 percent | 16 ms per MB retained |
| Hybrid | worst of both | 689 MB | 100 percent | 1 ms |

Rejected: raw-byte streaming and the hybrid. Both make every client re-parse
everything, hold history on the client, and need explicit backpressure the
grid gives for free.

## D3. History is a byte budget

Ghostty enforces bytes, not rows, and compressed pages make a byte cap
predictable while a row cap is not. The product's retention setting is
stated in bytes; the server reports rows retained.

## D4. Frames carry style runs per changed row

| Shape, uncompressed | vim session | monitor at 60 Hz | style churn |
| --- | ---: | ---: | ---: |
| One record per cell | 1,176 KB | 17,524 KB | 1,397 KB |
| Style runs | 181 KB | 2,750 KB | 1,129 KB |
| VT re-stream | 223 KB | 3,152 KB | 2,312 KB |

Runs are 4 to 8 times smaller than cells, undercut the VT re-stream, decode
in 3 to 12 µs, and need no emulator on the client. Rejected: cells for size;
VT re-stream because a client that wants a full emulator can regenerate it
locally from runs.

## D5. Serialization is postcard

Relative to postcard on the run shape: bincode within 10 percent;
MessagePack and protobuf 15 to 60 percent larger and up to 1.6 times the
CPU. Both ends are Rust. Revisit only if a non-Rust client appears.

## D6. Compression is zstd level 1 with a streaming context per connection

| Compression on run frames | build log | vim | monitor | style churn |
| --- | ---: | ---: | ---: | ---: |
| none | 25.1 KB | 181 KB | 2,750 KB | 1,129 KB |
| lz4 | 9.5 KB | 91 KB | 1,585 KB | 468 KB |
| zstd 1 per frame | 6.9 KB | 62 KB | 912 KB | 216 KB |
| zstd 1 streaming | 4.3 KB | 29 KB | 829 KB | 211 KB |

Streaming context halves interactive traffic at 15 µs per frame. Rejected:
lz4 for ratio; zstd level 3 for 1.7 ms frames on churn with no gain; a
trained dictionary because it overfits its training workload and the
streaming context already captures the repetition.

## D7. Transport is an abstract byte stream with channel framing

| Transport | Flood throughput | Control latency p50 under paced load |
| --- | ---: | ---: |
| stdio pipe | 150 MB/s | 0.18 ms |
| TCP loopback | 144 MB/s | 0.45 ms |
| Unix socket | 136 MB/s | 0.21 ms |

Every transport is ten times faster than the fastest realistic PTY producer.
Unix socket locally; stdio through whatever exec mechanism reaches a remote
host, which is what SSH, Docker exec, and kubectl exec present; TCP with
TLS only where a port is unavoidable. Rejected: a stream multiplexer library,
because D8 replaces per-stream windows with something better for this
domain.

## D8. Flow control is one merged pending frame per channel with one credit

| Slow client, 3 ms per frame | Server peak memory | Time to current state | Frames sent |
| --- | ---: | ---: | ---: |
| queue everything | 128 MB | 94 s | 22,200 |
| merge per channel | 3 MB | 10 s | 2,273 |

A newer frame for a row supersedes the older one, so merging loses nothing.
Control frames are written before data by a dedicated writer thread.

## D9. The app renders the run grid directly in GPUI and redraws on demand

| Panes, forced 60 Hz redraw | vim | monitor | style churn |
| --- | ---: | ---: | ---: |
| 1 | 60 fps, 20% of a core | 60 fps, 24% | 59 fps, 100% |
| 16 | 60 fps, 31% | 60 fps, 45% | 60 fps, 92% |

One shaped line per row with a text run per style run, one quad per run.
Shaped-line caching changed nothing measurable; painting, not shaping, is
the cost. A user-selectable full-emulator surface, Ghostty or Alacritty, is
a local conversion from runs and changes nothing on the server or the wire.

## D10. The PTY crate is portable-pty

All three candidates read at the kernel's pace with identical cost.
portable-pty alone has a ConPTY backend behind the same trait, which keeps
Windows cheap. The finding that matters is about the kernel, not the crate:
see [constraints.md](./constraints.md).
