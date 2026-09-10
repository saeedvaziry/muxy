//! Server-owned sessions, capability execution, settings, and client runtime.
//!
//! The server treats directories and session identifiers as raw inputs and
//! never models app-owned workspaces, projects, tabs, or panes.

mod archive;
pub mod connection;
mod error;
mod registry;
mod search;
mod session;
mod settings;
mod spawn;

pub use error::ServerError;
pub use registry::{Registry, ServerEvent};
pub use session::{AttachmentEvent, AttachmentId, SessionCommand, SessionHandle};
pub use settings::ServerSettings;

mod shell;
pub use shell::ShellIntegration;
