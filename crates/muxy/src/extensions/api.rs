use muxy_api::extensions::{
    ExtensionApiError, ExtensionApiRequest, ExtensionApiService, ExtensionApiServiceGroup,
    ExtensionServiceDispatch,
};
use muxy_core::extensions::manifest::ExtensionIcon;
use muxy_core::extensions::runtime::ExtensionRuntimeError;
use muxy_core::workspace::{Tab, TabKind};
use serde_json::{Value, json};

use super::surfaces::{ExtensionSurfaceKind, ExtensionSurfaceRegistry};
use super::{ExtensionRuntime, ExtensionStatusBarUpdate};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ExtensionAppEffect {
    OpenTab {
        tab: Tab,
        directory: Option<String>,
        command: Option<String>,
    },
    FocusTab(String),
    CycleTab(i32),
    SetTabTitle {
        tab_id: String,
        title: Option<String>,
    },
    SetTabIcon {
        tab_id: String,
        icon: Option<ExtensionIcon>,
    },
    SetTabData {
        tab_id: String,
        data: Value,
    },
    OpenPanel {
        extension_id: String,
        panel_id: String,
        data: Option<Value>,
        toggle: bool,
    },
    ClosePanel {
        extension_id: String,
        panel_id: String,
    },
    CloseSurface(String),
    ResizePopover {
        surface_id: String,
        width: f64,
        height: f64,
    },
    OpenModal {
        extension_id: String,
        entry: String,
        width: Option<f64>,
        height: Option<f64>,
        dismiss_on_outside_click: bool,
        data: Option<Value>,
        request_id: Option<String>,
        opener_surface_id: Option<String>,
    },
    ResolveModal {
        request_id: String,
        result: Value,
    },
    CloseActiveModal {
        extension_id: String,
    },
    Notify {
        title: String,
        body: String,
    },
    RebindShortcuts,
}

pub(crate) struct AppExtensionApiService<'a> {
    runtime: &'a mut ExtensionRuntime,
    state: Option<&'a crate::state::AppState>,
    surfaces: Option<&'a ExtensionSurfaceRegistry>,
    effects: Option<&'a mut Vec<ExtensionAppEffect>>,
}

impl<'a> AppExtensionApiService<'a> {
    pub(crate) fn new(runtime: &'a mut ExtensionRuntime) -> Self {
        Self {
            runtime,
            state: None,
            surfaces: None,
            effects: None,
        }
    }

    pub(crate) fn with_app(
        runtime: &'a mut ExtensionRuntime,
        state: &'a crate::state::AppState,
        surfaces: &'a ExtensionSurfaceRegistry,
        effects: &'a mut Vec<ExtensionAppEffect>,
    ) -> Self {
        Self {
            runtime,
            state: Some(state),
            surfaces: Some(surfaces),
            effects: Some(effects),
        }
    }

    fn effect(&mut self, effect: ExtensionAppEffect) -> Result<(), ExtensionApiError> {
        self.effects
            .as_mut()
            .ok_or_else(|| ExtensionApiError::ServiceUnavailable("application UI".to_owned()))?
            .push(effect);
        Ok(())
    }

    fn state(&self) -> Result<&crate::state::AppState, ExtensionApiError> {
        self.state
            .ok_or_else(|| ExtensionApiError::ServiceUnavailable("application state".to_owned()))
    }

    fn surfaces(&self) -> Result<&ExtensionSurfaceRegistry, ExtensionApiError> {
        self.surfaces
            .ok_or_else(|| ExtensionApiError::ServiceUnavailable("extension surfaces".to_owned()))
    }
}

impl ExtensionApiService for AppExtensionApiService<'_> {
    fn is_available(&self, group: ExtensionApiServiceGroup, method: &str) -> bool {
        match group {
            ExtensionApiServiceGroup::Storage | ExtensionApiServiceGroup::Settings => true,
            ExtensionApiServiceGroup::Shortcuts
            | ExtensionApiServiceGroup::Modals
            | ExtensionApiServiceGroup::Tabs
            | ExtensionApiServiceGroup::Notifications
            | ExtensionApiServiceGroup::Events => self.effects.is_some(),
            ExtensionApiServiceGroup::Execution
            | ExtensionApiServiceGroup::Dialogs
            | ExtensionApiServiceGroup::Git
            | ExtensionApiServiceGroup::Worktrees => self.state.is_some(),
            ExtensionApiServiceGroup::Surfaces => {
                matches!(method, "topbar.set" | "statusbar.set") || self.effects.is_some()
            }
            _ => false,
        }
    }

    fn dispatch(
        &mut self,
        group: ExtensionApiServiceGroup,
        request: &ExtensionApiRequest,
    ) -> ExtensionServiceDispatch {
        if matches!(
            group,
            ExtensionApiServiceGroup::Execution
                | ExtensionApiServiceGroup::Dialogs
                | ExtensionApiServiceGroup::Git
                | ExtensionApiServiceGroup::Worktrees
        ) {
            return ExtensionServiceDispatch::Deferred;
        }
        ExtensionServiceDispatch::Complete(self.dispatch_immediate(request))
    }
}

