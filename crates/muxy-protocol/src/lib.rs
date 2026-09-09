//! Transport-independent, versioned contracts shared by the app and server.
//!
//! This crate owns message meaning and validation, but never framing, I/O,
//! persistence, UI, PTYs, or runtime policy.

mod control;
mod ids;
mod message;
mod path;
mod samples;
mod screen;
mod session;
mod validate;
mod version;

pub use control::{ErrorCode, ErrorReply, ReplyBody, RequestBody, TerminalColors};
pub use ids::{CONTROL, ChannelId, RequestId, SessionId};
pub use message::{ChannelKind, Message};
pub use path::ServerPath;
pub use screen::{Color, Cursor, Modes, Row, Run, ScreenFrame, Size, Style};
pub use session::{
    AttachSnapshot, ExitReason, ForegroundProcess, HistoryCursor, HistoryPage, InputModes,
    MetadataEvent, Modifiers, MouseAction, MouseButton, MouseEvent, SavedScreen, ScrollDirection,
    SearchMatch, SearchPage, SearchSource, SessionInfo,
};
pub use validate::{
    MAX_COLS, MAX_INPUT, MAX_ROWS, validate_input, validate_path, validate_search, validate_size,
    validate_versions,
};
pub use version::{SUPPORTED, V1, Version};
