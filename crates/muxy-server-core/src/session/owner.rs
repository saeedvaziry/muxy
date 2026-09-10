use std::collections::HashMap;
use std::error::Error;
use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use muxy_protocol::{
    AttachSnapshot, ChannelId, ErrorCode, ExitReason, HistoryCursor, HistoryPage, InputModes,
    MetadataEvent, SavedScreen, ScreenFrame, SearchPage, SessionInfo, Size, TerminalColors,
};
use muxy_pty::{ExitStatus, Pty, PtyEvent};
use muxy_terminal::Terminal;

use crate::archive::{Archive, bound_history_page, history_range};
use crate::error::ServerError;
use crate::search::Search;
use crate::session::frames::{cursor, input_modes, modes, mouse, pty_size, rows, terminal_size};
use crate::session::metadata::Metadata;
use crate::session::{AttachmentEvent, AttachmentId, SessionCommand, SessionHandle};

const TICK: Duration = Duration::from_millis(16);
const CHECKPOINT: Duration = Duration::from_secs(1);
const METADATA_POLL: Duration = Duration::from_secs(1);

type Fault = Box<dyn Error + Send + Sync>;

#[derive(Debug)]
pub(crate) enum OwnerEvent {
    Command(SessionCommand),
    Pty(PtyEvent),
    WriteFailed(io::Error),
}

enum Wake {
    Event(OwnerEvent),
    Tick,
    Checkpoint,
    Metadata,
    Orphaned,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum OutputState {
    Open,
    Closed,
}

struct Attachment {
    sink: Sender<AttachmentEvent>,
    seq: u64,
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "Pending work flags and the application's cursor mode are independent states"
)]
struct Owner {
    info: SessionInfo,
    pty: Pty,
    terminal: Terminal,
    metadata: Metadata,
    size: Size,
    events: Receiver<OwnerEvent>,
    input: Sender<Vec<u8>>,
    attachments: HashMap<AttachmentId, Attachment>,
    output_pending: bool,
    resize_pending: bool,
    compress_pending: bool,
    history_total: u64,
    history_generation: u64,
    input_modes: InputModes,
    cursor_blinking: bool,
    links: Vec<muxy_protocol::LinkRow>,
    frame_state: Option<(muxy_protocol::Cursor, muxy_protocol::Modes)>,
    next_tick: Option<Instant>,
    output_state: OutputState,
    archive: Archive,
    next_checkpoint: Option<Instant>,
    next_metadata: Instant,
}

pub(crate) fn start(
    info: SessionInfo,
    mut pty: Pty,
    size: Size,
    history_budget_bytes: usize,
    archive: Archive,
    colors: Option<TerminalColors>,
    on_exit: impl FnOnce(ExitReason) + Send + 'static,
) -> Result<SessionHandle, ServerError> {
    let id = info.id.get();
    let (sender, receiver) = mpsc::channel();
    let (pty_sender, pty_receiver) = mpsc::channel();
    let (ready_sender, ready) = mpsc::channel();
    pty.start_reader(pty_sender)
        .map_err(ServerError::spawn_failed)?;

    let forward = sender.clone();
    thread::Builder::new()
        .name(format!("session-{id}-pty"))
        .spawn(move || {
            for event in pty_receiver {
                if forward.send(OwnerEvent::Pty(event)).is_err() {
                    return;
                }
            }
        })
        .map_err(ServerError::spawn_failed)?;

    let session = info.clone();
    let failed = sender.clone();
    thread::Builder::new()
        .name(format!("session-{id}"))
        .spawn(move || {
            let terminal = match Terminal::new(terminal_size(size), history_budget_bytes).and_then(
                |mut terminal| {
                    if let Some(colors) = colors {
                        set_colors(&mut terminal, colors)?;
                    }
                    Ok(terminal)
                },
            ) {
                Ok(terminal) => terminal,
                Err(error) => {
                    let _ = pty.kill();
                    let _ = pty.wait();
                    let _ = ready_sender.send(Err(ServerError::spawn_failed(error)));
                    return;
                }
            };
            let input = match start_input(&mut pty, failed) {
                Ok(input) => input,
                Err(error) => {
                    let _ = pty.kill();
                    let _ = pty.wait();
                    let _ = ready_sender.send(Err(error));
                    return;
                }
            };
            let _ = ready_sender.send(Ok(()));
            let owner = Owner {
                metadata: Metadata::new(session.directory.clone()),
                info: session,
                pty,
                terminal,
                size,
                events: receiver,
                input,
                attachments: HashMap::new(),
                output_pending: false,
                resize_pending: false,
                compress_pending: false,
                history_total: 0,
                history_generation: 0,
                input_modes: InputModes::default(),
                cursor_blinking: true,
                links: Vec::new(),
                frame_state: None,
                next_tick: None,
                output_state: OutputState::Open,
                archive,
                next_checkpoint: Some(Instant::now() + CHECKPOINT),
                next_metadata: Instant::now(),
            };
            on_exit(owner.run());
        })
        .map_err(ServerError::spawn_failed)?;

    ready
        .recv()
        .map_err(|_| ServerError::spawn_failed("session thread stopped before it was ready"))??;
    Ok(SessionHandle::new(info, sender))
}

