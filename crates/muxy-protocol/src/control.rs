use serde::{Deserialize, Serialize};

use crate::{
    AttachSnapshot, ChannelId, ForegroundProcess, HistoryCursor, HistoryPage, SavedScreen,
    SearchPage, SearchSource, ServerPath, SessionId, SessionInfo, Size,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RequestBody {
    ListSessions,
    CreateSession {
        directory: ServerPath,
        size: Size,
    },
    EndSession(SessionId),
    Attach {
        session: SessionId,
        size: Size,
    },
    Detach(ChannelId),
    Resize {
        channel: ChannelId,
        size: Size,
    },
    Ping,
    ReadSavedScreen(SessionId),
    DiscardSession(SessionId),
    HistoryPage {
        channel: ChannelId,
        before: HistoryCursor,
        max_rows: u16,
    },
    SavedHistoryPage {
        session: SessionId,
        before: HistoryCursor,
        max_rows: u16,
    },
    Search {
        source: SearchSource,
        query: String,
        ignore_case: bool,
        before: HistoryCursor,
        max_results: u16,
    },
    SetTerminalColors(TerminalColors),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalColors {
    pub foreground: [u8; 3],
    pub background: [u8; 3],
    pub cursor: [u8; 3],
    pub ansi: [[u8; 3]; 16],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReplyBody {
    Sessions(Vec<SessionInfo>),
    SessionCreated(SessionInfo),
    SessionEnded,
    Detached,
    Resized,
    Pong,
    Error(ErrorReply),
    SavedScreen(SavedScreen),
    SessionDiscarded,
    Attached {
        snapshot: Box<AttachSnapshot>,
        process: Option<ForegroundProcess>,
    },
    HistoryPage(HistoryPage),
    SearchPage(SearchPage),
    TerminalColorsSet,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ErrorReply {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ErrorCode {
    UnknownSession,
    UnknownChannel,
    BadSize,
    BadPath,
    SpawnFailed,
    BadRequest,
    SavedContentUnavailable,
    StaleHistoryCursor,
    HistoryUnavailable,
}
