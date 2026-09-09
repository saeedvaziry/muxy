# Terminal

Muxy's terminals are powered by [libghostty](https://github.com/ghostty-org/ghostty), running on a Metal layer for fast, GPU-accelerated rendering.

## Backend architecture

Muxy currently ships Ghostty as its terminal backend. Pane hosting, remote control, quick terminal creation, search, rich input, process detection, and offline lifecycle depend on Muxy's backend-neutral terminal surface contract. Optional integrations use dedicated capability protocols, so unsupported search, remote snapshots, client themes, offline lifecycle, raw output, and attachment upload behavior is never invoked. Image and file attachments fall back to escaped local file paths when a backend does not support attachment uploads. Ghostty-specific handles and callbacks stay inside the Ghostty implementation boundary. There is no user-facing backend selector until another implementation satisfies the capabilities required by these integrations.

## Background sessions

**Settings → Terminal → Background sessions** runs each new terminal in a detached process so it survives quitting Muxy, the way tmux does. It is off by default and only affects terminals opened after it is switched on; terminals that are already open keep their current behavior.

libghostty owns the PTY of every surface it creates and offers no way to hand a surface an existing PTY, so a background terminal cannot simply be adopted. Muxy uses the same split tmux does. A `muxy-session` daemon owns the real PTY of every background session and lives in its own session (`setsid`), which is what lets it outlive the app. The surface runs `muxy-session attach`, a small client that proxies bytes, window size, and the final exit status over a unix socket. Quitting Muxy kills the clients and leaves the daemon and its shells running.

Each pane carries a session ID that defaults to its own pane ID and is persisted in workspace snapshots, so reopening Muxy reattaches each restored pane to its own session. Because the two IDs are separate, a pane can also adopt a session that started somewhere else. On reattach the daemon replays a capped 256 KB buffer of recent ordinary output and sends `SIGWINCH` to the foreground process group so full-screen programs repaint. Replay is sanitized and best-effort: incomplete UTF-8, incomplete terminal control sequences, and terminal queries that would generate new input responses are dropped instead of sent to the new surface. Alternate-screen programs skip raw replay while they are active and rely on redraw. The daemon exits on its own once it has been idle with no sessions.

The attach client starts a daemon on demand and keeps retrying the whole connect-and-launch cycle until it succeeds or its budget runs out, so a terminal opened while an idle daemon is shutting down does not fail. A connection that drops before the session is handed over is retried on a fresh socket rather than reported as an exit, and a daemon that is idling out accepts anything still waiting in its listen backlog before it closes.

Every attach carries metadata identifying the project, worktree, tab, and title the session belongs to, and the daemon refreshes it each time a client reattaches. That is what lets Muxy group sessions by worktree and show which tab owns one. The attach also carries a protocol version; a session started by a different version of Muxy is refused with a clear message rather than being misread.

### Recovering sessions

Reopening Muxy reattaches every restored pane whose session is still running, without waiting for you to click the tab, so output keeps flowing and scrollback keeps building from the moment the window appears.

Closing a tab ends its session. To keep a local background-backed tab running without its visible terminal, right-click inside the terminal and choose **Send to Background**. The tab closes without stopping its processes and its sessions appear in the status bar as detached. A split tab moves all of its terminal sessions together. The action is unavailable for pinned tabs, mixed-content splits, remote terminals, and terminals opened without background sessions enabled.

Sessions are never killed behind your back otherwise: one whose tab is gone keeps running and shows up in the status bar as detached, where you can reattach or stop it. Sending the final visible tab to the background keeps its project and worktree open so the status-bar entry remains accessible.

When an attach client goes away, Muxy asks the daemon what actually happened before touching the tab. Only a session the daemon reports as gone closes its tab; a session that is still running, or a daemon that cannot be reached, is treated as a dropped connection and reattached with a short backoff. If several attempts in a row fail, the tab stays open showing a **Reconnect** placeholder and the session keeps running, so a daemon crash or a version mismatch after an update can never close tabs or discard work. A tab that has been healthy for a while starts its retry budget over.