fn set_colors(
    terminal: &mut Terminal,
    colors: TerminalColors,
) -> Result<(), muxy_terminal::TerminalError> {
    terminal.set_colors(
        colors.foreground,
        colors.background,
        colors.cursor,
        colors.ansi,
    )
}

fn start_input(pty: &mut Pty, events: Sender<OwnerEvent>) -> Result<Sender<Vec<u8>>, ServerError> {
    let mut writer = pty.take_writer().map_err(ServerError::spawn_failed)?;
    let (input, pending) = mpsc::channel::<Vec<u8>>();
    thread::Builder::new()
        .name(format!("session-{}-input", pty.child_pid()))
        .spawn(move || {
            for bytes in pending {
                if let Err(error) = writer.write_all(&bytes) {
                    let _ = events.send(OwnerEvent::WriteFailed(error));
                    break;
                }
            }
        })
        .map_err(ServerError::spawn_failed)?;
    Ok(input)
}

impl Owner {
    fn run(mut self) -> ExitReason {
        let reason = self.serve().unwrap_or_else(|_| {
            let _ = self.pty.kill();
            self.pty.wait().map_or(ExitReason::Ended, exit_reason)
        });
        self.drain_output();
        self.update_metadata();
        self.checkpoint(Some(reason));
        for attachment in self.attachments.values() {
            let _ = attachment.sink.send(AttachmentEvent::Ended(reason));
        }
        reason
    }

    fn serve(&mut self) -> Result<ExitReason, Fault> {
        loop {
            match self.next_wake() {
                Wake::Event(OwnerEvent::Pty(PtyEvent::Output(bytes))) => self.feed(&bytes)?,
                Wake::Event(OwnerEvent::Pty(PtyEvent::Closed)) => {
                    self.output_state = OutputState::Closed;
                }
                Wake::Event(OwnerEvent::WriteFailed(error)) => return Err(error.into()),
                Wake::Event(OwnerEvent::Command(SessionCommand::Input(bytes))) => {
                    self.input.send(bytes)?;
                }
                Wake::Event(OwnerEvent::Command(SessionCommand::Mouse(event))) => {
                    let bytes = self.terminal.encode_mouse(&mouse(event))?;
                    if !bytes.is_empty() {
                        self.input.send(bytes)?;
                    }
                }
                Wake::Event(OwnerEvent::Command(SessionCommand::SetColors(colors))) => {
                    set_colors(&mut self.terminal, colors)?;
                }
                Wake::Event(OwnerEvent::Command(SessionCommand::Resize(size))) => {
                    self.resize(size)?;
                }
                Wake::Event(OwnerEvent::Command(SessionCommand::ResizeAttachment { id, size })) => {
                    self.resize(size)?;
                    self.broadcast_frame(Some(id))?;
                    self.output_pending = false;
                    self.resize_pending = false;
                }
                Wake::Event(OwnerEvent::Command(SessionCommand::Attach {
                    id,
                    channel,
                    size,
                    sink,
                })) => self.attach(id, channel, size, sink)?,
                Wake::Event(OwnerEvent::Command(SessionCommand::HistoryPage {
                    before,
                    max_rows,
                    reply,
                })) => {
                    let _ = reply.send(self.history_page(before, max_rows));
                }
                Wake::Event(OwnerEvent::Command(SessionCommand::Detach(id))) => {
                    self.attachments.remove(&id);
                }
                Wake::Event(OwnerEvent::Command(SessionCommand::Search {
                    query,
                    ignore_case,
                    before,
                    max_results,
                    reply,
                })) => {
                    let _ = reply.send(self.search(&query, ignore_case, before, max_results));
                }
                Wake::Event(OwnerEvent::Command(SessionCommand::End)) => {
                    return self.terminate(ExitReason::Ended);
                }
                Wake::Event(OwnerEvent::Command(SessionCommand::Stop)) => {
                    return self.terminate(ExitReason::ServerStopped);
                }
                Wake::Tick => self.tick()?,
                Wake::Checkpoint => self.checkpoint(None),
                Wake::Metadata => self.update_metadata(),
                Wake::Orphaned => return self.terminate(ExitReason::ServerStopped),
            }
            if self.output_state == OutputState::Closed
                && let Some(status) = self.pty.try_wait()
            {
                return Ok(exit_reason(status));
            }
            if self.next_tick.is_none() && self.has_pending_work() {
                self.next_tick = Some(Instant::now() + TICK);
            }
        }
    }

