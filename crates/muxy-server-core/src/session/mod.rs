pub(crate) mod frames;
mod metadata;
mod owner;

use std::sync::mpsc::Sender;

use muxy_protocol::{
    AttachSnapshot, ChannelId, ExitReason, ForegroundProcess, HistoryCursor, HistoryPage,
    MetadataEvent, MouseEvent, ScreenFrame, SearchPage, SessionId, SessionInfo, Size,
    TerminalColors,
};

use crate::error::ServerError;
use owner::OwnerEvent;

pub(crate) use frames::pty_size;
pub(crate) use owner::start;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AttachmentId(pub u64);

#[derive(Debug)]
pub enum SessionCommand {
    Input(Vec<u8>),
    Mouse(MouseEvent),
    Resize(Size),
    SetColors(TerminalColors),
    ResizeAttachment {
        id: AttachmentId,
        size: Size,
    },
    Attach {
        id: AttachmentId,
        channel: ChannelId,
        size: Size,
        sink: Sender<AttachmentEvent>,
    },
    Detach(AttachmentId),
    HistoryPage {
        before: HistoryCursor,
        max_rows: u16,
        reply: Sender<Result<HistoryPage, ServerError>>,
    },
    End,
    Search {
        query: String,
        ignore_case: bool,
        before: HistoryCursor,
        max_results: u16,
        reply: Sender<Result<SearchPage, ServerError>>,
    },
    Stop,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttachmentEvent {
    Snapshot {
        snapshot: AttachSnapshot,
        process: Option<ForegroundProcess>,
    },
    Metadata(MetadataEvent),
    Frame(ScreenFrame),
    Resized(ScreenFrame),
    Ended(ExitReason),
}

#[derive(Clone, Debug)]
pub struct SessionHandle {
    info: SessionInfo,
    commands: Sender<OwnerEvent>,
}

impl SessionHandle {
    pub(crate) fn new(info: SessionInfo, commands: Sender<OwnerEvent>) -> Self {
        Self { info, commands }
    }

    pub fn id(&self) -> SessionId {
        self.info.id
    }

    pub fn info(&self) -> &SessionInfo {
        &self.info
    }

    pub fn send(&self, command: SessionCommand) -> Result<(), ServerError> {
        self.commands
            .send(OwnerEvent::Command(command))
            .map_err(|_| ServerError::unknown_session(self.info.id))
    }
}