The status bar lists only the sessions in the current project and worktree that **no tab currently owns**, since a session already open in a tab is not something you need to recover. Ownership is decided by the pane pointing at the session, not by whether a client happens to be connected, so a tab whose terminal was freed by the idle-memory setting still counts as owning its session. When every session in the worktree is open in a tab, the status bar item disappears entirely.

Each row shows the session's title and working directory with three actions:

- **Active tab** points the focused terminal tab at that session. The session the tab was showing keeps running and becomes detached, so it takes the vacated slot in the list.
- **New tab** opens a tab in the current worktree attached to that session.
- **Stop** ends the session and everything running inside it.

A session can only ever be owned by one tab, because the daemon accepts a single attached client. Adopting a session hands its previous owner back a fresh session of its own rather than leaving that tab looking at a dead terminal.

Ghostty only injects its shell integration into shells it spawns itself, so the daemon reproduces that injection using the contract the bundled scripts document: `ZDOTDIR` plus `GHOSTTY_ZSH_ZDOTDIR` for zsh, `ENV` plus `GHOSTTY_BASH_INJECT` and POSIX mode for bash, and `XDG_DATA_DIRS` plus `GHOSTTY_SHELL_INTEGRATION_XDG_DIR` for fish, elvish, and nushell. Background terminals in those shells keep working-directory tracking, tab titles, prompt marks, and AI progress. Other shells run normally but lose those integrations, exactly as they do under tmux.

Because the surface's own foreground process is the attach client, Muxy resolves the real foreground process from the session's tty through `sysctl(KERN_PROC_TTY)`. Agent detection, the running-process close confirmation, the idle-memory sweep, and AI hook to pane mapping behave the same as in a normal terminal. Idleness in particular has to come from the session rather than the surface: the attach client is not a shell, so judging it locally would keep every background terminal awake forever and disable the idle-memory setting app-wide. A session whose state cannot be resolved is treated as busy and left running.

The control socket lives in Muxy's Application Support directory with `0700` on the directory and `0600` on the socket, falling back to a user-scoped `/tmp/muxy-<uid>` only when the primary path exceeds the 104-byte `sun_path` limit. The daemon verifies the peer's uid with `LOCAL_PEERCRED` and refuses connections from other users. A development build uses its own `sessions-dev` directory and `/tmp/muxy-dev-<uid>` fallback, so running a dev build alongside the release app keeps two fully separate daemons — neither build can list, adopt, or stop the other's sessions.

Remote SSH panes and the quick terminal are never backed by a background session.

`muxy list-sessions` and `muxy kill-session --session <id>` manage sessions from a shell. See [Muxy CLI](muxy-cli.md).

## Quick terminal

Assign Double Shift or a conventional global shortcut to open the quick terminal from anywhere. On a display with a camera cutout, it expands out of the cutout like a dynamic island. It always starts in your home directory and keeps the same shell, working directory, and history while hidden.

Dismiss it with the assigned shortcut, the close button, or a click anywhere outside the panel. Moving the pointer and pressing Escape do not close it, so Escape reaches the terminal for `vim`, `less`, and other full-screen programs. On a display without a camera cutout, the terminal opens at the same top-center position.

Quick Terminal has no shortcut assigned by default. Open **Settings → Quick Terminal** to choose one. System-wide Double Shift requires **System Settings → Privacy & Security → Input Monitoring**; conventional global shortcuts do not.

The same settings section can disable Quick Terminal entirely and controls the terminal width, height, transparency, and background vibrancy. Disabling it stops the shortcut listener, closes an open panel, and releases its shell. The shortcut and appearance settings remain saved; enabling it again starts a fresh shell. The in-place gear button provides the size and appearance controls while the terminal is running. Sizes are stored in points, constrained to 480–1200 wide and 280–800 high, and automatically reduced when the active display is smaller. Transparency ranges from 0–55%, and vibrancy uses a continuous 0–100% native material intensity.

Vibrancy controls how much of the native macOS material participates in the background composition. It does not set a custom blur radius, which AppKit does not expose for system materials.

Transparency and vibrancy apply only to the terminal workspace while preserving the active Ghostty theme. The cutout bridge and its controls stay solid for readability, and the main window follows the separate **Settings → Interface → Appearance** controls. Muxy uses an opaque, unblurred fallback when macOS Reduce Transparency or Increase Contrast is enabled.

