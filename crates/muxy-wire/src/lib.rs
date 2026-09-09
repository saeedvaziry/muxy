//! Physical framing and serialization for `muxy-protocol` messages.
//!
//! This boundary validates bytes and codecs, not peer policy or runtime
//! behavior.

mod codec;
mod error;
mod header;
mod kind;
mod stream;

pub use codec::{decode, encode, message_version};
pub use error::WireError;
pub use header::{HEADER_LEN, Header, MAX_FRAME};
pub use kind::MessageKind;
pub use stream::{Decoder, Encoder};
