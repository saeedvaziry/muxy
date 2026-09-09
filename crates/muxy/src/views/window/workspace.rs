use super::*;

impl MainWindow {
    pub(crate) fn navigate(
        &mut self,
        direction: muxy_core::navigation::Direction,
        cx: &mut Context<Self>,
    ) {
        match self.state.navigate(direction) {
            Ok(true) => cx.notify(),
            Ok(false) => {}
            Err(error) => log::warn!("failed to navigate workspace history: {error}"),
        }
    }

    pub(crate) fn focus_workspace(&self, window: &mut Window) {
        window.focus(&self.view.workspace_focus);
    }

    pub(crate) fn active_workspace(&self) -> Option<&muxy_core::workspace::WorkspaceState> {
        self.state.active_tab_workspace()
    }

    pub(crate) fn select_root_tab(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        if self
            .state
            .active_tab_workspace_mut()
            .is_some_and(|workspace| workspace.select_root_tab(tab_id))
        {
            let _ = self.state.persist_tab_workspaces();
            cx.notify();
        }
    }

    pub(super) fn create_command_tab(&mut self, shortcut_id: &str, cx: &mut Context<Self>) {
        let Some(shortcut) = self.state.command_shortcuts.shortcut(shortcut_id) else {
            return;
        };
        let command = shortcut.trimmed_command();
        if command.is_empty() {
            return;
        }
        let title = shortcut.display_name();
        let Some(project_path) = self
            .state
            .active_project()
            .map(|project| self.state.active_worktree_path(project))
        else {
            return;
        };
        let mut tab = muxy_core::workspace::Tab::new(muxy_core::workspace::TabKind::Terminal);
        tab.project_path = Some(project_path.clone());
        tab.custom_title = Some(title);
        let tab_id = tab.id.clone();
        if self
            .state
            .active_tab_workspace_mut()
            .and_then(|workspace| workspace.new_top_level_tab(tab))
            .is_none()
        {
            return;
        }
        self.terminal_runtime
            .surfaces
            .queue_launch_directory(tab_id.clone(), std::path::PathBuf::from(project_path));
        self.terminal_runtime.surfaces.queue_launch_command(
            tab_id,
            crate::terminal::LaunchCommand {
                command,
                keeps_shell_open: true,
            },
        );
        let _ = self.state.persist_tab_workspaces();
        cx.notify();
    }

    pub(crate) fn new_terminal_tab(&mut self, cx: &mut Context<Self>) {
        self.new_tab(muxy_core::workspace::TabKind::Terminal, cx);
    }

    pub(crate) fn new_home_tab(&mut self, cx: &mut Context<Self>) {
        self.state.workspace.ensure_home();
        self.state.select_project(muxy_core::store::HOME_PROJECT_ID);
        self.new_terminal_tab(cx);
    }

    pub(crate) fn new_browser_tab(&mut self, cx: &mut Context<Self>) {
        if self.state.prefs.browser_enabled {
            self.new_tab(muxy_core::workspace::TabKind::Browser, cx);
        }
    }

    pub(super) fn new_tab(&mut self, kind: muxy_core::workspace::TabKind, cx: &mut Context<Self>) {
        let project_path = self
            .state
            .active_project()
            .map(|project| project.path.clone());
        let mut tab = muxy_core::workspace::Tab::new(kind);
        tab.project_path = project_path;
        if kind == muxy_core::workspace::TabKind::Browser {
            tab.static_title = Some("New Tab".to_owned());
        }
        let tab_id = tab.id.clone();
        let launch = (kind == muxy_core::workspace::TabKind::Terminal)
            .then(|| self.focused_launch_directory())
            .flatten();
        if self
            .state
            .active_tab_workspace_mut()
            .and_then(|workspace| workspace.new_top_level_tab(tab))
            .is_some()
        {
            if let Some(directory) = launch {
                self.terminal_runtime
                    .surfaces
                    .queue_launch_directory(tab_id, directory);
            }
            let _ = self.state.persist_tab_workspaces();
            cx.notify();
        }
    }

