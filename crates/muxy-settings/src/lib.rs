//! App configuration and keymap schema and storage.
//!
//! Server-owned settings, product entities, and presentation code stay in
//! their owning crates.

mod appearance;
mod chord;
mod error;
mod ghostty;
mod keymap;
mod settings;

pub use appearance::Appearance;
pub use chord::KeyChord;
pub use error::{Error, Result};
pub use ghostty::{CellHeight, TerminalSettings};
pub use keymap::{Action, Keymap};
pub use settings::{
    ClipboardSettings, NewPaneDirectory, OpenerSettings, PaneSettings, ProjectSettings, Settings,
    WindowSettings,
};
