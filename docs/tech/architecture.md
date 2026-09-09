# Architecture

This document turns the [decisions](./decisions.md) into components and
boundaries. It stays above code: names are roles, not types.

## Processes

```mermaid
flowchart LR
    subgraph APP["App process · Rust + GPUI"]
        UI["Windows, projects, tabs, panes"]
        GRID["Run grid per attached session"]
        CONN["Connection per server"]
    end
    subgraph SERVER["Server process · Rust, one per host"]
        ACCEPT["Listener / stdio entry"]
        CLIENT["Client connection<br/>reader · writer thread · outbox"]
        S1["Session thread 1"]
        S2["Session thread 2"]
        SN["…"]
    end
    UI --> GRID
    GRID <--> CONN
    CONN <-->|"byte stream"| ACCEPT
    ACCEPT --> CLIENT
    CLIENT <--> S1
    CLIENT <--> S2
    CLIENT <--> SN
```

- The app never parses terminal output. It holds, per attached session, the
  visible rows as style runs plus whatever history window it has fetched.
- The server knows sessions and clients, nothing about projects or panes,
  as the [server model](../product/server-model.md) requires.
- A server reached remotely is the same binary started in stdio mode by
  whatever exec mechanism reaches the host. The protocol is identical.

## Session thread

```mermaid
flowchart TB
    PTY["PTY reader thread<br/>blocking read, 64 KB buffer"] -->|"bytes over a channel"| OWNER["Session owner thread"]
    INPUT["Input, resize, attach requests"] --> OWNER
    OWNER --> TERM["Ghostty terminal<br/>grid + byte-budgeted history"]
    OWNER -->|"every 16 ms if anything changed"| FRAME["Frame: rows whose content hash changed, as style runs, plus cursor"]
    OWNER -->|"first quiet tick after output"| ZST["Compress history pages"]
    FRAME --> OUTBOX["Per-client outbox"]
```

- One thread owns the terminal for the whole session lifetime. The Ghostty
  terminal is not sendable, so nothing else touches it; all requests arrive
  over the owner's channel.
- The PTY reader is a separate blocking thread because the kernel delivers
  small reads for chatty producers and the read syscall is the dominant
  cost. It does nothing but read and forward.
- The tick collects rows whose content hash changed since last sent to each
  client. Nothing is emitted when nothing changed.
- History compression runs on the first tick with no new output after a
  burst. One full pass costs single-digit milliseconds per session.
- Retention is a byte budget on Ghostty's compressed pages. Saved history keeps
  that same retained window, not a second cutoff on decoded row sizes.
- The owner captures screen and history checkpoints for a background writer.
  The writer atomically replaces each saved record; newer pending checkpoints
  replace older ones. Final output is saved before normal exit is announced.
  Indexed saved records allow reading a screen or history page without decoding
  the entire history; search reads a bounded window of rows.

## Client connection

```mermaid
flowchart LR
    RX["Reader thread"] -->|"input, acks, pings"| ROUTE["Immediate routing"]
    RX -->|"other requests"| WORK["Bounded, ordered request worker"]
    OUTBOX["Outbox<br/>control queue · one pending merged frame per session · one credit per session"] --> TX["Writer thread<br/>control first, then any session with credit"]
    TX -->|"postcard · zstd streaming · channel framing"| STREAM["Byte stream"]
```

- The connection reader never waits for storage, process creation, or owner
  replies. Bounded background work handles these requests; lifecycle work stays
  ordered. Registry locks cover bookkeeping, not blocking operations.
- Frames for a session replace or merge into the single pending frame for
  that client. The frame is sent when the client's acknowledgement for the
  previous frame on that channel has arrived.
- Control messages, including pongs, attach replies, and history pages,
  never wait behind screen data.
- Attach sends the visible screen and a recent history window as runs.
  Older history is served in pages on request.
- Any number of clients may attach to a session; each has its own outbox
  and credits, so a slow client never affects a fast one.
- Merging, credits, control-first writing, and compression are runtime
  work; the [protocol](./protocol.md) says what v1 puts on the wire.

## App rendering

The app composes its views from muxy-ui, a reusable GPUI component library.
The library owns visual primitives and interaction behavior; app state and
server connections remain in the app. Shortcut identifiers, defaults, aliases,
and contexts share one headless catalog. Every module registers its handlers
through the same UI registration interface, resolved from the app keymap.
Widgets do not maintain a separate set of bindings.

The app dispatches blocking client requests on a bounded, ordered worker lane,
separate from input and frame acknowledgements. Events for established channels
remain immediate; bounded buffering keeps new-channel events and disconnects
behind their lifecycle completions. A flush waits for outstanding work.

```mermaid
flowchart LR
    STREAM["Byte stream"] --> DEC["Decode frame<br/>zstd streaming · postcard"]
    DEC --> GRID["Run grid for the session"]
    GRID -->|"on frame arrival"| PAINT["Paint pane<br/>one shaped line per row · one quad per run · cursor quad"]
    GRID -.->|"optional, per user setting"| EMU["Full-emulator surface<br/>Ghostty or Alacritty fed from runs"]
```

- The app redraws a pane only when a frame arrives or the viewport changes.
- Per-pane render state exists only for visible panes; the server holds
  everything else, and a pane that becomes visible attaches or re-fetches.
  Cached client content is displayed first, including while disconnected.
  Server reads refresh it lazily; the server owns durable retained history.
- The optional full-emulator surface is a local rendering choice that does
  not change the server or the protocol.

## Transport adapters

| Situation | Adapter | Notes |
| --- | --- | --- |
| Server on this device | Unix domain socket | Default. Path owned by the app's server settings. |
| Server reached through SSH, Docker exec, kubectl exec | Server started in stdio mode by the exec mechanism | Authentication and encryption are the exec mechanism's. No protocol change. |
| Server exposed on a port | TCP with TLS and a token | Last resort; costs twice the client CPU and latency of the socket on loopback. |

## Lifecycle notes

- Stopping the server ends its sessions. Settings and saved terminal records
  survive. A record holds the last saved screen, bounded history, and any
  known exit reason; recovery never restarts its process.
- The app retrieves saved content through the server protocol and keeps ended
  panes until the user closes them. Closing a pane discards its saved record.
- Graceful shutdown saves terminal content. An abrupt stop recovers the last
  completed checkpoint, which may omit the most recent output.
- A client that disconnects leaves sessions running; its outbox is dropped.
- Resize is a request to the session owner; the terminal reflows and the
  next tick emits the full visible screen.