    pub(crate) fn close_tab(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        let final_root = self
            .active_workspace()
            .and_then(|workspace| workspace.root_id_for_tab(tab_id).map(str::to_owned))
            .is_some_and(|root_id| {
                self.active_workspace().is_some_and(|workspace| {
                    workspace.root_tab_ids().len() == 1 && root_id == tab_id
                })
            });
        if final_root && !self.state.prefs.keep_projects_open {
            let tab_id = tab_id.to_owned();
            let answer = self.ask(
                "Close Project?".to_owned(),
                "This is the last tab. Closing it will remove the project from the sidebar."
                    .to_owned(),
                &["Close", "Cancel"],
                cx,
            );
            cx.spawn(async move |window, cx| {
                if answer.await == Some(0) {
                    let _ = window.update(cx, |window, cx| {
                        window.close_tab_after_confirmation(&tab_id, cx);
                    });
                }
            })
            .detach();
            return;
        }
        self.close_tab_after_confirmation(tab_id, cx);
    }

    fn close_tab_after_confirmation(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        if self.request_extension_webview_close(tab_id, cx) {
            return;
        }
        self.close_tab_confirmed(tab_id, cx);
    }

    pub(super) fn close_tab_confirmed(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        let project = self.state.active_project().map(|project| {
            let workspace_path = self
                .state
                .active_tab_workspace()
                .and_then(|workspace| workspace.worktree_path.clone())
                .unwrap_or_else(|| project.path.clone());
            (project.id.clone(), workspace_path, project.is_home())
        });
        let removed = self
            .state
            .active_tab_workspace_mut()
            .map(|workspace| workspace.close_tab(tab_id, muxy_core::workspace::CloseMode::Single))
            .unwrap_or_default();
        if removed.is_empty() {
            return;
        }
        for closed in &removed {
            if let Some(handle) = self.terminal_runtime.surfaces.handle(closed) {
                handle.request_close();
            }
        }
        let empty = self
            .state
            .active_tab_workspace()
            .is_some_and(|workspace| workspace.root.is_none());
        if empty
            && !self.state.prefs.keep_projects_open
            && let Some((project_id, workspace_path, is_home)) = project
            && !is_home
        {
            self.state
                .tab_workspaces
                .remove_workspace(&project_id, &workspace_path);
            if !self.state.tab_workspaces.has_project(&project_id) {
                self.remove_project(&project_id, cx);
            }
        }
        let _ = self.state.persist_tab_workspaces();
        cx.notify();
    }

    pub(crate) fn split_focused(
        &mut self,
        edge: muxy_core::workspace::Edge,
        cx: &mut Context<Self>,
    ) {
        let project_path = self
            .state
            .active_project()
            .map(|project| project.path.clone());
        let mut tab = muxy_core::workspace::Tab::new(muxy_core::workspace::TabKind::Terminal);
        tab.project_path = project_path;
        let tab_id = tab.id.clone();
        let launch = self.focused_launch_directory();
        if self
            .state
            .active_tab_workspace_mut()
            .and_then(|workspace| workspace.split_focused_area(edge, tab))
            .is_some()
        {
            if let Some(directory) = launch {
                self.terminal_runtime
                    .surfaces
                    .queue_launch_directory(tab_id, directory);
            }
            let _ = self.state.persist_tab_workspaces();
            cx.notify();
        }
    }

    pub(crate) fn toggle_maximize(&mut self, cx: &mut Context<Self>) {
        let area_id = self
            .active_workspace()
            .and_then(|workspace| workspace.focused_area_id.clone());
        if let Some(area_id) = area_id
            && self
                .state
                .active_tab_workspace_mut()
                .is_some_and(|workspace| workspace.toggle_maximized_area(&area_id))
        {
            cx.notify();
        }
    }

    pub(crate) fn record_tab_bounds(&mut self, id: &str, bounds: Bounds<Pixels>) {
        self.view.workspace.tab_bounds.insert(id.to_owned(), bounds);
    }