    fn next_wake(&self) -> Wake {
        let now = Instant::now();
        if self.next_tick.is_some_and(|tick| tick <= now) {
            return Wake::Tick;
        }
        if self
            .next_checkpoint
            .is_some_and(|checkpoint| checkpoint <= now)
        {
            return Wake::Checkpoint;
        }
        if self.next_metadata <= now {
            return Wake::Metadata;
        }
        let deadline = self
            .next_tick
            .unwrap_or(self.next_metadata)
            .min(self.next_checkpoint.unwrap_or(self.next_metadata))
            .min(self.next_metadata);
        let received = self
            .events
            .recv_timeout(deadline.saturating_duration_since(now));
        match received {
            Ok(event) => Wake::Event(event),
            Err(RecvTimeoutError::Timeout) if Some(deadline) == self.next_checkpoint => {
                Wake::Checkpoint
            }
            Err(RecvTimeoutError::Timeout) if deadline == self.next_metadata => Wake::Metadata,
            Err(RecvTimeoutError::Timeout) => Wake::Tick,
            Err(RecvTimeoutError::Disconnected) => Wake::Orphaned,
        }
    }

    fn has_pending_work(&self) -> bool {
        self.output_pending
            || self.resize_pending
            || self.compress_pending
            || self.output_state == OutputState::Closed
    }

    fn feed(&mut self, bytes: &[u8]) -> Result<(), Fault> {
        self.terminal.feed(bytes);
        let answers = self.terminal.take_pty_output();
        if !answers.is_empty() {
            self.input.send(answers)?;
        }
        self.output_pending = true;
        self.compress_pending = true;
        self.next_checkpoint
            .get_or_insert_with(|| Instant::now() + CHECKPOINT);
        Ok(())
    }

    fn resize(&mut self, size: Size) -> Result<(), Fault> {
        self.pty.resize(pty_size(size))?;
        self.terminal.resize(terminal_size(size))?;
        self.size = size;
        self.resize_pending = true;
        self.next_checkpoint
            .get_or_insert_with(|| Instant::now() + CHECKPOINT);
        Ok(())
    }