impl AppExtensionApiService<'_> {
    fn dispatch_immediate(
        &mut self,
        request: &ExtensionApiRequest,
    ) -> Result<Value, ExtensionApiError> {
        match request.method.as_str() {
            "storage.get" => self
                .runtime
                .storage()
                .get(&request.extension_id, required_string(request, "key")?)
                .map_err(storage_error),
            "storage.set" => {
                let key = required_string(request, "key")?;
                let value = request.args.get("value").cloned().ok_or_else(|| {
                    ExtensionApiError::InvalidArguments("storage.set requires a value".to_owned())
                })?;
                self.runtime
                    .storage()
                    .set(&request.extension_id, key, value)
                    .map_err(storage_error)?;
                Ok(Value::Null)
            }
            "storage.delete" => {
                self.runtime
                    .storage()
                    .delete(&request.extension_id, required_string(request, "key")?)
                    .map_err(storage_error)?;
                Ok(Value::Null)
            }
            "storage.keys" => self
                .runtime
                .storage()
                .keys(&request.extension_id)
                .map(|keys| Value::Array(keys.into_iter().map(Value::String).collect()))
                .map_err(storage_error),
            "extension.settings.get" => {
                let extension_id = request.extension_id.clone();
                let key = required_string(request, "key")?;
                self.runtime
                    .catalog()
                    .effective_setting(&extension_id, key)
                    .map(|value| value.unwrap_or(Value::Null))
                    .map_err(runtime_error)
            }
            "extension.settings.set" => {
                let extension_id = request.extension_id.clone();
                let key = required_string(request, "key")?.to_owned();
                let value = request.args.get("value").cloned().ok_or_else(|| {
                    ExtensionApiError::InvalidArguments(
                        "extension.settings.set requires a value".to_owned(),
                    )
                })?;
                self.runtime
                    .catalog_mut()
                    .set_setting(&extension_id, &key, value)
                    .map_err(runtime_error)?;
                Ok(Value::Null)
            }
            "extension.statusbar.set" => {
                let extension_id = request.extension_id.clone();
                let item_id = required_string(request, "id")?.to_owned();
                let text = request
                    .args
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                    .map(str::to_owned);
                if !self.runtime.set_status_bar_item(
                    &extension_id,
                    &item_id,
                    ExtensionStatusBarUpdate {
                        text,
                        clear_text: true,
                        ..ExtensionStatusBarUpdate::default()
                    },
                ) {
                    return Err(unknown_item("status bar", &item_id));
                }
                Ok(Value::Null)
            }
            "topbar.set" => {
                let extension_id = request.extension_id.clone();
                let item_id = required_string(request, "id")?.to_owned();
                let icon = request.args.get("icon").and_then(parse_icon);
                let visible = request.args.get("visible").and_then(Value::as_bool);
                if !self
                    .runtime
                    .set_topbar_item(&extension_id, &item_id, icon, visible)
                {
                    return Err(unknown_item("topbar", &item_id));
                }
                Ok(Value::Null)
            }
            "statusbar.set" => {
                let extension_id = request.extension_id.clone();
                let item_id = required_string(request, "id")?.to_owned();
                let icon = request.args.get("icon").and_then(parse_icon);
                let text = request
                    .args
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                    .map(str::to_owned);
                let visible = request.args.get("visible").and_then(Value::as_bool);
                let clear_text = request.args.contains_key("text");
                if !self.runtime.set_status_bar_item(
                    &extension_id,
                    &item_id,
                    ExtensionStatusBarUpdate {
                        icon,
                        text,
                        clear_text,
                        visible,
                    },
                ) {
                    return Err(unknown_item("status bar", &item_id));
                }
                Ok(Value::Null)
            }
            "shortcuts.register" => {
                let command_id = required_string(request, "id")?.to_owned();
                let combo = required_string(request, "combo")?;
                let Some(combo) = super::parse_extension_shortcut(combo) else {
                    return Ok(json!({
                        "ok": false,
                        "conflict": "shortcut must use a supported key with cmd, ctrl, or opt",
                    }));
                };
                let occupied = self.occupied_shortcuts()?;
                let result = self.runtime.register_runtime_shortcut(
                    &request.extension_id,
                    &command_id,
                    combo,
                    &occupied,
                );
                if let Err(conflict) = result {
                    return Ok(json!({"ok": false, "conflict": conflict}));
                }
                self.effect(ExtensionAppEffect::RebindShortcuts)?;
                Ok(json!({"ok": true}))
            }
            "shortcuts.unregister" => {
                let command_id = required_string(request, "id")?.to_owned();
                self.runtime
                    .unregister_runtime_shortcut(&request.extension_id, &command_id);
                self.effect(ExtensionAppEffect::RebindShortcuts)?;
                Ok(Value::Null)
            }
            "shortcuts.list" => {
                let bindings = self
                    .runtime
                    .shortcut_bindings(self.base_shortcuts()?)
                    .into_iter()
                    .filter(|binding| binding.extension_id == request.extension_id)
                    .map(|binding| {
                        json!({
                            "id": binding.command_id,
                            "combo": binding.combo.keystroke(),
                            "source": binding.source.as_str(),
                        })
                    })
                    .collect();
                Ok(Value::Array(bindings))
            }
            "tabs.open" => self.open_tab(request),
            "tabs.new" => self.open_terminal_tab(),
            "tabs.list" => self.list_tabs(),
            "tabs.switch" => {
                let identifier = required_string(request, "identifier")?.to_owned();
                self.effect(ExtensionAppEffect::FocusTab(identifier))?;
                Ok(Value::Null)
            }
            "tabs.next" | "tabs.previous" => {
                let direction = if request.method == "tabs.next" { 1 } else { -1 };
                self.effect(ExtensionAppEffect::CycleTab(direction))?;
                Ok(Value::Null)
            }
            "tabs.setTitle" => {
                let tab_id = calling_tab_id(request, self.surfaces()?)?;
                let title = request
                    .args
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|title| !title.is_empty())
                    .map(str::to_owned);
                self.effect(ExtensionAppEffect::SetTabTitle { tab_id, title })?;
                Ok(Value::Null)
            }
            "tabs.setIcon" => {
                let tab_id = calling_tab_id(request, self.surfaces()?)?;
                let icon = request.args.get("icon").and_then(parse_icon);
                self.effect(ExtensionAppEffect::SetTabIcon { tab_id, icon })?;
                Ok(Value::Null)
            }
            "panel.open" | "panel.toggle" => {
                let panel_id = required_string(request, "panel")?.to_owned();
                ensure_panel(self.runtime, &request.extension_id, &panel_id)?;
                let data = request
                    .args
                    .get("data")
                    .cloned()
                    .filter(|value| !value.is_null());
                self.effect(ExtensionAppEffect::OpenPanel {
                    extension_id: request.extension_id.clone(),
                    panel_id,
                    data,
                    toggle: request.method == "panel.toggle",
                })?;
                Ok(Value::Null)
            }
            "panel.close" => {
                let panel_id = required_string(request, "panel")?.to_owned();
                ensure_panel(self.runtime, &request.extension_id, &panel_id)?;
                self.effect(ExtensionAppEffect::ClosePanel {
                    extension_id: request.extension_id.clone(),
                    panel_id,
                })?;
                Ok(Value::Null)
            }
            "popover.close" => {
                let surface_id = calling_surface_id(request)?;
                ensure_surface_kind(self.surfaces()?, &surface_id, |kind| {
                    matches!(kind, ExtensionSurfaceKind::Popover(_))
                })?;
                self.effect(ExtensionAppEffect::CloseSurface(surface_id))?;
                Ok(Value::Null)
            }
            "popover.resize" => {
                let surface_id = calling_surface_id(request)?;
                ensure_surface_kind(self.surfaces()?, &surface_id, |kind| {
                    matches!(kind, ExtensionSurfaceKind::Popover(_))
                })?;
                let width = required_number(request, "width")?;
                let height = required_number(request, "height")?;
                self.effect(ExtensionAppEffect::ResizePopover {
                    surface_id,
                    width,
                    height,
                })?;
                Ok(Value::Null)
            }
            "modal.openWebview" => self.open_webview_modal(request),
            "modal.closeWebview" => {
                self.effect(ExtensionAppEffect::CloseActiveModal {
                    extension_id: request.extension_id.clone(),
                })?;
                Ok(Value::Null)
            }
            "modal.submitWebview" => {
                let request_id = required_string(request, "requestID")?.to_owned();
                let caller = calling_surface_id(request)?;
                if caller != request_id {
                    return Err(ExtensionApiError::InvalidArguments(
                        "modal results must be submitted by the active modal".to_owned(),
                    ));
                }
                ensure_surface_kind(self.surfaces()?, &caller, |kind| {
                    matches!(kind, ExtensionSurfaceKind::Modal { .. })
                })?;
                let result = request.args.get("result").cloned().unwrap_or(Value::Null);
                let encoded = serde_json::to_vec(&result).map_err(|_| {
                    ExtensionApiError::InvalidArguments(
                        "modal result is not serializable".to_owned(),
                    )
                })?;
                if encoded.len() > 256 * 1024 {
                    return Err(ExtensionApiError::InvalidArguments(
                        "modal result exceeds 256 KiB".to_owned(),
                    ));
                }
                self.effect(ExtensionAppEffect::ResolveModal { request_id, result })?;
                Ok(Value::Null)
            }
            "events.subscribe" | "events.unsubscribe" => {
                validate_event_subscription(self.runtime, request)?;
                Ok(Value::Null)
            }
            "events.emit" => self.emit_local_event(request),
            "toast" | "notifications.notify" => {
                let title = request
                    .args
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let body = request
                    .args
                    .get("body")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                if title.is_empty() && body.is_empty() {
                    return Err(ExtensionApiError::InvalidArguments(
                        "notification requires a title or body".to_owned(),
                    ));
                }
                self.effect(ExtensionAppEffect::Notify { title, body })?;
                Ok(Value::Null)
            }
            method => Err(ExtensionApiError::ServiceUnavailable(method.to_owned())),
        }
    }
}