    pub(crate) fn record_area_bounds(&mut self, id: &str, bounds: Bounds<Pixels>) {
        self.view
            .workspace
            .area_bounds
            .insert(id.to_owned(), bounds);
    }

    pub(crate) fn record_group_bounds(&mut self, id: &str, bounds: Bounds<Pixels>) {
        self.view
            .workspace
            .group_bounds
            .insert(id.to_owned(), bounds);
    }

    pub(crate) fn record_split_bounds(&mut self, id: &str, bounds: Bounds<Pixels>) {
        self.view
            .workspace
            .split_bounds
            .insert(id.to_owned(), bounds);
    }

    pub(crate) fn begin_tab_drag(
        &mut self,
        tab_id: String,
        group_id: String,
        position: Point<Pixels>,
    ) {
        let mut drag = muxy_core::workspace::DragCoordinator::new();
        drag.begin(muxy_core::workspace::Point::new(
            position.x.into(),
            position.y.into(),
        ));
        self.view.workspace.gesture = Some(WorkspaceGesture::Tab {
            tab_id,
            group_id,
            drag,
            target: None,
        });
    }

    pub(crate) fn focus_area(&mut self, area_id: &str, cx: &mut Context<Self>) {
        if self
            .state
            .active_tab_workspace_mut()
            .is_some_and(|workspace| workspace.focus_area(Some(area_id)))
        {
            let _ = self.state.persist_tab_workspaces();
            cx.notify();
        }
    }

    pub(crate) fn begin_pane_drag(
        &mut self,
        tab_id: String,
        _area_id: String,
        position: Point<Pixels>,
        platform_modifier: bool,
    ) {
        let mut drag = muxy_core::workspace::DragCoordinator::new();
        drag.begin(muxy_core::workspace::Point::new(
            position.x.into(),
            position.y.into(),
        ));
        self.view.workspace.gesture = Some(WorkspaceGesture::Pane {
            tab_id,
            drag,
            enabled: platform_modifier,
            target: None,
        });
    }

    pub(crate) fn begin_resize(
        &mut self,
        split_id: String,
        top_level: bool,
        axis: muxy_core::workspace::Axis,
        initial_ratio: f32,
        origin: Point<Pixels>,
    ) {
        self.view.workspace.gesture = Some(WorkspaceGesture::Resize {
            split_id,
            top_level,
            axis,
            initial_ratio,
            origin,
        });
    }

    pub(super) fn active_tab_id(&self) -> Option<String> {
        let workspace = self.active_workspace()?;
        let area = workspace.area(workspace.focused_area_id.as_deref()?)?;
        area.active_tab_id.clone()
    }

    pub(crate) fn select_relative_root(&mut self, delta: i32, cx: &mut Context<Self>) {
        let Some(workspace) = self.active_workspace() else {
            return;
        };
        if workspace.top_level_order.is_empty() {
            return;
        }
        let current = workspace
            .focused_root_tab_id()
            .and_then(|id| {
                workspace
                    .top_level_order
                    .iter()
                    .position(|candidate| candidate == id)
            })
            .unwrap_or_default();
        let index =
            (current as i32 + delta).rem_euclid(workspace.top_level_order.len() as i32) as usize;
        let id = workspace.top_level_order[index].clone();
        self.select_root_tab(&id, cx);
    }

    pub(crate) fn select_project_relative(&mut self, delta: i32, cx: &mut Context<Self>) {
        if self.state.select_project_relative(delta).is_some() {
            cx.notify();
        }
    }

    pub(crate) fn select_project_index(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.state.select_project_index(index).is_some() {
            cx.notify();
        }
    }

    pub(crate) fn select_root_index(&mut self, index: usize, cx: &mut Context<Self>) {
        let id = self
            .active_workspace()
            .and_then(|workspace| workspace.top_level_order.get(index))
            .cloned();
        if let Some(id) = id {
            self.select_root_tab(&id, cx);
        }
    }

    pub(crate) fn close_active_tab(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.active_tab_id() {
            self.close_tab(&id, cx);
        }
    }