    fn attach(
        &mut self,
        id: AttachmentId,
        channel: ChannelId,
        size: Size,
        sink: Sender<AttachmentEvent>,
    ) -> Result<(), Fault> {
        if self.attachments.is_empty() && size != self.size {
            self.resize(size)?;
        }
        self.update_metadata();
        self.update_cursor_blinking()?;
        // Establish one common baseline before adding an attachment. Existing
        // clients receive pending changes before the new client's snapshot.
        if self.attachments.is_empty() {
            self.terminal.take_changed_rows()?;
            self.links = protocol_links(self.terminal.screen_links());
            self.frame_state = Some((
                cursor(self.terminal.cursor()?),
                modes(self.terminal.modes()?),
            ));
        } else {
            self.broadcast_frame(None)?;
        }
        self.output_pending = false;
        self.resize_pending = false;
        let history = self.history_page(HistoryCursor(0), 200)?;
        let screen = history
            .screen
            .ok_or_else(|| io::Error::other("fresh history has no screen"))?;
        let snapshot = AttachSnapshot {
            channel,
            size: self.size,
            rows: screen.rows,
            cursor: screen.cursor,
            modes: modes(self.terminal.modes()?),
            title: self.metadata.title.clone(),
            directory: self.metadata.directory.clone(),
            history: history.rows,
            history_cursor: history.next,
            history_total: history.total_rows,
        };
        if sink
            .send(AttachmentEvent::Snapshot {
                snapshot,
                process: self.metadata.process.clone(),
            })
            .is_ok()
        {
            let _ = sink.send(AttachmentEvent::Metadata(MetadataEvent::InputModes(
                input_modes(self.terminal.input_modes()?),
            )));
            let _ = sink.send(AttachmentEvent::Metadata(MetadataEvent::CursorBlinking(
                self.cursor_blinking,
            )));
            let _ = sink.send(AttachmentEvent::Metadata(MetadataEvent::Links {
                seq: 0,
                rows: self.links.clone(),
            }));
            self.attachments.insert(id, Attachment { sink, seq: 1 });
        }
        Ok(())
    }

    fn history_page(
        &mut self,
        before: HistoryCursor,
        max_rows: u16,
    ) -> Result<HistoryPage, ServerError> {
        let terminal_error = |error| {
            ServerError::new(
                ErrorCode::HistoryUnavailable,
                format!("terminal history: {error}"),
            )
        };
        let generation = self.terminal.history_generation().map_err(terminal_error)?;
        let total = self.terminal.history_rows().map_err(terminal_error)?;
        let (range, next) = history_range(self.info.id, generation, before, max_rows, total)?;
        self.compress_pending = true;
        let history = rows(self.terminal.history(range).map_err(terminal_error)?);
        let screen = if before.0 == 0 {
            Some(SavedScreen {
                size: self.size,
                rows: rows(self.terminal.screen().map_err(terminal_error)?),
                cursor: cursor(self.terminal.cursor().map_err(terminal_error)?),
                reason: None,
            })
        } else {
            None
        };
        bound_history_page(
            self.info.id,
            generation,
            before,
            HistoryPage {
                rows: history,
                next,
                total_rows: total as u64,
                screen,
            },
        )
    }

    fn update_cursor_blinking(&mut self) -> Result<(), Fault> {
        let blinking = self.terminal.cursor_blinking()?;
        if blinking != self.cursor_blinking {
            self.cursor_blinking = blinking;
            self.attachments.retain(|_, attachment| {
                attachment
                    .sink
                    .send(AttachmentEvent::Metadata(MetadataEvent::CursorBlinking(
                        blinking,
                    )))
                    .is_ok()
            });
        }
        Ok(())
    }

    fn tick(&mut self) -> Result<(), Fault> {
        self.next_tick = None;
        self.update_metadata();
        let modes = input_modes(self.terminal.input_modes()?);
        if modes != self.input_modes {
            self.input_modes = modes;
            self.attachments.retain(|_, attachment| {
                attachment
                    .sink
                    .send(AttachmentEvent::Metadata(MetadataEvent::InputModes(modes)))
                    .is_ok()
            });
        }
        self.update_cursor_blinking()?;
        if self.output_pending || self.resize_pending {
            self.broadcast_frame(None)?;
            self.output_pending = false;
            self.resize_pending = false;
        } else if self.compress_pending {
            self.terminal.compress_idle()?;
            self.compress_pending = false;
        }
        Ok(())
    }

