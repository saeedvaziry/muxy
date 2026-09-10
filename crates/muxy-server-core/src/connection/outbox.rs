use std::collections::{HashMap, VecDeque};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

use muxy_protocol::{
    AttachSnapshot, CONTROL, ChannelId, ErrorCode, ExitReason, ForegroundProcess, Message,
    MetadataEvent, ReplyBody, RequestId, ScreenFrame, SessionId, Size, TerminalColors, Version,
};
use muxy_wire::message_version;

use crate::{AttachmentId, ServerError, SessionCommand, SessionHandle};

use super::merge::merge;

struct Attachment {
    handle: SessionHandle,
    id: AttachmentId,
    waiting: Option<RequestId>,
    resizes: VecDeque<RequestId>,
}

#[derive(Default)]
struct State {
    colors: Option<TerminalColors>,
    control: VecDeque<Message>,
    pending: HashMap<ChannelId, ScreenFrame>,
    metadata: HashMap<ChannelId, Vec<MetadataEvent>>,
    credit: HashMap<ChannelId, bool>,
    sent: HashMap<ChannelId, u64>,
    attachments: HashMap<ChannelId, Attachment>,
    closed: bool,
}

pub(super) struct Outbox {
    state: Mutex<State>,
    ready: Condvar,
    version: Version,
}

impl Outbox {
    pub(super) fn new(version: Version) -> Self {
        Self {
            state: Mutex::default(),
            ready: Condvar::default(),
            version,
        }
    }

    pub(super) fn colors(&self) -> Option<TerminalColors> {
        self.lock().colors
    }

    pub(super) fn set_colors(&self, colors: TerminalColors) {
        let mut state = self.lock();
        state.colors = Some(colors);
        for attachment in state.attachments.values() {
            let _ = attachment.handle.send(SessionCommand::SetColors(colors));
        }
    }

    pub(super) fn push_control(&self, message: Message) {
        let Some(message) = self.compatible(message) else {
            return;
        };
        let mut state = self.lock();
        if !state.closed {
            state.control.push_back(message);
            self.ready.notify_one();
        }
    }

    pub(super) fn attach(
        &self,
        channel: ChannelId,
        id: AttachmentId,
        request: RequestId,
        handle: SessionHandle,
        command: SessionCommand,
    ) -> Result<(), ServerError> {
        let mut state = self.lock();
        if state.closed {
            return Err(ServerError::new(
                ErrorCode::BadRequest,
                "connection is closed",
            ));
        }
        if let Some(colors) = state.colors {
            handle.send(SessionCommand::SetColors(colors))?;
        }
        handle.send(command)?;
        state.attachments.insert(
            channel,
            Attachment {
                handle,
                id,
                waiting: Some(request),
                resizes: VecDeque::new(),
            },
        );
        state.credit.insert(channel, true);
        Ok(())
    }

    pub(super) fn snapshot(
        &self,
        channel: ChannelId,
        snapshot: AttachSnapshot,
        process: Option<ForegroundProcess>,
    ) {
        let mut state = self.lock();
        if let Some(attachment) = state.attachments.get_mut(&channel)
            && let Some(id) = attachment.waiting.take()
        {
            if let Some(message) = self.compatible(Message::Reply {
                id,
                body: ReplyBody::Attached {
                    snapshot: Box::new(snapshot),
                    process,
                },
            }) {
                state.control.push_back(message);
            }
            self.ready.notify_one();
        }
    }

    pub(super) fn attach_failed(&self, channel: ChannelId, error: &ServerError) {
        let mut state = self.lock();
        state.retire(channel, error);
        self.ready.notify_one();
    }

    pub(super) fn handle(&self, channel: ChannelId) -> Option<SessionHandle> {
        self.lock()
            .attachments
            .get(&channel)
            .map(|attachment| attachment.handle.clone())
    }

    pub(super) fn resize(
        &self,
        channel: ChannelId,
        request: RequestId,
        size: Size,
    ) -> Result<(), ServerError> {
        let mut state = self.lock();
        let attachment = state
            .attachments
            .get_mut(&channel)
            .ok_or_else(|| ServerError::new(ErrorCode::UnknownChannel, "unknown channel"))?;
        attachment.handle.send(SessionCommand::ResizeAttachment {
            id: attachment.id,
            size,
        })?;
        attachment.resizes.push_back(request);
        Ok(())
    }

    pub(super) fn resized(&self, channel: ChannelId, frame: ScreenFrame) {
        let mut state = self.lock();
        if let Some(attachment) = state.attachments.get_mut(&channel)
            && let Some(id) = attachment.resizes.pop_front()
        {
            state.pending.insert(channel, frame);
            state.control.push_back(Message::Reply {
                id,
                body: ReplyBody::Resized,
            });
            self.ready.notify_one();
        }
    }