impl AppExtensionApiService<'_> {
    fn base_shortcuts(&self) -> Result<Vec<muxy_core::shortcuts::KeyCombo>, ExtensionApiError> {
        let state = self.state()?;
        let mut occupied = state.shortcuts.assigned_combos();
        occupied.push(state.command_shortcuts.prefix_combo.clone());
        Ok(occupied)
    }

    fn occupied_shortcuts(&self) -> Result<Vec<muxy_core::shortcuts::KeyCombo>, ExtensionApiError> {
        let base = self.base_shortcuts()?;
        let mut occupied = base.clone();
        occupied.extend(
            self.runtime
                .shortcut_bindings(base)
                .into_iter()
                .map(|binding| binding.combo),
        );
        Ok(occupied)
    }

    fn open_tab(&mut self, request: &ExtensionApiRequest) -> Result<Value, ExtensionApiError> {
        let state = self.state()?;
        if state.active_tab_workspace().is_none() {
            return Err(ExtensionApiError::InvalidArguments(
                "tabs.open requires an active workspace".to_owned(),
            ));
        }
        let kind = request
            .args
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("extensionWebView");
        let project_path = state
            .active_project()
            .map(|project| state.active_worktree_path(project));
        let mut directory = None;
        let mut command = None;
        let tab =
            match kind {
                "terminal" => {
                    directory = request
                        .args
                        .get("directory")
                        .and_then(Value::as_str)
                        .filter(|path| !path.is_empty())
                        .map(str::to_owned);
                    if let Some(relative) = directory.as_deref() {
                        let root = project_path.as_deref().ok_or_else(|| {
                            ExtensionApiError::InvalidArguments(
                                "terminal directory requires an active project".to_owned(),
                            )
                        })?;
                        let root = std::fs::canonicalize(root).map_err(|error| {
                            ExtensionApiError::Service(format!(
                                "could not resolve active worktree: {error}"
                            ))
                        })?;
                        let candidate = std::path::Path::new(relative);
                        let candidate = if candidate.is_absolute() {
                            candidate.to_path_buf()
                        } else {
                            root.join(candidate)
                        };
                        let candidate = std::fs::canonicalize(candidate).map_err(|error| {
                            ExtensionApiError::InvalidArguments(format!(
                                "terminal directory is unavailable: {error}"
                            ))
                        })?;
                        if !candidate.starts_with(&root) || !candidate.is_dir() {
                            return Err(ExtensionApiError::InvalidArguments(
                                "terminal directory must stay inside the active worktree"
                                    .to_owned(),
                            ));
                        }
                        directory = Some(candidate.to_string_lossy().into_owned());
                    }
                    command = request
                        .args
                        .get("command")
                        .and_then(Value::as_str)
                        .filter(|command| !command.is_empty())
                        .map(str::to_owned);
                    let mut tab = Tab::new(TabKind::Terminal);
                    tab.project_path = project_path;
                    tab
                }
                "extensionWebView" => {
                    let extension = request
                        .args
                        .get("extension")
                        .and_then(Value::as_object)
                        .ok_or_else(|| {
                            ExtensionApiError::InvalidArguments(
                                "tabs.open requires extension details".to_owned(),
                            )
                        })?;
                    let extension_id = extension
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                        .unwrap_or(&request.extension_id);
                    let tab_type_id = extension
                        .get("tabType")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| {
                            ExtensionApiError::InvalidArguments(
                                "tabs.open requires extension.tabType".to_owned(),
                            )
                        })?;
                    let record = self
                        .runtime
                        .catalog()
                        .records()
                        .get(extension_id)
                        .filter(|record| record.enabled)
                        .ok_or_else(|| {
                            ExtensionApiError::InvalidArguments(format!(
                                "extension '{extension_id}' is not loaded"
                            ))
                        })?;
                    let tab_type = record.extension.manifest.tab_type(tab_type_id).ok_or_else(
                        || {
                            ExtensionApiError::InvalidArguments(format!(
                                "unknown tab type '{tab_type_id}' for extension '{extension_id}'"
                            ))
                        },
                    )?;
                    let singleton = extension
                        .get("singleton")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let data = extension
                        .get("data")
                        .cloned()
                        .filter(|value| !value.is_null());
                    if singleton
                        && let Some(existing) = state.active_tab_workspace().and_then(|workspace| {
                            workspace
                                .root
                                .iter()
                                .flat_map(|root| root.tabs())
                                .find(|tab| {
                                    tab.kind == TabKind::ExtensionWebView
                                        && tab.extension_id.as_deref() == Some(extension_id)
                                        && tab.extension_web_view_id.as_deref() == Some(tab_type_id)
                                })
                        })
                    {
                        let existing_id = existing.id.clone();
                        if let Some(data) = data {
                            self.effect(ExtensionAppEffect::SetTabData {
                                tab_id: existing_id.clone(),
                                data,
                            })?;
                        }
                        self.effect(ExtensionAppEffect::FocusTab(existing_id.clone()))?;
                        return Ok(Value::String(existing_id));
                    }
                    let mut tab = Tab::new(TabKind::ExtensionWebView);
                    tab.project_path = project_path;
                    tab.static_title = Some(tab_type.title.clone());
                    tab.extension_id = Some(extension_id.to_owned());
                    tab.extension_web_view_id = Some(tab_type_id.to_owned());
                    tab.extension_data = data.or_else(|| tab_type.default_data.clone());
                    tab
                }
                _ => {
                    return Err(ExtensionApiError::InvalidArguments(format!(
                        "unsupported tab kind '{kind}'"
                    )));
                }
            };
        let tab_id = tab.id.clone();
        self.effect(ExtensionAppEffect::OpenTab {
            tab,
            directory,
            command,
        })?;
        Ok(Value::String(tab_id))
    }

    fn open_terminal_tab(&mut self) -> Result<Value, ExtensionApiError> {
        let project_path = self
            .state()?
            .active_project()
            .map(|project| project.path.clone());
        let mut tab = Tab::new(TabKind::Terminal);
        tab.project_path = project_path;
        let tab_id = tab.id.clone();
        self.effect(ExtensionAppEffect::OpenTab {
            tab,
            directory: None,
            command: None,
        })?;
        Ok(Value::String(tab_id))
    }

    fn list_tabs(&self) -> Result<Value, ExtensionApiError> {
        let state = self.state()?;
        let Some(workspace) = state.active_tab_workspace() else {
            return Ok(Value::Array(Vec::new()));
        };
        let focused = workspace.focused_root_tab_id();
        Ok(Value::Array(
            workspace
                .root
                .iter()
                .flat_map(|root| root.tabs())
                .map(|tab| {
                    json!({
                        "id": tab.id,
                        "title": tab.title(),
                        "kind": match tab.kind {
                            TabKind::Terminal => "terminal",
                            TabKind::Browser => "browser",
                            TabKind::ExtensionWebView => "extensionWebView",
                        },
                        "active": focused == Some(tab.id.as_str()),
                        "extensionID": tab.extension_id,
                        "tabTypeID": tab.extension_web_view_id,
                        "data": tab.extension_data,
                    })
                })
                .collect(),
        ))
    }

    fn open_webview_modal(
        &mut self,
        request: &ExtensionApiRequest,
    ) -> Result<Value, ExtensionApiError> {
        let entry = required_string(request, "entry")?.to_owned();
        let record = self
            .runtime
            .catalog()
            .records()
            .get(&request.extension_id)
            .filter(|record| record.enabled)
            .ok_or_else(|| {
                ExtensionApiError::InvalidArguments("extension is not loaded".to_owned())
            })?;
        if record
            .extension
            .resolve_resource(&entry)
            .is_none_or(|path| !path.is_file())
        {
            return Err(ExtensionApiError::InvalidArguments(format!(
                "modal entry '{entry}' is unavailable"
            )));
        }
        let request_id = format!("modal-{}", muxy_core::store::new_uuid());
        self.effect(ExtensionAppEffect::OpenModal {
            extension_id: request.extension_id.clone(),
            entry,
            width: optional_number(request, "width")?,
            height: optional_number(request, "height")?,
            dismiss_on_outside_click: request
                .args
                .get("dismissOnOutsideClick")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            data: request
                .args
                .get("data")
                .cloned()
                .filter(|value| !value.is_null()),
            request_id: Some(request_id.clone()),
            opener_surface_id: request.surface_id.clone(),
        })?;
        Ok(json!({"requestID": request_id}))
    }

    fn emit_local_event(
        &mut self,
        request: &ExtensionApiRequest,
    ) -> Result<Value, ExtensionApiError> {
        let event = required_string(request, "event")?.to_owned();
        let payload = request.args.get("payload").cloned().unwrap_or(Value::Null);
        let payload = serde_json::to_vec(&payload).map_err(|_| {
            ExtensionApiError::InvalidArguments("event payload is not serializable".to_owned())
        })?;
        let event = muxy_proto::extension::ExtensionLocalEvent {
            name: event,
            payload,
        };
        if event.encode().is_none() {
            return Err(ExtensionApiError::InvalidArguments(
                "invalid extension-local event".to_owned(),
            ));
        }
        self.runtime.push_local_event(&request.extension_id, event);
        Ok(Value::Null)
    }
}

