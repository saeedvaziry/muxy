//! Server-side terminal emulation, grid state, and retained history.
//!
//! This crate does not own PTY processes, protocol messages, transports, or
//! application policy.

mod error;
mod events;
mod ghostty;
mod links;
mod runs;
mod screen;

pub use error::{TerminalError, TerminalStep};
pub use events::TerminalEvent;
pub use ghostty::{Terminal, TerminalArchive};
pub use links::{LinkRow, LinkSpan, MAX_LINK_SPANS, MAX_LINK_URI};
pub use screen::{
    Color, Cursor, InputModes, Modes, Modifiers, MouseAction, MouseButton, MouseEvent, Row, Run,
    ScrollDirection, Size, Style,
};