The quick terminal is available while Muxy is running. Closing Muxy's main window still follows the existing quit behavior.

## App transparency

**Settings → Interface → Appearance** brings the same transparency and vibrancy controls to the main window: terminal panes, the top bar, and the status bar. Transparency ranges from 0–55% and defaults to 0, keeping the window opaque until it is raised. Vibrancy mixes the native macOS material from 0–100% behind the transparent background; because the main window itself stays opaque, the desktop shows through the vibrancy material and the effect needs a vibrancy above zero. The sidebar keeps its own vibrancy toggle.

Changes apply immediately to open terminals, including split panes, and to the top bar and status bar. The active Ghostty theme is preserved: its background color is drawn as a tint over the material at the configured opacity, and colored cell backgrounds render normally. Muxy renders the workspace opaque and unblurred while macOS Reduce Transparency or Increase Contrast is enabled. A terminal pane controlled by a remote device keeps its opaque client theme until control returns to the Mac. The settings are stored as `muxy.app.transparency` and `muxy.app.blur` in `settings.json`.

## Configuration

Muxy's active Ghostty config is `~/Library/Application Support/Muxy/ghostty.conf`. On first launch Muxy seeds it from `~/.config/ghostty/config` when that file exists; after that, Muxy reads and writes its own copy. Open it with **Muxy -> Open Configuration...**, reload after editing with `⌘⇧R`.

Most Ghostty options work — fonts, colors, padding, keybinds, shell integration. Muxy applies the active light/dark variant automatically when the system appearance changes.

### Chinese font rendering

Muxy maps common Chinese Unicode ranges to one font so Ghostty does not mix fallback faces within the same text. It uses the first configured `font-family` with broad Simplified Chinese, Traditional Chinese, and punctuation coverage; otherwise it uses the macOS system fallback.

Keep the Latin terminal font first and add the preferred Chinese font as a fallback:

```ini
font-family = JetBrains Mono
font-family = PingFang SC
```

Reload the configuration with `⌘⇧R`, then open a new terminal. Ghostty applies codepoint-map changes only to new terminals. Explicit `font-codepoint-map` entries in `ghostty.conf` take priority over Muxy's automatic mapping for overlapping ranges.

## Find in terminal

`⌘F` opens an inline search overlay scoped to the focused pane. Enter / Shift-Enter cycle through matches; Escape dismisses.

## Copy and paste

| Action | Shortcut |
| --- | --- |
| Copy (with selection) | `⌘C` |
| Send `^C` to program | `⌘C` with no selection |
| Paste | `⌘V` or right-click → Paste |
| X11 selection paste | Middle-click |

Enable **Settings -> Terminal -> Auto-copy terminal selection** to copy selected terminal text on mouse release.

### Attachments in SSH panes

A Mac file path does not resolve on a remote device, so an SSH pane uploads every attachment it receives and
inlines the remote path instead.

A pane counts as remote in two ways. A tab opened against a device configured under **Settings -> Remote Devices**
carries its destination directly. A pane where you typed `ssh` yourself is detected from the foreground process:
Muxy reads the running `ssh` invocation and reconstructs simple destinations using `user@host`, `-p`, `-l`, one
`-i`, or an `ssh://` URL. Config aliases are kept verbatim so `ssh_config` still resolves them. Relative identity
paths are resolved from the running SSH process's working directory. Uploads open their own connection to that
destination and share one multiplexed control socket, so a password-less key or agent is required. Invocations
with remote commands, multiple identities, or options Muxy cannot reproduce exactly are left alone and the pane
keeps local-path behavior. This includes `-J`, `-F`, `-W`, `-P`, `ProxyJump`, `ProxyCommand`, and unrepresented
`-o` settings. Identity paths containing environment or percent-token expansion are also refused.

Muxy refuses a chained invocation such as `ssh bastion ssh target`. If another SSH session is started later from
inside an interactive remote shell, macOS still exposes only the first local SSH process. Muxy cannot identify the
later hop, so attachment upload in nested interactive SSH sessions is unsupported.

Muxy accepts attachments through four routes:

