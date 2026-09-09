# Benchmarks

Methodology and summary results of the spike program that settled the
[decisions](./decisions.md). The spike code was discarded; this document is
enough to rebuild the measurements against product code.

Machine: Apple M3, 8 cores, 24 GB, macOS. Rust 1.92, release builds with
thin LTO. All numbers are medians of repeated runs unless stated.

## Workloads

Raw PTY output recorded once into replayable files with timestamps, so
every candidate saw identical bytes. Terminal size 200 by 50.

| ID | Workload | Size | Stresses |
| --- | --- | ---: | --- |
| W1 | Idle shell with prompt redraws every 2 s | 1 KB | Idle cost |
| W2 | Plain numbered-line dump | 200 MB | Parse throughput, coalescing |
| W3 | Coloured build log, 300K lines | 26 MB | Styled runs, history growth |
| W4 | Real vim session, recorded through a PTY with scripted keys, 11 s | 167 KB | Alternate screen, cursor moves, partial redraws |
| W5 | Full-screen monitor table redrawn at 10 Hz for 60 s | 2.7 MB | Periodic full redraws |
| W6 | Per-cell colour churn, 500 full screens | 56 MB | Pathological style churn |
| W7 | CJK, emoji, combining marks, box drawing | 4.6 MB | Width and grapheme handling |
| W8 | Long wrapped lines with resizes 200 to 120 to 200 to 80 to 200 columns | 18 MB | Reflow cost and correctness |
| Scale | 100 sessions, 30 running W3, 70 running W1 | | Aggregate memory, fairness |

## Metrics

| Metric | How |
| --- | --- |
| Memory | Physical footprint and RSS sampled every 100 ms; heap via a counting allocator where the code is Rust |
| CPU | Process user plus system time |
| Throughput | Input bytes per second of wall time |
| Wire bytes | Counted at the transport, before and after compression |
| Latency | Attach request to first complete screen; control ping round trip under load, p50 and p99 |
| Frame time | Paint duration p50 and p99 in the renderer |
| Correctness | Final screen text and cursor compared line by line across candidates, plus a mid-session dump for interactive workloads |

Hard fails: a wrong screen on W4, W7, or W8; an incompatible licence; an
unmaintained crate.

## Engine, 10K rows of history

| Engine | Memory, build log | Memory, 100K rows | Parse, plain dump | Parse, build log | Parse, vim | Parse, churn | Reflow peak, 100K rows |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Ghostty, compressed | 1.7 MB | 8.5 MB | 233 MB/s | 317 MB/s | 307 MB/s | 272 MB/s | 29 MB |
| WezTerm core | 3.3 MB | 38.8 MB | 37 MB/s | 49 MB/s | 27 MB/s | 115 MB/s | 68 MB |
| Custom minimal grid | 4.3 MB | 39.0 MB | 60 MB/s | 182 MB/s | 277 MB/s | 331 MB/s | no reflow |
| Ghostty, uncompressed | 19.8 MB | 196.8 MB | 233 MB/s | 317 MB/s | 307 MB/s | 272 MB/s | 175 MB |
| Alacritty | 51.0 MB | 507.4 MB | 146 MB/s | 123 MB/s | 172 MB/s | 271 MB/s | 1,300 MB |
| vt100 | 70.6 MB | 704.5 MB | 20 MB/s | 85 MB/s | 130 MB/s | 279 MB/s | no reflow |

Compression cost: 3 ms per 10K rows, 34 ms per 100K rows of build log.

## Placement, two processes over a Unix socket, Alacritty on the server

| Mode | Server memory, W2 flood | Server memory, 100 sessions | Client memory, 100 sessions | Wire, W3 flood | Wire, vim | Client CPU, W2 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Server grid, cells | 51 MB | 2,061 MB | 9 MB | 0.7% | 719% | 0.001 s |
| Server grid, VT re-stream | 51 MB | 2,060 MB | 561 MB | 0.2% | 137% | 0.002 s |
| Raw bytes | 389 MB | 1,280 MB | 2,340 MB | 100% | 100% | 1.4 s |
| Hybrid | 63 MB | 3,271 MB | 689 MB | 100% | 101% | 1.3 s |

Same server with Ghostty: 21 MB per busy session uncompressed, 3 MB
compressed, 135 MB for 100 sessions, 8.1 s CPU against 14.4 s.

