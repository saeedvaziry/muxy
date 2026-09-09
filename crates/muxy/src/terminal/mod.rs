pub mod input_queue;
pub mod surfaces;

#[cfg(target_os = "macos")]
mod ghostty;
#[cfg(not(target_os = "macos"))]
mod unsupported;

pub use muxy_terminal::backend::{
    LaunchCommand, PointerButton, PointerInput, PointerModifiers, SurfaceAction, SurfaceProgress,
    SurfaceProgressKind, SurfaceSignal,
};
pub use muxy_terminal::confirmation::{ConfirmationId, ConfirmationKind};
pub use muxy_terminal::search::{SearchDispatch, dispatch_for_query, match_display};
pub use surfaces::{StandaloneTerminal, TerminalSurfaces};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalColorScheme {
    Light,
    Dark,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SurfaceIdentity {
    Workspace(muxy_core::workspace::TabId),
    Standalone,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RoutedTerminalEvent {
    Workspace(muxy_core::workspace::TabId, SurfaceSignal),
    WorkspaceOpenUrl(muxy_core::workspace::TabId, String),
    Standalone(SurfaceSignal),
    StandaloneOpenUrl(String),
}

impl SurfaceIdentity {
    pub(crate) fn route(&self, signal: SurfaceSignal) -> RoutedTerminalEvent {
        match self {
            Self::Workspace(tab_id) => RoutedTerminalEvent::Workspace(tab_id.clone(), signal),
            Self::Standalone => RoutedTerminalEvent::Standalone(signal),
        }
    }

    pub(crate) fn route_open_url(&self, url: String) -> RoutedTerminalEvent {
        match self {
            Self::Workspace(tab_id) => RoutedTerminalEvent::WorkspaceOpenUrl(tab_id.clone(), url),
            Self::Standalone => RoutedTerminalEvent::StandaloneOpenUrl(url),
        }
    }
}

#[cfg(target_os = "macos")]
pub use ghostty::GhosttyBackend as Backend;
#[cfg(target_os = "macos")]
pub(crate) use ghostty::install_development_cli_environment;
#[cfg(not(target_os = "macos"))]
pub use unsupported::UnsupportedBackend as Backend;

#[cfg(target_os = "macos")]
pub struct TerminalEvent(ghostty_host::RuntimeEvent);

#[cfg(not(target_os = "macos"))]
pub struct TerminalEvent(());

#[cfg(target_os = "macos")]
pub struct TerminalEvents(async_channel::Receiver<ghostty_host::RuntimeEvent>);

#[cfg(not(target_os = "macos"))]
pub struct TerminalEvents(());

impl TerminalEvents {
    #[cfg(target_os = "macos")]
    pub async fn recv(&self) -> Option<TerminalEvent> {
        self.0.recv().await.ok().map(TerminalEvent)
    }

    #[cfg(not(target_os = "macos"))]
    pub async fn recv(&self) -> Option<TerminalEvent> {
        None
    }
}

pub struct TerminalWakeups(
    #[cfg(target_os = "macos")] async_channel::Receiver<()>,
    #[cfg(not(target_os = "macos"))] (),
);

impl TerminalWakeups {
    #[cfg(target_os = "macos")]
    pub async fn recv(&self) -> bool {
        self.0.recv().await.is_ok()
    }

    #[cfg(not(target_os = "macos"))]
    pub async fn recv(&self) -> bool {
        false
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn wrap_events(
    receiver: async_channel::Receiver<ghostty_host::RuntimeEvent>,
) -> TerminalEvents {
    TerminalEvents(receiver)
}

#[cfg(target_os = "macos")]
pub(crate) fn wrap_wakeups(receiver: async_channel::Receiver<()>) -> TerminalWakeups {
    TerminalWakeups(receiver)
}

#[cfg(target_os = "macos")]
pub(crate) fn unwrap_event(event: TerminalEvent) -> ghostty_host::RuntimeEvent {
    event.0
}
