use super::MainWindow;
use gpui::{Context, Window};
use muxy_api::extensions::{
    ExtensionApiError, decode_webview_request, encode_webview_result, preflight_request,
};
use muxy_core::workspace::TabKind;
use std::collections::{BTreeSet, HashSet};

impl MainWindow {
    pub(crate) fn apply_extension_app_effects(
        &mut self,
        effects: Vec<crate::extensions::api::ExtensionAppEffect>,
        cx: &mut Context<Self>,
    ) {
        use crate::extensions::api::ExtensionAppEffect;
        for effect in effects {
            match effect {
                ExtensionAppEffect::OpenTab {
                    tab,
                    directory,
                    command,
                } => {
                    let tab_id = tab.id.clone();
                    if self
                        .state
                        .active_tab_workspace_mut()
                        .and_then(|workspace| workspace.new_top_level_tab(tab))
                        .is_none()
                    {
                        continue;
                    }
                    if let Some(directory) = directory {
                        self.terminal_runtime.surfaces.queue_launch_directory(
                            tab_id.clone(),
                            std::path::PathBuf::from(directory),
                        );
                    }
                    if let Some(command) = command {
                        self.terminal_runtime.surfaces.queue_launch_command(
                            tab_id,
                            crate::terminal::LaunchCommand {
                                command,
                                keeps_shell_open: true,
                            },
                        );
                    }
                    let _ = self.state.persist_tab_workspaces();
                }
                ExtensionAppEffect::FocusTab(identifier) => {
                    let tab_id = self
                        .state
                        .active_tab_workspace()
                        .and_then(|workspace| workspace.tab(&identifier))
                        .map(|tab| tab.root_id().to_owned());
                    if let Some(tab_id) = tab_id {
                        self.select_root_tab(&tab_id, cx);
                    } else if let Ok(index) = identifier.parse::<usize>() {
                        self.select_root_index(index.saturating_sub(1), cx);
                    }
                }
                ExtensionAppEffect::CycleTab(direction) => {
                    self.select_relative_root(direction, cx);
                }
                ExtensionAppEffect::SetTabTitle { tab_id, title } => {
                    self.set_tab_title(&tab_id, title, cx);
                }
                ExtensionAppEffect::SetTabIcon { tab_id, icon } => {
                    let value = icon.map(|icon| match icon {
                        muxy_core::extensions::manifest::ExtensionIcon::Symbol(symbol) => symbol,
                        muxy_core::extensions::manifest::ExtensionIcon::Svg(path) => path,
                    });
                    if let Some(tab) = self
                        .state
                        .active_tab_workspace_mut()
                        .and_then(|workspace| workspace.tab_mut(&tab_id))
                    {
                        tab.custom_icon = value;
                        let _ = self.state.persist_tab_workspaces();
                    }
                }
                ExtensionAppEffect::SetTabData { tab_id, data } => {
                    if let Some(tab) = self
                        .state
                        .active_tab_workspace_mut()
                        .and_then(|workspace| workspace.tab_mut(&tab_id))
                    {
                        tab.extension_data = Some(data);
                        let _ = self.state.persist_tab_workspaces();
                    }
                }
                ExtensionAppEffect::OpenPanel {
                    extension_id,
                    panel_id,
                    data,
                    toggle,
                } => {
                    let result = if toggle {
                        self.extension_surfaces.toggle_panel(
                            self.extension_runtime.catalog(),
                            &extension_id,
                            &panel_id,
                            data,
                        )
                    } else {
                        self.extension_surfaces.open_panel(
                            self.extension_runtime.catalog(),
                            &extension_id,
                            &panel_id,
                            data,
                        )
                    };
                    match result {
                        Ok(changes) => self.apply_extension_surface_changes(changes, cx),
                        Err(error) => log::warn!("failed to open extension panel: {error}"),
                    }
                }
                ExtensionAppEffect::ClosePanel {
                    extension_id,
                    panel_id,
                } => {
                    let changes = self
                        .extension_surfaces
                        .close_panel(&extension_id, &panel_id);
                    self.apply_extension_surface_changes(changes, cx);
                }
                ExtensionAppEffect::CloseSurface(surface_id) => {
                    self.close_extension_surface(&surface_id, cx);
                }
                ExtensionAppEffect::ResizePopover {
                    surface_id,
                    width,
                    height,
                } => {
                    if let Err(error) =
                        self.extension_surfaces
                            .resize_popover(&surface_id, width, height)
                    {
                        log::warn!("failed to resize extension popover: {error}");
                    }
                }
                ExtensionAppEffect::OpenModal {
                    extension_id,
                    entry,
                    width,
                    height,
                    dismiss_on_outside_click,
                    data,
                    request_id,
                    opener_surface_id,
                } => match self.extension_surfaces.open_modal(
                    self.extension_runtime.catalog(),
                    &extension_id,
                    request_id,
                    &entry,
                    width,
                    height,
                    dismiss_on_outside_click,
                    data,
                    opener_surface_id,
                ) {
                    Ok(changes) => self.apply_extension_surface_changes(changes, cx),
                    Err(error) => log::warn!("failed to open extension modal: {error}"),
                },
                ExtensionAppEffect::ResolveModal { request_id, result } => {
                    self.extension_modal_results
                        .insert(request_id.clone(), result);
                    self.close_extension_surface_confirmed(&request_id, cx);
                }
                ExtensionAppEffect::CloseActiveModal { extension_id } => {
                    if let Some(surface_id) = self
                        .extension_surfaces
                        .active_modal()
                        .filter(|surface| surface.extension_id == extension_id)
                        .map(|surface| surface.instance_id.clone())
                    {
                        self.close_extension_surface(&surface_id, cx);
                    }
                }
                ExtensionAppEffect::Notify { title, body } => {
                    self.feedback(title, body, crate::toast::ToastTone::Success, cx);
                }
                ExtensionAppEffect::RebindShortcuts => self.rebind_shortcuts(cx),
            }
        }
        cx.notify();
    }

