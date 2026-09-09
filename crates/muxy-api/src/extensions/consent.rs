use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use muxy_core::extensions::contract::required_permission;
use muxy_core::extensions::logs::ExtensionAuditEntry;
use muxy_core::extensions::manifest::ExtensionPermission;
use muxy_core::extensions::state::{
    ExtensionGatedVerb, ExtensionGrantDecision, ExtensionGrantMatch, ExtensionGrantRule,
};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use url::Url;
use uuid::Uuid;

use super::{ExtensionApiError, ExtensionApiRequest};

pub const PROMPT_TIMEOUT: Duration = Duration::from_secs(60);
pub const MAX_QUEUED_PROMPTS_PER_EXTENSION: usize = 5;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExtensionGatedPayload {
    Exec {
        argv: Option<Vec<String>>,
        shell: Option<String>,
    },
    Pane {
        id: String,
    },
    ForeignTab {
        target_extension_id: String,
        tab_type_id: String,
    },
    Remote {
        action: String,
        device_name: String,
    },
    Git {
        operation: String,
        repo_path: String,
    },
    File {
        operation: String,
        path: String,
    },
    Http {
        hostname: String,
        method: String,
        url: String,
    },
    TabCommand {
        command: String,
    },
    Project {
        name: String,
        path: String,
    },
}

