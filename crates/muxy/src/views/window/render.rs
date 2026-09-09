use super::*;
use gpui::prelude::FluentBuilder;
use gpui::{ParentElement, Styled};

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_repository_context(cx);
        self.reconcile_terminals(window, cx);
        self.reconcile_extension_webviews(window, cx);
        self.sync_active_notification_read_state(cx);
        let project_ids = self
            .state
            .workspace
            .projects
            .iter()
            .map(|project| project.id.clone())
            .collect();
        self.view.worktrees.retain_projects(&project_ids);
        if let Some(handle) = self.view.pending_focus.take() {
            window.focus(&handle);
        } else if !self.view.overlay.is_open()
            && self.view.terminal.search_inputs.is_empty()
            && !self.composer_is_open()
            && !self.view.workspace_focus.contains_focused(window, cx)
        {
            window.focus(&self.view.workspace_focus);
        }
        let layout = crate::views::app::AppLayout::new(
            &self.state.prefs,
            self.state.metrics,
            self.view.sidebar_expanded,
        );
        if let Some(workspace) = self.state.active_tab_workspace() {
            let tabs: HashSet<String> = workspace.top_level_order.iter().cloned().collect();
            let areas: HashSet<String> = workspace
                .visible_area_tabs()
                .into_iter()
                .map(|(area_id, _)| area_id)
                .collect();
            let groups = workspace.top_level_root.clone();
            self.view
                .workspace
                .tab_bounds
                .retain(|id, _| tabs.contains(id));
            self.view
                .workspace
                .area_bounds
                .retain(|id, _| areas.contains(id));
            self.view
                .workspace
                .group_bounds
                .retain(|id, _| groups.as_ref().is_some_and(|root| root.contains_group(id)));
        } else {
            self.view.workspace.tab_bounds.clear();
            self.view.workspace.area_bounds.clear();
            self.view.workspace.group_bounds.clear();
            self.view.workspace.split_bounds.clear();
        }
        let focused_working_directory = self.focused_working_directory();
        let repository_state = self.view.repository.coordinator.state();
        let repository_mutation_busy = repository_state
            .key
            .as_ref()
            .is_some_and(|key| self.state.project_operations.is_mutating(&key.project_id));
        let repository_controls = crate::repository::repository_controls(
            repository_state,
            &self.state.prefs.repository_ai,
            repository_mutation_busy,
        );
        let drop_highlight = self.workspace_drop_highlight();
        let drag = self
            .view
            .terminal
            .scrollbar_drag
            .as_ref()
            .map(|drag| (drag.tab_id.as_str(), drag.origin));
        let theme = self.state.theme.clone();
        let metrics = self.state.metrics;
        let extension_topbar_items = self.extension_runtime.topbar_bindings();
        let extension_statusbar_items = self.extension_runtime.status_bar_bindings();
        let app = crate::views::app::render(
            crate::views::app::AppView {
                state: &self.state,
                repository_controls: &repository_controls,
                repository_state,
                repository_mutation_busy,
                layout,
                workspace_focus: &self.view.workspace_focus,
                menu_focus: &self.view.menu_focus,
                terminals: &self.terminal_runtime.surfaces,
                extension_webviews: &self.extension_webviews,
                extension_surfaces: &self.extension_surfaces,
                extension_popover_anchor: self.extension_popover_anchor,
                area_bounds: &self.view.workspace.area_bounds,
                search_inputs: &self.view.terminal.search_inputs,
                scrollbar_reveal: &self.view.terminal.scrollbar_reveal,
                terminal_attention: &self.view.terminal.attention,
                bell_flashes: &self.view.terminal.bell_flashes,
                drag,
                now: self.view.terminal.started_at.elapsed(),
                overlay: &self.view.overlay,
                toast: self
                    .view
                    .toast
                    .current()
                    .map(|content| (content, self.view.toast.generation())),
                drop_highlight,
                focused_working_directory,
                expanded_worktree_projects: self.view.worktrees.expanded_projects(),
                composer: &self.composer,
                panels: &self.panels,
                extension_topbar_items: &extension_topbar_items,
                extension_statusbar_items: &extension_statusbar_items,
            },
            window,
            cx,
        );
        let app = crate::panels::with_phase_3_component_proof(app, &theme, metrics, cx);
        let pending_consent = self.extension_runtime.pending_consent().cloned();
        gpui::div()
            .relative()
            .size_full()
            .child(app)
            .when_some(pending_consent, |app, request| {
                app.child(crate::extensions::consent_view::layer(
                    &request,
                    self.extension_consent_block_kind,
                    &self.state,
                    cx,
                ))
            })
    }
}