    pub(crate) fn rename_active_tab(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self
            .active_workspace()
            .and_then(|workspace| workspace.focused_root_tab_id())
            .map(str::to_owned)
        {
            self.start_tab_rename(id, None, cx);
        }
    }

    pub(crate) fn toggle_active_tab_pinned(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self
            .active_workspace()
            .and_then(|workspace| workspace.focused_root_tab_id())
            .map(str::to_owned)
        {
            self.toggle_tab_pinned(&id, cx);
        }
    }

    pub(crate) fn cycle_pane(&mut self, delta: i32, cx: &mut Context<Self>) {
        let Some(workspace) = self.active_workspace() else {
            return;
        };
        let visible_tabs: HashMap<String, String> =
            workspace.visible_area_tabs().into_iter().collect();
        let mut areas: Vec<String> = visible_tabs
            .keys()
            .filter(|area_id| self.view.workspace.area_bounds.contains_key(*area_id))
            .cloned()
            .collect();
        areas.sort_by(|left, right| {
            let left = self.view.workspace.area_bounds[left];
            let right = self.view.workspace.area_bounds[right];
            left.origin
                .y
                .partial_cmp(&right.origin.y)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    left.origin
                        .x
                        .partial_cmp(&right.origin.x)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });
        if areas.is_empty() {
            return;
        }
        let current = workspace
            .focused_area_id
            .as_ref()
            .and_then(|id| areas.iter().position(|candidate| candidate == id))
            .unwrap_or_default();
        let index = (current as i32 + delta).rem_euclid(areas.len() as i32) as usize;
        let area_id = &areas[index];
        if let Some(tab_id) = visible_tabs.get(area_id) {
            self.focus_workspace_tab(area_id, tab_id, cx);
        }
    }

    pub(crate) fn focus_pane_direction(
        &mut self,
        axis: muxy_core::workspace::Axis,
        positive: bool,
        move_pane: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.active_workspace() else {
            return;
        };
        if workspace.maximized_area_id.is_some() {
            return;
        }
        let Some(source_id) = workspace.focused_area_id.clone() else {
            return;
        };
        let visible_tabs: HashMap<String, String> =
            workspace.visible_area_tabs().into_iter().collect();
        let Some(source) = self.view.workspace.area_bounds.get(&source_id).copied() else {
            return;
        };
        let source_center = (
            f32::from(source.origin.x + source.size.width / 2.0),
            f32::from(source.origin.y + source.size.height / 2.0),
        );
        let target = self
            .view
            .workspace
            .area_bounds
            .iter()
            .filter(|(id, _)| *id != &source_id)
            .filter(|(id, _)| visible_tabs.contains_key(*id))
            .filter_map(|(id, bounds)| {
                let center = (
                    f32::from(bounds.origin.x + bounds.size.width / 2.0),
                    f32::from(bounds.origin.y + bounds.size.height / 2.0),
                );
                let main = match axis {
                    muxy_core::workspace::Axis::Horizontal => center.0 - source_center.0,
                    muxy_core::workspace::Axis::Vertical => center.1 - source_center.1,
                };
                if (positive && main <= 0.0) || (!positive && main >= 0.0) {
                    return None;
                }
                let cross = match axis {
                    muxy_core::workspace::Axis::Horizontal => (center.1 - source_center.1).abs(),
                    muxy_core::workspace::Axis::Vertical => (center.0 - source_center.0).abs(),
                };
                Some((id.clone(), main.abs(), cross))
            })
            .min_by(|left, right| {
                left.1
                    .total_cmp(&right.1)
                    .then_with(|| left.2.total_cmp(&right.2))
            })
            .map(|candidate| candidate.0);
        let Some(target) = target else {
            return;
        };
        if move_pane {
            let active_tab_id = visible_tabs.get(&source_id).cloned();
            if let Some(active_tab_id) = active_tab_id
                && self
                    .state
                    .active_tab_workspace_mut()
                    .is_some_and(|workspace| workspace.move_pane_center(&active_tab_id, &target))
            {
                let _ = self.state.persist_tab_workspaces();
                cx.notify();
            }
        } else if let Some(tab_id) = visible_tabs.get(&target) {
            self.focus_workspace_tab(&target, tab_id, cx);
        }
    }