pub(crate) fn extension_display_name(runtime: &ExtensionRuntime, extension_id: &str) -> String {
    runtime
        .catalog()
        .records()
        .get(extension_id)
        .map(|record| record.extension.display_name().to_owned())
        .unwrap_or_else(|| extension_id.to_owned())
}

fn parse_icon(value: &Value) -> Option<ExtensionIcon> {
    match value {
        Value::String(symbol) if !symbol.is_empty() => Some(ExtensionIcon::Symbol(symbol.clone())),
        Value::Object(values) => values
            .get("symbol")
            .and_then(Value::as_str)
            .filter(|symbol| !symbol.is_empty())
            .map(|symbol| ExtensionIcon::Symbol(symbol.to_owned()))
            .or_else(|| {
                values
                    .get("svg")
                    .and_then(Value::as_str)
                    .filter(|svg| !svg.is_empty())
                    .map(|svg| ExtensionIcon::Svg(svg.to_owned()))
            }),
        _ => None,
    }
}

fn unknown_item(kind: &str, item_id: &str) -> ExtensionApiError {
    ExtensionApiError::InvalidArguments(format!("unknown {kind} item '{item_id}'"))
}

fn calling_surface_id(request: &ExtensionApiRequest) -> Result<String, ExtensionApiError> {
    request.surface_id.clone().ok_or_else(|| {
        ExtensionApiError::InvalidArguments(format!(
            "{} requires an originating webview surface",
            request.method
        ))
    })
}

