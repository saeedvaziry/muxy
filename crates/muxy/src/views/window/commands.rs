use super::*;

impl MainWindow {
    pub(crate) fn perform(
        &mut self,
        command: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            Command::TogglePin(id) => {
                let pinned = self
                    .state
                    .workspace
                    .project(&id)
                    .map(|project| project.is_pinned)
                    .unwrap_or_default();
                self.state.workspace.update(&id, |project| {
                    project.is_pinned = !pinned;
                });
                self.state.workspace.sort();
                self.dismiss_overlay(cx);
            }
            Command::StartRename(id) => self.start_rename(id, cx),
            Command::OpenSymbolPicker(id) => self.open_symbol_picker(id, cx),
            Command::RemoveIcon(id) => self.set_icon(&id, None, cx),
            Command::OpenColorPicker(id) => self.open_project_color_picker(id, cx),
            Command::ResetIconColor(id) => self.set_icon_color(&id, None, cx),
            Command::PickLogo(id) => self.pick_logo(id, cx),
            Command::RemoveLogo(id) => {
                logo::remove(&id);
                self.state.workspace.update(&id, |project| {
                    project.logo = None;
                });
                self.dismiss_overlay(cx);
            }
            Command::ToggleWorktrees(id) => {
                let enabled = self
                    .state
                    .workspace
                    .project(&id)
                    .map(|project| project.worktrees_enabled)
                    .unwrap_or_default();
                self.state.workspace.update(&id, |project| {
                    project.worktrees_enabled = !enabled;
                });
                self.dismiss_overlay(cx);
                if enabled {
                    self.view.worktrees.clear_project(&id);
                } else {
                    self.request_worktree_refresh(id, None, cx);
                }
            }
            Command::ToggleWorktreeExpansion(id) => {
                self.view.worktrees.toggle(&id);
                self.dismiss_overlay(cx);
                cx.notify();
            }
            Command::SelectProject(id) => {
                self.state.select_project(&id);
                self.dismiss_overlay(cx);
                cx.notify();
            }
            Command::SelectWorktree {
                project_id,
                worktree_id,
            } => {
                self.state.select_worktree(&project_id, &worktree_id);
                self.dismiss_overlay(cx);
                cx.notify();
            }
            Command::RefreshWorktrees(id) => {
                self.dismiss_overlay(cx);
                self.request_worktree_refresh(id, None, cx);
            }
            Command::NewWorktree(id) => {
                self.dismiss_overlay(cx);
                self.open_create_worktree(&id, window, cx);
            }
            Command::RemoveWorktree {
                project_id,
                worktree_id,
            } => {
                self.dismiss_overlay(cx);
                self.request_worktree_removal_inspection(project_id, worktree_id, cx);
            }
            Command::CopyPath(id) => {
                if let Some(project) = self.state.workspace.project(&id) {
                    cx.write_to_clipboard(ClipboardItem::new_string(project.path.clone()));
                }
                self.dismiss_overlay(cx);
            }
            Command::CopyStatusPath(path) => {
                cx.write_to_clipboard(ClipboardItem::new_string(path));
                self.dismiss_overlay(cx);
            }
            Command::RevealStatusPath(path) => {
                self.dismiss_overlay(cx);
                self.reveal_status_path(path, cx);
            }
            Command::RemoveProject(id) => self.confirm_remove(id, cx),
            Command::SelectWorkspaceGroup(id) => {
                self.state.workspace.select_group(id);
                if let Some(first) = self
                    .state
                    .workspace
                    .visible_projects()
                    .first()
                    .map(|project| project.id.clone())
                {
                    self.state.select_project(&first);
                }
                self.dismiss_overlay(cx);
                cx.notify();
            }
            Command::SetProjectSort(mode) => {
                let (key, value) = project_sort_setting(mode);
                Prefs::store_settings_value(key, value);
                self.state.prefs.sort_mode = mode;
                self.state.workspace.set_sort_mode(mode);
                self.dismiss_overlay(cx);
                cx.notify();
            }
            Command::DeleteWorkspaceGroup(id) => self.confirm_delete_group(id, cx),
            Command::MoveProjectToWorkspace {
                project_id,
                group_id,
            } => {
                let is_member = self
                    .state
                    .workspace
                    .groups
                    .group_id_containing(&project_id)
                    .is_some_and(|id| id.eq_ignore_ascii_case(&group_id));
                if is_member {
                    self.state
                        .workspace
                        .groups
                        .remove_project(&project_id, &group_id);
                } else {
                    self.state
                        .workspace
                        .groups
                        .add_project(&project_id, &group_id);
                }
                self.dismiss_overlay(cx);
                cx.notify();
            }
            Command::CreateWorkspaceGroup => self.start_group_rename(None, cx),
            Command::RenameWorkspaceGroup(id) => self.start_group_rename(Some(id), cx),
            Command::HideHome => {
                Prefs::store_settings_value("muxy.showHomeProject", Value::Bool(false));
                self.state.prefs.show_home_project = false;
                self.state.workspace.hide_home();
                self.dismiss_overlay(cx);
            }
            Command::OpenInIde(bundle_identifier) => self.open_in_ide(&bundle_identifier, cx),
            Command::OpenProjectPicker => self.open_project_picker(window, cx),
            Command::NewTabLeft(id) => {
                self.dismiss_overlay(cx);
                self.create_adjacent_tab(&id, false, cx);
            }
            Command::NewTabRight(id) => {
                self.dismiss_overlay(cx);
                self.create_adjacent_tab(&id, true, cx);
            }
            Command::StartTabRename(id) => self.start_tab_rename(id, None, cx),
            Command::ResetTabTitle(id) => self.set_tab_title(&id, None, cx),
            Command::OpenTabColorPicker(id) => self.open_tab_color_picker(id, cx),
            Command::ResetTabColor(id) => self.set_tab_color(&id, None, cx),
            Command::ToggleTabPinned(id) => self.toggle_tab_pinned(&id, cx),
            Command::CloseTab(id) => {
                self.dismiss_overlay(cx);
                self.close_tab(&id, cx);
            }
            Command::CloseOtherTabs(id) => {
                self.close_tabs(&id, muxy_core::workspace::CloseMode::Others, cx)
            }
            Command::CloseTabsToLeft(id) => {
                self.close_tabs(&id, muxy_core::workspace::CloseMode::ToLeft, cx)
            }
            Command::CloseTabsToRight(id) => {
                self.close_tabs(&id, muxy_core::workspace::CloseMode::ToRight, cx)
            }
            Command::RunCommandShortcut(id) => {
                self.dismiss_overlay(cx);
                self.create_command_tab(&id, cx);
            }
            Command::ApplyLayout(path) => {
                self.dismiss_overlay(cx);
                self.apply_layout(path, cx);
            }
            Command::RestoreRecentlyRemoved(id) => {
                self.dismiss_overlay(cx);
                self.restore_removed_project(&id, cx);
            }
            Command::TerminalCopy(id) => {
                self.dismiss_overlay(cx);
                self.terminal_runtime
                    .surfaces
                    .perform(&id, SurfaceAction::Copy);
            }
            Command::TerminalPaste(id) => {
                self.dismiss_overlay(cx);
                self.terminal_runtime
                    .surfaces
                    .perform(&id, SurfaceAction::Paste);
            }
            Command::ToggleComposerBroadcast => {
                self.dismiss_overlay(cx);
                self.toggle_composer_broadcast(cx);
            }
            Command::SubmitComposerWithoutReturn => {
                self.dismiss_overlay(cx);
                self.submit_composer(false, cx);
            }
            Command::ToggleComposerClearAfterSending => {
                self.dismiss_overlay(cx);
                self.toggle_composer_boolean_setting(
                    "muxy.richInput.clearAfterSending",
                    "Clear After Sending",
                    cx,
                );
            }
            Command::ToggleComposerClearOnClose => {
                self.dismiss_overlay(cx);
                self.toggle_composer_boolean_setting(
                    "muxy.richInput.clearOnClose",
                    "Clear on Close",
                    cx,
                );
            }
            Command::DismissOverlay => self.dismiss_overlay(cx),
        }
    }

    fn reveal_status_path(&mut self, path: String, cx: &mut Context<Self>) {
        cx.spawn(async move |window, cx| {
            let reveal_path = path.clone();
            let result = cx
                .background_executor()
                .spawn(
                    async move { crate::platform::reveal_path(std::path::Path::new(&reveal_path)) },
                )
                .await;
            if let Err(error) = result {
                let _ = window.update(cx, |window, cx| {
                    let (title, message) = crate::views::status_bar::reveal_failure(&path, error);
                    window.feedback(title, message, crate::toast::ToastTone::Error, cx);
                });
            }
        })
        .detach();
    }

    pub(super) fn apply_layout(&mut self, path: String, cx: &mut Context<Self>) {
        let file = std::path::PathBuf::from(&path);
        let name = file
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_owned();
        let answer = self.ask(
            format!("Apply Layout '{name}'?"),
            "All terminals and tabs in this worktree will be closed and replaced with the layout."
                .to_owned(),
            &["Apply", "Cancel"],
            cx,
        );
        cx.spawn(async move |window, cx| {
            if answer.await != Some(0) {
                return;
            }
            let _ = window.update(cx, |window, cx| {
                window.apply_layout_confirmed(&file, cx);
            });
        })
        .detach();
    }

    pub(super) fn apply_layout_confirmed(
        &mut self,
        path: &std::path::Path,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.state.active_project().cloned() else {
            return;
        };
        let worktree_path = self.state.active_worktree_path(&project);
        let Some(config) = muxy_api::layouts::load(path) else {
            self.feedback(
                "Apply Layout",
                "Muxy could not read this layout file.",
                crate::toast::ToastTone::Error,
                cx,
            );
            return;
        };
        let Some(built) = muxy_api::layouts::build(&config, &worktree_path) else {
            self.feedback(
                "Apply Layout",
                "This layout has no panes to open.",
                crate::toast::ToastTone::Error,
                cx,
            );
            return;
        };

        let previous: Vec<String> = self
            .state
            .active_tab_workspace()
            .and_then(|workspace| workspace.root.as_ref())
            .map(|root| root.tabs().iter().map(|tab| tab.id.clone()).collect())
            .unwrap_or_default();
        for tab_id in &previous {
            if let Some(handle) = self.terminal_runtime.surfaces.handle(tab_id) {
                handle.request_close();
            }
        }

        let root_tab_id = built
            .root
            .area_by_id(&built.root.area_ids()[0])
            .and_then(|area| area.active_tab_id.clone());
        let top_level_order: Vec<String> = built
            .root
            .tabs()
            .into_iter()
            .filter(|tab| tab.parent_id.is_none())
            .map(|tab| tab.id.clone())
            .collect();
        let launches = built.launches.clone();

        let Some(workspace) = self.state.active_tab_workspace_mut() else {
            return;
        };
        workspace.root = Some(built.root);
        workspace.top_level_root = Some(muxy_core::workspace::TopLevelTabNode::group(
            top_level_order.clone(),
            root_tab_id,
        ));
        workspace.top_level_order = top_level_order;
        workspace.focused_area_id = Some(built.focused_area_id);
        workspace.focus_history.clear();
        workspace.maximized_area_id = None;
        workspace.reconcile();

        for (tab_id, command) in launches {
            self.terminal_runtime.surfaces.queue_launch_command(
                tab_id,
                crate::terminal::LaunchCommand {
                    command,
                    keeps_shell_open: false,
                },
            );
        }
        let _ = self.state.persist_tab_workspaces();
        cx.notify();
    }

    pub(super) fn rebind_shortcuts(&mut self, cx: &mut Context<Self>) {
        cx.clear_key_bindings();
        cx.bind_keys(key_bindings());
        cx.bind_keys(crate::keymap::key_bindings(&self.state.shortcuts));
        cx.bind_keys(crate::keymap::command_bindings(
            &self.state.command_shortcuts,
        ));
        cx.bind_keys(super::extensions::shortcut_bindings(
            &self.extension_runtime,
            &self.state,
        ));
        cx.set_menus(menu_bar::menus(&self.state));
        self.terminal_runtime
            .surfaces
            .set_shortcut_combos(terminal_shortcut_combos(&self.state));
        cx.notify();
    }

    pub(super) fn open_in_ide(&mut self, bundle_identifier: &str, cx: &mut Context<Self>) {
        self.dismiss_overlay(cx);
        let Some(project) = self.state.active_project() else {
            return;
        };
        let path = self.state.active_worktree_path(project);
        let selected = (!bundle_identifier.is_empty()).then_some(bundle_identifier);
        let Some(ide) = muxy_api::ide::resolve(selected) else {
            return;
        };
        if !muxy_api::ide::open_project(&path, &ide) {
            return;
        }
        self.state.ide_name = Some(ide.display_name.clone());
        self.state.prefs.ide_bundle_identifier = Some(ide.bundle_identifier);
        cx.notify();
    }

    pub(super) fn apply_settings(&mut self, effect: settings::Effect, cx: &mut Context<Self>) {
        match effect {
            settings::Effect::Chrome => self.apply_chrome_settings(cx),
            settings::Effect::Scale => self.apply_scale_setting(cx),
            settings::Effect::Theme => self.apply_theme_setting(cx),
            settings::Effect::Shortcuts => {
                self.state.shortcuts = muxy_core::shortcuts::ShortcutMap::load();
                self.rebind_shortcuts(cx);
            }
            settings::Effect::CommandShortcuts => {
                self.state.command_shortcuts = muxy_core::store::CommandShortcuts::load();
                self.rebind_shortcuts(cx);
            }
            settings::Effect::All => {
                self.apply_scale_setting(cx);
                self.apply_theme_setting(cx);
                self.apply_chrome_settings(cx);
                self.state.shortcuts = muxy_core::shortcuts::ShortcutMap::load();
                self.state.command_shortcuts = muxy_core::store::CommandShortcuts::load();
                self.rebind_shortcuts(cx);
            }
        }
    }

    pub(super) fn apply_theme_setting(&mut self, cx: &mut Context<Self>) {
        let prefs = Prefs::load();
        self.state.prefs.dark_theme = prefs.dark_theme;
        self.state.prefs.light_theme = prefs.light_theme;
        self.state.theme = match self.state.appearance {
            muxy_ui::theme::Appearance::Light => {
                crate::themes::load(&self.state.prefs.light_theme, "Muxy Light")
            }
            muxy_ui::theme::Appearance::Dark => {
                crate::themes::load(&self.state.prefs.dark_theme, "Muxy")
            }
        };
        let theme = self.state.theme.clone();
        self.terminal_runtime
            .surfaces
            .backend_mut()
            .set_backdrop(theme.bg.into());
        let metrics = self.state.metrics;
        match &self.view.overlay {
            Overlay::Settings(modal) => {
                modal.update(cx, |modal, cx| modal.set_appearance(theme, metrics, cx));
            }
            Overlay::ThemePicker { browser, .. } => {
                browser.update(cx, |browser, cx| {
                    browser.set_appearance(theme, metrics, cx);
                });
            }
            _ => {}
        }
        self.terminal_runtime.surfaces.backend_mut().reload_config();
        let quick_terminal_theme = self.state.theme.clone();
        let quick_terminal_appearance = self.state.appearance;
        let quick_terminal_metrics = self.state.metrics;
        cx.update_global::<crate::quick_terminal::runtime::QuickTerminalRuntime, _>(
            |runtime, cx| {
                runtime.update_appearance(
                    quick_terminal_theme,
                    quick_terminal_appearance,
                    quick_terminal_metrics,
                    cx,
                );
            },
        );
        cx.notify();
    }

    pub(super) fn apply_chrome_settings(&mut self, cx: &mut Context<Self>) {
        let fresh = Prefs::load();
        let prefs = &mut self.state.prefs;
        prefs.show_home_project = fresh.show_home_project;
        prefs.show_status_bar = fresh.show_status_bar;
        prefs.show_topbar_actions = fresh.show_topbar_actions;
        prefs.show_project_search = fresh.show_project_search;
        prefs.show_tips = fresh.show_tips;
        prefs.browser_enabled = fresh.browser_enabled;
        prefs.keep_projects_open = fresh.keep_projects_open;
        prefs.tab_max_width = fresh.tab_max_width;
        prefs.collapsed_style = fresh.collapsed_style;
        prefs.expanded_style = fresh.expanded_style;
        prefs.sort_mode = fresh.sort_mode;
        prefs.project_search_root = fresh.project_search_root;
        let show_home = prefs.show_home_project;
        let sort_mode = prefs.sort_mode;
        self.state.workspace.set_show_home(show_home);
        self.state.workspace.set_sort_mode(sort_mode);
        cx.notify();
    }

    pub(super) fn apply_scale_setting(&mut self, cx: &mut Context<Self>) {
        let preset = Prefs::load().scale;
        self.state.prefs.scale = preset;
        self.state.metrics = muxy_ui::theme::Metrics::new(preset.multiplier());
        let theme = self.state.theme.clone();
        let metrics = self.state.metrics;
        if let Overlay::Settings(modal) = &self.view.overlay {
            modal.update(cx, |modal, cx| modal.set_appearance(theme, metrics, cx));
        }
        let quick_terminal_theme = self.state.theme.clone();
        let quick_terminal_appearance = self.state.appearance;
        cx.update_global::<crate::quick_terminal::runtime::QuickTerminalRuntime, _>(
            |runtime, cx| {
                runtime.update_appearance(
                    quick_terminal_theme,
                    quick_terminal_appearance,
                    metrics,
                    cx,
                );
            },
        );
        cx.notify();
    }

    pub(crate) fn reload_configuration(&mut self, cx: &mut Context<Self>) {
        self.apply_theme_setting(cx);
    }
}

fn project_sort_setting(mode: muxy_core::prefs::SortMode) -> (&'static str, Value) {
    ("muxy.projectSortMode", Value::String(mode.raw().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxy_core::prefs::SortMode;

    #[test]
    fn chrome_sort_command_selects_the_portable_project_sort_key() {
        for mode in SortMode::ALL {
            assert_eq!(
                project_sort_setting(mode),
                ("muxy.projectSortMode", Value::String(mode.raw().to_owned()))
            );
        }
    }
}
