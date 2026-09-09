use super::MainWindow;
use gpui::{Context, KeyBinding, Pixels, Point};
use muxy_core::extensions::manifest::ExtensionCommandAction;
use muxy_core::workspace::{Tab, TabKind};

#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = extension_shortcuts, no_json)]
pub(crate) struct RunExtensionShortcut {
    pub extension_id: String,
    pub command_id: String,
}

pub(crate) fn shortcut_bindings(
    runtime: &crate::extensions::ExtensionRuntime,
    state: &crate::state::AppState,
) -> Vec<KeyBinding> {
    let mut occupied = state.shortcuts.assigned_combos();
    occupied.push(state.command_shortcuts.prefix_combo.clone());
    runtime
        .shortcut_bindings(occupied)
        .into_iter()
        .filter_map(|binding| {
            Some(KeyBinding::new(
                &binding.combo.keystroke()?,
                RunExtensionShortcut {
                    extension_id: binding.extension_id,
                    command_id: binding.command_id,
                },
                Some(muxy_core::shortcuts::KEY_CONTEXT),
            ))
        })
        .collect()
}

impl MainWindow {
    pub(crate) fn extension_settings_options(
        &self,
    ) -> crate::views::settings::ExtensionSettingsOptions {
        let mut sidebars = Vec::new();
        let mut file_openers = Vec::new();
        for (extension_id, record) in self.extension_runtime.catalog().records() {
            if !record.enabled {
                continue;
            }
            let name = record.extension.manifest.name.clone();
            if let Some(sidebar) = &record.extension.manifest.sidebar {
                sidebars.push((
                    extension_id.clone(),
                    sidebar.title.clone().unwrap_or_else(|| name.clone()),
                ));
            }
            for opener in &record.extension.manifest.file_openers {
                let label = opener
                    .title
                    .as_ref()
                    .map(|title| format!("{name} ({title})"))
                    .unwrap_or_else(|| name.clone());
                file_openers.push((format!("{extension_id}:{}", opener.id), label));
            }
        }
        crate::views::settings::ExtensionSettingsOptions {
            sidebars,
            file_openers,
        }
    }

    pub(crate) fn run_extension_command(
        &mut self,
        extension_id: &str,
        command_id: &str,
        anchor: Option<Point<Pixels>>,
        cx: &mut Context<Self>,
    ) {
        let Some((action, permission)) = self
            .extension_runtime
            .catalog()
            .records()
            .get(extension_id)
            .filter(|record| record.enabled)
            .and_then(|record| {
                record
                    .extension
                    .manifest
                    .commands
                    .iter()
                    .find(|command| command.id == command_id)
                    .map(|command| (command.action.clone(), command.action.required_permission()))
            })
        else {
            return;
        };
        if let Some(permission) = permission
            && !self
                .extension_runtime
                .catalog()
                .records()
                .get(extension_id)
                .is_some_and(|record| record.extension.manifest.permissions.contains(&permission))
        {
            self.feedback(
                "Extension Command",
                format!("Permission denied ({})", permission.as_str()),
                crate::toast::ToastTone::Error,
                cx,
            );
            return;
        }
        match action {
            ExtensionCommandAction::Event => {
                self.publish_extension_command_event(extension_id, command_id);
            }
            ExtensionCommandAction::OpenTab { tab_type, data } => {
                let tab_type = self
                    .extension_runtime
                    .catalog()
                    .records()
                    .get(extension_id)
                    .and_then(|record| record.extension.manifest.tab_type(&tab_type))
                    .cloned();
                let Some(tab_type) = tab_type else {
                    return;
                };
                let mut tab = Tab::new(TabKind::ExtensionWebView);
                tab.project_path = self
                    .state
                    .active_project()
                    .map(|project| self.state.active_worktree_path(project));
                tab.static_title = Some(tab_type.title);
                tab.extension_id = Some(extension_id.to_owned());
                tab.extension_web_view_id = Some(tab_type.id);
                tab.extension_data = data.or(tab_type.default_data);
                self.apply_extension_app_effects(
                    vec![crate::extensions::api::ExtensionAppEffect::OpenTab {
                        tab,
                        directory: None,
                        command: None,
                    }],
                    cx,
                );
            }
            ExtensionCommandAction::TogglePanel { panel } => {
                self.apply_extension_app_effects(
                    vec![crate::extensions::api::ExtensionAppEffect::OpenPanel {
                        extension_id: extension_id.to_owned(),
                        panel_id: panel,
                        data: None,
                        toggle: true,
                    }],
                    cx,
                );
            }
            ExtensionCommandAction::OpenPopover { popover } => {
                let Some(anchor) = anchor else {
                    return;
                };
                match self.extension_surfaces.toggle_popover(
                    self.extension_runtime.catalog(),
                    extension_id,
                    &popover,
                    None,
                ) {
                    Ok(changes) => {
                        self.extension_popover_anchor = Some(anchor);
                        self.publish_extension_surface_changes(&changes);
                        cx.notify();
                    }
                    Err(error) => log::warn!("failed to open extension popover: {error}"),
                }
            }
            ExtensionCommandAction::OpenModal {
                entry,
                width,
                height,
                dismiss_on_outside_click,
                data,
            } => {
                self.apply_extension_app_effects(
                    vec![crate::extensions::api::ExtensionAppEffect::OpenModal {
                        extension_id: extension_id.to_owned(),
                        entry,
                        width,
                        height,
                        dismiss_on_outside_click,
                        data,
                        request_id: None,
                        opener_surface_id: None,
                    }],
                    cx,
                );
            }
            ExtensionCommandAction::RunScript { script } => {
                if let Err(error) = self.extension_runtime.run_script(extension_id, &script) {
                    self.feedback(
                        "Extension Command",
                        error,
                        crate::toast::ToastTone::Error,
                        cx,
                    );
                }
            }
        }
    }

