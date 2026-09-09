use std::collections::BTreeSet;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use muxy_core::extensions::contract::ExtensionApiReply;
use serde_json::{Map, Value};

use super::ExtensionApiError;

pub const MAX_SOCKET_PAYLOAD_BYTES: usize = 128 * 1024;
pub const MAX_WEBVIEW_MESSAGE_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtensionApiSource {
    Background,
    Webview,
    Remote,
}

impl ExtensionApiSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Webview => "webview",
            Self::Remote => "remote",
        }
    }

    pub const fn audit_source(self) -> &'static str {
        match self {
            Self::Background | Self::Remote => "muxy-api",
            Self::Webview => "webview",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtensionSocketReplyKind {
    JsonBase64,
    SettingsGet,
    LegacyUnit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedSocketRequest {
    pub request: ExtensionApiRequest,
    pub reply_kind: ExtensionSocketReplyKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExtensionApiRequest {
    pub extension_id: String,
    pub surface_id: Option<String>,
    pub method: String,
    pub args: Map<String, Value>,
    pub request_id: Option<String>,
    pub source: ExtensionApiSource,
    pub granted_permissions: BTreeSet<String>,
}

impl ExtensionApiRequest {
    pub fn new(
        extension_id: impl Into<String>,
        method: impl Into<String>,
        args: Map<String, Value>,
        source: ExtensionApiSource,
        granted_permissions: BTreeSet<String>,
    ) -> Self {
        Self {
            extension_id: extension_id.into(),
            surface_id: None,
            method: method.into(),
            args,
            request_id: None,
            source,
            granted_permissions,
        }
    }
}

pub fn decode_extension_socket_request(
    extension_id: impl Into<String>,
    granted_permissions: BTreeSet<String>,
    command: &str,
) -> Result<DecodedSocketRequest, ExtensionApiError> {
    let extension_id = extension_id.into();
    let method = command.split('|').next().unwrap_or_default();
    let (request, reply_kind) = match method {
        "extension.settings.get" => {
            let key = command
                .split_once('|')
                .map(|(_, key)| key)
                .filter(|key| !key.is_empty())
                .ok_or_else(|| {
                    ExtensionApiError::InvalidArguments(
                        "usage extension.settings.get|key".to_owned(),
                    )
                })?;
            let args = Map::from_iter([("key".to_owned(), Value::String(key.to_owned()))]);
            (
                ExtensionApiRequest::new(
                    extension_id,
                    method,
                    args,
                    ExtensionApiSource::Background,
                    granted_permissions,
                ),
                ExtensionSocketReplyKind::SettingsGet,
            )
        }
        "extension.settings.set" => {
            let mut parts = command.splitn(3, '|');
            parts.next();
            let key = parts.next().filter(|key| !key.is_empty()).ok_or_else(|| {
                ExtensionApiError::InvalidArguments(
                    "usage extension.settings.set|key|<json-value>".to_owned(),
                )
            })?;
            let raw_value = parts.next().ok_or_else(|| {
                ExtensionApiError::InvalidArguments(
                    "usage extension.settings.set|key|<json-value>".to_owned(),
                )
            })?;
            if raw_value.len() > muxy_core::extensions::state::MAX_SETTING_VALUE_BYTES {
                return Err(ExtensionApiError::Service(format!(
                    "value exceeds {}-byte limit",
                    muxy_core::extensions::state::MAX_SETTING_VALUE_BYTES
                )));
            }
            let value = serde_json::from_str(raw_value).map_err(|error| {
                ExtensionApiError::Service(format!("invalid json value: {error}"))
            })?;
            let args = Map::from_iter([
                ("key".to_owned(), Value::String(key.to_owned())),
                ("value".to_owned(), value),
            ]);
            (
                ExtensionApiRequest::new(
                    extension_id,
                    method,
                    args,
                    ExtensionApiSource::Background,
                    granted_permissions,
                ),
                ExtensionSocketReplyKind::LegacyUnit,
            )
        }
        "extension.statusbar.set" => {
            let mut parts = command.splitn(3, '|');
            parts.next();
            let item_id = parts
                .next()
                .filter(|item_id| !item_id.is_empty())
                .ok_or_else(|| {
                    ExtensionApiError::InvalidArguments(
                        "usage extension.statusbar.set|itemID[|text]".to_owned(),
                    )
                })?;
            let mut args = Map::from_iter([("id".to_owned(), Value::String(item_id.to_owned()))]);
            if let Some(text) = parts.next().filter(|text| !text.is_empty()) {
                args.insert("text".to_owned(), Value::String(text.to_owned()));
            }
            (
                ExtensionApiRequest::new(
                    extension_id,
                    method,
                    args,
                    ExtensionApiSource::Background,
                    granted_permissions,
                ),
                ExtensionSocketReplyKind::LegacyUnit,
            )
        }
        _ => (
            decode_socket_request(extension_id, granted_permissions, command)?,
            ExtensionSocketReplyKind::JsonBase64,
        ),
    };
    Ok(DecodedSocketRequest {
        request,
        reply_kind,
    })
}

pub fn decode_socket_request(
    extension_id: impl Into<String>,
    granted_permissions: BTreeSet<String>,
    command: &str,
) -> Result<ExtensionApiRequest, ExtensionApiError> {
    let (method, payload) = command
        .split_once('|')
        .ok_or_else(|| invalid_payload(command.split('|').next().unwrap_or_default()))?;
    let bytes = STANDARD
        .decode(payload)
        .map_err(|_| invalid_payload(method))?;
    if bytes.len() > MAX_SOCKET_PAYLOAD_BYTES {
        return Err(ExtensionApiError::PayloadTooLarge {
            limit: MAX_SOCKET_PAYLOAD_BYTES,
        });
    }
    let value = serde_json::from_slice::<Value>(&bytes).map_err(|_| invalid_payload(method))?;
    let args = value
        .as_object()
        .cloned()
        .ok_or_else(|| invalid_payload(method))?;
    Ok(ExtensionApiRequest {
        extension_id: extension_id.into(),
        surface_id: None,
        method: method.to_owned(),
        args,
        request_id: None,
        source: ExtensionApiSource::Background,
        granted_permissions,
    })
}

pub fn decode_webview_request(
    extension_id: impl Into<String>,
    granted_permissions: BTreeSet<String>,
    message: Value,
) -> Result<ExtensionApiRequest, ExtensionApiError> {
    let size = serde_json::to_vec(&message)
        .map_err(|_| ExtensionApiError::InvalidMessage)?
        .len();
    if size > MAX_WEBVIEW_MESSAGE_BYTES {
        return Err(ExtensionApiError::PayloadTooLarge {
            limit: MAX_WEBVIEW_MESSAGE_BYTES,
        });
    }
    let payload = message
        .as_object()
        .ok_or(ExtensionApiError::InvalidMessage)?;
    let method = payload
        .get("verb")
        .and_then(Value::as_str)
        .filter(|method| !method.is_empty())
        .ok_or(ExtensionApiError::InvalidMessage)?;
    let request_id = payload
        .get("requestID")
        .and_then(Value::as_str)
        .filter(|request_id| !request_id.is_empty())
        .ok_or(ExtensionApiError::InvalidMessage)?;
    let args = payload
        .get("args")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    Ok(ExtensionApiRequest {
        extension_id: extension_id.into(),
        surface_id: None,
        method: method.to_owned(),
        args,
        request_id: Some(request_id.to_owned()),
        source: ExtensionApiSource::Webview,
        granted_permissions,
    })
}

pub fn encode_extension_socket_result(
    method: &str,
    reply_kind: ExtensionSocketReplyKind,
    result: Result<Value, ExtensionApiError>,
) -> String {
    match reply_kind {
        ExtensionSocketReplyKind::JsonBase64 => encode_socket_result(method, result),
        ExtensionSocketReplyKind::SettingsGet => match result {
            Ok(Value::Null) => "ok".to_owned(),
            Ok(value) => serde_json::to_string(&value)
                .map(|value| format!("ok\t{value}"))
                .unwrap_or_else(|_| "error:encode failed".to_owned()),
            Err(error) => format!("error:{error}"),
        },
        ExtensionSocketReplyKind::LegacyUnit => match result {
            Ok(_) => "ok".to_owned(),
            Err(error) => format!("error:{error}"),
        },
    }
}

pub fn encode_socket_result(method: &str, result: Result<Value, ExtensionApiError>) -> String {
    match result {
        Ok(value) => serde_json::to_vec(&value)
            .map(|encoded| STANDARD.encode(encoded))
            .unwrap_or_else(|_| format!("error:{method} result encoding failed")),
        Err(error) => format!("error:{error}"),
    }
}

pub fn encode_webview_result(
    request_id: Option<String>,
    result: Result<Value, ExtensionApiError>,
) -> Value {
    let reply = match result {
        Ok(value) => ExtensionApiReply::success(request_id, value),
        Err(error) => ExtensionApiReply::failure(request_id, error.to_string()),
    };
    serde_json::to_value(reply).unwrap_or_else(|_| invalid_webview_message())
}

pub fn invalid_webview_message() -> Value {
    serde_json::json!({"ok": false, "error": "invalid message"})
}

fn invalid_payload(method: &str) -> ExtensionApiError {
    ExtensionApiError::InvalidPayload {
        method: method.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn encoded(value: Value) -> String {
        STANDARD.encode(serde_json::to_vec(&value).unwrap())
    }

    #[test]
    fn socket_and_webview_decode_to_the_same_request_model() {
        let permissions = BTreeSet::from(["git:read".to_owned()]);
        let socket = decode_socket_request(
            "git",
            permissions.clone(),
            &format!("git.status|{}", encoded(json!({"project": "muxy"}))),
        )
        .unwrap();
        let webview = decode_webview_request(
            "git",
            permissions,
            json!({
                "verb": "git.status",
                "args": {"project": "muxy"},
                "requestID": "request-1"
            }),
        )
        .unwrap();
        assert_eq!(socket.extension_id, webview.extension_id);
        assert_eq!(socket.method, webview.method);
        assert_eq!(socket.args, webview.args);
        assert_eq!(socket.source, ExtensionApiSource::Background);
        assert_eq!(webview.source, ExtensionApiSource::Webview);
        assert_eq!(webview.request_id.as_deref(), Some("request-1"));
    }

    #[test]
    fn socket_payload_must_be_base64_json_object() {
        for command in ["git.status", "git.status|not-base64"] {
            assert_eq!(
                decode_socket_request("git", BTreeSet::new(), command)
                    .unwrap_err()
                    .to_string(),
                "invalid git.status payload"
            );
        }
        assert_eq!(
            decode_socket_request(
                "git",
                BTreeSet::new(),
                &format!("git.status|{}", encoded(json!([1, 2, 3])))
            )
            .unwrap_err()
            .to_string(),
            "invalid git.status payload"
        );
    }

    #[test]
    fn webview_decode_matches_swift_message_rules() {
        let request = decode_webview_request(
            "git",
            BTreeSet::new(),
            json!({"verb": "toast", "args": "ignored", "requestID": "r"}),
        )
        .unwrap();
        assert!(request.args.is_empty());
        assert_eq!(
            decode_webview_request("git", BTreeSet::new(), json!({"verb": "toast"})).unwrap_err(),
            ExtensionApiError::InvalidMessage
        );
        assert_eq!(
            invalid_webview_message(),
            json!({"ok": false, "error": "invalid message"})
        );
    }

    #[test]
    fn legacy_settings_socket_aliases_keep_the_swift_wire_shape() {
        let decoded = decode_extension_socket_request(
            "git",
            BTreeSet::new(),
            "extension.settings.set|branch|\"main|next\"",
        )
        .unwrap();
        assert_eq!(decoded.reply_kind, ExtensionSocketReplyKind::LegacyUnit);
        assert_eq!(decoded.request.args["key"], "branch");
        assert_eq!(decoded.request.args["value"], "main|next");
        assert_eq!(
            encode_extension_socket_result(
                "extension.settings.set",
                decoded.reply_kind,
                Ok(Value::Null)
            ),
            "ok"
        );
        let decoded = decode_extension_socket_request(
            "git",
            BTreeSet::new(),
            "extension.settings.get|branch",
        )
        .unwrap();
        assert_eq!(
            encode_extension_socket_result(
                "extension.settings.get",
                decoded.reply_kind,
                Ok(json!("main"))
            ),
            "ok\t\"main\""
        );
    }

    #[test]
    fn reply_encoders_match_the_swift_bridge_shapes() {
        let socket = encode_socket_result("git.status", Ok(json!({"clean": true})));
        assert_eq!(
            serde_json::from_slice::<Value>(&STANDARD.decode(socket).unwrap()).unwrap(),
            json!({"clean": true})
        );
        assert_eq!(
            encode_socket_result(
                "git.status",
                Err(ExtensionApiError::PermissionDenied("git:read".to_owned()))
            ),
            "error:permission denied (git:read)"
        );
        assert_eq!(
            encode_webview_result(Some("r".to_owned()), Ok(Value::Null)),
            json!({"requestID": "r", "ok": true, "value": null})
        );
    }
}
