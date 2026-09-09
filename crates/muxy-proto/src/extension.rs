use std::collections::BTreeMap;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use uuid::Uuid;

pub const EXTENSION_LOCAL_EVENT_HEAD: &str = "extension-event";
pub const EXTENSION_LOCAL_EVENT_NAME_PREFIX: &str = "extension.";
pub const EXTENSION_LOCAL_EVENT_MAX_NAME_LENGTH: usize = 200;
pub const EXTENSION_LOCAL_EVENT_MAX_PAYLOAD_BYTES: usize = 64 * 1024;
pub const EXTENSION_BROADCAST_HEAD: &str = "event";
pub const MODAL_RESULT_HEAD: &str = "modal-result";
pub const MODAL_QUERY_HEAD: &str = "modal-query";
pub const INVOKE_HEAD: &str = "invoke";
pub const INVOKE_RESULT_HEAD: &str = "invoke-result";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionLocalEvent {
    pub name: String,
    pub payload: Vec<u8>,
}

impl ExtensionLocalEvent {
    pub fn encode(&self) -> Option<String> {
        if !is_valid_extension_local_event_name(&self.name)
            || self.payload.len() > EXTENSION_LOCAL_EVENT_MAX_PAYLOAD_BYTES
        {
            return None;
        }
        Some(format!(
            "{EXTENSION_LOCAL_EVENT_HEAD}|{}|{}",
            STANDARD.encode(self.name.as_bytes()),
            STANDARD.encode(&self.payload)
        ))
    }

    pub fn parse(line: &str) -> Option<Self> {
        let parts = line.splitn(3, '|').collect::<Vec<_>>();
        if parts.len() != 3 || parts[0] != EXTENSION_LOCAL_EVENT_HEAD {
            return None;
        }
        let name = String::from_utf8(STANDARD.decode(parts[1]).ok()?).ok()?;
        if !is_valid_extension_local_event_name(&name) {
            return None;
        }
        let payload = STANDARD.decode(parts[2]).ok()?;
        if payload.len() > EXTENSION_LOCAL_EVENT_MAX_PAYLOAD_BYTES {
            return None;
        }
        Some(Self { name, payload })
    }
}