    pub(super) fn reconcile_extension_webviews(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.drain_extension_surface_events(cx);
        self.extension_surfaces
            .sync_tabs(&self.state.tab_workspaces, self.extension_runtime.catalog());
        let selected_sidebar = muxy_core::prefs::settings::string_value("muxy.activeSidebar", "");
        let sidebar_changes = self.extension_surfaces.sync_sidebar_selection(
            self.extension_runtime.catalog(),
            (!selected_sidebar.is_empty()).then_some(selected_sidebar.as_str()),
        );
        self.apply_extension_surface_changes(sidebar_changes, cx);
        let project_id = self.state.active_project_id.as_deref();
        let project_changes = self.extension_surfaces.set_active_project(project_id);
        self.apply_extension_surface_changes(project_changes, cx);
        let descriptors =
            crate::extensions::webview::extension_webview_descriptors(&self.extension_surfaces);
        let mut visible = self
            .state
            .active_tab_workspace()
            .map(|workspace| {
                workspace
                    .visible_area_tabs()
                    .into_iter()
                    .filter_map(|(_, tab_id)| {
                        workspace
                            .tab(&tab_id)
                            .is_some_and(|tab| tab.kind == TabKind::ExtensionWebView)
                            .then_some(tab_id)
                    })
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let sidebar_visible = self.view.sidebar_expanded
            || self.state.prefs.collapsed_style != muxy_core::prefs::CollapsedStyle::Hidden;
        for surface in self.extension_surfaces.webview_surfaces() {
            let visible_surface = match surface.kind {
                crate::extensions::surfaces::ExtensionSurfaceKind::Tab(_) => false,
                crate::extensions::surfaces::ExtensionSurfaceKind::Sidebar(_) => sidebar_visible,
                crate::extensions::surfaces::ExtensionSurfaceKind::Home(_) => self
                    .extension_surfaces
                    .active_home()
                    .is_some_and(|home| home.instance_id == surface.instance_id),
                _ => true,
            };
            if visible_surface {
                visible.insert(surface.instance_id.clone());
            }
        }
        if let Some(modal) = self.extension_surfaces.active_modal() {
            visible.clear();
            visible.insert(modal.instance_id.clone());
        }
        let overlay_active = self.view.overlay.is_open()
            || self.composer_is_open()
            || self.extension_runtime.pending_consent().is_some();
        if overlay_active {
            visible.clear();
        }
        let focused = (!overlay_active && window.is_window_active())
            .then(|| {
                self.extension_surfaces
                    .focused_surface_id()
                    .filter(|surface_id| visible.contains(*surface_id))
                    .map(str::to_owned)
                    .or_else(|| {
                        self.focused_tab_id()
                            .filter(|tab_id| visible.contains(tab_id))
                    })
            })
            .flatten();
        let theme = crate::extensions::webview::extension_theme_snapshot(
            &self.state.theme,
            self.state.appearance,
            self.state.metrics.title_bar_height(),
        );
        self.extension_webviews.reconcile(
            &descriptors,
            &visible,
            focused.as_deref(),
            &theme,
            self.state.theme.bg.into(),
        );
    }

    pub(super) fn focus_extension_webview(&mut self, surface_id: &str, cx: &mut Context<Self>) {
        if self.extension_surfaces.focus(surface_id)
            && self
                .extension_surfaces
                .surface(surface_id)
                .is_some_and(|surface| {
                    !matches!(
                        surface.kind,
                        crate::extensions::surfaces::ExtensionSurfaceKind::Tab(_)
                    )
                })
        {
            cx.notify();
            return;
        }
        let area_id = self
            .state
            .active_tab_workspace()
            .and_then(|workspace| visible_extension_webview_area(workspace, surface_id));
        if let Some(area_id) = area_id {
            self.focus_workspace_tab(&area_id, surface_id, cx);
        }
    }

    pub(super) fn dispatch_extension_webview_call(
        &mut self,
        call: crate::extensions::webview::ExtensionWebViewApiCall,
        cx: &mut Context<Self>,
    ) {
        if !self.extension_surface_matches(&call.surface_id, &call.extension_id) {
            let request_id = call
                .message
                .get("requestID")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            call.complete(encode_webview_result(
                request_id,
                Err(ExtensionApiError::InvalidArguments(
                    "extension surface is no longer available".to_owned(),
                )),
            ));
            return;
        }
        let permissions = self
            .extension_runtime
            .catalog()
            .records()
            .get(&call.extension_id)
            .filter(|record| record.enabled)
            .map(|record| {
                record
                    .extension
                    .manifest
                    .permissions
                    .iter()
                    .map(|permission| permission.as_str().to_owned())
                    .collect::<BTreeSet<_>>()
            });
        let Some(permissions) = permissions else {
            let request_id = call
                .message
                .get("requestID")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            let extension_id = call.extension_id.clone();
            call.complete(encode_webview_result(
                request_id,
                Err(ExtensionApiError::InvalidArguments(format!(
                    "extension '{extension_id}' is not loaded"
                ))),
            ));
            return;
        };
        let mut request =
            match decode_webview_request(&call.extension_id, permissions, call.message.clone()) {
                Ok(request) => request,
                Err(error) => {
                    let request_id = call
                        .message
                        .get("requestID")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                    call.complete(encode_webview_result(request_id, Err(error)));
                    return;
                }
            };
        request.surface_id = Some(call.surface_id.clone());
        if let Err(error) = preflight_request(&request) {
            let request_id = request.request_id.clone();
            call.complete(encode_webview_result(request_id, Err(error)));
            return;
        }
        if let Some(result) = self
            .extension_webviews
            .handle_lifecycle_message(&call.surface_id, &request)
        {
            let close_self = request.method == "lifecycle.closeSelf";
            let request_id = request.request_id.clone();
            let surface_id = call.surface_id.clone();
            call.complete(encode_webview_result(request_id, result));
            if close_self {
                self.close_extension_surface_confirmed(&surface_id, cx);
            }
            return;
        }
        let request_id = request.request_id.clone();
        self.dispatch_extension_api_request(
            request,
            Box::new(move |result| {
                call.complete(encode_webview_result(request_id, result));
            }),
            cx,
        );
    }

    pub(super) fn request_extension_webview_close(
        &mut self,
        surface_id: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let reason = self
            .extension_surfaces
            .surface(surface_id)
            .map(|surface| surface.kind.lifecycle_reason())
            .unwrap_or("tab");
        let Some((call_id, verdicts)) = self
            .extension_webviews
            .request_before_close(surface_id, reason)
        else {
            return false;
        };
        let timeout_call_id = call_id.clone();
        cx.spawn(async move |window, cx| {
            cx.background_executor()
                .timer(crate::extensions::webview::LIFECYCLE_ACKNOWLEDGEMENT_TIMEOUT)
                .await;
            let _ = window.update(cx, |window, _| {
                window
                    .extension_webviews
                    .expire_unacknowledged(&timeout_call_id);
            });
        })
        .detach();
        let surface_id = surface_id.to_owned();
        cx.spawn(async move |window, cx| {
            let Ok(verdict) = verdicts.recv().await else {
                return;
            };
            if verdict == crate::extensions::webview::ExtensionLifecycleVerdict::Allow {
                let _ = window.update(cx, |window, cx| {
                    window.close_extension_surface_confirmed(&surface_id, cx);
                });
            }
        })
        .detach();
        true
    }

    pub(crate) fn close_extension_surface(&mut self, surface_id: &str, cx: &mut Context<Self>) {
        if !self.request_extension_webview_close(surface_id, cx) {
            self.close_extension_surface_confirmed(surface_id, cx);
        }
    }

    fn close_extension_surface_confirmed(&mut self, surface_id: &str, cx: &mut Context<Self>) {
        let tab = self
            .extension_surfaces
            .surface(surface_id)
            .is_some_and(|surface| {
                matches!(
                    surface.kind,
                    crate::extensions::surfaces::ExtensionSurfaceKind::Tab(_)
                )
            });
        if tab {
            self.close_tab_confirmed(surface_id, cx);
            return;
        }
        let changes = self.extension_surfaces.close(surface_id);
        self.apply_extension_surface_changes(changes, cx);
        cx.notify();
    }

    fn extension_surface_matches(&self, surface_id: &str, extension_id: &str) -> bool {
        self.extension_surfaces
            .surface(surface_id)
            .is_some_and(|surface| surface.extension_id == extension_id)
    }

    pub(crate) fn publish_extension_surface_changes(
        &self,
        changes: &crate::extensions::surfaces::ExtensionSurfaceChanges,
    ) {
        for surface in &changes.closed {
            if let Some((name, payload)) = surface_event(surface, false) {
                self.publish_extension_event(&name, payload);
            }
        }
        for surface in &changes.opened {
            if let Some((name, payload)) = surface_event(surface, true) {
                self.publish_extension_event(&name, payload);
            }
        }
    }

    pub(crate) fn apply_extension_surface_changes(
        &mut self,
        mut changes: crate::extensions::surfaces::ExtensionSurfaceChanges,
        cx: &mut Context<Self>,
    ) {
        for surface in &changes.closed {
            if matches!(
                surface.kind,
                crate::extensions::surfaces::ExtensionSurfaceKind::Panel(_)
            ) {
                self.panels
                    .close(&muxy_ui::panel::PanelId::from(surface.instance_id.clone()));
            }
            if let crate::extensions::surfaces::ExtensionSurfaceKind::Modal {
                opener_surface_id,
                ..
            } = &surface.kind
            {
                let result = self
                    .extension_modal_results
                    .remove(&surface.instance_id)
                    .unwrap_or(serde_json::Value::Null);
                self.deliver_extension_modal_result(
                    &surface.extension_id,
                    opener_surface_id.as_deref(),
                    &surface.instance_id,
                    result,
                );
            }
        }
        let opened_panel_ids = changes
            .opened
            .iter()
            .filter(|surface| {
                matches!(
                    surface.kind,
                    crate::extensions::surfaces::ExtensionSurfaceKind::Panel(_)
                ) && surface.project_id.as_deref() == self.state.active_project_id.as_deref()
            })
            .map(|surface| surface.instance_id.clone())
            .collect::<Vec<_>>();
        for surface_id in opened_panel_ids {
            if !self.activate_extension_panel(&surface_id, cx) {
                let rejected = self.extension_surfaces.close(&surface_id);
                changes.closed.extend(rejected.closed);
                changes
                    .opened
                    .retain(|surface| surface.instance_id != surface_id);
            }
        }
        self.publish_extension_surface_changes(&changes);
    }

    fn deliver_extension_modal_result(
        &self,
        extension_id: &str,
        opener_surface_id: Option<&str>,
        request_id: &str,
        result: serde_json::Value,
    ) {
        if let Some(opener_surface_id) = opener_surface_id {
            self.extension_webviews
                .deliver_modal_result(opener_surface_id, request_id, &result);
            return;
        }
        if let Ok(payload) = serde_json::to_vec(&result) {
            self.extension_runtime.push_modal_result(
                extension_id,
                muxy_proto::extension::ModalResult {
                    request_id: request_id.to_owned(),
                    payload,
                },
            );
        }
    }

    pub(crate) fn activate_extension_panel(
        &mut self,
        surface_id: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(placement) = self.extension_panel_placement(surface_id) else {
            return false;
        };
        let composer_id = muxy_ui::panel::PanelId::from(muxy_core::composer::PANEL_ID);
        if self.panels.host().occupant(placement.slot()) == Some(&composer_id) {
            self.close_composer(cx);
            if self.panels.host().occupant(placement.slot()) == Some(&composer_id) {
                return false;
            }
        }
        self.place_panel(placement);
        true
    }

    fn extension_panel_placement(
        &self,
        surface_id: &str,
    ) -> Option<muxy_ui::panel::PanelPlacement> {
        let surface = self.extension_surfaces.surface(surface_id)?;
        let crate::extensions::surfaces::ExtensionSurfaceKind::Panel(panel) = &surface.kind else {
            return None;
        };
        let position = match panel.position {
            muxy_core::extensions::manifest::PanelPosition::Right => {
                muxy_ui::panel::PanelPosition::Right
            }
            muxy_core::extensions::manifest::PanelPosition::Bottom => {
                muxy_ui::panel::PanelPosition::Bottom
            }
        };
        let mode = match panel.mode {
            muxy_core::extensions::manifest::PanelMode::Pinned => muxy_ui::panel::PanelMode::Pinned,
            muxy_core::extensions::manifest::PanelMode::Floating => {
                muxy_ui::panel::PanelMode::Floating
            }
        };
        Some(muxy_ui::panel::PanelPlacement::new(
            surface_id.to_owned(),
            position,
            mode,
        ))
    }

    pub(super) fn publish_extension_event(&self, name: &str, payload: serde_json::Value) {
        let payload_map = payload
            .as_object()
            .into_iter()
            .flatten()
            .map(|(key, value)| {
                let value = value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string());
                (key.clone(), value)
            })
            .collect();
        self.extension_runtime
            .broadcast(muxy_proto::extension::ExtensionBroadcast {
                name: name.to_owned(),
                payload: payload_map,
            });
        for (extension_id, record) in self.extension_runtime.catalog().records() {
            if record.enabled
                && record
                    .extension
                    .manifest
                    .events
                    .iter()
                    .any(|event| event == name)
            {
                self.extension_webviews
                    .dispatch_event(Some(extension_id), name, &payload);
            }
        }
    }

    fn drain_extension_surface_events(&mut self, cx: &mut Context<Self>) {
        for event in self.extension_runtime.take_surface_events() {
            match event {
                crate::extensions::ExtensionSurfaceEvent::LocalEvent(event) => {
                    let payload = serde_json::from_slice(&event.event.payload)
                        .unwrap_or(serde_json::Value::Null);
                    self.extension_webviews.dispatch_event(
                        Some(&event.extension_id),
                        &event.event.name,
                        &payload,
                    );
                }
                crate::extensions::ExtensionSurfaceEvent::CloseAll { extension_id } => {
                    let changes = self.extension_surfaces.close_extension(&extension_id);
                    self.apply_extension_surface_changes(changes, cx);
                    let tab_ids = self
                        .state
                        .tab_workspaces
                        .states()
                        .iter()
                        .flat_map(|workspace| workspace.root.iter().flat_map(|root| root.tabs()))
                        .filter(|tab| {
                            tab.kind == TabKind::ExtensionWebView
                                && tab.extension_id.as_deref() == Some(extension_id.as_str())
                        })
                        .map(|tab| tab.id.clone())
                        .collect::<Vec<_>>();
                    if !tab_ids.is_empty() {
                        for workspace in self.state.tab_workspaces.states_mut() {
                            for tab_id in &tab_ids {
                                workspace
                                    .close_tab(tab_id, muxy_core::workspace::CloseMode::Single);
                            }
                        }
                        let _ = self.state.persist_tab_workspaces();
                    }
                }
            }
        }
    }
}

fn surface_event(
    surface: &crate::extensions::surfaces::ExtensionSurface,
    opened: bool,
) -> Option<(String, serde_json::Value)> {
    use crate::extensions::surfaces::ExtensionSurfaceKind;
    match &surface.kind {
        ExtensionSurfaceKind::Panel(panel) => Some((
            format!("panel.{}", if opened { "opened" } else { "closed" }),
            serde_json::json!({
                "extensionID": surface.extension_id,
                "panelID": panel.id,
            }),
        )),
        ExtensionSurfaceKind::Popover(popover) => Some((
            format!("popover.{}", if opened { "opened" } else { "closed" }),
            serde_json::json!({
                "extensionID": surface.extension_id,
                "popoverID": popover.id,
            }),
        )),
        _ => None,
    }
}

fn visible_extension_webview_area(
    workspace: &muxy_core::workspace::WorkspaceState,
    surface_id: &str,
) -> Option<String> {
    let area = workspace.area_containing_tab(surface_id)?;
    let tab = area.tab(surface_id)?;
    if tab.kind != TabKind::ExtensionWebView
        || area.active_tab_id.as_deref() != Some(surface_id)
        || !workspace
            .visible_area_tabs()
            .iter()
            .any(|(area_id, tab_id)| area_id == &area.id && tab_id == surface_id)
    {
        return None;
    }
    Some(area.id.clone())
}

#[cfg(test)]
mod tests {
    use super::visible_extension_webview_area;
    use muxy_core::workspace::{Tab, TabKind, WorkspaceState};

    fn tab(id: &str, kind: TabKind) -> Tab {
        let mut tab = Tab::new(kind);
        tab.id = id.to_owned();
        tab
    }

    #[test]
    fn focus_routing_accepts_only_the_visible_active_extension_surface() {
        let mut workspace = WorkspaceState::new("project");
        workspace.new_top_level_tab(tab("extension", TabKind::ExtensionWebView));
        let area_id = workspace
            .area_containing_tab("extension")
            .unwrap()
            .id
            .clone();
        workspace.area_mut(&area_id).unwrap().insert_tab(
            usize::MAX,
            tab("inactive", TabKind::ExtensionWebView),
            false,
        );

        assert_eq!(
            visible_extension_webview_area(&workspace, "extension"),
            Some(area_id)
        );
        assert_eq!(visible_extension_webview_area(&workspace, "inactive"), None);
        assert_eq!(visible_extension_webview_area(&workspace, "missing"), None);
    }

    #[test]
    fn focus_routing_rejects_non_extension_tabs() {
        let mut workspace = WorkspaceState::new("project");
        workspace.new_top_level_tab(tab("terminal", TabKind::Terminal));

        assert_eq!(visible_extension_webview_area(&workspace, "terminal"), None);
    }
}
