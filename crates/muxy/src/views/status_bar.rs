use crate::state::AppState;
use crate::views::menu::Item;
use crate::views::window::MainWindow;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, AppContext, Context, FontWeight, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, SharedString, StatefulInteractiveElement, Styled, canvas, div,
    px,
};
use muxy_core::prefs::home_dir;
use muxy_ui::components::{IconGlyph, Tooltip};
use muxy_ui::icon::Icon;
use std::cell::Cell;
use std::rc::Rc;

const PATH_MAX_CHARACTERS: usize = 40;

pub fn status_bar(
    state: &AppState,
    working_directory: Option<&str>,
    repository_controls: &[crate::repository::RepositoryControl],
    repository_mutation_busy: bool,
    repository_ai_menu_available: bool,
    trailing: Option<AnyElement>,
    extension_items: &[crate::extensions::ExtensionStatusBarBinding],
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let metrics = &state.metrics;
    let theme = &state.theme;

    let mut left = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .h_full()
        .min_w(px(0.0))
        .px(px(10.0))
        .flex_grow();

    let path =
        working_directory.or_else(|| state.active_project().map(|project| project.path.as_str()));
    for control in status_controls(path.is_some(), repository_controls) {
        match control {
            StatusControl::Path => {
                let path = path.expect("path control requires a path");
                let remote = state
                    .active_project()
                    .is_some_and(|project| project.is_remote());
                left = left.child(path_chip(state, path, remote, cx));
            }
            StatusControl::Separator => {
                left = left.child(status_separator(state));
            }
            StatusControl::Branch => {
                if let Some(control) = repository_controls.iter().find(|control| {
                    control.kind == crate::repository::RepositoryControlKind::Branch
                }) {
                    left = left.child(branch_chip(state, control, cx));
                }
            }
            StatusControl::Changes => {
                if let Some(control) = repository_controls.iter().find(|control| {
                    control.kind == crate::repository::RepositoryControlKind::Changes
                }) {
                    left = left.child(changes_chip(state, control, cx));
                }
            }
            StatusControl::CommitAi => {
                if let Some(control) = repository_controls.iter().find(|control| {
                    control.kind == crate::repository::RepositoryControlKind::CommitAi
                }) {
                    left = left.child(ai_chip(
                        "status-commit-ai",
                        state,
                        control,
                        repository_mutation_busy,
                        repository_ai_menu_available,
                        (
                            MainWindow::open_commit_ai_confirmation,
                            MainWindow::open_commit_ai_provider_menu,
                        ),
                        cx,
                    ));
                }
            }
            StatusControl::CreatePullRequestAi => {
                if let Some(control) = repository_controls.iter().find(|control| {
                    control.kind == crate::repository::RepositoryControlKind::CreatePullRequestAi
                }) {
                    left = left.child(ai_chip(
                        "status-create-pr-ai",
                        state,
                        control,
                        repository_mutation_busy,
                        repository_ai_menu_available,
                        (
                            MainWindow::open_create_pr_ai_confirmation,
                            MainWindow::open_create_pr_ai_provider_menu,
                        ),
                        cx,
                    ));
                }
            }
            StatusControl::PullRequest => {
                if let Some(control) = repository_controls.iter().find(|control| {
                    control.kind == crate::repository::RepositoryControlKind::PullRequest
                        && control.tone != crate::repository::RepositoryControlTone::Default
                }) {
                    left = left.child(pull_request_chip(state, control, cx));
                }
            }
        }
    }
    for item in extension_items
        .iter()
        .filter(|item| item.side == muxy_core::extensions::manifest::StatusBarSide::Left)
    {
        left = left.child(extension_item(state, item, cx));
    }

    let mut right = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .h_full()
        .flex_none()
        .px(px(10.0));
    for item in extension_items
        .iter()
        .filter(|item| item.side == muxy_core::extensions::manifest::StatusBarSide::Right)
    {
        right = right.child(extension_item(state, item, cx));
    }

    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .h(metrics.status_bar_height())
        .bg(theme.bg)
        .border_t(px(1.0))
        .border_color(theme.border)
        .child(left)
        .child(right)
        .when_some(trailing, |bar, trailing| {
            bar.child(status_separator(state)).child(trailing)
        })
        .into_any_element()
}

