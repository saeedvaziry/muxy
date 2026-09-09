//! Protocol-free ordered byte streams and their platform adapters.
//!
//! Framing, messages, sessions, and application policy are deliberately
//! outside this crate.

mod error;
mod stream;
mod unix;

pub use error::BindError;
pub use stream::{ByteStream, Listener, StreamCancellation};
pub use unix::{UnixSocketListener, connect};