    pub(super) fn focus_workspace_tab(
        &mut self,
        area_id: &str,
        tab_id: &str,
        cx: &mut Context<Self>,
    ) {
        if self
            .state
            .active_tab_workspace_mut()
            .is_some_and(|workspace| workspace.select_tab(area_id, tab_id))
        {
            let _ = self.state.persist_tab_workspaces();
            cx.notify();
        }
    }

    pub(super) fn create_adjacent_tab(
        &mut self,
        target_tab_id: &str,
        right: bool,
        cx: &mut Context<Self>,
    ) {
        let target_index = self
            .active_workspace()
            .and_then(|workspace| workspace.top_level_root.as_ref())
            .and_then(|root| {
                root.group_containing_tab(target_tab_id)
                    .map(|id| (root, id))
            })
            .and_then(|(root, group_id)| root.group_tab_ids(group_id))
            .and_then(|ids| ids.iter().position(|id| id == target_tab_id));
        let Some(target_index) = target_index else {
            return;
        };
        self.select_root_tab(target_tab_id, cx);
        let project_path = self
            .state
            .active_project()
            .map(|project| project.path.clone());
        let mut tab = muxy_core::workspace::Tab::new(muxy_core::workspace::TabKind::Terminal);
        tab.project_path = project_path;
        let created = self
            .state
            .active_tab_workspace_mut()
            .and_then(|workspace| workspace.new_top_level_tab(tab));
        if let Some(created) = created {
            let index = target_index + usize::from(right);
            if let Some(workspace) = self.state.active_tab_workspace_mut() {
                workspace.reorder_top_level_tab(&created, index);
            }
            let _ = self.state.persist_tab_workspaces();
            cx.notify();
        }
    }

    pub(super) fn set_tab_title(
        &mut self,
        tab_id: &str,
        title: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self
            .state
            .active_tab_workspace_mut()
            .and_then(|workspace| workspace.tab_mut(tab_id))
        {
            tab.custom_title = title.filter(|title| !title.trim().is_empty());
            let _ = self.state.persist_tab_workspaces();
            self.dismiss_overlay(cx);
        }
    }

    pub(crate) fn set_tab_color(
        &mut self,
        tab_id: &str,
        color: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self
            .state
            .active_tab_workspace_mut()
            .and_then(|workspace| workspace.tab_mut(tab_id))
        {
            tab.color_id = color;
            let _ = self.state.persist_tab_workspaces();
            self.dismiss_overlay(cx);
        }
    }

    pub(super) fn toggle_tab_pinned(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        let pinned = self
            .active_workspace()
            .and_then(|workspace| workspace.tab(tab_id))
            .is_some_and(|tab| tab.pinned);
        if self
            .state
            .active_tab_workspace_mut()
            .is_some_and(|workspace| workspace.set_tab_pinned(tab_id, !pinned))
        {
            let _ = self.state.persist_tab_workspaces();
        }
        self.dismiss_overlay(cx);
    }

    pub(super) fn close_tabs(
        &mut self,
        tab_id: &str,
        mode: muxy_core::workspace::CloseMode,
        cx: &mut Context<Self>,
    ) {
        let removed = self
            .state
            .active_tab_workspace_mut()
            .map(|workspace| workspace.close_tab(tab_id, mode))
            .unwrap_or_default();
        if !removed.is_empty() {
            let _ = self.state.persist_tab_workspaces();
        }
        self.dismiss_overlay(cx);
    }