    pub(crate) fn open_terminal_url(
        &mut self,
        source_tab_id: &str,
        value: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self
            .state
            .active_tab_workspace()
            .filter(|workspace| workspace.tab(source_tab_id).is_some())
        else {
            cx.open_url(value);
            return;
        };
        let Some(project) = self
            .state
            .active_project()
            .filter(|project| !project.is_remote())
        else {
            cx.open_url(value);
            return;
        };
        let project_root = workspace
            .worktree_path
            .as_deref()
            .unwrap_or(project.path.as_str());
        let Some(location) = muxy_core::extensions::file_openers::resolve_file_location(
            value,
            std::path::Path::new(project_root),
        ) else {
            cx.open_url(value);
            return;
        };
        let selection = muxy_core::prefs::settings::string_value("muxy.defaultFileOpener", "");
        let Some(binding) = muxy_core::extensions::file_openers::resolve_file_opener(
            self.extension_runtime.catalog(),
            &selection,
            &location.relative_path,
        ) else {
            cx.open_url(value);
            return;
        };
        let data = serde_json::json!({
            "filePath": location.relative_path,
            "line": location.line,
            "column": location.column,
            "source": "terminal",
        });
        if binding.opener.singleton {
            let existing = self.state.active_tab_workspace().and_then(|workspace| {
                workspace
                    .root
                    .iter()
                    .flat_map(|root| root.tabs())
                    .find_map(|tab| {
                        (tab.kind == TabKind::ExtensionWebView
                            && tab.extension_id.as_deref() == Some(binding.extension_id.as_str())
                            && tab.extension_web_view_id.as_deref()
                                == Some(binding.tab_type.id.as_str()))
                        .then(|| {
                            workspace
                                .area_containing_tab(&tab.id)
                                .map(|area| (area.id.clone(), tab.id.clone()))
                        })
                        .flatten()
                    })
            });
            if let Some((area_id, tab_id)) = existing {
                if let Some(tab) = self
                    .state
                    .active_tab_workspace_mut()
                    .and_then(|workspace| workspace.tab_mut(&tab_id))
                {
                    tab.extension_data = Some(data);
                }
                let _ = self.state.persist_tab_workspaces();
                self.focus_workspace_tab(&area_id, &tab_id, cx);
                return;
            }
        }
        let area_id = self
            .state
            .active_tab_workspace()
            .and_then(|workspace| workspace.area_containing_tab(source_tab_id))
            .map(|area| area.id.clone());
        let Some(area_id) = area_id else {
            cx.open_url(value);
            return;
        };
        let mut tab = Tab::new(TabKind::ExtensionWebView);
        tab.project_path = Some(project_root.to_owned());
        tab.static_title = Some(
            binding
                .opener
                .title
                .clone()
                .unwrap_or_else(|| binding.tab_type.title.clone()),
        );
        tab.extension_id = Some(binding.extension_id);
        tab.extension_web_view_id = Some(binding.tab_type.id);
        tab.extension_data = Some(data);
        if self
            .state
            .active_tab_workspace_mut()
            .and_then(|workspace| workspace.area_mut(&area_id))
            .map(|area| area.insert_tab(usize::MAX, tab, true))
            .is_some()
        {
            let _ = self.state.persist_tab_workspaces();
            cx.notify();
        }
    }

    fn publish_extension_command_event(&self, extension_id: &str, command_id: &str) {
        let name = format!("command.{command_id}");
        let payload = serde_json::json!({
            "command": command_id,
            "extension": extension_id,
        });
        self.extension_webviews
            .dispatch_event(Some(extension_id), &name, &payload);
        self.extension_runtime
            .broadcast(muxy_proto::extension::ExtensionBroadcast {
                name,
                payload: std::collections::BTreeMap::from([
                    ("command".to_owned(), command_id.to_owned()),
                    ("extension".to_owned(), extension_id.to_owned()),
                ]),
            });
    }
}
