use super::*;

pub(super) fn content(modal: &SettingsModal, cx: &mut Context<SettingsModal>) -> Vec<AnyElement> {
    let category = Category::Projects;
    let style = modal.style();
    let custom = settings::string_value(PICKER_MODE, "custom") == "custom";
    let mut sections = Vec::new();

    let mut project_rows = vec![picker_row(
        modal,
        "Muxy Picker",
        PICKER_MODE,
        "custom",
        vec![
            Choice::new("custom", "Custom"),
            Choice::new("finder", "Finder"),
        ],
        cx,
    )];
    if custom {
        project_rows.push(search_location_row(modal, cx));
    }
    project_rows.push(picker_row(
        modal,
        "Sort Projects By",
        SORT_MODE,
        "manual",
        vec![
            Choice::new("manual", "Manual"),
            Choice::new("nameAscending", "Name (A–Z)"),
            Choice::new("nameDescending", "Name (Z–A)"),
            Choice::new("recentlyActive", "Recently Active"),
            Choice::new("dateCreated", "Date Added"),
        ],
        cx,
    ));
    project_rows.push(toggle_row(
        style,
        "Keep projects open after closing the last tab",
        "muxy.projects.keepOpenWhenNoTabs",
        false,
        cx,
    ));

    let projects_footer = if custom {
        "Muxy Picker searches this location by folder name. Use App Default to search your home folder. Projects can stay in the sidebar after closing their last tab."
    } else {
        "Muxy Picker can use Finder or Muxy's picker. Projects can stay in the sidebar after closing their last tab."
    };
    sections.extend(visible(
        modal,
        category,
        "Projects",
        Some(projects_footer),
        true,
        project_rows,
    ));

    let opener = settings::string_value(FILE_OPENER, "");
    let opener_available = modal
        .extension_options()
        .file_openers
        .iter()
        .any(|(value, _)| value == &opener);
    let opener_footer = if opener.is_empty() || opener_available {
        "Terminal file links use this opener. Built-in and unmatched extension files use the project target selected separately in the top bar."
    } else {
        "The selected extension opener is unavailable, so terminal file links currently use the project target selected separately in the top bar."
    };
    sections.extend(visible(
        modal,
        category,
        "Open Files With",
        Some(opener_footer),
        true,
        vec![picker_row(
            modal,
            "Default Opener",
            FILE_OPENER,
            "",
            appended_stored(
                std::iter::once(Choice::new("", "Built-in (Top Bar Project Target)"))
                    .chain(
                        modal
                            .extension_options()
                            .file_openers
                            .iter()
                            .cloned()
                            .map(|(value, label)| Choice::new(value, label)),
                    )
                    .collect(),
                &opener,
                unavailable_label(&opener, "Unavailable Extension Opener"),
            ),
            cx,
        )],
    ));

    sections.extend(visible(
        modal,
        category,
        "Worktrees",
        Some(
            "Templates must include {branch}; {project-name} and {base-dir} are optional. Relative templates start from the project folder. Folder mode keeps the existing project and worktree subfolder layout.",
        ),
        false,
        vec![worktree_location(modal, cx)],
    ));

    sections
}

