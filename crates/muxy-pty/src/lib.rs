//! Portable pseudo-terminal process and byte-I/O adapters.
//!
//! Terminal emulation and session lifecycle are owned by higher-level server
//! crates.

mod error;
mod pty;
mod reader;

pub use error::{PtyError, PtyStep};
pub use pty::{ExitStatus, Pty, PtySize, SpawnRequest};
pub use reader::{PtyEvent, ReaderHandle};