| Route | Accepts |
| --- | --- |
| `Ctrl+V`, `Cmd+V`, right-click Paste | Clipboard image data, or files copied in Finder |
| Drag and drop onto the pane | Files |
| Composer image attachments | Images |
| Composer file attachments | Files |

Clipboard image data is converted to PNG away from the main UI thread. Files are streamed as-is with their
extension preserved, so the receiving TUI still recognizes the type; the remote name is the upload identifier
rather than the original file name. Each upload lands in a private, session-scoped temporary directory on the
remote device, and the remote path is pasted into the running TUI, letting tools such as Codex and Claude Code
read the attachment without access to the Mac.

Encoded image input and converted PNG output are limited to 25 MB, and decoded images are limited to 64
megapixels. Other files, including empty files, are limited to 100 MB. The upload timeout scales with payload size.
Every attachment must be a regular file, so directories, device files, and unbounded streams are rejected with a
toast naming the file.

Uploaded directories and files use owner-only permissions. Partial uploads are removed when an upload is
interrupted, and the session directory is removed when its terminal ends. A central cleanup coordinator retains
outstanding cleanup for both open and already-removed panes. On app quit, Muxy waits up to five seconds for those
tasks before allowing termination to continue. Text paste behavior is unchanged.

The **Settings -> Terminal -> Composer -> Image Submission** strategy applies to local panes only. When a Composer
upload fails, Muxy withholds Return and clears every line it has already submitted, so a partial prompt is never
left in the TUI. A dropped or pasted batch inlines whichever files uploaded successfully.

Local panes inline the escaped local path for dropped and pasted files, and are unaffected by upload limits.

## Mouse

Plain left-click and drag selects terminal text. `⌘` and right-click are reserved for Muxy, and neither starts
nor changes a text selection:

