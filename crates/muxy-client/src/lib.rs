//! App-side server connection, handshake, routing, and synchronization.
//!
//! This crate translates shared protocol traffic without owning project,
//! tab, pane, UI, or server-execution policy.

mod client;
mod error;
mod events;
mod grid;
mod handshake;
mod requests;

pub use client::{Attachment, Client};
pub use error::ClientError;
pub use events::ClientEvent;
pub use grid::RunGrid;
