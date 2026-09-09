use std::hash::{DefaultHasher, Hash, Hasher};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    pub strikethrough: bool,
    pub faint: bool,
}

impl Style {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Run {
    pub text: String,
    pub width: u16,
    pub style: Style,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Row {
    pub index: u16,
    pub runs: Vec<Run>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cursor {
    pub row: u16,
    pub col: u16,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Modes {
    pub application_cursor_keys: bool,
    pub bracketed_paste: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InputModes {
    pub mouse_tracking: bool,
    pub alternate_scroll: bool,
    pub focus_events: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouseEvent {
    pub action: MouseAction,
    pub button: Option<MouseButton>,
    pub column: u16,
    pub row: u16,
    pub scroll: Option<ScrollDirection>,
    pub modifiers: Modifiers,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
    Scroll,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Back,
    Forward,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrollDirection {
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

pub(crate) fn hash_runs(runs: &[Run]) -> u64 {
    let mut hasher = DefaultHasher::new();
    runs.hash(&mut hasher);
    hasher.finish()
}
