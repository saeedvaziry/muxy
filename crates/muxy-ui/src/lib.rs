//! Reusable GPUI components. Shared shortcut definitions come from muxy-core;
//! app settings resolve them through the registration interface.

pub mod assets;
pub mod command_popover;
pub mod components;
pub mod controls;
#[cfg(target_os = "macos")]
pub mod dialog;
pub mod icon;
pub mod motion;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub mod native_scroll;
pub mod panel;
pub mod popover;
pub mod scrollbar;
pub mod shortcuts;
pub mod symbols;
pub mod text_input;
pub mod theme;