## Wire encoding, run shape

| Format | Size vs postcard | Encode CPU vs postcard | Decode CPU vs postcard |
| --- | ---: | ---: | ---: |
| postcard | 1.00 | 1.0 | 1.0 |
| bincode 2 | 1.05 to 1.13 | 0.9 to 1.2 | 0.7 to 1.0 |
| MessagePack | 1.13 to 1.30 | 1.1 to 1.3 | 1.1 to 1.5 |
| protobuf | 1.15 to 1.60 | 1.0 to 1.6 | 1.1 to 1.5 |

| Compression | build log | vim | monitor | churn | Encode µs per vim frame |
| --- | ---: | ---: | ---: | ---: | ---: |
| none | 25.1 KB | 181 KB | 2,750 KB | 1,129 KB | 11 |
| lz4 | 9.5 KB | 91 KB | 1,585 KB | 468 KB | 12 |
| zstd 1 per frame | 6.9 KB | 62 KB | 912 KB | 216 KB | 19 |
| zstd 3 per frame | 6.8 KB | 60 KB | 932 KB | 197 KB | 22, 1.7 ms on churn |
| zstd 1 trained dictionary | 2.4 KB | 71 KB | 977 KB | 214 KB | 32 |
| zstd 1 streaming context | 4.3 KB | 29 KB | 829 KB | 211 KB | 15 |

| Message, zstd 1 streaming | Typical size |
| --- | ---: |
| Keystroke echo frame | under 300 bytes |
| Full-screen redraw, 200 by 50, text | 1 to 2 KB |
| Attach snapshot with 200 history rows | 2 to 5 KB |
| Pathological per-cell colour churn, one screen | about 11 KB |

## Transport and flow control, 100 channels

| Transport, queue everything | Flood throughput | Control RTT p50, paced | Control RTT p99, paced | Server CPU, 9.7 s paced | Client CPU |
| --- | ---: | ---: | ---: | ---: | ---: |
| stdio pipe | 150 MB/s | 0.18 ms | 1.5 ms | 1.27 s | 0.48 s |
| TCP loopback | 144 MB/s | 0.45 ms | 5.6 ms | 1.65 s | 0.90 s |
| Unix socket | 136 MB/s | 0.21 ms | 2.0 ms | 1.27 s | 0.51 s |

| Slow client, 3 ms per frame | Frames sent | Server peak memory | Wall | Client CPU |
| --- | ---: | ---: | ---: | ---: |
| queue everything | 22,200 | 128 MB | 93.6 s | 1.60 s |
| merge per channel with credit | 2,273 | 3.3 MB | 10.0 s | 0.13 s |

## PTY

| Crate | Spawn to exit | Read throughput, seq | Reader CPU for 168 MB | Bytes per read |
| --- | ---: | ---: | ---: | ---: |
| portable-pty | 4.6 ms | 17.0 MB/s | 9.3 s | 20 |
| rustix-openpty | 3.3 ms | 16.7 MB/s | 9.5 s | 20 |
| pty-process | 3.0 ms | 16.9 MB/s | 9.4 s | 20 |

Controls: seq through a pipe 68 MB/s; seq through a pty 12 MB/s; cat of a
17 MB file through a pty 280 MB/s.

## Rendering, GPUI, forced 60 Hz redraw

| Panes | vim fps / CPU / paint p50 | monitor fps / CPU / paint p50 | churn fps / CPU / paint p50 |
| --- | ---: | ---: | ---: |
| 1 | 60 / 20% / 1.3 ms | 60 / 24% / 3.3 ms | 59 / 100% / 16.5 ms |
| 4 | 60 / 26% / 2.8 ms | 60 / 34% / 4.9 ms | 44 / 100% / 21.2 ms |
| 16 | 60 / 31% / 3.9 ms | 60 / 45% / 6.7 ms | 60 / 92% / 12.4 ms |

## Appendix

The full per-metric tables generated from the raw measurements are in
[benchmarks/](./benchmarks/), one file per spike.

## Not measured

Real SSH and Docker exec transports; end-to-end input latency through a
PTY; style flags and history-on-attach through the Ghostty adapter; frame
chunking for control latency under floods; Linux and Windows.