    pub(super) fn detach(&self, channel: ChannelId) -> Result<(), ServerError> {
        let mut state = self.lock();
        if !state.attachments.contains_key(&channel) {
            return Err(ServerError::new(
                ErrorCode::UnknownChannel,
                "unknown channel",
            ));
        }
        state.retire(
            channel,
            &ServerError::new(ErrorCode::UnknownChannel, "channel detached"),
        );
        self.ready.notify_one();
        Ok(())
    }

    pub(super) fn session_ended(&self, session: SessionId, reason: ExitReason) {
        let mut state = self.lock();
        if state.closed {
            return;
        }
        let channels: Vec<_> = state
            .attachments
            .iter()
            .filter_map(|(&channel, attachment)| {
                (attachment.handle.id() == session).then_some(channel)
            })
            .collect();
        let error = ServerError::unknown_session(session);
        for channel in channels {
            state.retire(channel, &error);
        }
        state
            .control
            .push_back(Message::SessionEnded { session, reason });
        self.ready.notify_one();
    }

    pub(super) fn push_frame(&self, channel: ChannelId, frame: ScreenFrame) {
        let mut state = self.lock();
        if !state.credit.contains_key(&channel) || state.closed {
            return;
        }
        match state.pending.entry(channel) {
            std::collections::hash_map::Entry::Occupied(mut entry) => merge(entry.get_mut(), frame),
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(frame);
            }
        }
        self.ready.notify_one();
    }

    pub(super) fn push_metadata(&self, channel: ChannelId, event: MetadataEvent) {
        let Some(Message::Metadata(event)) = self.compatible(Message::Metadata(event)) else {
            return;
        };
        let mut state = self.lock();
        if !state.credit.contains_key(&channel) || state.closed {
            return;
        }
        let pending = state.metadata.entry(channel).or_default();
        if let Some(previous) = pending
            .iter_mut()
            .find(|previous| std::mem::discriminant(*previous) == std::mem::discriminant(&event))
        {
            *previous = event;
        } else {
            pending.push(event);
        }
        self.ready.notify_one();
    }

    pub(super) fn ack(&self, channel: ChannelId, seq: u64) {
        let mut state = self.lock();
        if state.sent.get(&channel) == Some(&seq)
            && let Some(credit) = state.credit.get_mut(&channel)
        {
            *credit = true;
            self.ready.notify_one();
        }
    }

    pub(super) fn next(&self) -> Option<(ChannelId, Message)> {
        let mut state = self.lock();
        loop {
            if let Some(message) = state.control.pop_front() {
                return Some((CONTROL, message));
            }
            if state.closed {
                return None;
            }
            if let Some(channel) = state.metadata.keys().next().copied()
                && let Some(pending) = state.metadata.get_mut(&channel)
            {
                let event = pending.remove(0);
                if pending.is_empty() {
                    state.metadata.remove(&channel);
                }
                return Some((channel, Message::Metadata(event)));
            }
            let channel = state
                .pending
                .keys()
                .find(|channel| state.credit.get(channel) == Some(&true))
                .copied();
            if let Some(channel) = channel
                && let Some(frame) = state.pending.remove(&channel)
            {
                state.credit.insert(channel, false);
                state.sent.insert(channel, frame.seq);
                return Some((channel, Message::Frame(frame)));
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    pub(super) fn is_closed(&self) -> bool {
        self.lock().closed
    }

    pub(super) fn close_with(&self, message: Message) {
        let mut state = self.lock();
        if !state.closed {
            if let Some(message) = self.compatible(message) {
                state.control.push_back(message);
            }
            state.close();
            self.ready.notify_all();
        }
    }

    pub(super) fn close(&self) {
        self.lock().close();
        self.ready.notify_all();
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn compatible(&self, message: Message) -> Option<Message> {
        (message_version(&message) <= self.version).then_some(message)
    }
}

impl State {
    fn close(&mut self) {
        self.closed = true;
        for (_, attachment) in self.attachments.drain() {
            let _ = attachment
                .handle
                .send(SessionCommand::Detach(attachment.id));
        }
        self.pending.clear();
        self.metadata.clear();
        self.credit.clear();
        self.sent.clear();
    }

    fn retire(&mut self, channel: ChannelId, error: &ServerError) {
        self.pending.remove(&channel);
        self.metadata.remove(&channel);
        self.credit.remove(&channel);
        self.sent.remove(&channel);
        if let Some(attachment) = self.attachments.remove(&channel) {
            let _ = attachment
                .handle
                .send(SessionCommand::Detach(attachment.id));
            for id in attachment.waiting.into_iter().chain(attachment.resizes) {
                self.control.push_back(Message::Reply {
                    id,
                    body: ReplyBody::Error(error.to_reply()),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::Duration;

    use muxy_protocol::{Cursor, Modes, Row};

    use super::*;

    fn frame(seq: u64, index: u16) -> ScreenFrame {
        ScreenFrame {
            seq,
            reset: false,
            rows: vec![Row {
                index,
                runs: vec![],
            }],
            cursor: Cursor {
                row: 0,
                col: 0,
                visible: true,
            },
            modes: Modes::default(),
        }
    }

    #[test]
    fn prompt_metadata_is_bounded_and_keeps_the_merged_frame_watermark() {
        let outbox = Outbox::new(muxy_protocol::V1);
        let channel = ChannelId(1);
        outbox.lock().credit.insert(channel, false);
        for seq in 1..1000 {
            outbox.push_metadata(channel, MetadataEvent::ScreenPrompts { seq, rows: vec![1] });
            outbox.push_frame(channel, frame(seq, 0));
        }
        assert_eq!(outbox.lock().metadata[&channel].len(), 1);
        assert_eq!(
            outbox.next(),
            Some((
                channel,
                Message::Metadata(MetadataEvent::ScreenPrompts {
                    seq: 999,
                    rows: vec![1]
                })
            ))
        );
        outbox.lock().credit.insert(channel, true);
        assert_eq!(
            outbox.next(),
            Some((channel, Message::Frame(frame(999, 0))))
        );
    }

    #[test]
    fn history_counts_are_coalesced() {
        for version in muxy_protocol::SUPPORTED.iter().copied() {
            let outbox = Outbox::new(version);
            let channel = ChannelId(1);
            outbox.lock().credit.insert(channel, true);
            for total_rows in [10, 20, 30] {
                outbox.push_metadata(channel, MetadataEvent::History { total_rows });
            }
            let state = outbox.lock();
            if version == muxy_protocol::V1 {
                assert_eq!(
                    state.metadata[&channel],
                    [MetadataEvent::History { total_rows: 30 }]
                );
            } else {
                assert!(state.metadata.is_empty());
            }
        }
    }

    #[test]
    fn cursor_blinking_is_coalesced_without_credit() {
        for version in muxy_protocol::SUPPORTED.iter().copied() {
            let outbox = Outbox::new(version);
            let channel = ChannelId(1);
            outbox.lock().credit.insert(channel, false);
            for blinking in [false, true, false] {
                outbox.push_metadata(channel, MetadataEvent::CursorBlinking(blinking));
            }
            let state = outbox.lock();
            if version == muxy_protocol::V1 {
                assert_eq!(
                    state.metadata[&channel],
                    [MetadataEvent::CursorBlinking(false)]
                );
            } else {
                assert!(state.metadata.is_empty());
            }
        }
    }

    #[test]
    fn input_modes_are_coalesced_without_credit() {
        for version in muxy_protocol::SUPPORTED.iter().copied() {
            let outbox = Outbox::new(version);
            let channel = ChannelId(1);
            outbox.lock().credit.insert(channel, false);
            let modes = muxy_protocol::InputModes {
                mouse_tracking: true,
                alternate_scroll: true,
                focus_events: true,
            };
            outbox.push_metadata(
                channel,
                MetadataEvent::InputModes(muxy_protocol::InputModes::default()),
            );
            outbox.push_metadata(channel, MetadataEvent::InputModes(modes));
            let state = outbox.lock();
            if version == muxy_protocol::V1 {
                assert_eq!(state.metadata[&channel], [MetadataEvent::InputModes(modes)]);
            } else {
                assert!(state.metadata.is_empty());
            }
        }
    }

    #[test]
    fn negotiated_development_protocol_preserves_every_control_message() {
        for message in Message::samples()
            .into_iter()
            .filter(|message| message.channel_kind() == muxy_protocol::ChannelKind::Control)
        {
            let outbox = Outbox::new(muxy_protocol::V1);
            outbox.push_control(message.clone());
            outbox.close();
            assert_eq!(outbox.next(), Some((CONTROL, message)));
            assert_eq!(outbox.next(), None);
        }
    }

    #[test]
    fn control_first_independent_credit_and_merged_pending_frames() {
        let outbox = Outbox::new(muxy_protocol::V1);
        outbox
            .lock()
            .credit
            .extend([(ChannelId(1), true), (ChannelId(2), true)]);
        outbox.push_frame(ChannelId(1), frame(1, 0));
        outbox.push_control(Message::HelloReply {
            versions: vec![muxy_protocol::V1],
        });
        assert!(matches!(
            outbox.next(),
            Some((CONTROL, Message::HelloReply { .. }))
        ));
        assert_eq!(
            outbox.next(),
            Some((ChannelId(1), Message::Frame(frame(1, 0))))
        );
        outbox.push_frame(ChannelId(1), frame(2, 1));
        outbox.push_frame(ChannelId(1), frame(4, 2));
        outbox.push_frame(ChannelId(2), frame(1, 0));
        assert_eq!(
            outbox.next(),
            Some((ChannelId(2), Message::Frame(frame(1, 0))))
        );
        outbox.ack(ChannelId(1), 0);
        outbox.ack(ChannelId(1), 5);
        assert!(!outbox.lock().credit[&ChannelId(1)]);
        assert_eq!(outbox.lock().pending.len(), 1);
        outbox.ack(ChannelId(1), 1);
        let mut merged = frame(4, 1);
        merged.rows.push(Row {
            index: 2,
            runs: vec![],
        });
        assert_eq!(outbox.next(), Some((ChannelId(1), Message::Frame(merged))));
        outbox.ack(ChannelId(1), 1);
        assert!(!outbox.lock().credit[&ChannelId(1)]);
    }

    #[test]
    fn fatal_seals_the_control_queue_before_other_producers_can_append() {
        let outbox = Outbox::new(muxy_protocol::V1);
        let fatal = super::super::handshake::fatal("invalid message");
        outbox.close_with(fatal.clone());
        outbox.push_control(Message::VersionUnsupported);
        outbox.close_with(Message::VersionUnsupported);
        assert_eq!(outbox.next(), Some((CONTROL, fatal)));
        assert_eq!(outbox.next(), None);
    }

    #[test]
    fn metadata_coalesces_without_frame_credit_and_is_retired_with_its_channel() {
        let outbox = Outbox::new(muxy_protocol::V1);
        let channel = ChannelId(1);
        outbox.lock().credit.insert(channel, false);
        for index in 0..1000 {
            outbox.push_metadata(channel, MetadataEvent::Title(index.to_string()));
            outbox.push_metadata(channel, MetadataEvent::Bell);
        }
        assert_eq!(outbox.lock().metadata[&channel].len(), 2);
        outbox.push_control(Message::VersionUnsupported);
        assert_eq!(outbox.next(), Some((CONTROL, Message::VersionUnsupported)));
        assert_eq!(
            outbox.next(),
            Some((
                channel,
                Message::Metadata(MetadataEvent::Title("999".into()))
            ))
        );
        assert_eq!(
            outbox.next(),
            Some((channel, Message::Metadata(MetadataEvent::Bell)))
        );
        outbox.push_metadata(channel, MetadataEvent::Bell);
        outbox.lock().retire(
            channel,
            &ServerError::new(ErrorCode::UnknownChannel, "detached"),
        );
        outbox.push_metadata(channel, MetadataEvent::Bell);
        assert!(outbox.lock().metadata.is_empty());
        outbox.close();
        assert_eq!(outbox.next(), None);
    }

    #[test]
    fn closing_wakes_writer_and_discards_pending_but_drains_control()
    -> Result<(), Box<dyn std::error::Error>> {
        let outbox = Arc::new(Outbox::new(muxy_protocol::V1));
        outbox.lock().credit.insert(ChannelId(1), false);
        outbox.push_frame(ChannelId(1), frame(1, 0));
        let (sender, receiver) = mpsc::channel();
        let waiting = Arc::clone(&outbox);
        let writer = thread::spawn(move || {
            let _ = sender.send(waiting.next());
        });
        assert!(receiver.recv_timeout(Duration::from_millis(50)).is_err());
        outbox.push_control(Message::VersionUnsupported);
        outbox.close();
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2))?,
            Some((CONTROL, Message::VersionUnsupported))
        );
        writer.join().map_err(|_| "writer panicked")?;
        assert_eq!(outbox.next(), None);
        outbox.push_frame(ChannelId(1), frame(2, 0));
        outbox.push_control(Message::VersionUnsupported);
        assert_eq!(outbox.next(), None);
        Ok(())
    }
    #[test]
    fn hyperlink_replacements_coalesce_before_frames_even_without_credit() {
        let outbox = Outbox::new(muxy_protocol::V1);
        let channel = ChannelId(1);
        outbox.lock().credit.insert(channel, false);
        for seq in 1..=3 {
            outbox.push_metadata(
                channel,
                MetadataEvent::Links {
                    seq,
                    rows: vec![muxy_protocol::LinkRow {
                        row: 0,
                        spans: vec![muxy_protocol::LinkSpan {
                            start: 0,
                            end: 1,
                            uri: format!("https://example.com/{seq}"),
                        }],
                    }],
                },
            );
            outbox.push_frame(channel, frame(seq, 0));
        }
        outbox.push_metadata(
            channel,
            MetadataEvent::Links {
                seq: 4,
                rows: vec![],
            },
        );
        assert_eq!(
            outbox.next(),
            Some((
                channel,
                Message::Metadata(MetadataEvent::Links {
                    seq: 4,
                    rows: vec![]
                })
            ))
        );
        assert!(outbox.lock().metadata.is_empty());
        assert_eq!(outbox.lock().pending.len(), 1);
    }
}
