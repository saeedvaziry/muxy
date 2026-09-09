use serde::{Deserialize, Serialize};

use crate::{ChannelId, Cursor, Modes, Row, ServerPath, SessionId, Size};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: SessionId,
    pub directory: ServerPath,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ExitReason {
    Exited(i32),
    Signaled(i32),
    Ended,
    ServerStopped,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AttachSnapshot {
    pub channel: ChannelId,
    pub size: Size,
    pub rows: Vec<Row>,
    pub cursor: Cursor,
    pub modes: Modes,
    pub title: String,
    pub directory: ServerPath,
    pub history: Vec<Row>,
    pub history_cursor: Option<HistoryCursor>,
    pub history_total: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct HistoryCursor(pub u64);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HistoryPage {
    pub rows: Vec<Row>,
    pub next: Option<HistoryCursor>,
    pub total_rows: u64,
    pub screen: Option<SavedScreen>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SearchSource {
    Live(ChannelId),
    Saved(SessionId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchMatch {
    pub row: u64,
    pub start: u16,
    pub end: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchPage {
    pub matches: Vec<SearchMatch>,
    pub next: Option<HistoryCursor>,
    pub total_rows: u64,
    pub scanned_rows: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ForegroundProcess {
    pub name: String,
    pub is_shell: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MetadataEvent {
    Title(String),
    Directory(ServerPath),
    ForegroundProcess { name: String, is_shell: bool },
    Bell,
    History { total_rows: u64 },
    InputModes(InputModes),
    CursorBlinking(bool),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct InputModes {
    pub mouse_tracking: bool,
    pub alternate_scroll: bool,
    pub focus_events: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MouseEvent {
    pub action: MouseAction,
    pub button: Option<MouseButton>,
    pub column: u16,
    pub row: u16,
    pub scroll: Option<ScrollDirection>,
    pub modifiers: Modifiers,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
    Scroll,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Back,
    Forward,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScrollDirection {
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SavedScreen {
    pub size: Size,
    pub rows: Vec<Row>,
    pub cursor: Cursor,
    pub reason: Option<ExitReason>,
}
