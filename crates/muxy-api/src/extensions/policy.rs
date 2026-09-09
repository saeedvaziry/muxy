use muxy_core::extensions::contract::{
    P9_BROWSER_API_METHODS, P10_EXTENSION_API_METHODS, WEBVIEW_CONTROL_METHODS,
    is_known_api_method, required_permission_for_args,
};
use thiserror::Error;

use super::{ExtensionApiRequest, ExtensionApiSource};

const SOCKET_ONLY_METHODS: [&str; 3] = [
    "extension.settings.get",
    "extension.settings.set",
    "extension.statusbar.set",
];

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ExtensionApiError {
    #[error("invalid message")]
    InvalidMessage,
    #[error("invalid {method} payload")]
    InvalidPayload { method: String },
    #[error("extension API payload exceeds {limit}-byte limit")]
    PayloadTooLarge { limit: usize },
    #[error("unknown verb {0}")]
    UnknownMethod(String),
    #[error("{0} is unavailable from {1} extensions")]
    UnavailableSurface(String, String),
    #[error("permission denied ({0})")]
    PermissionDenied(String),
    #[error("invalid arguments: {0}")]
    InvalidArguments(String),
    #[error("extension API service '{0}' is unavailable")]
    ServiceUnavailable(String),
    #[error("unsupported platform: {0}")]
    UnsupportedPlatform(String),
    #[error("user denied consent for {0}")]
    ConsentDenied(String),
    #[error("http: blocked request to private or loopback host '{0}'")]
    BlockedHttpHost(String),
    #[error("failed to persist extension consent: {0}")]
    ConsentPersistence(String),
    #[error("{0}")]
    Service(String),
}

pub fn preflight_request(request: &ExtensionApiRequest) -> Result<(), ExtensionApiError> {
    if !is_known_api_method(&request.method) {
        return Err(ExtensionApiError::UnknownMethod(request.method.clone()));
    }
    ensure_method_available(&request.method, request.source)?;
    if let Some(permission) = required_permission_for_args(&request.method, &request.args)
        && !request.granted_permissions.contains(permission.as_str())
    {
        return Err(ExtensionApiError::PermissionDenied(
            permission.as_str().to_owned(),
        ));
    }
    Ok(())
}

pub fn ensure_method_available(
    method: &str,
    source: ExtensionApiSource,
) -> Result<(), ExtensionApiError> {
    let available = match source {
        ExtensionApiSource::Background => {
            P9_BROWSER_API_METHODS.contains(&method) || P10_EXTENSION_API_METHODS.contains(&method)
        }
        ExtensionApiSource::Webview => {
            !SOCKET_ONLY_METHODS.contains(&method)
                && (P9_BROWSER_API_METHODS.contains(&method)
                    || P10_EXTENSION_API_METHODS.contains(&method)
                    || WEBVIEW_CONTROL_METHODS.contains(&method))
        }
        ExtensionApiSource::Remote => P10_EXTENSION_API_METHODS.contains(&method),
    };
    if available {
        Ok(())
    } else {
        Err(ExtensionApiError::UnavailableSurface(
            method.to_owned(),
            source.as_str().to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use muxy_core::extensions::contract::{
        P9_BROWSER_API_METHODS, P10_EXTENSION_API_METHODS, WEBVIEW_CONTROL_METHODS,
    };
    use serde_json::{Map, json};

    use super::*;

    fn request(method: &str, source: ExtensionApiSource) -> ExtensionApiRequest {
        ExtensionApiRequest::new("git", method, Map::new(), source, BTreeSet::new())
    }

    #[test]
    fn every_catalog_method_has_an_explicit_surface_decision() {
        for method in P9_BROWSER_API_METHODS {
            assert!(ensure_method_available(method, ExtensionApiSource::Background).is_ok());
            assert!(ensure_method_available(method, ExtensionApiSource::Webview).is_ok());
            assert!(ensure_method_available(method, ExtensionApiSource::Remote).is_err());
        }
        for method in P10_EXTENSION_API_METHODS {
            assert!(ensure_method_available(method, ExtensionApiSource::Background).is_ok());
            if SOCKET_ONLY_METHODS.contains(&method) {
                assert!(ensure_method_available(method, ExtensionApiSource::Webview).is_err());
            } else {
                assert!(ensure_method_available(method, ExtensionApiSource::Webview).is_ok());
            }
            assert!(ensure_method_available(method, ExtensionApiSource::Remote).is_ok());
        }
        for method in WEBVIEW_CONTROL_METHODS {
            assert!(ensure_method_available(method, ExtensionApiSource::Background).is_err());
            assert!(ensure_method_available(method, ExtensionApiSource::Webview).is_ok());
            assert!(ensure_method_available(method, ExtensionApiSource::Remote).is_err());
        }
    }

    #[test]
    fn permission_policy_is_shared_and_argument_sensitive() {
        let mut denied = request("browser.wait", ExtensionApiSource::Webview);
        denied.args = json!({"function": "document.title"})
            .as_object()
            .unwrap()
            .clone();
        denied.granted_permissions.insert("browser:read".to_owned());
        assert_eq!(
            preflight_request(&denied),
            Err(ExtensionApiError::PermissionDenied(
                "browser:write".to_owned()
            ))
        );
        denied
            .granted_permissions
            .insert("browser:write".to_owned());
        assert!(preflight_request(&denied).is_ok());
    }

    #[test]
    fn unknown_methods_never_reach_services() {
        assert_eq!(
            preflight_request(&request("git.future", ExtensionApiSource::Webview)),
            Err(ExtensionApiError::UnknownMethod("git.future".to_owned()))
        );
    }
}