fn extension_item(
    state: &AppState,
    item: &crate::extensions::ExtensionStatusBarBinding,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let extension_id = item.extension_id.clone();
    let command = item.command.clone();
    let icon = crate::extensions::surface_view::extension_icon(
        &item.icon,
        &item.resource_root,
        state.metrics.font_caption(),
        state.theme.fg_muted,
    );
    let tooltip = item
        .tooltip
        .clone()
        .unwrap_or_else(|| format!("{} extension action", item.item_id));
    div()
        .id(SharedString::from(format!(
            "extension-statusbar-{}-{}",
            item.extension_id, item.item_id
        )))
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(px(4.0))
        .h_full()
        .px(px(2.0))
        .cursor_pointer()
        .text_color(state.theme.fg_muted)
        .hover(|style| style.text_color(state.theme.fg))
        .tooltip({
            let tooltip = SharedString::from(tooltip);
            let background = state.theme.raised();
            let foreground = state.theme.fg;
            let border = state.theme.border;
            move |_, cx| {
                cx.new(|_| Tooltip::new(tooltip.clone(), background, foreground, border))
                    .into()
            }
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(
                move |window: &mut MainWindow, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    window.run_extension_command(&extension_id, &command, Some(event.position), cx);
                },
            ),
        )
        .child(icon)
        .children(item.text.clone().map(|text| {
            div()
                .text_size(state.metrics.font_footnote())
                .font_weight(FontWeight::MEDIUM)
                .child(SharedString::from(text))
        }))
        .into_any_element()
}

#[derive(Clone, Copy)]
enum StatusControl {
    Path,
    Separator,
    Branch,
    Changes,
    CommitAi,
    CreatePullRequestAi,
    PullRequest,
}

fn status_controls(
    has_path: bool,
    repository_controls: &[crate::repository::RepositoryControl],
) -> Vec<StatusControl> {
    let mut controls = Vec::new();
    if has_path {
        controls.push(StatusControl::Path);
    }
    for control in [
        StatusControl::Branch,
        StatusControl::Changes,
        StatusControl::CommitAi,
        StatusControl::CreatePullRequestAi,
        StatusControl::PullRequest,
    ] {
        let kind = match control {
            StatusControl::Branch => crate::repository::RepositoryControlKind::Branch,
            StatusControl::Changes => crate::repository::RepositoryControlKind::Changes,
            StatusControl::CommitAi => crate::repository::RepositoryControlKind::CommitAi,
            StatusControl::CreatePullRequestAi => {
                crate::repository::RepositoryControlKind::CreatePullRequestAi
            }
            StatusControl::PullRequest => crate::repository::RepositoryControlKind::PullRequest,
            StatusControl::Path | StatusControl::Separator => unreachable!(),
        };
        if !repository_controls.iter().any(|candidate| {
            candidate.kind == kind
                && (!matches!(control, StatusControl::PullRequest)
                    || candidate.tone != crate::repository::RepositoryControlTone::Default)
        }) {
            continue;
        }
        if !controls.is_empty() {
            controls.push(StatusControl::Separator);
        }
        controls.push(control);
    }
    controls
}

#[cfg(test)]
fn status_control_id(control: &StatusControl) -> &'static str {
    match control {
        StatusControl::Path => "status-path",
        StatusControl::Separator => "status-separator",
        StatusControl::Branch => "status-branch",
        StatusControl::Changes => "status-changes",
        StatusControl::CommitAi => "status-commit-ai",
        StatusControl::CreatePullRequestAi => "status-create-pr-ai",
        StatusControl::PullRequest => "status-pull-request",
    }
}

