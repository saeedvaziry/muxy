//! Product domain and persistence policy owned by the Muxy application.
//!
//! Workspaces, projects, tabs, panes, and window state live here; server
//! execution and UI toolkit code do not.

mod error;
mod home;
mod ids;
mod layout;
pub mod opener;
mod pane;
mod project;
pub mod restore;
mod state;
pub mod store;
mod tab;
pub mod title;
mod window;

pub use error::AppError;
pub use ids::{PaneId, ProjectId, ServerId, TabId};
pub use layout::{Axis, Branch, Direction, Layout};
pub use pane::{Pane, PaneContent};
pub use project::{Color, PROJECT_COLORS, Project, ProjectKind, ProjectStatus};
pub use state::AppState;
pub use tab::Tab;
pub use window::{WindowBounds, WindowState};
