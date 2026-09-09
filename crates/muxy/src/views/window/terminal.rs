use super::*;

fn desktop_notification_title(title: String) -> String {
    if title.is_empty() {
        "Command executed!".to_owned()
    } else {
        title
    }
}

fn desktop_notification_is_sound_only(window_active: bool, target_focused: bool) -> bool {
    window_active && target_focused
}

fn terminal_input_overlay_active(
    overlay_open: bool,
    search_open: bool,
    composer_input_focused: bool,
) -> bool {
    overlay_open || search_open || composer_input_focused
}

pub(crate) struct TerminalRuntime {
    pub(crate) surfaces: TerminalSurfaces,
    pub(super) _tasks: Vec<Task<()>>,
}

impl TerminalRuntime {
    pub(super) fn new(surfaces: TerminalSurfaces, tasks: Vec<Task<()>>) -> Self {
        Self {
            surfaces,
            _tasks: tasks,
        }
    }
}

impl MainWindow {
    pub(crate) fn submit_quick_terminal_notification(
        &mut self,
        title: String,
        body: String,
        focused_osc: bool,
        cx: &mut Context<Self>,
    ) {
        self.submit_notification(
            crate::notifications::ResolvedNotificationEvent {
                target: None,
                source: muxy_core::notifications::NotificationSource::Osc,
                origin: crate::notifications::NotificationOrigin::TerminalOsc,
                title: desktop_notification_title(title),
                body,
                timestamp: muxy_core::store::reference_now(),
            },
            focused_osc,
            cx,
        );
    }

    pub(super) fn elapsed(&self) -> Duration {
        self.view.terminal.started_at.elapsed()
    }

    pub(super) fn on_runtime_event(&mut self, event: TerminalEvent, cx: &mut Context<Self>) {
        let Some(event) = self.terminal_runtime.surfaces.route(event, cx) else {
            return;
        };
        let (tab_id, signal) = match event {
            crate::terminal::RoutedTerminalEvent::Workspace(tab_id, signal) => (tab_id, signal),
            crate::terminal::RoutedTerminalEvent::WorkspaceOpenUrl(tab_id, url) => {
                self.open_terminal_url(&tab_id, &url, cx);
                return;
            }
            crate::terminal::RoutedTerminalEvent::Standalone(_)
            | crate::terminal::RoutedTerminalEvent::StandaloneOpenUrl(_) => return,
        };
        match signal {
            SurfaceSignal::DesktopNotification { title, body } => {
                let Some(target) = self.state.notification_target_for_pane(&tab_id) else {
                    return;
                };
                let focused_osc = desktop_notification_is_sound_only(
                    self.view.window_active,
                    self.state.notification_target_is_focused(&target),
                );
                self.submit_notification(
                    crate::notifications::ResolvedNotificationEvent {
                        target: Some(target),
                        source: muxy_core::notifications::NotificationSource::Osc,
                        origin: crate::notifications::NotificationOrigin::TerminalOsc,
                        title: desktop_notification_title(title),
                        body,
                        timestamp: muxy_core::store::reference_now(),
                    },
                    focused_osc,
                    cx,
                );
            }
            SurfaceSignal::Exited => {
                if self
                    .terminal_runtime
                    .surfaces
                    .handle_exit(&mut self.state.tab_workspaces, &tab_id)
                {
                    let _ = self.state.persist_tab_workspaces();
                }
                cx.notify();
            }
            SurfaceSignal::Confirm { .. } => self.show_next_confirmation(cx),
            signal => {
                let metadata_before = self
                    .terminal_runtime
                    .surfaces
                    .handle(&tab_id)
                    .map(|handle| handle.metadata().clone());
                if self.terminal_runtime.surfaces.apply(&tab_id, signal) {
                    let metadata_after = self
                        .terminal_runtime
                        .surfaces
                        .handle(&tab_id)
                        .map(|handle| handle.metadata().clone());
                    if !self.terminal_runtime.surfaces.has_native_scrollbar(&tab_id)
                        && metadata_after.as_ref().map(|metadata| metadata.scrollbar)
                            != metadata_before.as_ref().map(|metadata| metadata.scrollbar)
                    {
                        self.reveal_scrollbar(&tab_id, cx);
                    }
                    if let (Some(before), Some(after)) = (metadata_before, metadata_after) {
                        self.record_terminal_indicators(&tab_id, &before, &after, cx);
                    }
                    self.sync_tab_metadata(&tab_id);
                    cx.notify();
                }
            }
        }
    }