| Gesture | Result |
| --- | --- |
| `⌘` + left-click | Opens the file path or link under the pointer |
| `⌘` + left-drag | Moves the pane to another area or split |
| Right-click | Opens the [right-click menu](#right-click-menu) |

A `⌘` + left-drag is decided when the gesture starts, so pressing or releasing `⌘` mid-drag never switches
between moving the pane and selecting text.

Programs that enable mouse reporting keep receiving right-click. Hold `Shift` while right-clicking such a program
to get Muxy's menu instead.

## Working directory

Muxy tracks the cwd via Ghostty's shell integration (OSC 7). The directory is persisted in workspace snapshots so newly recreated tabs land in the same folder when applicable.

Remote terminals use the selected SSH device's environment before starting the remote login shell. New SSH devices default to `TERM=xterm-256color`; edit the device in Settings -> Remote Devices to change or remove it.

## Muxy CLI

Use the `muxy` command to open projects and control panes from a shell or automation script. See [Muxy CLI](muxy-cli.md).

## Custom command shortcuts

Define reusable shell command shortcuts in **Settings → Commands**:

- Display name, command, optional icon, optional keybinding.
- Triggering one creates a new tab and runs the command.
- Useful for `npm run dev`, `make watch`, `just test`, …

## Composer

`Cmd+I` opens the composer for multiline prompts, files, images, and broadcast sends. The default **Panel**
presentation opens over the workspace without taking layout space. Use the panel header's pin control to dock it
beside the workspace or float it again, resize it from its workspace-facing edge, and move it between the right and
bottom positions. The panel's float/dock choice and position persist across app restarts.

Choose **Floating** under **Settings -> Composer -> Presentation** or **Use Floating Composer** from the panel's
More menu to open the separate centered modal instead. Choose **Use Composer Panel** from the floating Composer's
More menu to switch back. The presentation choice and both layouts' sizes also persist across app restarts, and
switching an open Composer preserves its draft and attachments.

Use **Settings -> Composer** or the Composer's More menu to control automatic clearing. **Clear After Sending**
removes the submitted text and attachments only after every target finishes successfully. If the draft changes
while a submission is finishing, Muxy preserves the newer content. **Clear on Close** removes the draft whenever
the Composer closes, including floating auto-close, target loss, and panel displacement. When an open Composer panel
moves to another worktree, it clears the previous worktree's draft while preserving drafts across pane changes in
the same worktree. Both options are off by default.

`Cmd+Shift+I` opens the legacy voice recorder normally, or starts on-device dictation inside either Composer
presentation when it is already open. Stop Composer dictation to insert the transcript at the editor cursor, or
press Return while recording, then edit or send it normally. Composer dictation requires an installed on-device
speech language plus microphone and speech recognition access.

Long prompts in the floating presentation can use a larger Composer. Drag its left, right, or bottom edge to
resize it; because the Composer stays centered, a dragged edge grows the box symmetrically and follows the
pointer. The expand button in the Composer header switches to a large preset instead, which is capped so it stays
readable on big displays and shrinks to fit smaller windows; dragging an edge is what grows the Composer beyond
that preset, and pressing the button again restores the size you dragged. Both the size and the expanded state
are stored independently of the UI Scale preset and always clamped to the current window, so a smaller window
shrinks the Composer without losing your size. **Reset Composer Size** in the Composer's More menu returns it to
the default size. The text editor takes whatever room the box has left, so attachments and the dictation status
line reduce it while they are visible. `Cmd+=` and `Cmd+-` still change the Composer font size rather than the box.

Each Composer submission is serialized with later keyboard input for its target terminal. Text, attachment paths,
and the optional Return are submitted as one transaction, including when a Composer message is broadcast to
several panes. Broadcast targets are processed one at a time, and each unique image attachment is normalized once
into validated immutable PNG data that is reused across those targets.

Composer submission controls stay unavailable while dictation is starting or recording. A dictation error does
not block typed text from being sent, and its inline message can be dismissed without closing the composer.
Only one focused overlay is shown at a time, so Composer shortcuts do not open it over another modal.

The status-bar microphone remains available as the legacy voice recorder. It inserts the final transcript into
the control that was focused before recording and can optionally press Return afterward.

## Right-click menu

Inside a terminal pane: **Paste**, **Send to Background**, **Split Right**, **Split Left**, **Split Down**, **Split Up**, and **Terminal Settings…**. Terminal Settings opens Muxy's settings directly on the Terminal section. Send to Background closes an eligible local background-backed tab while leaving its processes running.

Opening the menu never selects terminal text. While a program has mouse reporting enabled the right button belongs to that program, so hold `Shift` to open the menu instead.

Splitting creates a child pane inside the current top-level tab. Each pane keeps its own terminal, browser, source-control, or extension surface, while a one-pixel divider replaces the old per-pane tab strip. Child panes do not appear as separate entries in the window tab strip or the Tab Focused sidebar. An agent running in a child pane does appear as its own entry in the Agents Focused sidebar, and selecting it activates both the child pane and its parent top-level tab.

Dragging a top-level tab toward an edge docks the whole tab beside another top-level tab. Its child-pane layout moves with it and remains independent from the neighboring tab's child panes.

The Agents Focused layout keeps the normal top-level tab strip in the title bar and limits sidebar tab entries to detected AI agents, including idle sessions. An entry disappears as soon as its agent process exits, even when the tab keeps running a shell. Projects and worktrees remain visible when they have no agent sessions, and their add menu can start a new tab with any available agent provider. Clicking a project or worktree row activates it; clicking the already active row expands or collapses its agent list. When project sorting is set to Manual, local project headers can be dragged to reorder project blocks while their children remain grouped with the project. Agent rows can be dragged only within their current worktree and docked tab group without changing non-agent tab positions. A project with no tabs offers the same launchers as icons — a terminal plus one monochrome icon per installed provider — instead of the plain new-tab button. Tabs started from this menu appear immediately. Local launch attribution is confirmed by process detection and removed if the command exits before confirmation. Remote availability is checked through the configured SSH connection before the menu enables a provider.

## Notifications from the terminal

OSC 9 and OSC 777 notification escape sequences are routed into Muxy's notification panel and (optionally) macOS notifications.

For AI coding agents (Antigravity CLI, Claude Code, Codex, Cursor, Droid, Grok, Kiro, OpenCode, Pi, Xal), Muxy uses hook-based lifecycle events rather than escape sequences — see [AI notifications](ai-notifications.md).

## Quick-select labels

Ghostty's quick-select feature lets you focus a pane or surface by typing a label key. Labels and bindings are configured in the Ghostty config.