impl ExtensionGatedPayload {
    pub fn matches(&self, match_rule: &ExtensionGrantMatch) -> bool {
        match (self, match_rule) {
            (_, ExtensionGrantMatch::Any) => true,
            (Self::Exec { argv, .. }, ExtensionGrantMatch::ArgvExact { value }) => {
                argv.as_ref() == Some(value)
            }
            (Self::Exec { argv, .. }, ExtensionGrantMatch::ArgvPrefix { value }) => argv
                .as_ref()
                .is_some_and(|argv| argv.starts_with(value.as_slice())),
            (Self::Exec { shell, .. }, ExtensionGrantMatch::ShellExact { string }) => {
                shell.as_ref() == Some(string)
            }
            (Self::Pane { id }, ExtensionGrantMatch::PaneEquals { string }) => id == string,
            (
                Self::ForeignTab {
                    target_extension_id,
                    tab_type_id,
                },
                ExtensionGrantMatch::ForeignTabEquals { target, string },
            ) => target_extension_id == target && tab_type_id == string,
            (Self::Remote { action, .. }, ExtensionGrantMatch::RemoteActionEquals { string }) => {
                action == string
            }
            (Self::Git { operation, .. }, ExtensionGrantMatch::GitOperationEquals { string }) => {
                operation == string
            }
            (Self::File { operation, .. }, ExtensionGrantMatch::FileOperationEquals { string }) => {
                operation == string
            }
            (Self::Http { hostname, .. }, ExtensionGrantMatch::HostEquals { string }) => {
                hostname == string
            }
            (Self::TabCommand { command }, ExtensionGrantMatch::ShellExact { string }) => {
                command == string
            }
            (Self::Project { name, .. }, ExtensionGrantMatch::ProjectNameEquals { string }) => {
                name == string
            }
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsentRequest {
    pub id: String,
    pub extension_id: String,
    pub extension_display_name: String,
    pub verb: ExtensionGatedVerb,
    pub payload: ExtensionGatedPayload,
    pub payload_summary: String,
    pub payload_details: Vec<String>,
    pub suggested_match: ExtensionGrantMatch,
    pub source: String,
}

impl ConsentRequest {
    pub fn new(
        extension_id: impl Into<String>,
        extension_display_name: impl Into<String>,
        verb: ExtensionGatedVerb,
        payload: ExtensionGatedPayload,
        source: impl Into<String>,
    ) -> Self {
        let (payload_summary, payload_details) = describe(verb, &payload);
        let suggested_match = suggested_match(verb, &payload);
        Self {
            id: Uuid::new_v4().to_string().to_uppercase(),
            extension_id: extension_id.into(),
            extension_display_name: extension_display_name.into(),
            verb,
            payload,
            payload_summary,
            payload_details,
            suggested_match,
            source: source.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GrantEvaluation {
    Allow { rule_id: String },
    Deny { rule_id: String },
    Ask,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsentChoice {
    AllowOnce,
    AllowAndRemember,
    DenyOnce,
    DenyAndRemember,
    BlockKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConsentGate {
    Allow { audit: ExtensionAuditEntry },
    Deny { audit: ExtensionAuditEntry },
    Pending { request_id: String, active: bool },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsentResolution {
    pub request: ConsentRequest,
    pub decision: ExtensionGrantDecision,
    pub audit: ExtensionAuditEntry,
    pub rules_changed: bool,
    pub next_prompt: Option<ConsentRequest>,
}

#[derive(Clone, Debug)]
struct QueuedConsent {
    request: ConsentRequest,
}

#[derive(Clone, Debug, Default)]
pub struct ConsentCoordinator {
    pending: Option<QueuedConsent>,
    queued: VecDeque<QueuedConsent>,
    deadlines: HashMap<String, u64>,
}

impl ConsentCoordinator {
    pub fn pending_prompt(&self) -> Option<&ConsentRequest> {
        self.pending.as_ref().map(|pending| &pending.request)
    }

    pub fn queued_prompts(&self) -> impl Iterator<Item = &ConsentRequest> {
        self.queued.iter().map(|queued| &queued.request)
    }

    pub fn gate(
        &mut self,
        request: ConsentRequest,
        rules: &[ExtensionGrantRule],
        now_millis: u64,
        timestamp: &str,
    ) -> ConsentGate {
        match evaluate_grants(rules, &request) {
            GrantEvaluation::Allow { rule_id } => ConsentGate::Allow {
                audit: audit_entry(
                    &request,
                    ExtensionGrantDecision::Allow,
                    Some(rule_id),
                    timestamp,
                    None,
                ),
            },
            GrantEvaluation::Deny { rule_id } => ConsentGate::Deny {
                audit: audit_entry(
                    &request,
                    ExtensionGrantDecision::Deny,
                    Some(rule_id),
                    timestamp,
                    None,
                ),
            },
            GrantEvaluation::Ask => {
                if self.prompt_count(&request.extension_id) >= MAX_QUEUED_PROMPTS_PER_EXTENSION {
                    return ConsentGate::Deny {
                        audit: audit_entry(
                            &request,
                            ExtensionGrantDecision::Deny,
                            None,
                            timestamp,
                            Some("queue-flood"),
                        ),
                    };
                }
                let request_id = request.id.clone();
                let deadline_millis = now_millis
                    .saturating_add(PROMPT_TIMEOUT.as_millis().try_into().unwrap_or(u64::MAX));
                self.deadlines.insert(request_id.clone(), deadline_millis);
                let active = self.pending.is_none();
                let queued = QueuedConsent { request };
                if active {
                    self.pending = Some(queued);
                } else {
                    self.queued.push_back(queued);
                }
                ConsentGate::Pending { request_id, active }
            }
        }
    }

    pub fn respond(
        &mut self,
        request_id: &str,
        choice: ConsentChoice,
        rules: &mut Vec<ExtensionGrantRule>,
        timestamp: &str,
    ) -> Option<ConsentResolution> {
        let request = self.remove(request_id)?;
        let before = rules.clone();
        let (decision, rule_id) = apply_consent_choice(rules, &request, choice, timestamp);
        Some(ConsentResolution {
            audit: audit_entry(&request, decision, rule_id, timestamp, None),
            request,
            decision,
            rules_changed: *rules != before,
            next_prompt: self.pending_prompt().cloned(),
        })
    }

    pub fn cancel(&mut self, request_id: &str, timestamp: &str) -> Option<ConsentResolution> {
        self.resolve_without_rule(request_id, timestamp, "cancelled")
    }

    pub fn expire(&mut self, now_millis: u64, timestamp: &str) -> Vec<ConsentResolution> {
        let expired = self
            .deadlines
            .iter()
            .filter(|(_, deadline)| **deadline <= now_millis)
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        expired
            .into_iter()
            .filter_map(|request_id| self.resolve_without_rule(&request_id, timestamp, "timeout"))
            .collect()
    }

    pub fn cancel_extension(
        &mut self,
        extension_id: &str,
        timestamp: &str,
    ) -> Vec<ConsentResolution> {
        let ids = self
            .pending
            .iter()
            .chain(self.queued.iter())
            .filter(|queued| queued.request.extension_id == extension_id)
            .map(|queued| queued.request.id.clone())
            .collect::<Vec<_>>();
        ids.into_iter()
            .filter_map(|request_id| self.cancel(&request_id, timestamp))
            .collect()
    }

    pub fn cancel_all(&mut self, timestamp: &str) -> Vec<ConsentResolution> {
        let ids = self
            .pending
            .iter()
            .chain(self.queued.iter())
            .map(|queued| queued.request.id.clone())
            .collect::<Vec<_>>();
        ids.into_iter()
            .filter_map(|request_id| self.cancel(&request_id, timestamp))
            .collect()
    }

    fn prompt_count(&self, extension_id: &str) -> usize {
        self.pending
            .iter()
            .chain(self.queued.iter())
            .filter(|queued| queued.request.extension_id == extension_id)
            .count()
    }

    fn resolve_without_rule(
        &mut self,
        request_id: &str,
        timestamp: &str,
        reason: &str,
    ) -> Option<ConsentResolution> {
        let request = self.remove(request_id)?;
        Some(ConsentResolution {
            audit: audit_entry(
                &request,
                ExtensionGrantDecision::Deny,
                None,
                timestamp,
                Some(reason),
            ),
            request,
            decision: ExtensionGrantDecision::Deny,
            rules_changed: false,
            next_prompt: self.pending_prompt().cloned(),
        })
    }

    fn remove(&mut self, request_id: &str) -> Option<ConsentRequest> {
        self.deadlines.remove(request_id)?;
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.request.id == request_id)
        {
            let request = self.pending.take().map(|pending| pending.request)?;
            self.pending = self.queued.pop_front();
            return Some(request);
        }
        let index = self
            .queued
            .iter()
            .position(|queued| queued.request.id == request_id)?;
        self.queued.remove(index).map(|queued| queued.request)
    }
}

pub fn evaluate_grants(rules: &[ExtensionGrantRule], request: &ConsentRequest) -> GrantEvaluation {
    let mut winner: Option<&ExtensionGrantRule> = None;
    for candidate in rules.iter().filter(|rule| {
        rule.extension_id == request.extension_id
            && rule.verb == request.verb
            && request.payload.matches(&rule.match_rule)
    }) {
        if winner.is_none_or(|current| outranks(candidate, current)) {
            winner = Some(candidate);
        }
    }
    match winner {
        Some(rule) if rule.decision == ExtensionGrantDecision::Allow => GrantEvaluation::Allow {
            rule_id: rule.id.clone(),
        },
        Some(rule) => GrantEvaluation::Deny {
            rule_id: rule.id.clone(),
        },
        None => GrantEvaluation::Ask,
    }
}

pub fn apply_consent_choice(
    rules: &mut Vec<ExtensionGrantRule>,
    request: &ConsentRequest,
    choice: ConsentChoice,
    timestamp: &str,
) -> (ExtensionGrantDecision, Option<String>) {
    match choice {
        ConsentChoice::AllowOnce => (ExtensionGrantDecision::Allow, None),
        ConsentChoice::DenyOnce => (ExtensionGrantDecision::Deny, None),
        ConsentChoice::AllowAndRemember => {
            let rule = new_rule(request, ExtensionGrantDecision::Allow, timestamp);
            let id = rule.id.clone();
            replace_matching_rule(rules, rule);
            (ExtensionGrantDecision::Allow, Some(id))
        }
        ConsentChoice::DenyAndRemember => {
            let rule = new_rule(request, ExtensionGrantDecision::Deny, timestamp);
            let id = rule.id.clone();
            replace_matching_rule(rules, rule);
            (ExtensionGrantDecision::Deny, Some(id))
        }
        ConsentChoice::BlockKind => {
            rules.retain(|rule| {
                rule.extension_id != request.extension_id || rule.verb != request.verb
            });
            let rule = ExtensionGrantRule {
                id: Uuid::new_v4().to_string().to_uppercase(),
                extension_id: request.extension_id.clone(),
                verb: request.verb,
                match_rule: ExtensionGrantMatch::Any,
                decision: ExtensionGrantDecision::Blocked,
                created_at: timestamp.to_owned(),
            };
            let id = rule.id.clone();
            rules.push(rule);
            (ExtensionGrantDecision::Blocked, Some(id))
        }
    }
}

pub fn audit_entry(
    request: &ConsentRequest,
    decision: ExtensionGrantDecision,
    rule_id: Option<String>,
    timestamp: &str,
    reason: Option<&str>,
) -> ExtensionAuditEntry {
    let decision = match decision {
        ExtensionGrantDecision::Allow => "allow",
        ExtensionGrantDecision::Deny => "deny",
        ExtensionGrantDecision::Blocked => "blocked",
    };
    let payload_summary = reason
        .map(|reason| format!("{} [{reason}]", request.payload_summary))
        .unwrap_or_else(|| request.payload_summary.clone());
    ExtensionAuditEntry {
        timestamp: timestamp.to_owned(),
        extension_id: request.extension_id.clone(),
        verb: gated_verb_name(request.verb).to_owned(),
        payload_summary,
        decision: decision.to_owned(),
        rule_id,
        source: request.source.clone(),
    }
}

pub fn consent_request(
    request: &ExtensionApiRequest,
    extension_display_name: &str,
) -> Result<Option<ConsentRequest>, ExtensionApiError> {
    consent_request_with_host_lookup(request, extension_display_name, &system_host_lookup)
}

pub fn consent_request_with_host_lookup(
    request: &ExtensionApiRequest,
    extension_display_name: &str,
    host_lookup: &impl Fn(&str) -> Option<Vec<IpAddr>>,
) -> Result<Option<ConsentRequest>, ExtensionApiError> {
    let args = &request.args;
    let source = request.source.audit_source();
    let gated = match request.method.as_str() {
        "exec" => Some((
            ExtensionGatedVerb::Exec,
            exec_payload(args)?,
            source.to_owned(),
        )),
        "panes.send" => Some((
            ExtensionGatedVerb::PanesSend,
            ExtensionGatedPayload::Pane {
                id: required_string(args.get("paneID"), "panes.send requires paneID")?,
            },
            source.to_owned(),
        )),
        "panes.sendKeys" => Some((
            ExtensionGatedVerb::PanesSendKeys,
            ExtensionGatedPayload::Pane {
                id: required_string(args.get("paneID"), "panes.sendKeys requires paneID")?,
            },
            source.to_owned(),
        )),
        "panes.readScreen" => Some((
            ExtensionGatedVerb::PanesReadScreen,
            ExtensionGatedPayload::Pane {
                id: required_string(args.get("paneID"), "panes.readScreen requires paneID")?,
            },
            source.to_owned(),
        )),
        "tabs.open" => tab_consent(request)?,
        "projects.delete" => {
            let name = required_string(
                args.get("name").or_else(|| args.get("identifier")),
                "projects.delete requires identifier",
            )?;
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            Some((
                ExtensionGatedVerb::ProjectsDelete,
                ExtensionGatedPayload::Project { name, path },
                source.to_owned(),
            ))
        }
        "projects.create"
            if args
                .get("createIfMissing")
                .and_then(Value::as_bool)
                .unwrap_or(false) =>
        {
            let path = required_string(args.get("path"), "projects.create requires path")?;
            (!Path::new(&path).exists()).then_some((
                ExtensionGatedVerb::FilesWrite,
                ExtensionGatedPayload::File {
                    operation: "mkdir".to_owned(),
                    path,
                },
                source.to_owned(),
            ))
        }
        "http.fetch" => Some(http_consent(args, host_lookup)?),
        method if required_permission(method) == Some(ExtensionPermission::GitWrite) => {
            let operation = method.strip_prefix("git.").unwrap_or(method).to_owned();
            let repo_path = args
                .get("repoPath")
                .or_else(|| args.get("project"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            Some((
                ExtensionGatedVerb::GitWrite,
                ExtensionGatedPayload::Git {
                    operation,
                    repo_path,
                },
                source.to_owned(),
            ))
        }
        method if required_permission(method) == Some(ExtensionPermission::FilesWrite) => {
            let operation = method.strip_prefix("files.").unwrap_or(method).to_owned();
            let path = match method {
                "files.move" => args.get("into").and_then(Value::as_str).unwrap_or_default(),
                "files.delete" => args
                    .get("paths")
                    .and_then(Value::as_array)
                    .and_then(|paths| paths.first())
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                _ => args.get("path").and_then(Value::as_str).unwrap_or_default(),
            }
            .to_owned();
            Some((
                ExtensionGatedVerb::FilesWrite,
                ExtensionGatedPayload::File { operation, path },
                source.to_owned(),
            ))
        }
        _ => None,
    };
    Ok(gated.map(|(verb, payload, source)| {
        ConsentRequest::new(
            request.extension_id.clone(),
            extension_display_name,
            verb,
            payload,
            source,
        )
    }))
}

pub fn now_timestamp() -> String {
    let now = OffsetDateTime::now_utc();
    now.format(&Rfc3339)
        .unwrap_or_else(|_| now.unix_timestamp().to_string())
}

fn exec_payload(
    args: &serde_json::Map<String, Value>,
) -> Result<ExtensionGatedPayload, ExtensionApiError> {
    let argv = args
        .get("argv")
        .map(|value| {
            value
                .as_array()
                .ok_or_else(|| {
                    ExtensionApiError::InvalidArguments("exec argv must be an array".to_owned())
                })?
                .iter()
                .map(|item| {
                    item.as_str().map(str::to_owned).ok_or_else(|| {
                        ExtensionApiError::InvalidArguments(
                            "exec argv values must be strings".to_owned(),
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    let shell = args
        .get("shell")
        .map(|value| {
            value.as_str().map(str::to_owned).ok_or_else(|| {
                ExtensionApiError::InvalidArguments("exec shell must be a string".to_owned())
            })
        })
        .transpose()?;
    match (&argv, &shell) {
        (None, None) => Err(ExtensionApiError::InvalidArguments(
            "exec requires argv or shell".to_owned(),
        )),
        (Some(_), Some(_)) => Err(ExtensionApiError::InvalidArguments(
            "exec accepts either argv or shell, not both".to_owned(),
        )),
        (Some(argv), None) if argv.is_empty() => Err(ExtensionApiError::InvalidArguments(
            "exec argv must be non-empty".to_owned(),
        )),
        _ => Ok(ExtensionGatedPayload::Exec { argv, shell }),
    }
}

fn tab_consent(
    request: &ExtensionApiRequest,
) -> Result<Option<(ExtensionGatedVerb, ExtensionGatedPayload, String)>, ExtensionApiError> {
    let kind = request
        .args
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if kind == "terminal" {
        let command = request
            .args
            .get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        return Ok((!command.is_empty()).then(|| {
            (
                ExtensionGatedVerb::TabsRunCommand,
                ExtensionGatedPayload::TabCommand {
                    command: command.to_owned(),
                },
                request.source.audit_source().to_owned(),
            )
        }));
    }
    if kind == "extensionWebView" {
        let payload = request
            .args
            .get("extension")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                ExtensionApiError::InvalidArguments(
                    "extensionWebView tabs require extension payload".to_owned(),
                )
            })?;
        let target = required_string(payload.get("id"), "extension tab requires id")?;
        if target == request.extension_id {
            return Ok(None);
        }
        let tab_type = required_string(payload.get("tabType"), "extension tab requires tabType")?;
        return Ok(Some((
            ExtensionGatedVerb::TabsOpenForeign,
            ExtensionGatedPayload::ForeignTab {
                target_extension_id: target,
                tab_type_id: tab_type,
            },
            request.source.audit_source().to_owned(),
        )));
    }
    Ok(None)
}

fn http_consent(
    args: &serde_json::Map<String, Value>,
    host_lookup: &impl Fn(&str) -> Option<Vec<IpAddr>>,
) -> Result<(ExtensionGatedVerb, ExtensionGatedPayload, String), ExtensionApiError> {
    let raw_url = required_string(args.get("url"), "http requires a url")?;
    let url = Url::parse(raw_url.trim())
        .map_err(|_| ExtensionApiError::InvalidArguments("http: invalid URL".to_owned()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ExtensionApiError::InvalidArguments(
            "http: only http and https URLs are allowed".to_owned(),
        ));
    }
    let hostname = url
        .host_str()
        .filter(|host| !host.is_empty())
        .ok_or_else(|| ExtensionApiError::InvalidArguments("http: URL has no host".to_owned()))?
        .to_lowercase();
    let method = args
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .trim()
        .to_uppercase();
    let method = if method.is_empty() {
        "GET".to_owned()
    } else {
        method
    };
    if !["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"].contains(&method.as_str()) {
        return Err(ExtensionApiError::InvalidArguments(format!(
            "http: unsupported method '{method}'"
        )));
    }
    if host_is_blocked(&hostname, host_lookup) {
        return Err(ExtensionApiError::BlockedHttpHost(hostname));
    }
    Ok((
        ExtensionGatedVerb::HttpFetch,
        ExtensionGatedPayload::Http {
            hostname,
            method,
            url: raw_url,
        },
        "http".to_owned(),
    ))
}

fn host_is_blocked(host: &str, host_lookup: &impl Fn(&str) -> Option<Vec<IpAddr>>) -> bool {
    let normalized = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_lowercase();
    if normalized.is_empty()
        || normalized == "localhost"
        || normalized.ends_with(".localhost")
        || normalized.ends_with(".local")
    {
        return true;
    }
    let Some(addresses) = host_lookup(&normalized) else {
        return true;
    };
    addresses.is_empty() || addresses.into_iter().any(private_address)
}

fn system_host_lookup(host: &str) -> Option<Vec<IpAddr>> {
    (host, 0)
        .to_socket_addrs()
        .ok()
        .map(|addresses| addresses.map(|address| address.ip()).collect())
}

fn private_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let [first, second, _, _] = address.octets();
            first == 0
                || first == 10
                || first == 127
                || (first == 169 && second == 254)
                || (first == 172 && (16..=31).contains(&second))
                || (first == 192 && second == 168)
        }
        IpAddr::V6(address) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                return private_address(IpAddr::V4(mapped));
            }
            let first = address.segments()[0];
            address.is_unspecified()
                || address.is_loopback()
                || (first & 0xffc0) == 0xfe80
                || (first & 0xfe00) == 0xfc00
        }
    }
}

fn outranks(candidate: &ExtensionGrantRule, current: &ExtensionGrantRule) -> bool {
    let candidate_specificity = candidate.match_rule.specificity();
    let current_specificity = current.match_rule.specificity();
    if candidate_specificity != current_specificity {
        return candidate_specificity > current_specificity;
    }
    let candidate_denies = candidate.decision != ExtensionGrantDecision::Allow;
    let current_denies = current.decision != ExtensionGrantDecision::Allow;
    if candidate_denies != current_denies {
        return candidate_denies;
    }
    candidate.created_at < current.created_at
}

fn new_rule(
    request: &ConsentRequest,
    decision: ExtensionGrantDecision,
    timestamp: &str,
) -> ExtensionGrantRule {
    ExtensionGrantRule {
        id: Uuid::new_v4().to_string().to_uppercase(),
        extension_id: request.extension_id.clone(),
        verb: request.verb,
        match_rule: request.suggested_match.clone(),
        decision,
        created_at: timestamp.to_owned(),
    }
}

fn replace_matching_rule(rules: &mut Vec<ExtensionGrantRule>, rule: ExtensionGrantRule) {
    rules.retain(|existing| {
        existing.extension_id != rule.extension_id
            || existing.verb != rule.verb
            || existing.match_rule != rule.match_rule
    });
    rules.push(rule);
}

fn required_string(value: Option<&Value>, message: &str) -> Result<String, ExtensionApiError> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ExtensionApiError::InvalidArguments(message.to_owned()))
}

fn suggested_match(
    verb: ExtensionGatedVerb,
    payload: &ExtensionGatedPayload,
) -> ExtensionGrantMatch {
    match (verb, payload) {
        (ExtensionGatedVerb::Exec, ExtensionGatedPayload::Exec { argv, shell }) => argv
            .as_ref()
            .and_then(|argv| argv.first())
            .map(|base| ExtensionGrantMatch::ArgvPrefix {
                value: vec![base.clone()],
            })
            .or_else(|| {
                shell.as_ref().map(|shell| ExtensionGrantMatch::ShellExact {
                    string: shell.clone(),
                })
            })
            .unwrap_or(ExtensionGrantMatch::Any),
        (ExtensionGatedVerb::RemoteInvoke, ExtensionGatedPayload::Remote { action, .. }) => {
            ExtensionGrantMatch::RemoteActionEquals {
                string: action.clone(),
            }
        }
        (ExtensionGatedVerb::GitWrite, ExtensionGatedPayload::Git { operation, .. }) => {
            ExtensionGrantMatch::GitOperationEquals {
                string: operation.clone(),
            }
        }
        (ExtensionGatedVerb::FilesWrite, ExtensionGatedPayload::File { operation, .. }) => {
            ExtensionGrantMatch::FileOperationEquals {
                string: operation.clone(),
            }
        }
        (ExtensionGatedVerb::ProjectsDelete, ExtensionGatedPayload::Project { name, .. }) => {
            ExtensionGrantMatch::ProjectNameEquals {
                string: name.clone(),
            }
        }
        (ExtensionGatedVerb::HttpFetch, ExtensionGatedPayload::Http { hostname, .. }) => {
            ExtensionGrantMatch::HostEquals {
                string: hostname.clone(),
            }
        }
        (ExtensionGatedVerb::TabsRunCommand, ExtensionGatedPayload::TabCommand { command }) => {
            ExtensionGrantMatch::ShellExact {
                string: command.clone(),
            }
        }
        _ => ExtensionGrantMatch::Any,
    }
}

fn describe(verb: ExtensionGatedVerb, payload: &ExtensionGatedPayload) -> (String, Vec<String>) {
    match (verb, payload) {
        (
            ExtensionGatedVerb::Exec,
            ExtensionGatedPayload::Exec {
                argv: Some(argv), ..
            },
        ) => {
            let joined = argv.join(" ");
            (joined.clone(), vec![format!("argv: {joined}")])
        }
        (
            ExtensionGatedVerb::Exec,
            ExtensionGatedPayload::Exec {
                shell: Some(shell), ..
            },
        ) => ("sh -c …".to_owned(), vec![format!("shell: {shell}")]),
        (ExtensionGatedVerb::PanesSend, ExtensionGatedPayload::Pane { id }) => {
            (format!("send to pane {id}"), vec![format!("pane: {id}")])
        }
        (ExtensionGatedVerb::PanesSendKeys, ExtensionGatedPayload::Pane { id }) => (
            format!("send-keys to pane {id}"),
            vec![format!("pane: {id}")],
        ),
        (ExtensionGatedVerb::PanesReadScreen, ExtensionGatedPayload::Pane { id }) => (
            format!("read screen of pane {id}"),
            vec![format!("pane: {id}")],
        ),
        (
            ExtensionGatedVerb::TabsOpenForeign,
            ExtensionGatedPayload::ForeignTab {
                target_extension_id,
                tab_type_id,
            },
        ) => (
            format!("open {target_extension_id} tab {tab_type_id}"),
            vec![
                format!("extension: {target_extension_id}"),
                format!("tab type: {tab_type_id}"),
            ],
        ),
        (
            ExtensionGatedVerb::RemoteInvoke,
            ExtensionGatedPayload::Remote {
                action,
                device_name,
            },
        ) => (
            format!("{device_name} calls {action}"),
            vec![
                format!("device: {device_name}"),
                format!("action: {action}"),
            ],
        ),
        (
            ExtensionGatedVerb::GitWrite,
            ExtensionGatedPayload::Git {
                operation,
                repo_path,
            },
        ) => (
            format!("git {operation}"),
            vec![
                format!("operation: {operation}"),
                format!("repo: {repo_path}"),
            ],
        ),
        (ExtensionGatedVerb::FilesWrite, ExtensionGatedPayload::File { operation, path }) => (
            format!("file {operation}"),
            vec![format!("operation: {operation}"), format!("path: {path}")],
        ),
        (
            ExtensionGatedVerb::HttpFetch,
            ExtensionGatedPayload::Http {
                hostname,
                method,
                url,
            },
        ) => (
            format!("fetch from {hostname}"),
            vec![
                format!("host: {hostname}"),
                format!("method: {method}"),
                format!("url: {url}"),
            ],
        ),
        (ExtensionGatedVerb::TabsRunCommand, ExtensionGatedPayload::TabCommand { command }) => {
            (command.clone(), vec![format!("command: {command}")])
        }
        (ExtensionGatedVerb::ProjectsDelete, ExtensionGatedPayload::Project { name, path }) => (
            format!("delete project {name}"),
            vec![format!("project: {name}"), format!("path: {path}")],
        ),
        _ => ("(unknown)".to_owned(), Vec::new()),
    }
}

pub fn gated_verb_name(verb: ExtensionGatedVerb) -> &'static str {
    match verb {
        ExtensionGatedVerb::Exec => "exec",
        ExtensionGatedVerb::PanesSend => "panes.send",
        ExtensionGatedVerb::PanesSendKeys => "panes.sendKeys",
        ExtensionGatedVerb::PanesReadScreen => "panes.readScreen",
        ExtensionGatedVerb::TabsOpenForeign => "tabs.openForeign",
        ExtensionGatedVerb::TabsRunCommand => "tabs.runCommand",
        ExtensionGatedVerb::RemoteInvoke => "remote.invoke",
        ExtensionGatedVerb::GitWrite => "git.write",
        ExtensionGatedVerb::FilesWrite => "files.write",
        ExtensionGatedVerb::HttpFetch => "http.fetch",
        ExtensionGatedVerb::ProjectsDelete => "projects.delete",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::net::{Ipv4Addr, Ipv6Addr};

    use serde_json::{Map, json};

    use super::*;
    use crate::extensions::ExtensionApiSource;

    fn request(method: &str, args: Value) -> ExtensionApiRequest {
        ExtensionApiRequest::new(
            "git",
            method,
            args.as_object().cloned().unwrap_or_else(Map::new),
            ExtensionApiSource::Webview,
            BTreeSet::new(),
        )
    }

    fn consent(method: &str, args: Value) -> ConsentRequest {
        consent_request_with_host_lookup(&request(method, args), "Git", &|_| {
            Some(vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))])
        })
        .unwrap()
        .unwrap()
    }

    fn rule(
        id: &str,
        match_rule: ExtensionGrantMatch,
        decision: ExtensionGrantDecision,
        created_at: &str,
    ) -> ExtensionGrantRule {
        ExtensionGrantRule {
            id: id.to_owned(),
            extension_id: "git".to_owned(),
            verb: ExtensionGatedVerb::Exec,
            match_rule,
            decision,
            created_at: created_at.to_owned(),
        }
    }

    #[test]
    fn gated_payloads_and_default_matches_follow_swift() {
        let exec = consent("exec", json!({"argv": ["git", "status"]}));
        assert_eq!(exec.payload_summary, "git status");
        assert_eq!(
            exec.suggested_match,
            ExtensionGrantMatch::ArgvPrefix {
                value: vec!["git".to_owned()]
            }
        );
        let shell = consent("exec", json!({"shell": "git status"}));
        assert_eq!(
            shell.suggested_match,
            ExtensionGrantMatch::ShellExact {
                string: "git status".to_owned()
            }
        );
        let git = consent("git.pr.merge", json!({"project": "muxy"}));
        assert_eq!(git.verb, ExtensionGatedVerb::GitWrite);
        assert_eq!(
            git.suggested_match,
            ExtensionGrantMatch::GitOperationEquals {
                string: "pr.merge".to_owned()
            }
        );
        let moved = consent("files.move", json!({"paths": ["a"], "into": "b"}));
        assert!(matches!(
            moved.payload,
            ExtensionGatedPayload::File { path, .. } if path == "b"
        ));
    }

    #[test]
    fn grant_winner_uses_specificity_deny_precedence_and_oldest_rule() {
        let request = consent("exec", json!({"argv": ["git", "status"]}));
        let rules = vec![
            rule(
                "any-deny",
                ExtensionGrantMatch::Any,
                ExtensionGrantDecision::Deny,
                "2026-01-01T00:00:00Z",
            ),
            rule(
                "prefix-allow-new",
                ExtensionGrantMatch::ArgvPrefix {
                    value: vec!["git".to_owned()],
                },
                ExtensionGrantDecision::Allow,
                "2026-01-03T00:00:00Z",
            ),
            rule(
                "prefix-deny",
                ExtensionGrantMatch::ArgvPrefix {
                    value: vec!["git".to_owned()],
                },
                ExtensionGrantDecision::Deny,
                "2026-01-02T00:00:00Z",
            ),
            rule(
                "exact-allow-old",
                ExtensionGrantMatch::ArgvExact {
                    value: vec!["git".to_owned(), "status".to_owned()],
                },
                ExtensionGrantDecision::Allow,
                "2026-01-01T00:00:00Z",
            ),
            rule(
                "exact-allow-new",
                ExtensionGrantMatch::ArgvExact {
                    value: vec!["git".to_owned(), "status".to_owned()],
                },
                ExtensionGrantDecision::Allow,
                "2026-01-04T00:00:00Z",
            ),
        ];
        assert_eq!(
            evaluate_grants(&rules, &request),
            GrantEvaluation::Allow {
                rule_id: "exact-allow-old".to_owned()
            }
        );
        let tied = &rules[..3];
        assert_eq!(
            evaluate_grants(tied, &request),
            GrantEvaluation::Deny {
                rule_id: "prefix-deny".to_owned()
            }
        );
    }

    #[test]
    fn remembered_and_block_choices_replace_the_right_rules() {
        let request = consent("exec", json!({"argv": ["git", "status"]}));
        let mut rules = Vec::new();
        let (_, first) = apply_consent_choice(
            &mut rules,
            &request,
            ConsentChoice::AllowAndRemember,
            "2026-01-01T00:00:00Z",
        );
        assert_eq!(rules.len(), 1);
        let (_, second) = apply_consent_choice(
            &mut rules,
            &request,
            ConsentChoice::DenyAndRemember,
            "2026-01-02T00:00:00Z",
        );
        assert_eq!(rules.len(), 1);
        assert_ne!(first, second);
        let (decision, blocked) = apply_consent_choice(
            &mut rules,
            &request,
            ConsentChoice::BlockKind,
            "2026-01-03T00:00:00Z",
        );
        assert_eq!(decision, ExtensionGrantDecision::Blocked);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].match_rule, ExtensionGrantMatch::Any);
        assert_eq!(rules[0].id, blocked.unwrap());
    }

    #[test]
    fn coordinator_handles_queue_flood_response_timeout_and_cancellation() {
        let mut coordinator = ConsentCoordinator::default();
        let rules = Vec::new();
        let timestamp = "2026-01-01T00:00:00Z";
        let mut ids = Vec::new();
        for index in 0..MAX_QUEUED_PROMPTS_PER_EXTENSION {
            let mut request = consent("exec", json!({"argv": [format!("cmd-{index}")]}));
            request.id = format!("request-{index}");
            match coordinator.gate(request, &rules, 1_000, timestamp) {
                ConsentGate::Pending { request_id, active } => {
                    assert_eq!(active, index == 0);
                    ids.push(request_id);
                }
                result => panic!("unexpected gate result: {result:?}"),
            }
        }
        let mut flooded = consent("exec", json!({"argv": ["overflow"]}));
        flooded.id = "overflow".to_owned();
        match coordinator.gate(flooded, &rules, 1_000, timestamp) {
            ConsentGate::Deny { audit } => {
                assert!(audit.payload_summary.ends_with("[queue-flood]"));
            }
            result => panic!("unexpected gate result: {result:?}"),
        }
        let mut mutable_rules = Vec::new();
        let resolved = coordinator
            .respond(
                &ids[0],
                ConsentChoice::AllowAndRemember,
                &mut mutable_rules,
                timestamp,
            )
            .unwrap();
        assert_eq!(resolved.decision, ExtensionGrantDecision::Allow);
        assert!(resolved.rules_changed);
        assert_eq!(resolved.next_prompt.unwrap().id, ids[1]);
        let cancelled = coordinator.cancel(&ids[1], timestamp).unwrap();
        assert!(cancelled.audit.payload_summary.ends_with("[cancelled]"));
        let expired = coordinator.expire(61_000, timestamp);
        assert_eq!(expired.len(), 3);
        assert!(
            expired
                .iter()
                .all(|resolution| resolution.audit.payload_summary.ends_with("[timeout]"))
        );
        assert!(coordinator.pending_prompt().is_none());
    }

    #[test]
    fn cancel_all_resolves_active_and_queued_prompts() {
        let mut coordinator = ConsentCoordinator::default();
        let timestamp = "2026-01-01T00:00:00Z";
        for (extension_id, command) in [("git", "status"), ("files", "list")] {
            let mut request = consent("exec", json!({"argv": [command]}));
            request.extension_id = extension_id.to_owned();
            assert!(matches!(
                coordinator.gate(request, &[], 1_000, timestamp),
                ConsentGate::Pending { .. }
            ));
        }
        let resolutions = coordinator.cancel_all(timestamp);
        assert_eq!(resolutions.len(), 2);
        assert!(resolutions.iter().all(|resolution| {
            resolution.decision == ExtensionGrantDecision::Deny
                && resolution.audit.payload_summary.ends_with("[cancelled]")
        }));
        assert!(coordinator.pending_prompt().is_none());
        assert_eq!(coordinator.queued_prompts().count(), 0);
        assert!(coordinator.cancel_all(timestamp).is_empty());
    }

    #[test]
    fn http_policy_blocks_private_loopback_mapped_and_resolution_failures() {
        let cases = [
            IpAddr::V4(Ipv4Addr::new(0, 1, 2, 3)),
            IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3)),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(172, 16, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            IpAddr::V6("fe80::1".parse().unwrap()),
            IpAddr::V6("fc00::1".parse().unwrap()),
            IpAddr::V6("::ffff:127.0.0.1".parse().unwrap()),
        ];
        for address in cases {
            let error = consent_request_with_host_lookup(
                &request("http.fetch", json!({"url": "https://example.com"})),
                "Git",
                &|_| Some(vec![address]),
            )
            .unwrap_err();
            assert_eq!(
                error,
                ExtensionApiError::BlockedHttpHost("example.com".to_owned())
            );
        }
        for host in ["localhost", "api.localhost", "service.local"] {
            let error = consent_request_with_host_lookup(
                &request("http.fetch", json!({"url": format!("https://{host}/")})),
                "Git",
                &|_| Some(vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))]),
            )
            .unwrap_err();
            assert_eq!(error, ExtensionApiError::BlockedHttpHost(host.to_owned()));
        }
        assert!(matches!(
            consent_request_with_host_lookup(
                &request("http.fetch", json!({"url": "https://unknown.invalid"})),
                "Git",
                &|_| None,
            ),
            Err(ExtensionApiError::BlockedHttpHost(_))
        ));
    }

    #[test]
    fn http_consent_normalizes_method_and_uses_http_audit_source() {
        let request = consent(
            "http.fetch",
            json!({
                "url": "https://Example.COM/path",
                "method": " post "
            }),
        );
        assert_eq!(request.source, "http");
        assert_eq!(
            request.suggested_match,
            ExtensionGrantMatch::HostEquals {
                string: "example.com".to_owned()
            }
        );
        assert!(matches!(
            request.payload,
            ExtensionGatedPayload::Http { method, .. } if method == "POST"
        ));
    }
}