    fn search(
        &mut self,
        query: &str,
        ignore_case: bool,
        before: HistoryCursor,
        max_results: u16,
    ) -> Result<SearchPage, ServerError> {
        let failed =
            |error| ServerError::new(ErrorCode::HistoryUnavailable, format!("search: {error}"));
        let history_rows = self.terminal.history_rows().map_err(failed)?;
        let search = Search {
            session: self.info.id,
            generation: self.terminal.history_generation().map_err(failed)?,
            query,
            ignore_case,
            before,
            max_results,
            history_rows,
            screen_rows: usize::from(self.size.rows),
        };
        let range = search.range()?;
        self.compress_pending = true;
        let history = rows(
            self.terminal
                .history(range.start.min(history_rows)..range.end.min(history_rows))
                .map_err(failed)?,
        );
        let screen = if range.end > history_rows {
            rows(self.terminal.screen().map_err(failed)?)
        } else {
            Vec::new()
        };
        search.scan(|index| {
            Ok(if index < history_rows {
                &history[index - range.start].runs
            } else {
                &screen[index - history_rows].runs
            })
        })
    }

    fn update_metadata(&mut self) {
        let terminal = self.terminal.take_events();
        for event in self.metadata.update(&self.pty, terminal) {
            self.attachments.retain(|_, attachment| {
                attachment
                    .sink
                    .send(AttachmentEvent::Metadata(event.clone()))
                    .is_ok()
            });
        }
        self.next_metadata = Instant::now() + METADATA_POLL;
    }

    fn broadcast_frame(&mut self, resized: Option<AttachmentId>) -> Result<(), Fault> {
        if self.attachments.is_empty() {
            return Ok(());
        }
        let total_rows = self.terminal.history_rows()? as u64;
        let generation = self.terminal.history_generation()?;
        if total_rows != self.history_total || generation != self.history_generation {
            self.history_total = total_rows;
            self.history_generation = generation;
            self.attachments.retain(|_, attachment| {
                attachment
                    .sink
                    .send(AttachmentEvent::Metadata(MetadataEvent::History {
                        total_rows,
                    }))
                    .is_ok()
            });
        }
        let frame = ScreenFrame {
            seq: 0,
            reset: self.resize_pending,
            rows: rows(self.terminal.take_changed_rows()?),
            cursor: cursor(self.terminal.cursor()?),
            modes: modes(self.terminal.modes()?),
        };
        let links = protocol_links(self.terminal.screen_links());
        let links_changed = self.links != links;
        self.links = links;
        let state = (frame.cursor, frame.modes);
        if !links_changed
            && !frame.reset
            && frame.rows.is_empty()
            && self.frame_state == Some(state)
        {
            return Ok(());
        }
        self.frame_state = Some(state);
        self.attachments.retain(|id, attachment| {
            let frame = ScreenFrame {
                seq: attachment.seq,
                ..frame.clone()
            };
            if links_changed || frame.reset {
                let _ = attachment
                    .sink
                    .send(AttachmentEvent::Metadata(MetadataEvent::Links {
                        seq: frame.seq,
                        rows: self.links.clone(),
                    }));
            }
            attachment.seq += 1;
            let event = if resized == Some(*id) {
                AttachmentEvent::Resized(frame)
            } else {
                AttachmentEvent::Frame(frame)
            };
            attachment.sink.send(event).is_ok()
        });
        Ok(())
    }

    fn terminate(&mut self, reason: ExitReason) -> Result<ExitReason, Fault> {
        self.pty.kill()?;
        self.pty.wait()?;
        Ok(reason)
    }