    pub(super) fn scrollbar_metrics(
        &self,
        tab_id: &str,
    ) -> Option<muxy_terminal::scrollbar::ScrollbarMetrics> {
        self.terminal_runtime
            .surfaces
            .handle(tab_id)
            .map(|handle| handle.metadata().scrollbar)
    }

    pub(super) fn record_terminal_indicators(
        &mut self,
        tab_id: &str,
        before: &muxy_terminal::backend::SurfaceMetadata,
        after: &muxy_terminal::backend::SurfaceMetadata,
        cx: &mut Context<Self>,
    ) {
        if before.progress.is_active() && !after.progress.is_active() {
            self.view.terminal.attention.insert(tab_id.to_owned());
        }
        if before.bell_generation == after.bell_generation {
            return;
        }
        self.view.terminal.attention.insert(tab_id.to_owned());
        self.view
            .terminal
            .bell_flashes
            .insert(tab_id.to_owned(), self.elapsed() + BELL_FLASH_DURATION);
        self.view.terminal.bell_expiry = Some(cx.spawn(async move |window, cx| {
            cx.background_executor().timer(BELL_FLASH_DURATION).await;
            let _ = window.update(cx, |window, cx| {
                let now = window.elapsed();
                window
                    .view
                    .terminal
                    .bell_flashes
                    .retain(|_, deadline| *deadline > now);
                cx.notify();
            });
        }));
    }

    pub(super) fn show_next_confirmation(&mut self, cx: &mut Context<Self>) {
        if self.view.overlay.is_open() {
            cx.notify();
            return;
        }
        let Some((tab_id, id, kind)) = self.terminal_runtime.surfaces.active_confirmation() else {
            cx.notify();
            return;
        };
        self.view.subscriptions.clear();
        self.view.pending_focus = Some(self.view.menu_focus.clone());
        self.view.overlay = Overlay::TerminalConfirm { tab_id, id, kind };
        cx.notify();
    }

    pub(crate) fn resolve_terminal_confirmation(&mut self, approved: bool, cx: &mut Context<Self>) {
        let Overlay::TerminalConfirm { tab_id, id, kind } = &self.view.overlay else {
            return;
        };
        let (tab_id, id, kind) = (tab_id.clone(), *id, *kind);
        self.terminal_runtime
            .surfaces
            .perform(&tab_id, SurfaceAction::ClipboardDecision { id, approved });
        self.view.overlay = Overlay::None;
        if approved && kind == ConfirmationKind::ActiveProcessClose {
            self.close_tab(&tab_id, cx);
        }
        if self
            .terminal_runtime
            .surfaces
            .active_confirmation()
            .is_none()
        {
            self.view.pending_focus = Some(self.view.workspace_focus.clone());
        }
        self.show_next_confirmation(cx);
    }

    pub(super) fn sync_tab_metadata(&mut self, tab_id: &str) {
        let Some(title) = self
            .terminal_runtime
            .surfaces
            .handle(tab_id)
            .and_then(|handle| handle.metadata().title.clone())
        else {
            return;
        };
        let mut changed = false;
        for state in self.state.tab_workspaces.states_mut() {
            if let Some(tab) = state.tab_mut(tab_id) {
                if tab.pane_title.as_deref() != Some(title.as_str()) {
                    tab.pane_title = Some(title);
                    changed = true;
                }
                break;
            }
        }
        if changed {
            let _ = self.state.persist_tab_workspaces();
        }
    }

