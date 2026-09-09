use serde_json::Value;

use super::{
    ConsentRequest, ExtensionApiError, ExtensionApiRequest, ExtensionApiServiceGroup,
    consent_request, extension_api_service_group, preflight_request,
};

pub trait ExtensionApiService {
    fn is_available(&self, group: ExtensionApiServiceGroup, method: &str) -> bool;

    fn dispatch(
        &mut self,
        group: ExtensionApiServiceGroup,
        request: &ExtensionApiRequest,
    ) -> ExtensionServiceDispatch;
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExtensionServiceDispatch {
    Complete(Result<Value, ExtensionApiError>),
    Deferred,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DispatchOutcome {
    Complete(Result<Value, ExtensionApiError>),
    Deferred {
        request: ExtensionApiRequest,
        group: ExtensionApiServiceGroup,
    },
    ConsentRequired {
        request: ExtensionApiRequest,
        consent: Box<ConsentRequest>,
    },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ExtensionApiDispatcher;

impl ExtensionApiDispatcher {
    pub fn dispatch(
        request: ExtensionApiRequest,
        extension_display_name: &str,
        service: &mut impl ExtensionApiService,
    ) -> DispatchOutcome {
        if let Err(error) = preflight_request(&request) {
            return DispatchOutcome::Complete(Err(error));
        }
        let Some(group) = extension_api_service_group(&request.method) else {
            return DispatchOutcome::Complete(Err(ExtensionApiError::UnknownMethod(
                request.method,
            )));
        };
        if !service.is_available(group, &request.method) {
            return DispatchOutcome::Complete(Err(ExtensionApiError::ServiceUnavailable(
                request.method,
            )));
        }
        match consent_request(&request, extension_display_name) {
            Ok(Some(consent)) => DispatchOutcome::ConsentRequired {
                request,
                consent: Box::new(consent),
            },
            Ok(None) => service_outcome(request, group, service),
            Err(error) => DispatchOutcome::Complete(Err(error)),
        }
    }

    pub fn dispatch_authorized(
        request: ExtensionApiRequest,
        service: &mut impl ExtensionApiService,
    ) -> DispatchOutcome {
        if let Err(error) = preflight_request(&request) {
            return DispatchOutcome::Complete(Err(error));
        }
        let Some(group) = extension_api_service_group(&request.method) else {
            return DispatchOutcome::Complete(Err(ExtensionApiError::UnknownMethod(
                request.method,
            )));
        };
        if !service.is_available(group, &request.method) {
            return DispatchOutcome::Complete(Err(ExtensionApiError::ServiceUnavailable(
                request.method,
            )));
        }
        service_outcome(request, group, service)
    }
}

fn service_outcome(
    request: ExtensionApiRequest,
    group: ExtensionApiServiceGroup,
    service: &mut impl ExtensionApiService,
) -> DispatchOutcome {
    match service.dispatch(group, &request) {
        ExtensionServiceDispatch::Complete(result) => DispatchOutcome::Complete(result),
        ExtensionServiceDispatch::Deferred => DispatchOutcome::Deferred { request, group },
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UnavailableExtensionApiService;

impl ExtensionApiService for UnavailableExtensionApiService {
    fn is_available(&self, _: ExtensionApiServiceGroup, _: &str) -> bool {
        false
    }

    fn dispatch(
        &mut self,
        _: ExtensionApiServiceGroup,
        request: &ExtensionApiRequest,
    ) -> ExtensionServiceDispatch {
        ExtensionServiceDispatch::Complete(Err(ExtensionApiError::ServiceUnavailable(
            request.method.clone(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use muxy_core::extensions::manifest::ExtensionPermission;
    use serde_json::{Map, json};

    use super::*;
    use crate::extensions::ExtensionApiSource;

    struct EchoService {
        calls: Vec<String>,
    }

    impl ExtensionApiService for EchoService {
        fn is_available(&self, _: ExtensionApiServiceGroup, _: &str) -> bool {
            true
        }

        fn dispatch(
            &mut self,
            _: ExtensionApiServiceGroup,
            request: &ExtensionApiRequest,
        ) -> ExtensionServiceDispatch {
            self.calls.push(request.method.clone());
            ExtensionServiceDispatch::Complete(Ok(Value::Object(request.args.clone())))
        }
    }

    fn permissions() -> BTreeSet<String> {
        ExtensionPermission::ALL
            .into_iter()
            .map(|permission| permission.as_str().to_owned())
            .collect()
    }

    fn request(method: &str, args: Value, source: ExtensionApiSource) -> ExtensionApiRequest {
        ExtensionApiRequest::new(
            "git",
            method,
            args.as_object().cloned().unwrap_or_else(Map::new),
            source,
            permissions(),
        )
    }

    #[test]
    fn background_and_webview_use_the_same_dispatch_path() {
        let mut service = EchoService { calls: Vec::new() };
        for source in [ExtensionApiSource::Background, ExtensionApiSource::Webview] {
            let outcome = ExtensionApiDispatcher::dispatch(
                request("git.status", json!({"project": "muxy"}), source),
                "Git",
                &mut service,
            );
            assert_eq!(
                outcome,
                DispatchOutcome::Complete(Ok(json!({"project": "muxy"})))
            );
        }
        assert_eq!(service.calls, ["git.status", "git.status"]);
    }

    #[test]
    fn sensitive_calls_stop_for_consent_before_service_execution() {
        let mut service = EchoService { calls: Vec::new() };
        let outcome = ExtensionApiDispatcher::dispatch(
            request(
                "exec",
                json!({"argv": ["git", "status"]}),
                ExtensionApiSource::Background,
            ),
            "Git",
            &mut service,
        );
        assert!(matches!(outcome, DispatchOutcome::ConsentRequired { .. }));
        assert!(service.calls.is_empty());
    }

    #[test]
    fn unavailable_services_return_a_stable_error_before_consent() {
        let mut service = UnavailableExtensionApiService;
        assert_eq!(
            ExtensionApiDispatcher::dispatch(
                request(
                    "exec",
                    json!({"argv": ["git", "status"]}),
                    ExtensionApiSource::Background,
                ),
                "Git",
                &mut service,
            ),
            DispatchOutcome::Complete(Err(ExtensionApiError::ServiceUnavailable(
                "exec".to_owned()
            )))
        );
    }
}