fn calling_tab_id(
    request: &ExtensionApiRequest,
    surfaces: &ExtensionSurfaceRegistry,
) -> Result<String, ExtensionApiError> {
    let surface_id = calling_surface_id(request)?;
    ensure_surface_kind(surfaces, &surface_id, |kind| {
        matches!(kind, ExtensionSurfaceKind::Tab(_))
    })?;
    Ok(surface_id)
}

fn ensure_surface_kind(
    surfaces: &ExtensionSurfaceRegistry,
    surface_id: &str,
    accepts: impl FnOnce(&ExtensionSurfaceKind) -> bool,
) -> Result<(), ExtensionApiError> {
    let surface = surfaces.surface(surface_id).ok_or_else(|| {
        ExtensionApiError::InvalidArguments("calling surface is no longer available".to_owned())
    })?;
    if accepts(&surface.kind) {
        Ok(())
    } else {
        Err(ExtensionApiError::InvalidArguments(
            "method is unavailable from this surface kind".to_owned(),
        ))
    }
}

fn ensure_panel(
    runtime: &ExtensionRuntime,
    extension_id: &str,
    panel_id: &str,
) -> Result<(), ExtensionApiError> {
    if runtime
        .catalog()
        .records()
        .get(extension_id)
        .filter(|record| record.enabled)
        .is_some_and(|record| record.extension.manifest.panel(panel_id).is_some())
    {
        Ok(())
    } else {
        Err(ExtensionApiError::InvalidArguments(format!(
            "unknown panel '{panel_id}'"
        )))
    }
}