    pub(crate) fn reconcile_terminals(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let visible: Vec<String> = self
            .state
            .active_tab_workspace()
            .map(|workspace| {
                workspace
                    .visible_area_tabs()
                    .into_iter()
                    .filter(|(_, tab_id)| {
                        workspace
                            .tab(tab_id)
                            .is_some_and(|tab| tab.kind == muxy_core::workspace::TabKind::Terminal)
                    })
                    .map(|(_, tab_id)| tab_id)
                    .collect()
            })
            .unwrap_or_default();
        self.terminal_runtime
            .surfaces
            .reconcile(&self.state.tab_workspaces, &visible, window, cx);
        self.reconcile_search_bars(&visible, window, cx);
        self.view
            .terminal
            .scrollbar_reveal
            .retain(|tab_id, _| visible.contains(tab_id));
        let terminals = &self.terminal_runtime.surfaces;
        self.view
            .terminal
            .attention
            .retain(|tab_id| terminals.handle(tab_id).is_some());
        let now = self.elapsed();
        self.view
            .terminal
            .bell_flashes
            .retain(|tab_id, deadline| terminals.handle(tab_id).is_some() && *deadline > now);
        if !self.view.overlay.is_open()
            && let Some((tab_id, id, kind)) = self.terminal_runtime.surfaces.active_confirmation()
        {
            self.view.subscriptions.clear();
            self.view.pending_focus = Some(self.view.menu_focus.clone());
            self.view.overlay = Overlay::TerminalConfirm { tab_id, id, kind };
        }

        self.reconcile_composer_target(cx);
        let overlay_open = terminal_input_overlay_active(
            self.view.overlay.is_open(),
            !self.view.terminal.search_inputs.is_empty(),
            self.composer_input_is_focused(window, cx),
        );
        if overlay_open != self.view.terminal.overlay_was_open {
            self.terminal_runtime
                .surfaces
                .set_overlay_active(overlay_open);
            self.view.terminal.overlay_was_open = overlay_open;
        }

        self.view.window_active = window.is_window_active();
        self.terminal_runtime
            .surfaces
            .set_window_active(self.view.window_active);

        let focused = if overlay_open {
            None
        } else {
            self.state
                .active_tab_workspace()
                .and_then(|workspace| {
                    let area_id = workspace.focused_area_id.as_deref()?;
                    workspace.area(area_id)?.active_tab_id.clone()
                })
                .or_else(|| visible.first().cloned())
        };
        self.terminal_runtime
            .surfaces
            .set_focused_tab(focused.as_deref());
        if window.is_window_active()
            && let Some(tab_id) = focused
        {
            self.view.terminal.attention.remove(&tab_id);
        }
    }

    pub(super) fn reconcile_search_bars(
        &mut self,
        visible: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let searching: Vec<String> = visible
            .iter()
            .filter(|tab_id| {
                self.terminal_runtime
                    .surfaces
                    .handle(tab_id)
                    .is_some_and(|handle| handle.metadata().search_totals.active)
            })
            .cloned()
            .collect();
        self.view
            .terminal
            .search_inputs
            .retain(|tab_id, _| searching.contains(tab_id));
        self.view
            .terminal
            .search_subscriptions
            .retain(|tab_id, _| searching.contains(tab_id));

        let style = self.input_style();
        for tab_id in searching {
            if self.view.terminal.search_inputs.contains_key(&tab_id) {
                continue;
            }
            let input = cx.new(|cx| {
                TextInput::new(style, cx)
                    .with_key_context(muxy_ui::text_input::SEARCH_CONTEXT)
                    .with_placeholder("Search")
            });
            let owner = tab_id.clone();
            let subscription = cx.subscribe(&input, move |window: &mut Self, input, event, cx| {
                let owner = owner.clone();
                match event {
                    InputEvent::Changed => {
                        let needle = input.read(cx).text().to_owned();
                        window.dispatch_search_query(owner, needle, cx);
                    }
                    InputEvent::Submitted => window.navigate_search(true, cx),
                    InputEvent::Cancelled => window.close_search(&owner, cx),
                }
            });
            self.view
                .terminal
                .search_subscriptions
                .insert(tab_id.clone(), subscription);
            self.view.terminal.search_inputs.insert(tab_id, input);
        }

        if let Some(tab_id) = self.view.terminal.pending_search_focus.clone()
            && let Some(input) = self.view.terminal.search_inputs.get(&tab_id)
        {
            self.view.terminal.pending_search_focus = None;
            let handle = input.read(cx).focus_handle(cx);
            window.focus(&handle);
        }
    }

