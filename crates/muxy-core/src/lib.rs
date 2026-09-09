//! Narrow primitives that are genuinely shared across Muxy boundaries.
//!
//! Wire-visible types belong in `muxy-protocol`; app and server concepts stay
//! in their respective domain crates. The shared shortcut catalog and bounded
//! worker primitive are independent of GPUI and server execution.

pub mod dirs;
pub mod shortcuts;
pub mod worker;