fn validate_event_subscription(
    runtime: &ExtensionRuntime,
    request: &ExtensionApiRequest,
) -> Result<(), ExtensionApiError> {
    let event = required_string(request, "event")?;
    if muxy_proto::extension::is_valid_extension_local_event_name(event) {
        return Ok(());
    }
    let record = runtime
        .catalog()
        .records()
        .get(&request.extension_id)
        .filter(|record| record.enabled)
        .ok_or_else(|| ExtensionApiError::InvalidArguments("extension is not loaded".to_owned()))?;
    let command_event = event.strip_prefix("command.").is_some_and(|id| {
        record
            .extension
            .manifest
            .commands
            .iter()
            .any(|command| command.id == id)
    });
    if !command_event
        && !record
            .extension
            .manifest
            .events
            .iter()
            .any(|item| item == event)
    {
        return Err(ExtensionApiError::InvalidArguments(format!(
            "event '{event}' is not declared in the manifest"
        )));
    }
    if let Some(permission) = muxy_core::extensions::contract::required_event_permission(event)
        && !request.granted_permissions.contains(permission.as_str())
    {
        return Err(ExtensionApiError::PermissionDenied(
            permission.as_str().to_owned(),
        ));
    }
    Ok(())
}

fn required_number(request: &ExtensionApiRequest, key: &str) -> Result<f64, ExtensionApiError> {
    optional_number(request, key)?.ok_or_else(|| {
        ExtensionApiError::InvalidArguments(format!("{} requires {key}", request.method))
    })
}