    pub(crate) fn forward_pane_pointer(
        &mut self,
        tab_id: &str,
        area_id: &str,
        input: PointerInput,
        cx: &mut Context<Self>,
    ) -> bool {
        if let PointerInput::Moved { x, .. } = input {
            if self.view.terminal.scrollbar_drag.is_some() {
                return true;
            }
            self.reveal_scrollbar_near_track(tab_id, area_id, x, cx);
        }
        self.terminal_runtime
            .surfaces
            .forward_pointer(tab_id, input)
    }

    pub(super) fn release_pointer_outside_panes(&mut self, position: Point<Pixels>) {
        let Some(tab_id) = self.terminal_runtime.surfaces.pointer_tab() else {
            return;
        };
        let inside = self
            .state
            .active_tab_workspace()
            .and_then(|workspace| workspace.area_containing_tab(tab_id))
            .filter(|area| area.active_tab_id.as_deref() == Some(tab_id))
            .and_then(|area| self.view.workspace.area_bounds.get(&area.id))
            .is_some_and(|bounds| bounds.contains(&position));
        if !inside {
            self.terminal_runtime.surfaces.clear_pointer_tab();
        }
    }

    pub(crate) fn focus_pane(&mut self, tab_id: &str, area_id: &str, cx: &mut Context<Self>) {
        self.close_search_except(Some(tab_id), cx);
        self.focus_area(area_id, cx);
    }

    pub(super) fn focused_tab_id(&self) -> Option<String> {
        let workspace = self.state.active_tab_workspace()?;
        let area_id = workspace.focused_area_id.as_deref()?;
        workspace.area(area_id)?.active_tab_id.clone()
    }

    pub(crate) fn open_search(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.focused_tab_id() else {
            return;
        };
        self.terminal_runtime
            .surfaces
            .perform(&tab_id, SurfaceAction::SearchStart);
        self.view.terminal.pending_search_focus = Some(tab_id);
        cx.notify();
    }

    pub(crate) fn close_search(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        if self.view.terminal.search_inputs.remove(tab_id).is_none() {
            return;
        }
        self.view.terminal.search_subscriptions.remove(tab_id);
        self.view.terminal.search_debounce = None;
        self.terminal_runtime
            .surfaces
            .perform(tab_id, SurfaceAction::SearchEnd);
        self.view.pending_focus = Some(self.view.workspace_focus.clone());
        cx.notify();
    }

    pub(super) fn close_search_except(&mut self, keep: Option<&str>, cx: &mut Context<Self>) {
        let stale: Vec<String> = self
            .view
            .terminal
            .search_inputs
            .keys()
            .filter(|tab_id| Some(tab_id.as_str()) != keep)
            .cloned()
            .collect();
        for tab_id in stale {
            self.close_search(&tab_id, cx);
        }
    }

    pub(crate) fn navigate_search(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(tab_id) = self.searching_tab_id() else {
            return;
        };
        let action = if forward {
            SurfaceAction::SearchNext
        } else {
            SurfaceAction::SearchPrevious
        };
        self.terminal_runtime.surfaces.perform(&tab_id, action);
        cx.notify();
    }

