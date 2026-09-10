# Protocol

The app and server talk over a reliable, ordered byte stream. This is
terminal/session protocol. Exact types, kind numbers, and limits live in the
protocol and wire crates and their fixtures; this document says what they mean.

## Versions

Versions describe how peers encode requests and responses, not whether released
clients and servers may communicate. Every released version remains supported
permanently. Peers negotiate a mutually supported contract, so a newer release
can always communicate with an older release.

Until the first official release there is one mutable development schema, V1.
All current messages use it. Do not add versions or pre-release compatibility
adapters. Development app and server builds must use the same schema. This does
not permit losing saved user data when storage formats change.

After release, versioned contracts are immutable and the envelope and hello
remain stable. New contracts may be added, but released contracts are never
removed or inferred from a highest version number alone.

## Framing

| Field | Size | Meaning |
| --- | --- | --- |
| length | `u32` | Bytes that follow |
| version | `u16` | Version this frame is written in |
| channel | `u32` | `0` is control; each attachment gets its own |
| kind | `u8` | Six bits of kind, two flag bits reserved for compression and continuation |
| payload | the rest | Postcard, except raw terminal input |

Integers are little endian. A frame is at most 16 MiB. The flag bits are
zero in v1.

## Handshake

The client opens with a hello listing its versions and waits. The server
replies with its own; both choose a mutually supported contract. A malformed or
unsupported implementation may be rejected and closed. Official releases always
share a supported contract. Any other traffic before hello is fatal.

## Messages

| Message | From | Channel |
| --- | --- | --- |
| Hello, hello reply, version unsupported | client, server | control |
| Attach, detach, resize, and their replies | client, server | control |
| List, create, and end session, and their replies | client, server | control |
| Read saved terminal content, discard session and saved content, and their replies | client, server | control |
| History page and search, and their replies | client, server | control |
| Set terminal colors and its reply | client, server | control |
| Ping, pong | client, server | control |
| Frame ack | client | control |
| Session ended | server | control |
| Error | server | control |
| Input | client | session |
| Screen frame, metadata event | server | session |

A request carries a client-chosen ID and gets exactly one reply, in any
order. Errors about a request, such as a bad path, size, limit, or cursor,
an unknown session or channel, or a failed spawn, are correlated and leave
the connection usable. Anything malformed or out of place is fatal: the
server reports it and closes, and a client that sees it closes. Only the
server sends errors. Any number of clients may attach to one session.

## Screen

Rows are style runs with server-supplied cell boundaries, so the client
needs no width table. A row in a message replaces that row entirely. Frames
carry only visible rows, are numbered from one per attachment, and are
acked cumulatively. A resize carries the whole screen as one reset.
OSC 8 hyperlinks are bounded URI spans sent as whole-screen metadata
replacements, including an empty replacement when cleared. Their attachment
frame sequence prevents activation ahead of the matching screen; zero refers
to the attach snapshot. History carries no OSC 8 links. Detecting plain links
and choosing browser, editor, or Finder openers are app policy.

## Terminal colors

A client can send RGB defaults for foreground, background, cursor,
and the first 16 ANSI colors. These defaults apply to sessions created or
attached through that connection and updates are queued to its existing
attachments. New sessions receive them before processing terminal output.
The emulator uses them to answer terminal color queries; theme selection
stays in the app. Like size, defaults are session-wide: the latest update
or colored attach wins, and detaching leaves them unchanged. Clients resend
colors on reconnect and theme changes.

## Attach and metadata

Attach returns an atomic snapshot: size, screen, cursor, recent history
with a cursor to older rows, title, and working directory. The initial
hyperlink replacement follows the snapshot on the attachment channel.
Title, directory, process, and bell events follow on that channel; bell is
transient. Live prompt starts accompany attach snapshots and history pages,
indexed within their history rows followed by their screen rows. Screen prompt
updates are full replacements tied to an attachment frame sequence, so marks
never get ahead of the screen. Saved records do not yet retain prompt marks.
The app derives the pane title as program title, then process
name, then working directory, and the foreground process's shell flag
drives close confirmation.

## History

The server never pushes history; the app pages for it. A page or search
request freezes a view of the retained rows and walks it with an opaque
cursor until exhausted. Reflow or eviction makes a cursor stale. Search is
literal, optionally case-insensitive, within a row, and bounded in
results and work, so an empty reply may still continue.

## Paths

Server paths are lossless Unix bytes, preserved as sent. NUL is rejected
only when a path is handed to the OS.

## Lifecycle

Session ended carries the exit reason and reaches every connection once,
after final terminal content is saved and the session leaves live listings.
Saved content is read by session ID without creating an input channel.
Discard ends a live process and removes its saved content; repeated discard
is harmless. Its reply confirms completion, including pending saves.
Detach and session end retire the channel and drop its pending output;
late traffic on it is ignored.
Ordering across channels is guaranteed only at handshake, attach, resize,
metadata watermark, detach, and session end.

## Deferred

Compression, chunking, frame merging, credits, and flow control are runtime
work for the connectivity epics; D6 and D8 remain the target. Any wire changes
follow the version policy above.