fn ai_chip(
    id: &'static str,
    state: &AppState,
    control: &crate::repository::RepositoryControl,
    mutation_busy: bool,
    menu_available: bool,
    openers: (RepositoryPopoverOpener, RepositoryPopoverOpener),
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let metrics = &state.metrics;
    let theme = &state.theme;
    let (confirm, providers) = openers;
    let running = control.label.ends_with('…');
    let (primary_enabled, menu_enabled) =
        ai_chip_availability(control.enabled, running, mutation_busy, menu_available);
    let bounds = Rc::new(Cell::new(None));
    let recorder = bounds.clone();
    let confirm_bounds = bounds.clone();
    let provider_bounds = bounds.clone();
    div()
        .id(id)
        .relative()
        .flex()
        .items_center()
        .h_full()
        .text_color(if primary_enabled || menu_enabled {
            theme.fg_muted
        } else {
            theme.fg_dim
        })
        .child(
            canvas(
                move |bounds, _, _| recorder.set(Some(bounds)),
                |_, _: (), _, _| {},
            )
            .absolute()
            .size_full(),
        )
        .child(
            div()
                .h_full()
                .px(px(2.0))
                .flex()
                .items_center()
                .gap(px(4.0))
                .when(primary_enabled, |element| {
                    element
                        .cursor_pointer()
                        .hover(|style| style.text_color(theme.fg))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |window, _: &MouseDownEvent, view, cx| {
                                if let Some(bounds) = confirm_bounds.get() {
                                    confirm(window, bounds, view, cx);
                                }
                            }),
                        )
                })
                .child(IconGlyph::new(
                    Icon::Lightbulb,
                    metrics.font_caption(),
                    if primary_enabled {
                        theme.fg_muted
                    } else {
                        theme.fg_dim
                    },
                ))
                .child(
                    div()
                        .text_size(metrics.font_footnote())
                        .font_weight(FontWeight::MEDIUM)
                        .child(SharedString::from(control.label.clone())),
                ),
        )
        .when(menu_enabled, |element| {
            element.child(
                div()
                    .h_full()
                    .px(px(3.0))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .hover(|style| style.text_color(theme.fg))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |window, _: &MouseDownEvent, view, cx| {
                            cx.stop_propagation();
                            if let Some(bounds) = provider_bounds.get() {
                                providers(window, bounds, view, cx);
                            }
                        }),
                    )
                    .child(IconGlyph::new(
                        Icon::ChevronDown,
                        metrics.icon_xs(),
                        theme.fg_muted,
                    )),
            )
        })
        .tooltip({
            let text = SharedString::from(control.tooltip.clone());
            let background = theme.raised();
            let foreground = theme.fg;
            let border = theme.border;
            move |_, cx| {
                cx.new(|_| Tooltip::new(text.clone(), background, foreground, border))
                    .into()
            }
        })
        .into_any_element()
}

fn ai_chip_availability(
    control_enabled: bool,
    running: bool,
    mutation_busy: bool,
    menu_available: bool,
) -> (bool, bool) {
    (
        control_enabled && (!mutation_busy || running),
        menu_available && !mutation_busy && !running,
    )
}