    pub(super) fn searching_tab_id(&self) -> Option<String> {
        self.focused_tab_id()
            .filter(|tab_id| self.view.terminal.search_inputs.contains_key(tab_id))
            .or_else(|| self.view.terminal.search_inputs.keys().next().cloned())
    }

    pub(super) fn dispatch_search_query(
        &mut self,
        tab_id: String,
        needle: String,
        cx: &mut Context<Self>,
    ) {
        match dispatch_for_query(needle.clone()) {
            crate::terminal::SearchDispatch::Immediate(_) => {
                self.view.terminal.search_debounce = None;
                self.terminal_runtime
                    .surfaces
                    .perform(&tab_id, SurfaceAction::SearchQuery(needle));
                cx.notify();
            }
            crate::terminal::SearchDispatch::Debounced { delay, .. } => {
                self.view.terminal.search_debounce = Some(cx.spawn(async move |window, cx| {
                    cx.background_executor().timer(delay).await;
                    let _ = window.update(cx, |window, cx| {
                        let current = window
                            .view
                            .terminal
                            .search_inputs
                            .get(&tab_id)
                            .map(|input| input.read(cx).text().to_owned());
                        if current.as_deref() != Some(needle.as_str()) {
                            return;
                        }
                        window
                            .terminal_runtime
                            .surfaces
                            .perform(&tab_id, SurfaceAction::SearchQuery(needle));
                        cx.notify();
                    });
                }));
            }
        }
    }

    pub(super) fn reveal_scrollbar(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        let now = self.elapsed();
        self.view
            .terminal
            .scrollbar_reveal
            .entry(tab_id.to_owned())
            .or_default()
            .reveal(now);
        self.view.terminal.scrollbar_expiry = Some(cx.spawn(async move |window, cx| {
            cx.background_executor()
                .timer(muxy_ui::scrollbar::REVEAL_DURATION)
                .await;
            let _ = window.update(cx, |_, cx| cx.notify());
        }));
        cx.notify();
    }

    pub(super) fn reveal_scrollbar_near_track(
        &mut self,
        tab_id: &str,
        area_id: &str,
        x: f64,
        cx: &mut Context<Self>,
    ) {
        if self.terminal_runtime.surfaces.has_native_scrollbar(tab_id) {
            return;
        }
        let Some(bounds) = self.view.workspace.area_bounds.get(area_id).copied() else {
            return;
        };
        let now = self.elapsed();
        let pointer_x = x - f64::from(bounds.origin.x);
        let width = f64::from(bounds.size.width);
        let near = self
            .view
            .terminal
            .scrollbar_reveal
            .entry(tab_id.to_owned())
            .or_default()
            .extend_near_track(now, pointer_x, width, f64::from(SCROLLBAR_WIDTH));
        if near {
            self.reveal_scrollbar(tab_id, cx);
        }
    }

    pub(crate) fn begin_scrollbar_drag(
        &mut self,
        tab_id: &str,
        area_id: &str,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.view.workspace.area_bounds.get(area_id).copied() else {
            return;
        };
        let Some(geometry) = self.thumb_geometry(tab_id, bounds) else {
            return;
        };
        let pointer_y = f64::from(position.y - bounds.origin.y);
        self.view
            .terminal
            .scrollbar_reveal
            .entry(tab_id.to_owned())
            .or_default()
            .begin_drag();
        self.view.terminal.scrollbar_drag = Some(ScrollbarDrag {
            tab_id: tab_id.to_owned(),
            area_id: area_id.to_owned(),
            grab: pointer_y - f64::from(SCROLLBAR_TRACK_INSET) - geometry.origin,
            origin: geometry.origin,
            last_row: None,
        });
        cx.notify();
    }