    pub(crate) fn start_tab_rename(
        &mut self,
        tab_id: String,
        anchor: Option<Point<Pixels>>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self
            .active_workspace()
            .and_then(|workspace| workspace.tab(&tab_id))
        else {
            return;
        };
        let displayed_title = tab.title().to_owned();
        let original_custom_title = tab.custom_title.clone();
        let bounds = self
            .view
            .workspace
            .tab_bounds
            .get(&tab_id)
            .copied()
            .or_else(|| {
                anchor.map(|anchor| {
                    Bounds::new(
                        anchor,
                        gpui::size(
                            self.state.metrics.scaled(200.0),
                            self.state.metrics.title_bar_height(),
                        ),
                    )
                })
            })
            .unwrap_or_else(|| {
                Bounds::new(
                    self.menu_anchor(),
                    gpui::size(
                        self.state.metrics.scaled(200.0),
                        self.state.metrics.title_bar_height(),
                    ),
                )
            });
        let style = self.input_style();
        let input = cx.new(|cx| TextInput::new(style, cx).with_text(displayed_title.clone()));
        input.update(cx, |input, cx| input.select_all_text(cx));
        let committed = tab_id.clone();
        let subscription =
            cx.subscribe(
                &input,
                move |window: &mut Self, input, event, cx| match event {
                    InputEvent::Submitted => {
                        let value = input.read(cx).text().trim().to_owned();
                        let custom_title = if value.is_empty()
                            || (original_custom_title.is_none() && value == displayed_title)
                        {
                            None
                        } else {
                            Some(value)
                        };
                        window.set_tab_title(&committed, custom_title, cx);
                    }
                    InputEvent::Cancelled => window.dismiss_overlay(cx),
                    InputEvent::Changed => {}
                },
            );
        self.view.pending_focus = Some(input.focus_handle(cx));
        self.view.subscriptions = vec![subscription];
        self.view.overlay = Overlay::TabRename { input, bounds };
        cx.notify();
    }

    pub(crate) fn open_tab_menu(
        &mut self,
        tab_id: &str,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.active_workspace() else {
            return;
        };
        let Some(tab) = workspace.tab(tab_id) else {
            return;
        };
        let root_id = tab.root_id().to_owned();
        let Some(group_id) = workspace
            .top_level_root
            .as_ref()
            .and_then(|root| root.group_containing_tab(&root_id))
        else {
            return;
        };
        let group_tabs = workspace
            .top_level_root
            .as_ref()
            .and_then(|root| root.group_tab_ids(group_id))
            .unwrap_or_default();
        let index = group_tabs
            .iter()
            .position(|id| id == &root_id)
            .unwrap_or_default();
        let closable_others = group_tabs.iter().any(|id| {
            id != &root_id && workspace.tab(id).is_some_and(|candidate| !candidate.pinned)
        });
        let has_left = group_tabs[..index]
            .iter()
            .any(|id| workspace.tab(id).is_some_and(|candidate| !candidate.pinned));
        let has_right = group_tabs[index + 1..]
            .iter()
            .any(|id| workspace.tab(id).is_some_and(|candidate| !candidate.pinned));
        let mut items = vec![
            Item::action("New Tab to the Left", Command::NewTabLeft(root_id.clone())),
            Item::action(
                "New Tab to the Right",
                Command::NewTabRight(root_id.clone()),
            ),
            Item::Separator,
            Item::action("Rename Tab", Command::StartTabRename(root_id.clone())),
        ];
        if tab.custom_title.is_some() {
            items.push(Item::action(
                "Reset Title",
                Command::ResetTabTitle(root_id.clone()),
            ));
        }
        items.push(Item::action(
            "Set Tab Color…",
            Command::OpenTabColorPicker(root_id.clone()),
        ));
        if tab.color_id.is_some() {
            items.push(Item::action(
                "Reset Tab Color",
                Command::ResetTabColor(root_id.clone()),
            ));
        }
        items.extend([
            Item::Separator,
            Item::action(
                if tab.pinned { "Unpin Tab" } else { "Pin Tab" },
                Command::ToggleTabPinned(root_id.clone()),
            ),
            Item::Separator,
        ]);
        if !tab.pinned {
            items.push(Item::action(
                "Close Tab",
                Command::CloseTab(root_id.clone()),
            ));
        }
        items.push(
            Item::action("Close Other Tabs", Command::CloseOtherTabs(root_id.clone()))
                .disabled(!closable_others),
        );
        items.push(
            Item::action(
                "Close Tabs to the Left",
                Command::CloseTabsToLeft(root_id.clone()),
            )
            .disabled(!has_left),
        );
        items.push(
            Item::action(
                "Close Tabs to the Right",
                Command::CloseTabsToRight(root_id),
            )
            .disabled(!has_right),
        );
        self.open_menu(items, position, cx);
        window.focus(&self.view.menu_focus);
    }