fn status_separator(state: &AppState) -> AnyElement {
    div()
        .w(px(1.0))
        .h_full()
        .flex_none()
        .bg(state.theme.border)
        .into_any_element()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusPathAction {
    Copy,
    Reveal,
}

fn primary_path_action(remote: bool) -> StatusPathAction {
    if remote {
        StatusPathAction::Copy
    } else {
        StatusPathAction::Reveal
    }
}

fn context_path_actions(remote: bool) -> Vec<StatusPathAction> {
    let mut actions = vec![StatusPathAction::Copy];
    if !remote {
        actions.push(StatusPathAction::Reveal);
    }
    actions
}

fn path_command(action: StatusPathAction, path: String) -> crate::command::Command {
    match action {
        StatusPathAction::Copy => crate::command::Command::CopyStatusPath(path),
        StatusPathAction::Reveal => crate::command::Command::RevealStatusPath(path),
    }
}

pub(crate) fn path_menu_items(path: String, remote: bool) -> Vec<Item> {
    context_path_actions(remote)
        .into_iter()
        .map(|action| {
            let label = match action {
                StatusPathAction::Copy => "Copy Path",
                StatusPathAction::Reveal => "Reveal",
            };
            Item::action(label, path_command(action, path.clone()))
        })
        .collect()
}

pub(crate) fn reveal_failure(path: &str, error: impl std::fmt::Display) -> (String, String) {
    (
        "Could Not Reveal Path".to_owned(),
        format!("Muxy could not reveal {path}: {error}"),
    )
}

fn path_chip(
    state: &AppState,
    path: &str,
    remote: bool,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let metrics = &state.metrics;
    let theme = &state.theme;
    let primary_path = path.to_owned();
    let menu_path = path.to_owned();
    let primary = primary_path_action(remote);

    div()
        .id("status-path")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .h_full()
        .text_color(theme.fg_muted)
        .cursor_pointer()
        .on_click(cx.listener(move |window: &mut MainWindow, _, view, cx| {
            window.perform(path_command(primary, primary_path.clone()), view, cx);
        }))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(
                move |window: &mut MainWindow, event: &MouseDownEvent, view, cx| {
                    window.open_status_path_menu(
                        menu_path.clone(),
                        remote,
                        event.position,
                        view,
                        cx,
                    );
                },
            ),
        )
        .child(IconGlyph::new(
            Icon::Folder,
            metrics.font_caption(),
            theme.fg_muted,
        ))
        .child(
            div()
                .text_size(metrics.font_footnote())
                .font_weight(FontWeight::MEDIUM)
                .child(SharedString::from(truncate_path(&abbreviate_path(path)))),
        )
        .into_any_element()
}

fn branch_chip(
    state: &AppState,
    control: &crate::repository::RepositoryControl,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    repository_chip(
        "status-branch",
        Icon::GitBranch,
        state,
        control,
        MainWindow::open_branch_popover,
        cx,
    )
}

fn changes_chip(
    state: &AppState,
    control: &crate::repository::RepositoryControl,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    repository_chip(
        "status-changes",
        Icon::ArrowUpDown,
        state,
        control,
        MainWindow::open_changes_popover,
        cx,
    )
}

fn pull_request_chip(
    state: &AppState,
    control: &crate::repository::RepositoryControl,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    repository_chip(
        "status-pull-request",
        Icon::GitBranch,
        state,
        control,
        MainWindow::open_pull_request_popover,
        cx,
    )
}

type RepositoryPopoverOpener =
    fn(&mut MainWindow, gpui::Bounds<gpui::Pixels>, &mut gpui::Window, &mut Context<MainWindow>);

fn repository_chip(
    id: &'static str,
    icon: Icon,
    state: &AppState,
    control: &crate::repository::RepositoryControl,
    open: RepositoryPopoverOpener,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let metrics = &state.metrics;
    let theme = &state.theme;
    let enabled = control.enabled;
    let bounds = Rc::new(Cell::new(None));
    let recorder = bounds.clone();
    let click_bounds = bounds.clone();
    let enabled_color = match control.tone {
        crate::repository::RepositoryControlTone::Default => theme.fg_muted,
        crate::repository::RepositoryControlTone::Clean => theme.accent,
        crate::repository::RepositoryControlTone::Dirty => theme.warning,
        crate::repository::RepositoryControlTone::Danger => theme.danger,
    };
    div()
        .id(id)
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .h_full()
        .text_color(if enabled { enabled_color } else { theme.fg_dim })
        .when(enabled, |element| {
            element
                .cursor_pointer()
                .hover(|style| style.text_color(theme.fg))
        })
        .when(enabled, |element| {
            element.on_mouse_down(
                MouseButton::Left,
                cx.listener(
                    move |window: &mut MainWindow, _: &MouseDownEvent, view, cx| {
                        if let Some(bounds) = click_bounds.get() {
                            open(window, bounds, view, cx);
                        }
                    },
                ),
            )
        })
        .child(
            canvas(
                move |bounds, _, _| recorder.set(Some(bounds)),
                |_, _: (), _, _| {},
            )
            .absolute()
            .size_full(),
        )
        .child(IconGlyph::new(
            icon,
            metrics.font_caption(),
            if enabled { enabled_color } else { theme.fg_dim },
        ))
        .child(
            div()
                .text_size(metrics.font_footnote())
                .font_weight(FontWeight::MEDIUM)
                .child(SharedString::from(control.label.clone())),
        )
        .tooltip({
            let text = SharedString::from(control.tooltip.clone());
            let background = theme.raised();
            let foreground = theme.fg;
            let border = theme.border;
            move |_, cx| {
                cx.new(|_| Tooltip::new(text.clone(), background, foreground, border))
                    .into()
            }
        })
        .into_any_element()
}

