pub(crate) mod api;
pub(crate) mod consent_view;
mod process;
pub(crate) mod surface_view;
pub(crate) mod surfaces;
pub(crate) mod webview;

use muxy_api::extensions::{
    ConsentChoice, ConsentCoordinator, ConsentGate, ConsentRequest, ConsentResolution,
    ExtensionApiRequest, now_timestamp,
};
use muxy_core::extensions::logs::{ExtensionAuditLog, ExtensionLogStore};
use muxy_core::extensions::manifest::ExtensionIcon;
use muxy_core::extensions::runtime::{
    ExtensionHostLifecycle, ExtensionRuntimeCatalog, ExtensionRuntimeError,
    ExtensionRuntimeSnapshot, ExtensionRuntimeStatus, HostLifecycleAction, HostTermination,
    RuntimeSubscriptionAccess,
};
use muxy_core::extensions::storage::ExtensionStorage;
use muxy_proto::extension::{
    ExtensionBroadcast, ExtensionLocalEvent, InvokeRequest, ModalQuery, ModalResult,
};
use muxy_proto::server::{
    ExtensionLocalEventIngress, ExtensionSnapshot, ExtensionSnapshotEntry, SocketServerHandle,
    SubscriptionAccess,
};
use process::{HostLauncher, HostProcessExit, RunningHost, SystemHostLauncher};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExtensionSurfaceEvent {
    LocalEvent(ExtensionLocalEventIngress),
    CloseAll { extension_id: String },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExtensionItemOverride {
    pub icon: Option<ExtensionIcon>,
    pub text: Option<String>,
    pub visible: Option<bool>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExtensionStatusBarUpdate {
    pub icon: Option<ExtensionIcon>,
    pub text: Option<String>,
    pub clear_text: bool,
    pub visible: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExtensionTopbarBinding {
    pub extension_id: String,
    pub item_id: String,
    pub resource_root: std::path::PathBuf,
    pub icon: ExtensionIcon,
    pub tooltip: Option<String>,
    pub command: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExtensionStatusBarBinding {
    pub extension_id: String,
    pub item_id: String,
    pub resource_root: std::path::PathBuf,
    pub icon: ExtensionIcon,
    pub text: Option<String>,
    pub tooltip: Option<String>,
    pub side: muxy_core::extensions::manifest::StatusBarSide,
    pub command: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExtensionShortcutSource {
    Manifest,
    Runtime,
}

impl ExtensionShortcutSource {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Manifest => "manifest",
            Self::Runtime => "runtime",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExtensionShortcutBinding {
    pub extension_id: String,
    pub command_id: String,
    pub combo: muxy_core::shortcuts::KeyCombo,
    pub source: ExtensionShortcutSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExtensionRestartRequest {
    pub extension_id: String,
    pub delay: Duration,
}

pub(crate) type ExtensionApiCompletion =
    Box<dyn FnOnce(Result<serde_json::Value, muxy_api::extensions::ExtensionApiError>)>;

pub(crate) struct PendingExtensionApiCall {
    pub request: ExtensionApiRequest,
    completion: Option<ExtensionApiCompletion>,
}

impl PendingExtensionApiCall {
    pub(crate) fn complete(
        mut self,
        result: Result<serde_json::Value, muxy_api::extensions::ExtensionApiError>,
    ) {
        if let Some(completion) = self.completion.take() {
            completion(result);
        }
    }

    pub(crate) fn into_parts(mut self) -> (ExtensionApiRequest, ExtensionApiCompletion) {
        (self.request, self.completion.take().unwrap())
    }
}

#[derive(Default)]
pub(crate) struct PendingExtensionApiCalls {
    calls: HashMap<String, PendingExtensionApiCall>,
}

impl PendingExtensionApiCalls {
    pub(crate) fn insert(
        &mut self,
        request_id: String,
        request: ExtensionApiRequest,
        completion: ExtensionApiCompletion,
    ) {
        if let Some(call) = self.calls.insert(
            request_id,
            PendingExtensionApiCall {
                request,
                completion: Some(Box::new(completion)),
            },
        ) {
            let method = call.request.method.clone();
            call.complete(Err(muxy_api::extensions::ExtensionApiError::Service(
                format!("duplicate pending request for {method}"),
            )));
        }
    }

    pub(crate) fn remove(&mut self, request_id: &str) -> Option<PendingExtensionApiCall> {
        self.calls.remove(request_id)
    }

    pub(crate) fn extension_ids(&self) -> std::collections::BTreeSet<String> {
        self.calls
            .values()
            .map(|call| call.request.extension_id.clone())
            .collect()
    }

    pub(crate) fn drain(&mut self) -> Vec<PendingExtensionApiCall> {
        self.calls.drain().map(|(_, call)| call).collect()
    }
}

struct ManagedHost {
    generation: u64,
    process: Box<dyn RunningHost>,
}

struct ExtensionSocket {
    handle: SocketServerHandle,
    path: PathBuf,
}

pub struct ExtensionRuntime {
    catalog: ExtensionRuntimeCatalog,
    socket: Option<ExtensionSocket>,
    launcher: Box<dyn HostLauncher>,
    hosts: BTreeMap<String, ManagedHost>,
    lifecycles: BTreeMap<String, ExtensionHostLifecycle>,
    host_event_sender: async_channel::Sender<HostProcessExit>,
    host_event_receiver: async_channel::Receiver<HostProcessExit>,
    surface_events: VecDeque<ExtensionSurfaceEvent>,
    logs: ExtensionLogStore,
    audit: ExtensionAuditLog,
    storage: ExtensionStorage,
    consent: ConsentCoordinator,
    topbar_overrides: BTreeMap<String, BTreeMap<String, ExtensionItemOverride>>,
    status_bar_overrides: BTreeMap<String, BTreeMap<String, ExtensionItemOverride>>,
    runtime_shortcuts: BTreeMap<String, BTreeMap<String, muxy_core::shortcuts::KeyCombo>>,
}

impl ExtensionRuntime {
    pub fn load(
        paths: muxy_core::extensions::paths::ExtensionPaths,
    ) -> Result<Self, ExtensionRuntimeError> {
        Self::load_with_launcher(paths, Box::new(SystemHostLauncher::resolve()))
    }

    fn load_with_launcher(
        paths: muxy_core::extensions::paths::ExtensionPaths,
        launcher: Box<dyn HostLauncher>,
    ) -> Result<Self, ExtensionRuntimeError> {
        let catalog = ExtensionRuntimeCatalog::load(paths)?;
        let logs = ExtensionLogStore::new(catalog.paths());
        let audit = ExtensionAuditLog::new(catalog.paths());
        let storage = ExtensionStorage::new(catalog.paths());
        let (host_event_sender, host_event_receiver) = async_channel::unbounded();
        Ok(Self {
            catalog,
            socket: None,
            launcher,
            hosts: BTreeMap::new(),
            lifecycles: BTreeMap::new(),
            host_event_sender,
            host_event_receiver,
            surface_events: VecDeque::new(),
            logs,
            audit,
            storage,
            consent: ConsentCoordinator::default(),
            topbar_overrides: BTreeMap::new(),
            status_bar_overrides: BTreeMap::new(),
            runtime_shortcuts: BTreeMap::new(),
        })
    }

    pub fn socket_snapshot(&self) -> ExtensionSnapshot {
        socket_snapshot(self.catalog.snapshot())
    }

    pub fn catalog(&self) -> &ExtensionRuntimeCatalog {
        &self.catalog
    }

    pub fn catalog_mut(&mut self) -> &mut ExtensionRuntimeCatalog {
        &mut self.catalog
    }

    pub fn storage(&self) -> &ExtensionStorage {
        &self.storage
    }

    pub fn audit(&self) -> &ExtensionAuditLog {
        &self.audit
    }

    pub fn topbar_overrides(&self) -> &BTreeMap<String, BTreeMap<String, ExtensionItemOverride>> {
        &self.topbar_overrides
    }

    pub fn status_bar_overrides(
        &self,
    ) -> &BTreeMap<String, BTreeMap<String, ExtensionItemOverride>> {
        &self.status_bar_overrides
    }

    pub(crate) fn topbar_bindings(&self) -> Vec<ExtensionTopbarBinding> {
        self.catalog
            .records()
            .iter()
            .filter(|(_, record)| record.enabled)
            .flat_map(|(extension_id, record)| {
                record
                    .extension
                    .manifest
                    .topbar_items
                    .iter()
                    .filter_map(move |item| {
                        let override_item = self
                            .topbar_overrides
                            .get(extension_id)
                            .and_then(|items| items.get(&item.id));
                        if override_item
                            .and_then(|item| item.visible)
                            .unwrap_or(item.visible)
                        {
                            Some(ExtensionTopbarBinding {
                                extension_id: extension_id.clone(),
                                item_id: item.id.clone(),
                                resource_root: record.extension.resource_root.clone(),
                                icon: override_item
                                    .and_then(|item| item.icon.clone())
                                    .unwrap_or_else(|| item.icon.clone()),
                                tooltip: item.tooltip.clone(),
                                command: item.command.clone(),
                            })
                        } else {
                            None
                        }
                    })
            })
            .collect()
    }

    pub(crate) fn status_bar_bindings(&self) -> Vec<ExtensionStatusBarBinding> {
        self.catalog
            .records()
            .iter()
            .filter(|(_, record)| record.enabled)
            .flat_map(|(extension_id, record)| {
                record
                    .extension
                    .manifest
                    .status_bar_items
                    .iter()
                    .filter_map(move |item| {
                        let override_item = self
                            .status_bar_overrides
                            .get(extension_id)
                            .and_then(|items| items.get(&item.id));
                        if override_item
                            .and_then(|item| item.visible)
                            .unwrap_or(item.visible)
                        {
                            Some(ExtensionStatusBarBinding {
                                extension_id: extension_id.clone(),
                                item_id: item.id.clone(),
                                resource_root: record.extension.resource_root.clone(),
                                icon: override_item
                                    .and_then(|item| item.icon.clone())
                                    .unwrap_or_else(|| item.icon.clone()),
                                text: override_item
                                    .and_then(|item| item.text.clone())
                                    .or_else(|| item.text.clone()),
                                tooltip: item.tooltip.clone(),
                                side: item.side,
                                command: item.command.clone(),
                            })
                        } else {
                            None
                        }
                    })
            })
            .collect()
    }

    pub(crate) fn shortcut_bindings(
        &self,
        occupied: impl IntoIterator<Item = muxy_core::shortcuts::KeyCombo>,
    ) -> Vec<ExtensionShortcutBinding> {
        let mut occupied = occupied.into_iter().collect::<Vec<_>>();
        let mut bindings = Vec::new();
        for (extension_id, record) in self.catalog.records() {
            if !record.enabled {
                continue;
            }
            for command in &record.extension.manifest.commands {
                let Some(combo) = command
                    .default_shortcut
                    .as_deref()
                    .and_then(parse_extension_shortcut)
                else {
                    continue;
                };
                if occupied
                    .iter()
                    .any(|existing| existing.conflicts_with(&combo))
                {
                    continue;
                }
                occupied.push(combo.clone());
                bindings.push(ExtensionShortcutBinding {
                    extension_id: extension_id.clone(),
                    command_id: command.id.clone(),
                    combo,
                    source: ExtensionShortcutSource::Manifest,
                });
            }
            if let Some(runtime) = self.runtime_shortcuts.get(extension_id) {
                for (command_id, combo) in runtime {
                    if occupied
                        .iter()
                        .any(|existing| existing.conflicts_with(combo))
                    {
                        continue;
                    }
                    occupied.push(combo.clone());
                    bindings.push(ExtensionShortcutBinding {
                        extension_id: extension_id.clone(),
                        command_id: command_id.clone(),
                        combo: combo.clone(),
                        source: ExtensionShortcutSource::Runtime,
                    });
                }
            }
        }
        bindings
    }

    pub(crate) fn register_runtime_shortcut(
        &mut self,
        extension_id: &str,
        command_id: &str,
        combo: muxy_core::shortcuts::KeyCombo,
        occupied: &[muxy_core::shortcuts::KeyCombo],
    ) -> Result<(), String> {
        let record = self
            .catalog
            .records()
            .get(extension_id)
            .filter(|record| record.enabled)
            .ok_or_else(|| "extension is not loaded".to_owned())?;
        if record
            .extension
            .manifest
            .commands
            .iter()
            .any(|command| command.id == command_id)
        {
            return Err("runtime shortcut id conflicts with a manifest command".to_owned());
        }
        if !combo.is_supported_shortcut() {
            return Err("shortcut must use a supported key with cmd, ctrl, or opt".to_owned());
        }
        let current = self
            .runtime_shortcuts
            .get(extension_id)
            .and_then(|items| items.get(command_id));
        if occupied
            .iter()
            .filter(|candidate| current != Some(*candidate))
            .any(|candidate| candidate.conflicts_with(&combo))
        {
            return Err("shortcut conflicts with an existing binding".to_owned());
        }
        self.runtime_shortcuts
            .entry(extension_id.to_owned())
            .or_default()
            .insert(command_id.to_owned(), combo);
        Ok(())
    }

    pub(crate) fn unregister_runtime_shortcut(&mut self, extension_id: &str, command_id: &str) {
        if let Some(shortcuts) = self.runtime_shortcuts.get_mut(extension_id) {
            shortcuts.remove(command_id);
            if shortcuts.is_empty() {
                self.runtime_shortcuts.remove(extension_id);
            }
        }
    }

    pub fn set_topbar_item(
        &mut self,
        extension_id: &str,
        item_id: &str,
        icon: Option<ExtensionIcon>,
        visible: Option<bool>,
    ) -> bool {
        let declared = self
            .catalog
            .records()
            .get(extension_id)
            .is_some_and(|record| {
                record
                    .extension
                    .manifest
                    .topbar_items
                    .iter()
                    .any(|item| item.id == item_id)
            });
        if !declared {
            return false;
        }
        update_item_override(&mut self.topbar_overrides, extension_id, item_id, |item| {
            if let Some(icon) = icon {
                item.icon = Some(icon);
            }
            if let Some(visible) = visible {
                item.visible = Some(visible);
            }
        });
        true
    }

    pub fn set_status_bar_item(
        &mut self,
        extension_id: &str,
        item_id: &str,
        update: ExtensionStatusBarUpdate,
    ) -> bool {
        let declared = self
            .catalog
            .records()
            .get(extension_id)
            .is_some_and(|record| {
                record
                    .extension
                    .manifest
                    .status_bar_items
                    .iter()
                    .any(|item| item.id == item_id)
            });
        if !declared {
            return false;
        }
        update_item_override(
            &mut self.status_bar_overrides,
            extension_id,
            item_id,
            |item| {
                if let Some(icon) = update.icon {
                    item.icon = Some(icon);
                }
                if update.clear_text {
                    item.text = update.text;
                }
                if let Some(visible) = update.visible {
                    item.visible = Some(visible);
                }
            },
        );
        true
    }

    pub fn pending_consent(&self) -> Option<&ConsentRequest> {
        self.consent.pending_prompt()
    }

    pub fn gate_consent(&mut self, request: ConsentRequest) -> ConsentGate {
        let timestamp = now_timestamp();
        let gate = self.consent.gate(
            request,
            self.catalog.grants(),
            unix_time_millis(),
            &timestamp,
        );
        match &gate {
            ConsentGate::Allow { audit } | ConsentGate::Deny { audit } => {
                let _ = self.audit.append(audit);
            }
            ConsentGate::Pending { .. } => {}
        }
        gate
    }

    pub fn respond_to_consent(
        &mut self,
        request_id: &str,
        choice: ConsentChoice,
    ) -> Result<Option<ConsentResolution>, String> {
        let timestamp = now_timestamp();
        let mut rules = self.catalog.grants().to_vec();
        let Some(resolution) = self
            .consent
            .respond(request_id, choice, &mut rules, &timestamp)
        else {
            return Ok(None);
        };
        if resolution.rules_changed {
            self.catalog
                .set_grants(rules)
                .map_err(|error| error.to_string())?;
        }
        self.audit
            .append(&resolution.audit)
            .map_err(|error| error.to_string())?;
        Ok(Some(resolution))
    }

    pub fn cancel_consent(
        &mut self,
        request_id: &str,
    ) -> Result<Option<ConsentResolution>, String> {
        let timestamp = now_timestamp();
        let Some(resolution) = self.consent.cancel(request_id, &timestamp) else {
            return Ok(None);
        };
        self.audit
            .append(&resolution.audit)
            .map_err(|error| error.to_string())?;
        Ok(Some(resolution))
    }

    pub fn expire_consents(&mut self) -> Vec<ConsentResolution> {
        let timestamp = now_timestamp();
        let resolutions = self.consent.expire(unix_time_millis(), &timestamp);
        self.append_consent_audits(&resolutions);
        resolutions
    }

    pub fn cancel_consents_for_extension(&mut self, extension_id: &str) -> Vec<ConsentResolution> {
        let timestamp = now_timestamp();
        let resolutions = self.consent.cancel_extension(extension_id, &timestamp);
        self.append_consent_audits(&resolutions);
        resolutions
    }

    pub fn cancel_all_consents(&mut self) -> Vec<ConsentResolution> {
        let timestamp = now_timestamp();
        let resolutions = self.consent.cancel_all(&timestamp);
        self.append_consent_audits(&resolutions);
        resolutions
    }

    pub fn bind_socket(&mut self, handle: SocketServerHandle, path: PathBuf) {
        self.socket = Some(ExtensionSocket { handle, path });
        self.publish_snapshot();
    }

    pub(crate) fn host_events(&self) -> async_channel::Receiver<HostProcessExit> {
        self.host_event_receiver.clone()
    }

    pub fn start_all(&mut self) {
        self.publish_snapshot();
        let extension_ids = self
            .catalog
            .host_launches()
            .into_iter()
            .map(|launch| launch.extension_id)
            .collect::<Vec<_>>();
        for extension_id in extension_ids {
            self.start_host(&extension_id);
        }
    }

    pub fn set_enabled(
        &mut self,
        extension_id: &str,
        enabled: bool,
    ) -> Result<(), ExtensionRuntimeError> {
        self.catalog.set_enabled(extension_id, enabled)?;
        self.publish_snapshot();
        if enabled {
            self.start_host(extension_id);
        } else {
            self.close_surfaces(extension_id);
            self.clear_item_overrides(extension_id);
            self.stop_host(extension_id);
        }
        Ok(())
    }

    pub fn rescan(&mut self) -> Result<(), ExtensionRuntimeError> {
        let previous = self
            .catalog
            .records()
            .iter()
            .map(|(extension_id, record)| (extension_id.clone(), record.enabled))
            .collect::<BTreeMap<_, _>>();
        self.stop_all_hosts();
        self.catalog.rescan()?;
        self.publish_snapshot();
        for (extension_id, was_enabled) in previous {
            let remains_enabled = self
                .catalog
                .records()
                .get(&extension_id)
                .is_some_and(|record| record.enabled);
            if was_enabled && !remains_enabled {
                self.close_surfaces(&extension_id);
                self.clear_item_overrides(&extension_id);
            }
        }
        self.start_all();
        Ok(())
    }

    pub(crate) fn handle_host_exit(
        &mut self,
        event: HostProcessExit,
    ) -> Option<ExtensionRestartRequest> {
        if self
            .hosts
            .get(&event.extension_id)
            .map(|host| host.generation)
            != Some(event.generation)
        {
            return None;
        }
        self.hosts.remove(&event.extension_id);
        let enabled = self
            .catalog
            .records()
            .get(&event.extension_id)
            .is_some_and(|record| record.enabled);
        let lifecycle = self.lifecycles.get_mut(&event.extension_id)?;
        let action = lifecycle.terminated(
            event.generation,
            HostTermination::Exited(event.status),
            enabled,
            Instant::now(),
        );
        let error =
            (event.status != 0).then(|| format!("Process exited with status {}", event.status));
        let _ = self
            .catalog
            .set_status(&event.extension_id, lifecycle.status, error.clone());
        let line = if event.status == 0 {
            "[muxy] exited cleanly".to_owned()
        } else {
            format!("[muxy] {}", error.unwrap())
        };
        let _ = self.logs.append(&event.extension_id, &line);
        match action {
            HostLifecycleAction::None => None,
            HostLifecycleAction::RestartAfter(delay) => Some(ExtensionRestartRequest {
                extension_id: event.extension_id,
                delay,
            }),
        }
    }

    pub fn restart_if_due(&mut self, extension_id: &str) {
        let enabled = self
            .catalog
            .records()
            .get(extension_id)
            .is_some_and(|record| record.enabled);
        if self
            .lifecycles
            .get(extension_id)
            .is_some_and(|lifecycle| lifecycle.restart_is_due(Instant::now(), enabled))
        {
            self.start_host(extension_id);
        }
    }

    pub fn route_local_event(&mut self, event: ExtensionLocalEventIngress) {
        if self
            .catalog
            .records()
            .get(&event.extension_id)
            .is_some_and(|record| record.enabled)
        {
            self.surface_events
                .push_back(ExtensionSurfaceEvent::LocalEvent(event));
        }
    }

    pub fn take_surface_events(&mut self) -> Vec<ExtensionSurfaceEvent> {
        self.surface_events.drain(..).collect()
    }

    pub fn broadcast(&self, event: ExtensionBroadcast) {
        if let Some(socket) = &self.socket {
            socket.handle.broadcast(event);
        }
    }

    pub fn push_local_event(&self, extension_id: &str, event: ExtensionLocalEvent) {
        if let Some(socket) = &self.socket {
            socket.handle.push_extension_event(extension_id, event);
        }
    }

    pub fn push_modal_result(&self, extension_id: &str, result: ModalResult) {
        if let Some(socket) = &self.socket {
            socket.handle.push_modal_result(extension_id, result);
        }
    }

    pub fn push_modal_query(&self, extension_id: &str, query: ModalQuery) {
        if let Some(socket) = &self.socket {
            socket.handle.push_modal_query(extension_id, query);
        }
    }

    pub fn invoke(
        &self,
        extension_id: &str,
        request: InvokeRequest,
    ) -> Option<std::sync::mpsc::Receiver<muxy_proto::extension::InvokeOutcome>> {
        self.socket
            .as_ref()
            .map(|socket| socket.handle.invoke(extension_id, request))
    }

    pub(crate) fn run_script(&self, extension_id: &str, script: &str) -> Result<(), String> {
        let record = self
            .catalog
            .records()
            .get(extension_id)
            .filter(|record| record.enabled)
            .ok_or_else(|| format!("extension '{extension_id}' is not loaded"))?;
        let script_path = record
            .extension
            .resolve_resource(script)
            .filter(|path| path.is_file())
            .ok_or_else(|| format!("script '{script}' is unavailable"))?;
        let socket = self
            .socket
            .as_ref()
            .ok_or_else(|| "extension socket is unavailable".to_owned())?;
        let token = record
            .token()
            .ok_or_else(|| "extension authentication token is unavailable".to_owned())?;
        SystemHostLauncher::run_script(
            &script_path,
            &record.extension.resource_root,
            &socket.path,
            extension_id,
            token,
        )
        .map_err(|error| error.to_string())
    }

    pub fn shutdown(&mut self) {
        if let Some(socket) = &self.socket {
            socket
                .handle
                .replace_extension_snapshot(ExtensionSnapshot::default());
        }
        let enabled = self
            .catalog
            .records()
            .iter()
            .filter(|(_, record)| record.enabled)
            .map(|(extension_id, _)| extension_id.clone())
            .collect::<Vec<_>>();
        for extension_id in enabled {
            self.close_surfaces(&extension_id);
        }
        self.stop_all_hosts();
    }

    fn publish_snapshot(&self) {
        if let Some(socket) = &self.socket {
            socket
                .handle
                .replace_extension_snapshot(self.socket_snapshot());
        }
    }

    fn start_host(&mut self, extension_id: &str) {
        if self.hosts.contains_key(extension_id) {
            return;
        }
        let Some(socket) = &self.socket else {
            return;
        };
        let Some(launch) = self
            .catalog
            .host_launches()
            .into_iter()
            .find(|launch| launch.extension_id == extension_id)
        else {
            return;
        };
        let lifecycle = self.lifecycles.entry(extension_id.to_owned()).or_default();
        let generation = lifecycle.request_start();
        let _ = self
            .catalog
            .set_status(extension_id, ExtensionRuntimeStatus::Loading, None);
        match self.launcher.launch(
            &launch,
            &socket.path,
            generation,
            self.host_event_sender.clone(),
            self.logs.clone(),
        ) {
            Ok(process) => {
                lifecycle.started(generation, Instant::now());
                let _ =
                    self.catalog
                        .set_status(extension_id, ExtensionRuntimeStatus::Running, None);
                self.hosts.insert(
                    extension_id.to_owned(),
                    ManagedHost {
                        generation,
                        process,
                    },
                );
                let _ = self.logs.append(
                    extension_id,
                    &format!(
                        "[muxy] started {} v{}",
                        extension_id,
                        launch_extension_version(&self.catalog, extension_id)
                    ),
                );
            }
            Err(error) => {
                lifecycle.start_failed(generation);
                let status = if error.is_unsupported() {
                    ExtensionRuntimeStatus::Unsupported
                } else {
                    ExtensionRuntimeStatus::Failed
                };
                let message = error.to_string();
                let _ = self
                    .catalog
                    .set_status(extension_id, status, Some(message.clone()));
                let _ = self
                    .logs
                    .append(extension_id, &format!("[muxy] failed to start: {message}"));
            }
        }
    }

    fn stop_host(&mut self, extension_id: &str) {
        if let Some(lifecycle) = self.lifecycles.get_mut(extension_id) {
            lifecycle.disable();
        }
        if let Some(mut host) = self.hosts.remove(extension_id) {
            host.process.stop();
        }
    }

    fn stop_all_hosts(&mut self) {
        let extension_ids = self.hosts.keys().cloned().collect::<Vec<_>>();
        for extension_id in extension_ids {
            self.stop_host(&extension_id);
        }
    }

    fn append_consent_audits(&self, resolutions: &[ConsentResolution]) {
        for resolution in resolutions {
            if let Err(error) = self.audit.append(&resolution.audit) {
                log::warn!("failed to append extension consent audit: {error}");
            }
        }
    }

    fn clear_item_overrides(&mut self, extension_id: &str) {
        self.topbar_overrides.remove(extension_id);
        self.status_bar_overrides.remove(extension_id);
        self.runtime_shortcuts.remove(extension_id);
    }

    fn close_surfaces(&mut self, extension_id: &str) {
        self.surface_events
            .push_back(ExtensionSurfaceEvent::CloseAll {
                extension_id: extension_id.to_owned(),
            });
    }
}

impl Drop for ExtensionRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn update_item_override(
    overrides: &mut BTreeMap<String, BTreeMap<String, ExtensionItemOverride>>,
    extension_id: &str,
    item_id: &str,
    update: impl FnOnce(&mut ExtensionItemOverride),
) {
    let extension = overrides.entry(extension_id.to_owned()).or_default();
    let item = extension.entry(item_id.to_owned()).or_default();
    update(item);
    if *item == ExtensionItemOverride::default() {
        extension.remove(item_id);
    }
    if extension.is_empty() {
        overrides.remove(extension_id);
    }
}

fn unix_time_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or_default()
}

pub(crate) fn parse_extension_shortcut(value: &str) -> Option<muxy_core::shortcuts::KeyCombo> {
    use muxy_core::shortcuts::{COMMAND, CONTROL, OPTION, SHIFT};
    let mut modifiers = 0;
    let mut key = None;
    for part in value
        .split(['+', '-'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        match part.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "meta" => modifiers |= COMMAND,
            "ctrl" | "control" => modifiers |= CONTROL,
            "opt" | "option" | "alt" => modifiers |= OPTION,
            "shift" => modifiers |= SHIFT,
            candidate if key.is_none() => key = Some(candidate.to_owned()),
            _ => return None,
        }
    }
    let combo = muxy_core::shortcuts::KeyCombo::new(&key?, modifiers).canonicalized();
    combo.is_supported_shortcut().then_some(combo)
}

fn launch_extension_version(catalog: &ExtensionRuntimeCatalog, extension_id: &str) -> String {
    catalog
        .records()
        .get(extension_id)
        .map(|record| record.extension.manifest.version.clone())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn socket_snapshot(snapshot: ExtensionRuntimeSnapshot) -> ExtensionSnapshot {
    ExtensionSnapshot {
        entries: snapshot
            .entries
            .into_iter()
            .map(|(extension_id, entry)| {
                let subscription_access = entry
                    .subscription_access
                    .into_iter()
                    .map(|(event, access)| {
                        let access = match access {
                            RuntimeSubscriptionAccess::Allowed => SubscriptionAccess::Allowed,
                            RuntimeSubscriptionAccess::Denied(error) => {
                                SubscriptionAccess::Denied(error)
                            }
                        };
                        (event, access)
                    })
                    .collect();
                (
                    extension_id,
                    ExtensionSnapshotEntry {
                        token: entry.token,
                        granted_permissions: entry.granted_permissions,
                        subscription_access,
                        can_write_notifications: entry.can_write_notifications,
                    },
                )
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxy_api::extensions::{ExtensionApiRequest, ExtensionApiService, ExtensionApiSource};
    use muxy_core::environment::{BuildMode, RuntimePathPolicy};
    use muxy_core::extensions::paths::ExtensionPaths;
    use muxy_core::extensions::state::ExtensionStateStore;
    use muxy_proto::server::{ServerConfig, ServerLimits, SocketServer};
    use serde_json::json;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    struct MockHost;

    impl RunningHost for MockHost {
        fn stop(&mut self) {}
    }

    struct AuthenticatingLauncher {
        replies: Arc<Mutex<Vec<String>>>,
    }

    impl HostLauncher for AuthenticatingLauncher {
        fn launch(
            &self,
            request: &muxy_core::extensions::runtime::ExtensionHostLaunch,
            socket_path: &Path,
            _generation: u64,
            _events: async_channel::Sender<HostProcessExit>,
            _logs: ExtensionLogStore,
        ) -> Result<Box<dyn RunningHost>, process::HostProcessError> {
            let mut stream = UnixStream::connect(socket_path).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            writeln!(
                stream,
                "identify|{}|{}",
                request.extension_id, request.token
            )
            .unwrap();
            let mut reply = String::new();
            reader.read_line(&mut reply).unwrap();
            self.replies.lock().unwrap().push(reply);
            Ok(Box::new(MockHost))
        }
    }

    fn paths(root: &Path) -> ExtensionPaths {
        ExtensionPaths::new(RuntimePathPolicy::new(BuildMode::Production), root)
    }

    fn package(paths: &ExtensionPaths, enabled: bool) {
        let package = paths.packages.join("sample");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("background.js"), b"muxy.log('ready')").unwrap();
        std::fs::write(
            package.join("package.json"),
            serde_json::to_vec(&json!({
                "name": "sample",
                "version": "1.0.0",
                "muxy": {
                    "background": "background.js",
                    "events": ["agent.status", "theme.changed"],
                    "permissions": ["notifications:write", "panels:write"],
                    "commands": [{"id": "sample.command", "title": "Sample command"}],
                    "topbarItems": [{
                        "id": "top",
                        "icon": "bolt",
                        "command": "sample.command"
                    }],
                    "statusBarItems": [{
                        "id": "status",
                        "icon": "bolt",
                        "side": "left",
                        "command": "sample.command"
                    }]
                }
            }))
            .unwrap(),
        )
        .unwrap();
        if enabled {
            let mut state = ExtensionStateStore::open(paths.state_file()).unwrap();
            state.set_enabled("sample", Some(true)).unwrap();
        }
    }

    fn socket(path: &Path, snapshot: ExtensionSnapshot) -> (SocketServer, SocketServerHandle) {
        let (server, handle, _) = SocketServer::start(ServerConfig {
            socket_path: path.to_path_buf(),
            recognized_command_heads: Default::default(),
            no_response_command_routes: Vec::new(),
            limits: ServerLimits::default(),
            initial_extension_snapshot: snapshot,
        })
        .unwrap();
        (server, handle)
    }

    #[test]
    fn pending_api_calls_complete_once_and_reject_duplicate_ids() {
        let results = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let request = ExtensionApiRequest::new(
            "sample",
            "storage.keys",
            serde_json::Map::new(),
            ExtensionApiSource::Background,
            std::collections::BTreeSet::new(),
        );
        let mut pending = PendingExtensionApiCalls::default();
        let first_results = results.clone();
        pending.insert(
            "request".to_owned(),
            request.clone(),
            Box::new(move |result| {
                first_results.borrow_mut().push(result);
            }),
        );
        let replacement_results = results.clone();
        pending.insert(
            "request".to_owned(),
            request,
            Box::new(move |result| {
                replacement_results.borrow_mut().push(result);
            }),
        );
        assert!(matches!(
            results.borrow().as_slice(),
            [Err(muxy_api::extensions::ExtensionApiError::Service(message))]
                if message == "duplicate pending request for storage.keys"
        ));
        let call = pending.remove("request").unwrap();
        call.complete(Ok(json!(["one"])));
        assert_eq!(results.borrow().len(), 2);
        assert_eq!(results.borrow()[1], Ok(json!(["one"])));
        assert!(pending.drain().is_empty());
    }

    #[test]
    fn pending_api_calls_resolve_allow_deny_timeout_cancellation_and_shutdown() {
        let results = std::rc::Rc::new(std::cell::RefCell::new(BTreeMap::new()));
        let mut pending = PendingExtensionApiCalls::default();
        for request_id in ["allow", "deny", "timeout", "cancel", "shutdown"] {
            let request = ExtensionApiRequest::new(
                "sample",
                "exec",
                serde_json::Map::new(),
                ExtensionApiSource::Background,
                std::collections::BTreeSet::new(),
            );
            let results = results.clone();
            let result_id = request_id.to_owned();
            pending.insert(
                request_id.to_owned(),
                request,
                Box::new(move |result| {
                    results.borrow_mut().insert(result_id, result);
                }),
            );
        }
        pending.remove("allow").unwrap().complete(Ok(json!(null)));
        pending.remove("deny").unwrap().complete(Err(
            muxy_api::extensions::ExtensionApiError::ConsentDenied("exec".to_owned()),
        ));
        pending.remove("timeout").unwrap().complete(Err(
            muxy_api::extensions::ExtensionApiError::ConsentDenied("exec".to_owned()),
        ));
        pending.remove("cancel").unwrap().complete(Err(
            muxy_api::extensions::ExtensionApiError::ConsentDenied("exec".to_owned()),
        ));
        for call in pending.drain() {
            let method = call.request.method.clone();
            call.complete(Err(muxy_api::extensions::ExtensionApiError::ConsentDenied(
                method,
            )));
        }

        let results = results.borrow();
        assert_eq!(results.len(), 5);
        assert_eq!(results["allow"], Ok(json!(null)));
        for request_id in ["deny", "timeout", "cancel", "shutdown"] {
            assert_eq!(
                results[request_id],
                Err(muxy_api::extensions::ExtensionApiError::ConsentDenied(
                    "exec".to_owned()
                ))
            );
        }
    }

    #[test]
    fn runtime_snapshot_converts_without_losing_authentication_policy() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        package(&paths, true);
        let runtime = ExtensionRuntime::load(paths).unwrap();
        let snapshot = runtime.socket_snapshot();
        let entry = &snapshot.entries["sample"];
        assert_eq!(entry.token.len(), 64);
        assert!(entry.can_write_notifications);
        assert_eq!(
            entry.subscription_access["agent.status"],
            SubscriptionAccess::Denied("permission denied (agents:read)".to_owned())
        );
        assert_eq!(
            entry.subscription_access["theme.changed"],
            SubscriptionAccess::Allowed
        );
    }

    #[test]
    fn snapshot_is_installed_before_an_enabled_host_launches() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        package(&paths, true);
        let replies = Arc::new(Mutex::new(Vec::new()));
        let mut runtime = ExtensionRuntime::load_with_launcher(
            paths,
            Box::new(AuthenticatingLauncher {
                replies: replies.clone(),
            }),
        )
        .unwrap();
        let socket_path = directory.path().join("main.sock");
        let (_server, handle) = socket(&socket_path, ExtensionSnapshot::default());
        runtime.bind_socket(handle, socket_path);
        runtime.start_all();
        assert_eq!(*replies.lock().unwrap(), ["ok\n"]);
        assert_eq!(
            runtime.catalog().records()["sample"].status,
            ExtensionRuntimeStatus::Running
        );
    }

    #[test]
    fn disabling_removes_authentication_and_queues_surface_closure() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        package(&paths, true);
        let mut runtime = ExtensionRuntime::load_with_launcher(
            paths,
            Box::new(AuthenticatingLauncher {
                replies: Arc::new(Mutex::new(Vec::new())),
            }),
        )
        .unwrap();
        let token = runtime.socket_snapshot().entries["sample"].token.clone();
        let socket_path = directory.path().join("main.sock");
        let (_server, handle) = socket(&socket_path, runtime.socket_snapshot());
        runtime.bind_socket(handle, socket_path.clone());
        runtime.start_all();
        runtime.set_enabled("sample", false).unwrap();

        let mut stream = UnixStream::connect(socket_path).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        writeln!(stream, "identify|sample|{token}").unwrap();
        let mut reply = String::new();
        reader.read_line(&mut reply).unwrap();
        assert_eq!(reply, "error:unknown extension sample\n");
        assert_eq!(
            runtime.take_surface_events(),
            [ExtensionSurfaceEvent::CloseAll {
                extension_id: "sample".to_owned()
            }]
        );
    }

    #[test]
    fn consent_responses_persist_rules_and_append_audit_entries() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        package(&paths, false);
        let mut runtime = ExtensionRuntime::load(paths.clone()).unwrap();
        let request = ConsentRequest::new(
            "sample",
            "Sample",
            muxy_core::extensions::state::ExtensionGatedVerb::GitWrite,
            muxy_api::extensions::ExtensionGatedPayload::Git {
                operation: "commit".to_owned(),
                repo_path: "/tmp/repo".to_owned(),
            },
            "webview",
        );
        let request_id = request.id.clone();
        assert!(matches!(
            runtime.gate_consent(request.clone()),
            ConsentGate::Pending { active: true, .. }
        ));
        let resolution = runtime
            .respond_to_consent(&request_id, ConsentChoice::AllowAndRemember)
            .unwrap()
            .unwrap();
        assert_eq!(
            resolution.decision,
            muxy_core::extensions::state::ExtensionGrantDecision::Allow
        );
        assert_eq!(runtime.catalog().grants().len(), 1);
        assert!(matches!(
            runtime.gate_consent(request),
            ConsentGate::Allow { .. }
        ));
        let audit = std::fs::read_to_string(paths.audit_file()).unwrap();
        assert_eq!(audit.lines().count(), 2);
        assert!(
            audit
                .lines()
                .all(|line| line.contains("\"verb\":\"git.write\""))
        );
        let reloaded = ExtensionRuntime::load(paths).unwrap();
        assert_eq!(reloaded.catalog().grants().len(), 1);
    }

    #[test]
    fn api_item_updates_preserve_independent_fields_and_clear_on_disable() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        package(&paths, true);
        let mut runtime = ExtensionRuntime::load(paths).unwrap();
        let permissions = std::collections::BTreeSet::from(["panels:write".to_owned()]);
        {
            let mut service = api::AppExtensionApiService::new(&mut runtime);
            assert!(matches!(
                service.dispatch(
                    muxy_api::extensions::ExtensionApiServiceGroup::Surfaces,
                    &ExtensionApiRequest::new(
                        "sample",
                        "topbar.set",
                        json!({"id": "top", "icon": {"svg": "<svg/>"}, "visible": false})
                            .as_object()
                            .unwrap()
                            .clone(),
                        ExtensionApiSource::Webview,
                        permissions.clone(),
                    ),
                ),
                muxy_api::extensions::ExtensionServiceDispatch::Complete(Ok(_))
            ));
            assert!(matches!(
                service.dispatch(
                    muxy_api::extensions::ExtensionApiServiceGroup::Surfaces,
                    &ExtensionApiRequest::new(
                        "sample",
                        "statusbar.set",
                        json!({"id": "status", "icon": "checkmark", "text": "Ready"})
                            .as_object()
                            .unwrap()
                            .clone(),
                        ExtensionApiSource::Webview,
                        permissions.clone(),
                    ),
                ),
                muxy_api::extensions::ExtensionServiceDispatch::Complete(Ok(_))
            ));
            assert!(matches!(
                service.dispatch(
                    muxy_api::extensions::ExtensionApiServiceGroup::Surfaces,
                    &ExtensionApiRequest::new(
                        "sample",
                        "statusbar.set",
                        json!({"id": "status", "text": "", "visible": false})
                            .as_object()
                            .unwrap()
                            .clone(),
                        ExtensionApiSource::Webview,
                        permissions,
                    ),
                ),
                muxy_api::extensions::ExtensionServiceDispatch::Complete(Ok(_))
            ));
        }
        assert_eq!(
            runtime.topbar_overrides()["sample"]["top"],
            ExtensionItemOverride {
                icon: Some(ExtensionIcon::Svg("<svg/>".to_owned())),
                text: None,
                visible: Some(false),
            }
        );
        assert_eq!(
            runtime.status_bar_overrides()["sample"]["status"],
            ExtensionItemOverride {
                icon: Some(ExtensionIcon::Symbol("checkmark".to_owned())),
                text: None,
                visible: Some(false),
            }
        );
        runtime.set_enabled("sample", false).unwrap();
        assert!(runtime.topbar_overrides().is_empty());
        assert!(runtime.status_bar_overrides().is_empty());
    }

    #[test]
    fn local_events_are_routed_only_for_enabled_extensions() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        package(&paths, true);
        let mut runtime = ExtensionRuntime::load(paths).unwrap();
        let ingress = ExtensionLocalEventIngress {
            extension_id: "sample".to_owned(),
            event: ExtensionLocalEvent {
                name: "extension.ready".to_owned(),
                payload: b"{}".to_vec(),
            },
        };
        runtime.route_local_event(ingress.clone());
        assert_eq!(
            runtime.take_surface_events(),
            [ExtensionSurfaceEvent::LocalEvent(ingress.clone())]
        );
        runtime.set_enabled("sample", false).unwrap();
        runtime.take_surface_events();
        runtime.route_local_event(ingress);
        assert!(runtime.take_surface_events().is_empty());
    }
}