fn search_location_row(modal: &SettingsModal, cx: &mut Context<SettingsModal>) -> AnyElement {
    let style = modal.style();
    let metrics = style.metrics;
    let service = PathService::default();
    let stored = settings::string_value(SEARCH_LOCATION, "");
    let uses_default = stored.trim().is_empty();
    let path = service.expanded_path(if uses_default {
        &service.home_directory
    } else {
        stored.trim()
    });
    let status = service.location_status(&path);
    let warning = match status {
        LocationStatus::Ready => None,
        LocationStatus::Missing => {
            Some("Search location no longer exists. Choose another folder or use the app default.")
        }
        LocationStatus::NotDirectory => {
            Some("Search location is not a folder. Choose another folder or use the app default.")
        }
        LocationStatus::Unreadable => Some(
            "Search location can’t be read. Choose another folder, fix permissions, or use the app default.",
        ),
    };
    let initial = if status.is_ready() {
        path.clone()
    } else {
        service.home_directory.clone()
    };

    let mut block =
        div()
            .flex()
            .flex_col()
            .gap(metrics.spacing3())
            .px(metrics.spacing6())
            .py(metrics.spacing3())
            .child(
                div()
                    .text_size(metrics.font_body())
                    .text_color(style.theme.fg)
                    .child(SharedString::from("Folder search location")),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(metrics.spacing4())
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .flex_grow()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .h(metrics.control_medium())
                            .px(metrics.spacing4())
                            .rounded(metrics.radius_sm())
                            .bg(style.theme.surface)
                            .border_1()
                            .border_color(style.theme.border)
                            .text_size(metrics.font_footnote())
                            .text_color(if uses_default {
                                style.theme.fg_muted
                            } else {
                                style.theme.fg
                            })
                            .child(div().flex_grow().min_w(px(0.0)).truncate().child(
                                SharedString::from(service.abbreviated_display_path(&path)),
                            )),
                    )
                    .child(controls::button(
                        style,
                        "search-location-choose",
                        "Choose Folder...",
                        true,
                        cx.listener(move |modal: &mut SettingsModal, _, _, cx| {
                            choose_search_location(modal, initial.clone(), cx);
                        }),
                    ))
                    .child(controls::button(
                        style,
                        "search-location-reset",
                        "Use App Default",
                        !uses_default,
                        cx.listener(|modal: &mut SettingsModal, _, _, cx| {
                            modal.write(SEARCH_LOCATION, Value::Null, cx);
                        }),
                    )),
            );

    if let Some(warning) = warning {
        block = block.child(
            div()
                .text_size(metrics.font_footnote())
                .text_color(style.theme.warning)
                .child(SharedString::from(warning)),
        );
    }

    block.into_any_element()
}

fn choose_search_location(
    modal: &mut SettingsModal,
    directory: String,
    cx: &mut Context<SettingsModal>,
) {
    modal.close_picker(cx);
    cx.spawn(async move |modal, cx| {
        let request = crate::views::file_dialog::FolderRequest {
            message: "Select where Muxy searches for project folders",
            directory: Some(directory),
        };
        let Some(path) = crate::views::file_dialog::pick_folder(request).await else {
            return;
        };
        let path = path.to_string_lossy().into_owned();
        let _ = modal.update(cx, |modal, cx| {
            modal.write(SEARCH_LOCATION, Value::String(path), cx);
        });
    })
    .detach();
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum WorktreeMode {
    Default,
    Template,
    Folder,
}

impl WorktreeMode {
    fn raw(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Template => "template",
            Self::Folder => "folder",
        }
    }

    fn parse(raw: &str) -> Self {
        match raw {
            "template" => Self::Template,
            "folder" => Self::Folder,
            _ => Self::Default,
        }
    }
}

pub(super) fn worktree_mode(modal: &SettingsModal) -> WorktreeMode {
    if let Some(raw) = modal.selection(WORKTREE_MODE) {
        return WorktreeMode::parse(raw);
    }
    if !settings::string_value(WORKTREE_TEMPLATE, "")
        .trim()
        .is_empty()
    {
        return WorktreeMode::Template;
    }
    if !settings::string_value(WORKTREE_PARENT, "")
        .trim()
        .is_empty()
    {
        return WorktreeMode::Folder;
    }
    WorktreeMode::Default
}

fn template_error(template: &str) -> Option<&'static str> {
    if template.trim().is_empty() {
        return Some("Path template is required.");
    }
    if !template.contains("{branch}") {
        return Some("Path template must include {branch}.");
    }
    None
}

pub(super) fn persist_worktree_location(
    modal: &mut SettingsModal,
    mode: WorktreeMode,
    value: &str,
    cx: &mut Context<SettingsModal>,
) {
    match mode {
        WorktreeMode::Default => {
            modal.write(WORKTREE_TEMPLATE, Value::String(String::new()), cx);
            modal.write(WORKTREE_PARENT, Value::String(String::new()), cx);
        }
        WorktreeMode::Template => {
            if template_error(value).is_some() {
                return;
            }
            modal.write(
                WORKTREE_TEMPLATE,
                Value::String(value.trim().to_owned()),
                cx,
            );
            modal.write(WORKTREE_PARENT, Value::String(String::new()), cx);
        }
        WorktreeMode::Folder => {
            if value.trim().is_empty() {
                return;
            }
            modal.write(WORKTREE_PARENT, Value::String(value.trim().to_owned()), cx);
            modal.write(WORKTREE_TEMPLATE, Value::String(String::new()), cx);
        }
    }
}