pub fn is_valid_extension_local_event_name(name: &str) -> bool {
    let length = name.chars().count();
    name.starts_with(EXTENSION_LOCAL_EVENT_NAME_PREFIX)
        && length > EXTENSION_LOCAL_EVENT_NAME_PREFIX.len()
        && length <= EXTENSION_LOCAL_EVENT_MAX_NAME_LENGTH
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionBroadcast {
    pub name: String,
    pub payload: BTreeMap<String, String>,
}

impl ExtensionBroadcast {
    pub fn encode(&self) -> String {
        let mut line = format!("{EXTENSION_BROADCAST_HEAD}|{}", self.name);
        for (key, value) in &self.payload {
            let key = key.replace(['|', '='], "_");
            let value = value.replace(['|', '\n'], " ");
            line.push('|');
            line.push_str(&key);
            line.push('=');
            line.push_str(&value);
        }
        line
    }

    pub fn parse(line: &str) -> Option<Self> {
        let mut parts = line.split('|');
        if parts.next()? != EXTENSION_BROADCAST_HEAD {
            return None;
        }
        let name = parts.next()?.to_owned();
        if name.is_empty() {
            return None;
        }
        let mut payload = BTreeMap::new();
        for segment in parts {
            let Some((key, value)) = segment.split_once('=') else {
                continue;
            };
            if !key.is_empty() {
                payload.insert(key.to_owned(), value.to_owned());
            }
        }
        Some(Self { name, payload })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModalResult {
    pub request_id: String,
    pub payload: Vec<u8>,
}

impl ModalResult {
    pub fn encode(&self) -> Option<String> {
        if self.request_id.is_empty() || self.request_id.contains('|') {
            return None;
        }
        Some(format!(
            "{MODAL_RESULT_HEAD}|{}|{}",
            self.request_id,
            STANDARD.encode(&self.payload)
        ))
    }

    pub fn parse(line: &str) -> Option<Self> {
        let parts = line.splitn(3, '|').collect::<Vec<_>>();
        if parts.len() != 3 || parts[0] != MODAL_RESULT_HEAD || parts[1].is_empty() {
            return None;
        }
        Some(Self {
            request_id: parts[1].to_owned(),
            payload: STANDARD.decode(parts[2]).ok()?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModalQuery {
    pub request_id: String,
    pub query_id: i64,
    pub query: String,
    pub options: BTreeMap<String, bool>,
}

impl ModalQuery {
    pub fn encode(&self) -> Option<String> {
        if self.request_id.is_empty() || self.request_id.contains('|') {
            return None;
        }
        let query = STANDARD.encode(self.query.as_bytes());
        if self.options.is_empty() {
            return Some(format!(
                "{MODAL_QUERY_HEAD}|{}|{}|{query}",
                self.request_id, self.query_id
            ));
        }
        let options = serde_json::to_vec(&self.options).ok()?;
        Some(format!(
            "{MODAL_QUERY_HEAD}|{}|{}|{query}|{}",
            self.request_id,
            self.query_id,
            STANDARD.encode(options)
        ))
    }

    pub fn parse(line: &str) -> Option<Self> {
        let parts = line.splitn(5, '|').collect::<Vec<_>>();
        if !(parts.len() == 4 || parts.len() == 5)
            || parts[0] != MODAL_QUERY_HEAD
            || parts[1].is_empty()
        {
            return None;
        }
        let query_id = parts[2].parse().ok()?;
        let query = String::from_utf8(STANDARD.decode(parts[3]).ok()?).ok()?;
        let options = if parts.len() == 5 {
            serde_json::from_slice(&STANDARD.decode(parts[4]).ok()?).ok()?
        } else {
            BTreeMap::new()
        };
        Some(Self {
            request_id: parts[1].to_owned(),
            query_id,
            query,
            options,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvokeRequest {
    pub call_id: String,
    pub action: String,
    pub payload: Vec<u8>,
}

impl InvokeRequest {
    pub fn new(action: impl Into<String>, payload: Vec<u8>) -> Self {
        Self {
            call_id: Uuid::new_v4().hyphenated().to_string().to_uppercase(),
            action: action.into(),
            payload,
        }
    }

    pub fn encode(&self) -> Option<String> {
        if self.call_id.is_empty()
            || self.call_id.contains('|')
            || self.action.is_empty()
            || self.action.contains('|')
        {
            return None;
        }
        Some(format!(
            "{INVOKE_HEAD}|{}|{}|{}",
            self.call_id,
            self.action,
            STANDARD.encode(&self.payload)
        ))
    }

    pub fn parse(line: &str) -> Option<Self> {
        let parts = line.splitn(4, '|').collect::<Vec<_>>();
        if parts.len() != 4 || parts[0] != INVOKE_HEAD || parts[1].is_empty() || parts[2].is_empty()
        {
            return None;
        }
        Some(Self {
            call_id: parts[1].to_owned(),
            action: parts[2].to_owned(),
            payload: STANDARD.decode(parts[3]).ok()?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvokeResult {
    pub call_id: String,
    pub ok: bool,
    pub body: Vec<u8>,
}

impl InvokeResult {
    pub fn encode(&self) -> Option<String> {
        if self.call_id.is_empty() || self.call_id.contains('|') {
            return None;
        }
        let status = if self.ok { "ok" } else { "err" };
        Some(format!(
            "{INVOKE_RESULT_HEAD}|{}|{status}|{}",
            self.call_id,
            STANDARD.encode(&self.body)
        ))
    }

    pub fn parse(line: &str) -> Option<Self> {
        let parts = line.splitn(4, '|').collect::<Vec<_>>();
        if parts.len() < 3 || parts[0] != INVOKE_RESULT_HEAD || parts[1].is_empty() {
            return None;
        }
        let ok = match parts[2] {
            "ok" => true,
            "err" => false,
            _ => return None,
        };
        let body = parts
            .get(3)
            .and_then(|body| STANDARD.decode(body).ok())
            .unwrap_or_default();
        Some(Self {
            call_id: parts[1].to_owned(),
            ok,
            body,
        })
    }

    pub fn outcome(self) -> InvokeOutcome {
        if self.ok {
            InvokeOutcome::Success(self.body)
        } else {
            InvokeOutcome::Error(
                String::from_utf8(self.body).unwrap_or_else(|_| "extension error".to_owned()),
            )
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvokeOutcome {
    Success(Vec<u8>),
    Error(String),
    Unavailable,
    Timeout,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_events_round_trip_name_and_payload_base64() {
        let event = ExtensionLocalEvent {
            name: "extension.build:finished".to_owned(),
            payload: br#"{"ok":true}|raw"#.to_vec(),
        };
        let encoded = event.encode().unwrap();
        assert_eq!(
            encoded,
            "extension-event|ZXh0ZW5zaW9uLmJ1aWxkOmZpbmlzaGVk|eyJvayI6dHJ1ZX18cmF3"
        );
        assert_eq!(ExtensionLocalEvent::parse(&encoded), Some(event));
    }

    #[test]
    fn local_event_names_match_the_retained_ascii_contract() {
        assert!(is_valid_extension_local_event_name("extension.a"));
        assert!(is_valid_extension_local_event_name(
            "extension.A0_value:part-name"
        ));
        for name in [
            "extension.",
            "other.event",
            "extension.bad/name",
            "extension.é",
        ] {
            assert!(!is_valid_extension_local_event_name(name), "{name}");
        }
        let maximum = format!(
            "extension.{}",
            "a".repeat(EXTENSION_LOCAL_EVENT_MAX_NAME_LENGTH - "extension.".len())
        );
        assert_eq!(maximum.len(), EXTENSION_LOCAL_EVENT_MAX_NAME_LENGTH);
        assert!(is_valid_extension_local_event_name(&maximum));
        assert!(!is_valid_extension_local_event_name(&format!("{maximum}a")));
    }

    #[test]
    fn local_event_payload_limit_is_exact() {
        let at_limit = ExtensionLocalEvent {
            name: "extension.payload".to_owned(),
            payload: vec![0; EXTENSION_LOCAL_EVENT_MAX_PAYLOAD_BYTES],
        };
        let encoded = at_limit.encode().unwrap();
        assert_eq!(ExtensionLocalEvent::parse(&encoded), Some(at_limit));

        let over_limit = ExtensionLocalEvent {
            name: "extension.payload".to_owned(),
            payload: vec![0; EXTENSION_LOCAL_EVENT_MAX_PAYLOAD_BYTES + 1],
        };
        assert_eq!(over_limit.encode(), None);
        let encoded = format!(
            "extension-event|{}|{}",
            STANDARD.encode("extension.payload"),
            STANDARD.encode(&over_limit.payload)
        );
        assert_eq!(ExtensionLocalEvent::parse(&encoded), None);
    }

    #[test]
    fn malformed_local_events_are_rejected() {
        for line in [
            "extension-event",
            "extension-event||",
            "extension-event|%%%|",
            "extension-event|ZXh0ZW5zaW9uLg==|%%%",
            "other|ZXh0ZW5zaW9uLmE=|bnVsbA==",
        ] {
            assert_eq!(ExtensionLocalEvent::parse(line), None, "{line}");
        }
        let invalid_utf8_name = format!("extension-event|{}|", STANDARD.encode([0xff, 0xfe]));
        assert_eq!(ExtensionLocalEvent::parse(&invalid_utf8_name), None);
    }

    #[test]
    fn broadcasts_sort_keys_and_sanitize_reserved_characters() {
        let broadcast = ExtensionBroadcast {
            name: "sample.event".to_owned(),
            payload: BTreeMap::from([
                ("z|=key".to_owned(), "line|one\nline two\r".to_owned()),
                ("a".to_owned(), "first".to_owned()),
            ]),
        };
        let encoded = broadcast.encode();
        assert_eq!(
            encoded,
            "event|sample.event|a=first|z__key=line one line two\r"
        );
        assert_eq!(
            ExtensionBroadcast::parse(&encoded),
            Some(ExtensionBroadcast {
                name: "sample.event".to_owned(),
                payload: BTreeMap::from([
                    ("a".to_owned(), "first".to_owned()),
                    ("z__key".to_owned(), "line one line two\r".to_owned()),
                ])
            })
        );
        assert_eq!(ExtensionBroadcast::parse("event|"), None);
        assert_eq!(ExtensionBroadcast::parse("other|name"), None);
    }

    #[test]
    fn modal_results_match_the_retained_form() {
        let result = ModalResult {
            request_id: "request-1".to_owned(),
            payload: vec![0, 1, 2, 255],
        };
        let encoded = result.encode().unwrap();
        assert_eq!(encoded, "modal-result|request-1|AAEC/w==");
        assert_eq!(ModalResult::parse(&encoded), Some(result));
        assert_eq!(
            ModalResult {
                request_id: "".to_owned(),
                payload: Vec::new()
            }
            .encode(),
            None
        );
        assert_eq!(ModalResult::parse("modal-result||"), None);
        assert_eq!(ModalResult::parse("modal-result|id|%%%"), None);
    }

    #[test]
    fn modal_queries_support_absent_and_boolean_options() {
        let query = ModalQuery {
            request_id: "request-1".to_owned(),
            query_id: -42,
            query: "ready? ✓".to_owned(),
            options: BTreeMap::new(),
        };
        let encoded = query.encode().unwrap();
        assert_eq!(encoded, "modal-query|request-1|-42|cmVhZHk/IOKckw==");
        assert_eq!(ModalQuery::parse(&encoded), Some(query));

        let query = ModalQuery {
            request_id: "request-2".to_owned(),
            query_id: 7,
            query: "choose".to_owned(),
            options: BTreeMap::from([
                ("allowCancel".to_owned(), true),
                ("multi".to_owned(), false),
            ]),
        };
        let encoded = query.encode().unwrap();
        assert_eq!(ModalQuery::parse(&encoded), Some(query));
    }

    #[test]
    fn malformed_modal_queries_are_rejected() {
        let non_boolean_options = STANDARD.encode(br#"{"allowCancel":1}"#);
        let invalid_utf8_query = STANDARD.encode([0xff]);
        for line in [
            "modal-query|id|not-an-int|cXVlcnk=",
            "modal-query||1|cXVlcnk=",
            "modal-query|id|1|%%%",
            &format!("modal-query|id|1|{invalid_utf8_query}"),
            &format!("modal-query|id|1|cXVlcnk=|{non_boolean_options}"),
            "modal-query|id|1|cXVlcnk=|%%%",
        ] {
            assert_eq!(ModalQuery::parse(line), None, "{line}");
        }
    }

    #[test]
    fn invoke_requests_use_uppercase_hyphenated_v4_call_ids() {
        let request = InvokeRequest::new("sample.action", b"payload".to_vec());
        assert_eq!(request.call_id.len(), 36);
        assert_eq!(request.call_id, request.call_id.to_uppercase());
        assert_eq!(request.call_id.as_bytes()[8], b'-');
        assert_eq!(
            Uuid::parse_str(&request.call_id).unwrap().get_version_num(),
            4
        );
        let encoded = request.encode().unwrap();
        assert_eq!(InvokeRequest::parse(&encoded), Some(request));
    }

    #[test]
    fn invoke_parsing_preserves_max_split_payload_behavior() {
        let request = InvokeRequest {
            call_id: "CALL".to_owned(),
            action: "action".to_owned(),
            payload: b"a|b".to_vec(),
        };
        assert_eq!(request.encode().unwrap(), "invoke|CALL|action|YXxi");
        assert_eq!(InvokeRequest::parse("invoke|CALL|action|%%%"), None);
        assert_eq!(InvokeRequest::parse("invoke||action|"), None);
        assert_eq!(
            InvokeRequest {
                call_id: "CALL".to_owned(),
                action: "bad|action".to_owned(),
                payload: Vec::new()
            }
            .encode(),
            None
        );
    }

    #[test]
    fn invoke_results_match_status_body_and_fallback_rules() {
        let result = InvokeResult {
            call_id: "CALL".to_owned(),
            ok: true,
            body: b"abc".to_vec(),
        };
        assert_eq!(
            result.encode().as_deref(),
            Some("invoke-result|CALL|ok|YWJj")
        );
        assert_eq!(
            InvokeResult::parse("invoke-result|CALL|ok|YWJj").unwrap(),
            result
        );
        assert_eq!(
            InvokeResult::parse("invoke-result|CALL|ok").unwrap().body,
            Vec::<u8>::new()
        );
        assert_eq!(
            InvokeResult::parse("invoke-result|CALL|ok|%%%")
                .unwrap()
                .body,
            Vec::<u8>::new()
        );
        assert_eq!(
            InvokeResult::parse("invoke-result|CALL|ok|YWJj|tail")
                .unwrap()
                .body,
            Vec::<u8>::new()
        );
        assert_eq!(InvokeResult::parse("invoke-result||ok|"), None);
        assert_eq!(InvokeResult::parse("invoke-result|CALL|unknown|"), None);
    }

    #[test]
    fn invoke_outcomes_preserve_success_and_decode_errors() {
        assert_eq!(
            InvokeResult {
                call_id: "CALL".to_owned(),
                ok: true,
                body: vec![0xff]
            }
            .outcome(),
            InvokeOutcome::Success(vec![0xff])
        );
        assert_eq!(
            InvokeResult {
                call_id: "CALL".to_owned(),
                ok: false,
                body: b"denied".to_vec()
            }
            .outcome(),
            InvokeOutcome::Error("denied".to_owned())
        );
        assert_eq!(
            InvokeResult {
                call_id: "CALL".to_owned(),
                ok: false,
                body: vec![0xff]
            }
            .outcome(),
            InvokeOutcome::Error("extension error".to_owned())
        );
    }
}