    pub(crate) fn handle_workspace_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        self.release_pointer_outside_panes(event.position);
        if self.drag_scrollbar(event.position, cx) {
            return;
        }
        let Some(mut gesture) = self.view.workspace.gesture.take() else {
            return;
        };
        let point = muxy_core::workspace::Point::new(
            f32::from(event.position.x),
            f32::from(event.position.y),
        );
        match &mut gesture {
            WorkspaceGesture::Tab {
                tab_id,
                group_id,
                drag,
                target,
            } => {
                if drag.update(point) {
                    let pinned = self
                        .active_workspace()
                        .and_then(|workspace| workspace.tab(tab_id))
                        .is_some_and(|tab| tab.pinned);
                    if drag.top_level_transitioned() && !pinned {
                        *target = closest_drop_target(&self.view.workspace.group_bounds, point);
                    } else {
                        *target = None;
                        let hovered = containing_id(&self.view.workspace.tab_bounds, point);
                        if let Some(hovered) = hovered.filter(|id| id != tab_id) {
                            let index = self
                                .active_workspace()
                                .and_then(|workspace| workspace.top_level_root.as_ref())
                                .and_then(|root| root.group_tab_ids(group_id))
                                .and_then(|ids| ids.iter().position(|id| id == &hovered));
                            if let Some(index) = index
                                && self
                                    .state
                                    .active_tab_workspace_mut()
                                    .is_some_and(|workspace| {
                                        workspace.reorder_top_level_tab(tab_id, index)
                                    })
                            {
                                cx.notify();
                            }
                        }
                    }
                }
            }
            WorkspaceGesture::Pane {
                tab_id,
                drag,
                enabled,
                target,
                ..
            } => {
                if *enabled && drag.update(point) {
                    let root_id = self
                        .active_workspace()
                        .and_then(|workspace| workspace.root_id_for_tab(tab_id))
                        .map(str::to_owned);
                    *target = closest_drop_target(&self.view.workspace.area_bounds, point).filter(
                        |(area_id, _)| {
                            root_id.as_ref().is_some_and(|root_id| {
                                self.active_workspace()
                                    .and_then(|workspace| workspace.area(area_id))
                                    .and_then(|area| area.selected_for_root(root_id))
                                    .is_some()
                            })
                        },
                    );
                }
            }
            WorkspaceGesture::Resize {
                split_id,
                top_level,
                axis,
                initial_ratio,
                origin,
            } => {
                let Some(bounds) = self.view.workspace.split_bounds.get(split_id) else {
                    self.view.workspace.gesture = Some(gesture);
                    return;
                };
                let dimension = match axis {
                    muxy_core::workspace::Axis::Horizontal => f32::from(bounds.size.width),
                    muxy_core::workspace::Axis::Vertical => f32::from(bounds.size.height),
                };
                if !dimension.is_finite() || dimension <= 0.0 {
                    self.view.workspace.gesture = Some(gesture);
                    return;
                }
                let delta = match axis {
                    muxy_core::workspace::Axis::Horizontal => {
                        f32::from(event.position.x - origin.x) / dimension
                    }
                    muxy_core::workspace::Axis::Vertical => {
                        f32::from(event.position.y - origin.y) / dimension
                    }
                };
                let ratio = *initial_ratio + delta;
                let changed = self
                    .state
                    .active_tab_workspace_mut()
                    .is_some_and(|workspace| {
                        if *top_level {
                            workspace.resize_top_level_split(split_id, ratio)
                        } else {
                            workspace.resize_split(split_id, ratio)
                        }
                    });
                if changed {
                    cx.notify();
                }
            }
        }
        self.view.workspace.gesture = Some(gesture);
    }

    pub(super) fn workspace_drop_highlight(
        &self,
    ) -> Option<(Bounds<Pixels>, muxy_core::workspace::DropZone)> {
        match &self.view.workspace.gesture {
            Some(WorkspaceGesture::Tab {
                target: Some((id, zone)),
                ..
            }) => self
                .view
                .workspace
                .group_bounds
                .get(id)
                .copied()
                .map(|bounds| (bounds, *zone)),
            Some(WorkspaceGesture::Pane {
                target: Some((id, zone)),
                ..
            }) => self
                .view
                .workspace
                .area_bounds
                .get(id)
                .copied()
                .map(|bounds| (bounds, *zone)),
            _ => None,
        }
    }

    pub(crate) fn handle_workspace_mouse_up(&mut self, _: &MouseUpEvent, cx: &mut Context<Self>) {
        self.end_scrollbar_drag(cx);
        let Some(gesture) = self.view.workspace.gesture.take() else {
            return;
        };
        let changed = match gesture {
            WorkspaceGesture::Tab {
                tab_id,
                group_id: source_group_id,
                target: Some((group_id, zone)),
                ..
            } => self
                .state
                .active_tab_workspace_mut()
                .is_some_and(|workspace| match zone {
                    muxy_core::workspace::DropZone::Center => {
                        if source_group_id == group_id {
                            return false;
                        }
                        let index = workspace
                            .top_level_root
                            .as_ref()
                            .and_then(|root| root.group_tab_ids(&group_id))
                            .map(<[String]>::len)
                            .unwrap_or_default();
                        workspace.dock_top_level_center(&tab_id, &group_id, index)
                    }
                    muxy_core::workspace::DropZone::Left => workspace
                        .dock_top_level_edge(&tab_id, &group_id, muxy_core::workspace::Edge::Left)
                        .is_some(),
                    muxy_core::workspace::DropZone::Right => workspace
                        .dock_top_level_edge(&tab_id, &group_id, muxy_core::workspace::Edge::Right)
                        .is_some(),
                    muxy_core::workspace::DropZone::Top => workspace
                        .dock_top_level_edge(&tab_id, &group_id, muxy_core::workspace::Edge::Top)
                        .is_some(),
                    muxy_core::workspace::DropZone::Bottom => workspace
                        .dock_top_level_edge(&tab_id, &group_id, muxy_core::workspace::Edge::Bottom)
                        .is_some(),
                }),
            WorkspaceGesture::Pane {
                tab_id,
                target: Some((area_id, zone)),
                ..
            } => self
                .state
                .active_tab_workspace_mut()
                .and_then(|workspace| workspace.move_pane(&tab_id, &area_id, zone))
                .is_some(),
            WorkspaceGesture::Tab { .. } | WorkspaceGesture::Resize { .. } => true,
            _ => false,
        };
        if changed {
            let _ = self.state.persist_tab_workspaces();
            cx.notify();
        }
    }
}

pub(super) fn containing_id(
    bounds: &HashMap<String, Bounds<Pixels>>,
    point: muxy_core::workspace::Point,
) -> Option<String> {
    bounds
        .iter()
        .find(|(_, bounds)| rect_from_bounds(**bounds).contains(point))
        .map(|(id, _)| id.clone())
}

pub(super) fn closest_drop_target(
    bounds: &HashMap<String, Bounds<Pixels>>,
    point: muxy_core::workspace::Point,
) -> Option<(String, muxy_core::workspace::DropZone)> {
    bounds
        .iter()
        .filter_map(|(id, bounds)| {
            let rect = rect_from_bounds(*bounds);
            let zone = muxy_core::workspace::DropZone::at(point, rect)?;
            let dx = point.x - rect.mid_x();
            let dy = point.y - rect.mid_y();
            Some((id.clone(), zone, dx * dx + dy * dy))
        })
        .min_by(|left, right| left.2.total_cmp(&right.2))
        .map(|(id, zone, _)| (id, zone))
}

pub(super) fn rect_from_bounds(bounds: Bounds<Pixels>) -> muxy_core::workspace::Rect {
    muxy_core::workspace::Rect::new(
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
    )
}