fn worktree_location(modal: &SettingsModal, cx: &mut Context<SettingsModal>) -> AnyElement {
    let style = modal.style();
    let metrics = style.metrics;
    let mode = worktree_mode(modal);

    let mut block = div()
        .flex()
        .flex_col()
        .gap(metrics.spacing4())
        .px(metrics.spacing6())
        .py(metrics.spacing3())
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .child(
                    div()
                        .flex_shrink()
                        .min_w(px(0.0))
                        .text_size(metrics.font_body())
                        .text_color(style.theme.fg)
                        .child(SharedString::from("Default worktree location")),
                )
                .child(div().flex_grow().min_w(metrics.spacing6()))
                .child(controls::segmented(
                    style,
                    WORKTREE_MODE,
                    vec![
                        Choice::new("default", "App Default"),
                        Choice::new("template", "Template"),
                        Choice::new("folder", "Folder"),
                    ],
                    mode.raw(),
                    cx.listener(|modal: &mut SettingsModal, value: &SharedString, _, cx| {
                        let mode = WorktreeMode::parse(value);
                        modal.set_selection(WORKTREE_MODE, mode.raw(), cx);
                        let current = match mode {
                            WorktreeMode::Default => String::new(),
                            WorktreeMode::Template => settings::string_value(WORKTREE_TEMPLATE, ""),
                            WorktreeMode::Folder => settings::string_value(WORKTREE_PARENT, ""),
                        };
                        persist_worktree_location(modal, mode, &current, cx);
                    }),
                )),
        );

    match mode {
        WorktreeMode::Default => {
            block = block.child(
                div()
                    .text_size(metrics.font_footnote())
                    .text_color(style.theme.fg_muted)
                    .child(SharedString::from("Muxy App Support")),
            );
        }
        WorktreeMode::Template => {
            if let Some(field) = modal.field(WORKTREE_TEMPLATE) {
                block = block.child(controls::text_field(style, WORKTREE_TEMPLATE, field, None));
            }
        }
        WorktreeMode::Folder => {
            if let Some(field) = modal.field(WORKTREE_PARENT) {
                block = block.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(metrics.spacing4())
                        .child(controls::text_field(style, WORKTREE_PARENT, field, None))
                        .child(controls::button(
                            style,
                            "worktree-parent-choose",
                            "Choose Folder...",
                            true,
                            cx.listener(|modal: &mut SettingsModal, _, _, cx| {
                                choose_worktree_parent(modal, cx);
                            }),
                        )),
                );
            }
        }
    }

    if let Some(message) = worktree_validation_message(modal, mode, cx) {
        block = block.child(
            div()
                .text_size(metrics.font_footnote())
                .font_weight(FontWeight::NORMAL)
                .text_color(style.theme.danger)
                .child(SharedString::from(message)),
        );
    }

    block.into_any_element()
}

fn worktree_validation_message(
    modal: &SettingsModal,
    mode: WorktreeMode,
    cx: &Context<SettingsModal>,
) -> Option<String> {
    let message = match mode {
        WorktreeMode::Default => None,
        WorktreeMode::Template => template_error(&modal.field_text(WORKTREE_TEMPLATE, cx)),
        WorktreeMode::Folder => {
            if modal.field_text(WORKTREE_PARENT, cx).trim().is_empty() {
                Some("Folder is required.")
            } else {
                None
            }
        }
    }?;
    Some(format!(
        "{message} {} remains active.",
        persisted_worktree_description()
    ))
}

fn persisted_worktree_description() -> String {
    let template = settings::string_value(WORKTREE_TEMPLATE, "");
    if !template.trim().is_empty() {
        return format!("Saved template {}", template.trim());
    }
    let parent = settings::string_value(WORKTREE_PARENT, "");
    if !parent.trim().is_empty() {
        return format!("Saved folder {}", parent.trim());
    }
    "App Default".to_owned()
}

fn choose_worktree_parent(modal: &mut SettingsModal, cx: &mut Context<SettingsModal>) {
    modal.close_picker(cx);
    let directory = path_service::standardize(&settings::string_value(WORKTREE_PARENT, ""));
    cx.spawn(async move |modal, cx| {
        let request = crate::views::file_dialog::FolderRequest {
            message: "Select the default folder for new worktrees",
            directory: (!directory.trim().is_empty()).then_some(directory),
        };
        let Some(path) = crate::views::file_dialog::pick_folder(request).await else {
            return;
        };
        let path = path.to_string_lossy().into_owned();
        let _ = modal.update(cx, |modal, cx| {
            modal.set_selection(WORKTREE_MODE, WorktreeMode::Folder.raw(), cx);
            persist_worktree_location(modal, WorktreeMode::Folder, &path, cx);
        });
    })
    .detach();
}
