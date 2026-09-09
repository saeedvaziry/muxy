use super::{ExtensionLifecycleCoordinator, ExtensionWebViewDescriptor, ExtensionWebViewEvent};
use gpui::{AnyElement, Rgba};
use muxy_api::extensions::{ExtensionApiError, ExtensionApiRequest};
use serde_json::Value;
use std::collections::HashSet;

pub(crate) struct ExtensionWebViewRegistry {
    lifecycle: ExtensionLifecycleCoordinator,
}

impl ExtensionWebViewRegistry {
    pub(crate) fn new(
        _terminals: &crate::terminal::TerminalSurfaces,
    ) -> (Self, async_channel::Receiver<ExtensionWebViewEvent>) {
        let (_sender, receiver) = async_channel::unbounded();
        (
            Self {
                lifecycle: ExtensionLifecycleCoordinator::default(),
            },
            receiver,
        )
    }

    pub(crate) fn reconcile(
        &mut self,
        _descriptors: &[ExtensionWebViewDescriptor],
        _visible: &HashSet<String>,
        _focused: Option<&str>,
        _theme: &Value,
        _background: Rgba,
    ) {
    }

    pub(crate) fn element(&self, _surface_id: &str, _visible: bool) -> Option<AnyElement> {
        None
    }

    pub(crate) fn request_before_close(
        &mut self,
        _surface_id: &str,
        _reason: &str,
    ) -> Option<(
        String,
        async_channel::Receiver<super::ExtensionLifecycleVerdict>,
    )> {
        None
    }

    pub(crate) fn expire_unacknowledged(&mut self, _call_id: &str) -> bool {
        false
    }

    pub(crate) fn handle_lifecycle_message(
        &mut self,
        surface_id: &str,
        request: &ExtensionApiRequest,
    ) -> Option<Result<Value, ExtensionApiError>> {
        super::handle_lifecycle_message(&mut self.lifecycle, surface_id, request)
    }

    pub(crate) fn dispatch_event(
        &self,
        _extension_id: Option<&str>,
        _name: &str,
        _payload: &Value,
    ) {
    }

    pub(crate) fn deliver_modal_result(
        &self,
        _surface_id: &str,
        _request_id: &str,
        _result: &Value,
    ) {
    }

    pub(crate) fn unsupported_error() -> ExtensionApiError {
        ExtensionApiError::UnsupportedPlatform(
            "extension webviews are only available on macOS".to_owned(),
        )
    }
}