    fn drain_output(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(1);
        while self.output_state == OutputState::Open {
            match self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(OwnerEvent::Pty(PtyEvent::Output(bytes))) => {
                    self.terminal.feed(&bytes);
                    self.terminal.take_pty_output();
                }
                Ok(OwnerEvent::Pty(PtyEvent::Closed)) => self.output_state = OutputState::Closed,
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }

    fn checkpoint(&mut self, reason: Option<ExitReason>) {
        self.next_checkpoint = None;
        self.compress_pending = true;
        let result = self
            .terminal
            .archive()
            .map_err(io::Error::other)
            .and_then(|terminal| self.archive.save(self.info.id, terminal, reason));
        if let Err(error) = result {
            log::error!(
                "session {} could not save terminal content: {error}",
                self.info.id.get()
            );
        }
    }
}

fn exit_reason(status: ExitStatus) -> ExitReason {
    match (status.code, status.signal) {
        (Some(code), _) => ExitReason::Exited(code),
        (None, Some(signal)) => ExitReason::Signaled(signal),
        (None, None) => ExitReason::Ended,
    }
}

fn protocol_links(rows: Vec<muxy_terminal::LinkRow>) -> Vec<muxy_protocol::LinkRow> {
    rows.into_iter()
        .map(|row| muxy_protocol::LinkRow {
            row: row.row,
            spans: row
                .spans
                .into_iter()
                .map(|span| muxy_protocol::LinkSpan {
                    start: span.start,
                    end: span.end,
                    uri: span.uri,
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxy_protocol::{ServerPath, SessionId};
    use muxy_pty::SpawnRequest;
    use std::num::NonZeroU64;
    use std::os::unix::ffi::OsStrExt;

    fn owner() -> Result<Owner, Fault> {
        let size = Size { cols: 20, rows: 3 };
        let cwd = std::env::temp_dir();
        let mut pty = Pty::spawn(SpawnRequest {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "exit 0".into()],
            cwd: cwd.clone(),
            env: vec![],
            size: pty_size(size),
        })?;
        pty.wait()?;
        let directory = ServerPath(cwd.as_os_str().as_bytes().to_vec());
        Ok(Owner {
            info: SessionInfo {
                id: SessionId::from(NonZeroU64::MIN),
                directory: directory.clone(),
            },
            pty,
            terminal: Terminal::new(terminal_size(size), 1024)?,
            metadata: Metadata::new(directory),
            size,
            events: mpsc::channel().1,
            input: mpsc::channel().0,
            attachments: HashMap::new(),
            output_pending: false,
            resize_pending: false,
            compress_pending: false,
            history_total: 0,
            history_generation: 0,
            input_modes: InputModes::default(),
            cursor_blinking: true,
            links: Vec::new(),
            frame_state: None,
            next_tick: None,
            output_state: OutputState::Closed,
            archive: Archive::memory(1024),
            next_checkpoint: None,
            next_metadata: Instant::now(),
        })
    }

    fn blink_modes(events: &Receiver<AttachmentEvent>) -> Vec<bool> {
        events
            .try_iter()
            .filter_map(|event| match event {
                AttachmentEvent::Metadata(MetadataEvent::CursorBlinking(blinking)) => {
                    Some(blinking)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn attaching_between_cursor_mode_changes_keeps_every_attachment_current() -> Result<(), Fault> {
        for initial in [true, false] {
            let mut owner = owner()?;
            let sequence = |blinking| {
                if blinking {
                    &b"\x1b[1 q"[..]
                } else {
                    &b"\x1b[2 q"[..]
                }
            };
            owner.feed(sequence(initial))?;
            owner.tick()?;
            let (first, first_events) = mpsc::channel();
            owner.attach(AttachmentId(1), ChannelId(1), owner.size, first)?;
            assert_eq!(blink_modes(&first_events), [initial]);

            owner.feed(sequence(!initial))?;
            let (second, second_events) = mpsc::channel();
            owner.attach(AttachmentId(2), ChannelId(2), owner.size, second)?;
            owner.feed(sequence(initial))?;
            owner.tick()?;

            assert_eq!(blink_modes(&second_events), [!initial, initial]);
            assert_eq!(blink_modes(&first_events), [!initial, initial]);
        }
        Ok(())
    }
    #[test]
    fn new_attachment_receives_its_own_baseline_correction() -> Result<(), Fault> {
        let mut owner = owner()?;
        owner.feed(b"\x1b[1;1HA")?;
        let (first, first_events) = mpsc::channel();
        owner.attach(AttachmentId(1), ChannelId(1), owner.size, first)?;
        owner.tick()?;
        first_events.try_iter().for_each(drop);
        owner.feed(b"\x1b[1;1HB")?;
        let (second, second_events) = mpsc::channel();
        owner.attach(AttachmentId(2), ChannelId(2), owner.size, second)?;
        let mut displayed = match second_events.recv()? {
            AttachmentEvent::Snapshot { snapshot, .. } => snapshot.rows,
            other => return Err(format!("expected snapshot, got {other:?}").into()),
        };
        owner.feed(b"\x1b[1;1HA")?;
        owner.tick()?;
        for event in second_events.try_iter() {
            if let AttachmentEvent::Frame(frame) = event {
                for row in frame.rows {
                    let index = usize::from(row.index);
                    displayed[index] = row;
                }
            }
        }
        let current = rows(owner.terminal.screen()?);
        assert_eq!(
            displayed, current,
            "each attachment must converge to the server screen"
        );
        Ok(())
    }

    #[test]
    fn replaced_history_invalidates_attached_caches_without_a_screen_frame() -> Result<(), Fault> {
        let mut owner = owner()?;
        let burst = |label: &str| format!("{}\x1b[2J\x1b[H", format!("{label}\r\n").repeat(10));
        owner.feed(burst("old").as_bytes())?;
        let (first, _first_events) = mpsc::channel();
        owner.attach(AttachmentId(1), ChannelId(1), owner.size, first)?;
        let (second, second_events) = mpsc::channel();
        owner.attach(AttachmentId(2), ChannelId(2), owner.size, second)?;
        let previous = match second_events.recv()? {
            AttachmentEvent::Snapshot { snapshot, .. } => snapshot,
            other => return Err(format!("expected snapshot, got {other:?}").into()),
        };
        second_events.try_iter().for_each(drop);

        owner.feed(b"\x1b[3J")?;
        owner.feed(burst("new").as_bytes())?;
        owner.tick()?;
        let current = owner.history_page(HistoryCursor(0), 200)?;
        assert_eq!(previous.history_total, current.total_rows);
        assert_ne!(previous.history, current.rows);
        let events: Vec<_> = second_events.try_iter().collect();
        assert!(matches!(
            events.as_slice(),
            [AttachmentEvent::Metadata(MetadataEvent::History { total_rows })]
                if *total_rows == current.total_rows
        ));
        Ok(())
    }

    #[test]
    fn unchanged_output_emits_no_frame() -> Result<(), Fault> {
        let mut owner = owner()?;
        owner.feed(b"\x1b[1;1HA")?;
        let (sink, events) = mpsc::channel();
        owner.attach(AttachmentId(1), ChannelId(1), owner.size, sink)?;
        owner.tick()?;
        events.try_iter().for_each(drop);
        owner.feed(b"\x1b[1;1HA")?;
        owner.tick()?;
        let frames: Vec<_> = events
            .try_iter()
            .filter_map(|event| match event {
                AttachmentEvent::Frame(frame) => Some(frame),
                _ => None,
            })
            .collect();
        assert!(
            frames.is_empty(),
            "unchanged screen, cursor and modes should not emit a frame"
        );
        Ok(())
    }

    #[test]
    fn cursor_only_mode_only_and_reset_frames_are_not_suppressed() -> Result<(), Fault> {
        let mut owner = owner()?;
        let (sink, events) = mpsc::channel();
        owner.attach(AttachmentId(1), ChannelId(1), owner.size, sink)?;
        events.try_iter().for_each(drop);
        for sequence in [b"\x1b[1;2H".as_slice(), b"\x1b[?2004h", b"\x1b[?1h"] {
            owner.feed(sequence)?;
            owner.tick()?;
            let frames: Vec<_> = events
                .try_iter()
                .filter_map(|event| match event {
                    AttachmentEvent::Frame(frame) => Some(frame),
                    _ => None,
                })
                .collect();
            assert_eq!(frames.len(), 1);
            assert!(frames[0].rows.is_empty());
        }
        owner.resize(Size {
            cols: owner.size.cols + 1,
            ..owner.size
        })?;
        owner.tick()?;
        assert!(events.try_iter().any(|event| matches!(event, AttachmentEvent::Frame(frame) if frame.reset && !frame.rows.is_empty())));
        Ok(())
    }

    #[test]
    fn repeated_identical_output_has_no_frame_cost() -> Result<(), Fault> {
        let mut owner = owner()?;
        let (sink, events) = mpsc::channel();
        owner.feed(b"\x1b[1;1HA")?;
        owner.attach(AttachmentId(1), ChannelId(1), owner.size, sink)?;
        events.try_iter().for_each(drop);
        let mut frames = 0;
        for _ in 0..10_000 {
            owner.feed(b"\x1b[1;1HA")?;
            owner.tick()?;
            frames += events
                .try_iter()
                .filter(|event| matches!(event, AttachmentEvent::Frame(_)))
                .count();
        }
        assert_eq!(frames, 0);
        Ok(())
    }
}