fn abbreviate_path(path: &str) -> String {
    let home = home_dir();
    let home = home.to_string_lossy();
    match !home.is_empty() && path.starts_with(home.as_ref()) {
        true => format!("~{}", &path[home.len()..]),
        false => path.to_owned(),
    }
}

fn truncate_path(path: &str) -> String {
    if path.chars().count() <= PATH_MAX_CHARACTERS {
        return path.to_owned();
    }
    let suffix: String = path
        .chars()
        .skip(path.chars().count() - (PATH_MAX_CHARACTERS - 1))
        .collect();
    format!("…{suffix}")
}

#[cfg(test)]
fn status_control_ids(has_path: bool) -> Vec<&'static str> {
    status_controls(has_path, &[])
        .iter()
        .map(status_control_id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_path_local_and_remote_actions_are_truthful() {
        assert_eq!(primary_path_action(false), StatusPathAction::Reveal);
        assert_eq!(primary_path_action(true), StatusPathAction::Copy);
        assert_eq!(
            context_path_actions(false),
            vec![StatusPathAction::Copy, StatusPathAction::Reveal]
        );
        assert_eq!(context_path_actions(true), vec![StatusPathAction::Copy]);
    }

    #[test]
    fn status_path_reveal_errors_map_to_the_existing_alert_surface() {
        assert_eq!(
            reveal_failure("/missing/project", "launch failed"),
            (
                "Could Not Reveal Path".to_owned(),
                "Muxy could not reveal /missing/project: launch failed".to_owned(),
            )
        );
    }

    #[test]
    fn chrome_status_contract_excludes_later_owned_controls() {
        assert_eq!(status_control_ids(true), vec!["status-path"]);
        assert!(status_control_ids(false).is_empty());
        assert_eq!(status_control_ids(true), vec!["status-path"]);
    }

    #[test]
    fn phase_six_separates_the_path_and_branch_without_a_leading_separator() {
        let repository = vec![crate::repository::RepositoryControl {
            kind: crate::repository::RepositoryControlKind::Branch,
            label: "main".to_owned(),
            tooltip: "No upstream".to_owned(),
            enabled: true,
            tone: crate::repository::RepositoryControlTone::Default,
        }];
        assert_eq!(
            status_controls(true, &repository)
                .iter()
                .map(status_control_id)
                .collect::<Vec<_>>(),
            ["status-path", "status-separator", "status-branch"]
        );
        assert_eq!(
            status_controls(false, &repository)
                .iter()
                .map(status_control_id)
                .collect::<Vec<_>>(),
            ["status-branch"]
        );
    }

    #[test]
    fn phase_seven_orders_branch_and_changes_with_section_separators() {
        let repository = vec![
            crate::repository::RepositoryControl {
                kind: crate::repository::RepositoryControlKind::Branch,
                label: "main".to_owned(),
                tooltip: "No upstream".to_owned(),
                enabled: true,
                tone: crate::repository::RepositoryControlTone::Default,
            },
            crate::repository::RepositoryControl {
                kind: crate::repository::RepositoryControlKind::Changes,
                label: "2 Changes".to_owned(),
                tooltip: "1 staged · 1 unstaged".to_owned(),
                enabled: true,
                tone: crate::repository::RepositoryControlTone::Dirty,
            },
        ];
        assert_eq!(
            status_controls(true, &repository)
                .iter()
                .map(status_control_id)
                .collect::<Vec<_>>(),
            [
                "status-path",
                "status-separator",
                "status-branch",
                "status-separator",
                "status-changes",
            ]
        );
    }

    #[test]
    fn phase_eight_adds_only_actionable_pull_requests_after_changes() {
        let mut repository = vec![
            crate::repository::RepositoryControl {
                kind: crate::repository::RepositoryControlKind::Branch,
                label: "main".to_owned(),
                tooltip: "No upstream".to_owned(),
                enabled: true,
                tone: crate::repository::RepositoryControlTone::Default,
            },
            crate::repository::RepositoryControl {
                kind: crate::repository::RepositoryControlKind::Changes,
                label: "Clean".to_owned(),
                tooltip: "Clean".to_owned(),
                enabled: true,
                tone: crate::repository::RepositoryControlTone::Clean,
            },
            crate::repository::RepositoryControl {
                kind: crate::repository::RepositoryControlKind::PullRequest,
                label: "#42".to_owned(),
                tooltip: "Pull request #42".to_owned(),
                enabled: true,
                tone: crate::repository::RepositoryControlTone::Clean,
            },
        ];
        let ids = |repository: &[crate::repository::RepositoryControl]| {
            status_controls(true, repository)
                .iter()
                .map(status_control_id)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(&repository),
            [
                "status-path",
                "status-separator",
                "status-branch",
                "status-separator",
                "status-changes",
                "status-separator",
                "status-pull-request",
            ]
        );

        repository[2].enabled = false;
        assert!(ids(&repository).contains(&"status-pull-request"));
        repository[2].label = "Pull Request".to_owned();
        repository[2].tone = crate::repository::RepositoryControlTone::Default;
        assert!(!ids(&repository).contains(&"status-pull-request"));
        repository[2].label = "Retry PR".to_owned();
        repository[2].tone = crate::repository::RepositoryControlTone::Danger;
        repository[2].enabled = true;
        assert!(ids(&repository).contains(&"status-pull-request"));
    }

    #[test]
    fn phase_ten_orders_ai_actions_between_changes_and_pull_request_state() {
        let control = |kind, label: &str, enabled| crate::repository::RepositoryControl {
            kind,
            label: label.to_owned(),
            tooltip: label.to_owned(),
            enabled,
            tone: crate::repository::RepositoryControlTone::Default,
        };
        let repository = vec![
            control(
                crate::repository::RepositoryControlKind::Branch,
                "main",
                true,
            ),
            control(
                crate::repository::RepositoryControlKind::Changes,
                "2 Changes",
                true,
            ),
            control(
                crate::repository::RepositoryControlKind::CommitAi,
                "Commit",
                true,
            ),
            control(
                crate::repository::RepositoryControlKind::CreatePullRequestAi,
                "Create PR",
                true,
            ),
        ];
        assert_eq!(
            status_controls(true, &repository)
                .iter()
                .map(status_control_id)
                .collect::<Vec<_>>(),
            [
                "status-path",
                "status-separator",
                "status-branch",
                "status-separator",
                "status-changes",
                "status-separator",
                "status-commit-ai",
                "status-separator",
                "status-create-pr-ai",
            ]
        );

        let mut pull_request = control(
            crate::repository::RepositoryControlKind::PullRequest,
            "#42",
            true,
        );
        pull_request.tone = crate::repository::RepositoryControlTone::Clean;
        let found = vec![
            repository[0].clone(),
            repository[1].clone(),
            repository[2].clone(),
            pull_request,
        ];
        let ids = status_controls(false, &found)
            .iter()
            .map(status_control_id)
            .collect::<Vec<_>>();
        assert!(ids.contains(&"status-commit-ai"));
        assert!(ids.contains(&"status-pull-request"));
        assert!(!ids.contains(&"status-create-pr-ai"));
    }

    #[test]
    fn provider_menu_remains_available_when_the_ai_action_cannot_run() {
        assert_eq!(
            ai_chip_availability(false, false, false, true),
            (false, true)
        );
        assert_eq!(ai_chip_availability(true, true, true, true), (true, false));
        assert_eq!(
            ai_chip_availability(true, false, true, true),
            (false, false)
        );
    }
}