    pub(super) fn drag_scrollbar(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(drag) = &self.view.terminal.scrollbar_drag else {
            return false;
        };
        let (tab_id, area_id, grab) = (drag.tab_id.clone(), drag.area_id.clone(), drag.grab);
        let Some(bounds) = self.view.workspace.area_bounds.get(&area_id).copied() else {
            return true;
        };
        let Some(metrics) = self.scrollbar_metrics(&tab_id) else {
            return true;
        };
        let track = workspace_view::scrollbar_track_length(bounds);
        let Some(geometry) = ThumbGeometry::from_lengths(
            metrics.total as f64,
            metrics.visible as f64,
            metrics.offset as f64,
            track,
            SCROLLBAR_MIN_THUMB,
        ) else {
            return true;
        };
        let pointer_y = f64::from(position.y - bounds.origin.y);
        let travel = (track - geometry.length).max(0.0);
        let origin = (pointer_y - f64::from(SCROLLBAR_TRACK_INSET) - grab).clamp(0.0, travel);
        let row = muxy_terminal::scrollbar::row_offset_for_thumb_origin(
            metrics,
            origin,
            track,
            geometry.length,
        );
        if let Some(drag) = &mut self.view.terminal.scrollbar_drag
            && drag.origin != origin
        {
            drag.origin = origin;
            cx.notify();
        }
        if let Some(drag) = &mut self.view.terminal.scrollbar_drag
            && drag.last_row != Some(row)
        {
            drag.last_row = Some(row);
            self.terminal_runtime
                .surfaces
                .perform(&tab_id, SurfaceAction::ScrollToRow(row));
            cx.notify();
        }
        true
    }

    pub(super) fn end_scrollbar_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.view.terminal.scrollbar_drag.take() else {
            return;
        };
        let now = self.elapsed();
        if let Some(reveal) = self.view.terminal.scrollbar_reveal.get_mut(&drag.tab_id) {
            reveal.end_drag(now);
        }
        self.reveal_scrollbar(&drag.tab_id, cx);
    }

    pub(super) fn thumb_geometry(
        &self,
        tab_id: &str,
        bounds: Bounds<Pixels>,
    ) -> Option<ThumbGeometry> {
        let metrics = self.scrollbar_metrics(tab_id)?;
        ThumbGeometry::from_lengths(
            metrics.total as f64,
            metrics.visible as f64,
            metrics.offset as f64,
            workspace_view::scrollbar_track_length(bounds),
            SCROLLBAR_MIN_THUMB,
        )
    }

    pub(super) fn focused_working_directory(&self) -> Option<String> {
        let workspace = self.state.active_tab_workspace()?;
        let area_id = workspace.focused_area_id.as_deref()?;
        let tab_id = workspace.area(area_id)?.active_tab_id.as_deref()?;
        self.terminal_runtime
            .surfaces
            .handle(tab_id)?
            .metadata()
            .working_directory
            .clone()
    }

    pub(super) fn focused_launch_directory(&self) -> Option<std::path::PathBuf> {
        let workspace = self.state.active_tab_workspace()?;
        let area_id = workspace.focused_area_id.as_deref()?;
        let tab_id = workspace.area(area_id)?.active_tab_id.as_deref()?;
        self.terminal_runtime
            .surfaces
            .handle(tab_id)?
            .metadata()
            .working_directory
            .as_ref()
            .map(std::path::PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_notification_title_default_and_focus_policy_are_exact() {
        assert_eq!(
            desktop_notification_title(String::new()),
            "Command executed!"
        );
        assert_eq!(desktop_notification_title("  ".to_owned()), "  ",);
        assert_eq!(desktop_notification_title("Done".to_owned()), "Done");
        assert!(desktop_notification_is_sound_only(true, true));
        assert!(!desktop_notification_is_sound_only(true, false));
        assert!(!desktop_notification_is_sound_only(false, true));
        assert!(!desktop_notification_is_sound_only(false, false));
    }

    #[test]
    fn composer_only_suppresses_terminal_input_while_its_editor_is_focused() {
        assert!(terminal_input_overlay_active(false, false, true));
        assert!(!terminal_input_overlay_active(false, false, false));
        assert!(terminal_input_overlay_active(true, false, false));
        assert!(terminal_input_overlay_active(false, true, false));
    }
}
