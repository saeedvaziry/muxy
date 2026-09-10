# Constraints

Facts established during the spikes that constrain implementation. Each one
cost time to learn.

## Platform

- The first release supports macOS only. Server paths on the wire are Unix
  pathname bytes; other platforms are a later release.

## Ghostty terminal core

- Build with `LIBGHOSTTY_VT_SYS_OPTIMIZE=ReleaseFast`. The default Zig
  build is Debug with integrity assertions and runs hundreds of times
  slower; the spike lost an hour to it before a stack sample showed the
  assertion.
- The terminal is not sendable and the C API is not thread-safe. One thread
  owns each terminal; everything else talks to that thread.
- `max_scrollback` is a byte budget in practice, whatever the Rust docs say.
  The product's retention is bytes.
- Compression is caller-driven. The server decides when a session is idle
  and calls one full pass; decompression on access is transparent.
- Physical footprint, not RSS, is the metric that reflects compression,
  because released pages are `madvise`d rather than freed.
- The crate API is marked unstable. Pin the version and wrap it behind one
  module.
- Style flags and history rows through the render iterator are not yet
  wired in the spike adapter. Both exist in the API.

## PTY

- The kernel charges per producer write on both sides of a pty. A
  line-at-a-time producer caps near 12 MB/s on macOS and forces one read
  syscall per line; a block writer moves 280 MB/s through the same pty.
- Reads therefore arrive small and frequent for chatty programs. The reader
  must be a dedicated blocking thread that only reads and forwards; sleeping
  to batch reads stalls the producer because the kernel pty buffer is only
  a few kilobytes.
- Consequence: engine parse speed is never the server's bottleneck. CPU
  budget follows the producer's write pattern, not its byte volume.

## Sockets and processes

- On macOS an accepted Unix socket inherits the listener's non-blocking
  flag. Set blocking explicitly on every accepted stream.
- TCP on loopback costs about twice the client CPU and twice the control
  latency of a Unix socket for the same traffic.
- A queue that is not bounded by merging grows by the full output rate
  whenever a client stalls; 128 MB in one slow-client run. Merging per
  channel is not an optimisation, it is the memory bound.

## Rendering

- GPUI holds 60 fps for 16 panes of ordinary content at 31 to 45 percent
  of a core with a forced redraw every frame. Redraw on demand makes idle
  panes free.
- Per-cell colour churn is the pathological case, at 10,000 runs per
  screen. Merge quads by colour and skip shaping for blank runs before
  worrying about anything else.
- Shaped-line caching does not pay; paint submission is the cost.

## Measurement

- The workloads, metrics, and hard-fail rules in
  [benchmarks.md](./benchmarks.md) are the regression baseline. Re-measure
  after any change to the engine, the frame shape, or the flow control.

## Shell startup

The server installs private hooks beside its socket. zsh and fish load them
without editing user startup files. `shell_integration = false` in `server.toml`
disables them for new sessions after a server restart. Shell-native integration,
such as fish 4's prompt marks, is left alone.

Bash keeps its normal login startup. To opt in, source the hook from the
interactive startup file your Bash profile loads:

```bash
if [[ ${MUXY_SHELL_INTEGRATION:-0} == 1 ]]; then
    source "$MUXY_SHELL_INTEGRATION_DIR/muxy.bash"
fi
```

An existing Bash DEBUG trap is preserved; command-start and exit-status marks
are omitted in that case, but prompt navigation still works.