fn optional_number(
    request: &ExtensionApiRequest,
    key: &str,
) -> Result<Option<f64>, ExtensionApiError> {
    let Some(value) = request.args.get(key) else {
        return Ok(None);
    };
    value
        .as_f64()
        .filter(|value| value.is_finite())
        .map(Some)
        .ok_or_else(|| {
            ExtensionApiError::InvalidArguments(format!(
                "{} requires a finite {key}",
                request.method
            ))
        })
}

fn required_string<'a>(
    request: &'a ExtensionApiRequest,
    key: &str,
) -> Result<&'a str, ExtensionApiError> {
    request
        .args
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ExtensionApiError::InvalidArguments(format!(
                "{} requires a non-empty {key}",
                request.method
            ))
        })
}

fn storage_error(error: impl ToString) -> ExtensionApiError {
    ExtensionApiError::Service(error.to_string())
}

fn runtime_error(error: ExtensionRuntimeError) -> ExtensionApiError {
    match error {
        ExtensionRuntimeError::UnknownExtension(_) => {
            ExtensionApiError::Service("unknown extension".to_owned())
        }
        ExtensionRuntimeError::UnknownSetting { key, .. } => {
            ExtensionApiError::Service(format!("setting '{key}' not declared in manifest"))
        }
        ExtensionRuntimeError::State(
            muxy_core::extensions::state::ExtensionStateError::SettingValueTooLarge,
        ) => ExtensionApiError::Service(format!(
            "value exceeds {}-byte limit",
            muxy_core::extensions::state::MAX_SETTING_VALUE_BYTES
        )),
        error => ExtensionApiError::Service(error.to_string()),
    }
}
