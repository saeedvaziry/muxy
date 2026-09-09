use muxy_core::prefs::{CollapsedStyle, ExpandedStyle, Prefs};
use muxy_ui::theme::Metrics;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AppLayout {
    pub sidebar_width: Pixels,
    pub nav_overlay_width: Pixels,
    pub main_titlebar_leading_inset: Pixels,
    pub sidebar_border_top_pad: Pixels,
    pub wide_sidebar: bool,
    pub hidden_sidebar: bool,
}

impl AppLayout {
    pub fn new(prefs: &Prefs, metrics: Metrics, sidebar_expanded: bool) -> Self {
        let wide_sidebar = sidebar_expanded && prefs.expanded_style == ExpandedStyle::Wide;
        let hidden_sidebar = !sidebar_expanded && prefs.collapsed_style == CollapsedStyle::Hidden;
        let collapsed_width = match prefs.collapsed_style {
            CollapsedStyle::Hidden => px(0.0),
            CollapsedStyle::Icons => metrics.sidebar_collapsed_width(),
        };
        let sidebar_width = if !sidebar_expanded {
            collapsed_width
        } else if prefs.expanded_style != ExpandedStyle::Wide {
            metrics.sidebar_collapsed_width()
        } else {
            match prefs.sidebar_expanded_custom_width {
                Some(width) => px(width)
                    .max(metrics.sidebar_expanded_min_width())
                    .min(metrics.sidebar_expanded_max_width()),
                None => metrics.sidebar_expanded_width(),
            }
        };
        let titlebar_nav_width = metrics.traffic_light_width() + metrics.navigation_arrows_width();
        let nav_overlay_width = sidebar_width.max(titlebar_nav_width);
        let main_titlebar_leading_inset = (nav_overlay_width - sidebar_width).max(px(0.0));
        let sidebar_border_top_pad = if nav_overlay_width > sidebar_width {
            metrics.title_bar_height() + px(1.0)
        } else {
            px(0.0)
        };
        Self {
            sidebar_width,
            nav_overlay_width,
            main_titlebar_leading_inset,
            sidebar_border_top_pad,
            wide_sidebar,
            hidden_sidebar,
        }
    }
}

use crate::command::Command;
use crate::state::AppState;
use crate::terminal::TerminalSurfaces;
use crate::views::overlay::Overlay;
use crate::views::window::MainWindow;
use crate::views::window::menu_bar;
use crate::views::{
    omnibox, overlay, sidebar, status_bar, titlebar, toast, welcome, workspace_view,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, Bounds, Context, Entity, FocusHandle, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, NavigationDirection, ParentElement, Pixels,
    Point, Styled, Window, actions, div, px, relative,
};
use muxy_ui::scrollbar::ScrollbarRevealState;
use muxy_ui::text_input::TextInput;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

const SYSTEM_LINE_HEIGHT: f32 = 1.2;

actions!(terminal_surface, [SearchPrevious]);

fn merge_composer_footer_with_status_bar(
    show_status_bar: bool,
    placement: Option<&muxy_ui::panel::PanelPlacement>,
) -> bool {
    show_status_bar
        && placement.is_some_and(|placement| {
            placement.mode == muxy_ui::panel::PanelMode::Pinned
                && placement.position == muxy_ui::panel::PanelPosition::Bottom
        })
}

pub(crate) struct AppView<'a> {
    pub state: &'a AppState,
    pub repository_controls: &'a [crate::repository::RepositoryControl],
    pub repository_state: &'a crate::repository::RepositoryState,
    pub repository_mutation_busy: bool,
    pub layout: AppLayout,
    pub workspace_focus: &'a FocusHandle,
    pub menu_focus: &'a FocusHandle,
    pub terminals: &'a TerminalSurfaces,
    pub extension_webviews: &'a crate::extensions::webview::ExtensionWebViewRegistry,
    pub extension_surfaces: &'a crate::extensions::surfaces::ExtensionSurfaceRegistry,
    pub extension_popover_anchor: Option<Point<Pixels>>,
    pub area_bounds: &'a HashMap<String, Bounds<Pixels>>,
    pub search_inputs: &'a HashMap<String, Entity<TextInput>>,
    pub scrollbar_reveal: &'a HashMap<String, ScrollbarRevealState>,
    pub terminal_attention: &'a HashSet<String>,
    pub bell_flashes: &'a HashMap<String, Duration>,
    pub drag: Option<(&'a str, f64)>,
    pub now: Duration,
    pub overlay: &'a Overlay,
    pub toast: Option<(&'a crate::toast::ToastContent, u64)>,
    pub drop_highlight: Option<(Bounds<Pixels>, muxy_core::workspace::DropZone)>,
    pub focused_working_directory: Option<String>,
    pub expanded_worktree_projects: &'a HashSet<String>,
    pub composer: &'a crate::composer::ComposerController,
    pub panels: &'a crate::panels::PanelRuntime,
    pub extension_topbar_items: &'a [crate::extensions::ExtensionTopbarBinding],
    pub extension_statusbar_items: &'a [crate::extensions::ExtensionStatusBarBinding],
}

pub(crate) fn render(
    view: AppView<'_>,
    window: &mut Window,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let AppView {
        state,
        repository_controls,
        repository_state,
        repository_mutation_busy,
        layout,
        workspace_focus,
        menu_focus,
        terminals,
        extension_webviews,
        extension_surfaces,
        extension_popover_anchor,
        area_bounds,
        search_inputs,
        scrollbar_reveal,
        terminal_attention,
        bell_flashes,
        drag,
        now,
        overlay,
        toast: active_toast,
        drop_highlight,
        focused_working_directory,
        expanded_worktree_projects,
        composer,
        panels,
        extension_topbar_items,
        extension_statusbar_items,
    } = view;
    let theme = state.theme.clone();
    let metrics = state.metrics;
    let sidebar_content = extension_surfaces
        .active_sidebar()
        .and_then(|surface| extension_webviews.element(&surface.instance_id, true))
        .unwrap_or_else(|| {
            sidebar::sidebar(state, layout, expanded_worktree_projects, cx).into_any_element()
        });
    let sidebar_column = div()
        .flex()
        .flex_col()
        .flex_none()
        .w(layout.sidebar_width)
        .h_full()
        .bg(theme.bg)
        .overflow_hidden()
        .child(div().h(metrics.title_bar_height()).flex_none())
        .child(
            div()
                .flex()
                .flex_col()
                .flex_grow()
                .min_h(px(0.0))
                .child(sidebar_content),
        );

    let sidebar_border = div()
        .absolute()
        .top(layout.sidebar_border_top_pad)
        .bottom_0()
        .left(layout.sidebar_width - px(1.0))
        .w(px(1.0))
        .bg(theme.border_solid());

    let tab_workspace = state.active_tab_workspace().cloned();
    let composer_id = muxy_ui::panel::PanelId::from(muxy_core::composer::PANEL_ID);
    let composer_placement = panels.host().placement(&composer_id);
    let pinned_right_slot = muxy_ui::panel::PanelSlot {
        position: muxy_ui::panel::PanelPosition::Right,
        mode: muxy_ui::panel::PanelMode::Pinned,
    };
    let consumed_width = panels
        .host()
        .occupant(pinned_right_slot)
        .map(|panel_id| {
            if panel_id == &composer_id {
                state.prefs.composer.panel_width as f32
            } else {
                extension_surfaces
                    .panel_dimension(panel_id.as_str())
                    .unwrap_or(360.0) as f32
            }
        })
        .unwrap_or(0.0);
    let main_width =
        (f32::from(window.viewport_size().width - layout.sidebar_width) - consumed_width).max(0.0);
    let panes = workspace_view::Panes {
        terminals,
        extension_webviews,
        area_bounds,
        search_inputs,
        reveal: scrollbar_reveal,
        attention: terminal_attention,
        bell_flashes,
        notifications: &state.notification_store,
        drag,
        now,
    };
    let topbar = match &tab_workspace {
        Some(workspace)
            if matches!(
                workspace.top_level_root,
                Some(muxy_core::workspace::TopLevelTabNode::Group { .. }) | None
            ) =>
        {
            workspace_view::titlebar_tab_strip(
                state,
                layout,
                &panes,
                workspace,
                true,
                main_width,
                extension_topbar_items,
                cx,
            )
        }
        _ => titlebar::main_titlebar(state, layout, extension_topbar_items, cx),
    };
    let content = match &tab_workspace {
        Some(workspace) if !shows_welcome(Some(workspace)) => {
            workspace_view::workspace_content(state, &panes, workspace, main_width, cx)
        }
        _ => welcome::workspace_content(state, cx),
    };
    let merge_composer_footer =
        merge_composer_footer_with_status_bar(state.prefs.show_status_bar, composer_placement);
    let composer_panel = crate::composer::view::render(
        composer,
        panels,
        &state.prefs.composer,
        &state.shortcuts,
        muxy_ui::panel::PanelStyle::new(theme.clone(), metrics),
        merge_composer_footer,
        window,
        cx,
    );
    let extension_panels = extension_surfaces
        .active_panels()
        .filter_map(|surface| {
            let placement = panels
                .host()
                .placement(&muxy_ui::panel::PanelId::from(surface.instance_id.clone()))?
                .clone();
            let resize_state = panels.resize_state(&placement.id)?;
            crate::extensions::surface_view::render_panel(
                surface,
                placement,
                extension_surfaces
                    .panel_dimension(&surface.instance_id)
                    .unwrap_or(360.0),
                resize_state,
                extension_webviews,
                &theme,
                metrics,
                cx,
            )
        })
        .collect::<Vec<_>>();
    let repository_ai_menu_available = !repository_mutation_busy
        && matches!(
            repository_state.ai,
            crate::repository::RepositoryAiRunState::Idle
        )
        && matches!(
            repository_state.providers,
            crate::repository::LoadState::Ready(_)
        );
    let mut main_column = div()
        .flex()
        .flex_col()
        .flex_grow()
        .min_w(px(0.0))
        .h_full()
        .child(topbar)
        .child(div().h(px(1.0)).flex_none().bg(theme.border_solid()))
        .child(content);

    if state.prefs.show_status_bar && !merge_composer_footer {
        main_column = main_column.child(status_bar::status_bar(
            state,
            focused_working_directory.as_deref(),
            repository_controls,
            repository_mutation_busy,
            repository_ai_menu_available,
            None,
            extension_statusbar_items,
            cx,
        ));
    }
    let mut pinned_right = None;
    let mut pinned_bottom = None;
    let mut floating_right = None;
    let mut floating_bottom = None;
    let mut merged_footer = None;
    if let Some(panel) = composer_panel {
        let target = match (panel.placement.position, panel.placement.mode) {
            (muxy_ui::panel::PanelPosition::Right, muxy_ui::panel::PanelMode::Pinned) => {
                &mut pinned_right
            }
            (muxy_ui::panel::PanelPosition::Bottom, muxy_ui::panel::PanelMode::Pinned) => {
                merged_footer = panel.merged_footer;
                &mut pinned_bottom
            }
            (muxy_ui::panel::PanelPosition::Right, muxy_ui::panel::PanelMode::Floating) => {
                &mut floating_right
            }
            (muxy_ui::panel::PanelPosition::Bottom, muxy_ui::panel::PanelMode::Floating) => {
                &mut floating_bottom
            }
        };
        *target = Some(panel.element);
    }
    for panel in extension_panels {
        let target = match (panel.placement.position, panel.placement.mode) {
            (muxy_ui::panel::PanelPosition::Right, muxy_ui::panel::PanelMode::Pinned) => {
                &mut pinned_right
            }
            (muxy_ui::panel::PanelPosition::Bottom, muxy_ui::panel::PanelMode::Pinned) => {
                &mut pinned_bottom
            }
            (muxy_ui::panel::PanelPosition::Right, muxy_ui::panel::PanelMode::Floating) => {
                &mut floating_right
            }
            (muxy_ui::panel::PanelPosition::Bottom, muxy_ui::panel::PanelMode::Floating) => {
                &mut floating_bottom
            }
        };
        *target = Some(panel.element);
    }
    let mut main_region = div()
        .relative()
        .flex()
        .flex_grow()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .h_full()
        .child(main_column)
        .when_some(pinned_right, |region, panel| {
            div()
                .relative()
                .flex()
                .flex_row()
                .flex_grow()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .h_full()
                .child(region)
                .child(panel)
        })
        .into_any_element();
    if let Some(panel) = pinned_bottom {
        main_region = div()
            .relative()
            .flex()
            .flex_col()
            .flex_grow()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .h_full()
            .child(main_region)
            .child(panel)
            .when_some(merged_footer, |region, footer| {
                region.child(status_bar::status_bar(
                    state,
                    focused_working_directory.as_deref(),
                    repository_controls,
                    repository_mutation_busy,
                    repository_ai_menu_available,
                    Some(footer),
                    extension_statusbar_items,
                    cx,
                ))
            })
            .into_any_element();
    }
    if floating_right.is_some() || floating_bottom.is_some() {
        main_region = div()
            .relative()
            .flex()
            .flex_grow()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .h_full()
            .child(main_region)
            .when_some(floating_right, |region, panel| region.child(panel))
            .when_some(floating_bottom, |region, panel| region.child(panel))
            .into_any_element();
    }

    let mut columns = div()
        .id("main-window")
        .key_context(muxy_core::shortcuts::KEY_CONTEXT)
        .track_focus(workspace_focus)
        .on_action(
            cx.listener(|window, _: &crate::keymap::ToggleRichInput, _, cx| {
                window.toggle_composer(cx);
            }),
        )
        .on_action(cx.listener(
            |window, _: &crate::keymap::SubmitRichInput, window_handle, cx| {
                if window.composer_input_is_focused(window_handle, cx) {
                    window.submit_composer(true, cx);
                }
            },
        ))
        .on_action(cx.listener(
            |window, _: &crate::keymap::SubmitRichInputWithoutReturn, window_handle, cx| {
                if window.composer_input_is_focused(window_handle, cx) {
                    window.submit_composer(false, cx);
                }
            },
        ))
        .on_action(cx.listener(|window, _: &crate::keymap::NewTab, _, cx| {
            window.new_terminal_tab(cx);
        }))
        .on_action(cx.listener(|window, _: &crate::keymap::NewHomeTab, _, cx| {
            window.new_home_tab(cx);
        }))
        .when(
            state.prefs.browser_enabled && state.active_project().is_some(),
            |element| {
                element.on_action(
                    cx.listener(|window, _: &crate::keymap::NewBrowserTab, _, cx| {
                        window.new_browser_tab(cx);
                    }),
                )
            },
        )
        .on_action(
            cx.listener(|window, _: &menu_bar::OpenSettings, window_handle, cx| {
                window.open_settings(window_handle, cx);
            }),
        )
        .on_action(cx.listener(
            |window, _: &crate::keymap::TerminalOmnibox, window_handle, cx| {
                window.open_omnibox(omnibox::Scope::OpenTabs, window_handle, cx);
            },
        ))
        .on_action(cx.listener(
            |window, _: &crate::keymap::TerminalOmniboxProjects, window_handle, cx| {
                window.open_omnibox(omnibox::Scope::Projects, window_handle, cx);
            },
        ))
        .on_action(cx.listener(
            |window, _: &crate::keymap::TerminalOmniboxWorktrees, window_handle, cx| {
                window.open_omnibox(omnibox::Scope::Worktrees, window_handle, cx);
            },
        ))
        .on_action(cx.listener(
            |window, _: &crate::keymap::TerminalOmniboxWorkspaces, window_handle, cx| {
                window.open_omnibox(omnibox::Scope::Workspaces, window_handle, cx);
            },
        ))
        .on_action(cx.listener(
            |window, _: &crate::keymap::TerminalOmniboxCommands, window_handle, cx| {
                window.open_omnibox(omnibox::Scope::CommandShortcuts, window_handle, cx);
            },
        ))
        .on_action(cx.listener(
            |window, _: &crate::keymap::RecentlyRemovedProjects, window_handle, cx| {
                window.open_omnibox(omnibox::Scope::RecentlyRemovedProjects, window_handle, cx);
            },
        ))
        .on_action(
            cx.listener(|window, _: &crate::keymap::RefreshWorktrees, _, cx| {
                let Some(project_id) = window.state.active_project_id.clone() else {
                    return;
                };
                window.request_worktree_refresh(project_id, None, cx);
            }),
        )
        .on_action(cx.listener(
            |window, _: &crate::keymap::CreateWorktree, window_handle, cx| {
                let Some(project_id) = window.state.active_project_id.clone() else {
                    return;
                };
                window.open_create_worktree(&project_id, window_handle, cx);
            },
        ))
        .on_action(
            cx.listener(|window, _: &crate::keymap::RemoveCurrentWorktree, _, cx| {
                let Some(project_id) = window.state.active_project_id.clone() else {
                    return;
                };
                let Some(worktree_id) = window
                    .state
                    .prefs
                    .active_worktree_ids
                    .get(&project_id)
                    .cloned()
                else {
                    return;
                };
                window.request_worktree_removal_inspection(project_id, worktree_id, cx);
            }),
        )
        .on_action(cx.listener(
            |window, action: &crate::keymap::RunCommandShortcut, window_handle, cx| {
                window.perform(
                    Command::RunCommandShortcut(action.id.clone()),
                    window_handle,
                    cx,
                );
            },
        ))
        .on_action(cx.listener(
            |window, action: &crate::views::window::extensions::RunExtensionShortcut, _, cx| {
                window.run_extension_command(&action.extension_id, &action.command_id, None, cx);
            },
        ))
        .on_action(cx.listener(
            |window, _: &crate::keymap::ToggleThemePicker, window_handle, cx| {
                window.open_theme_picker(window_handle, cx);
            },
        ))
        .on_action(
            cx.listener(|window, _: &crate::keymap::ReloadConfig, _, cx| {
                window.reload_configuration(cx);
            }),
        )
        .on_action(cx.listener(|_, _: &menu_bar::OpenConfiguration, _, _| open_configuration()))
        .on_action(
            cx.listener(|window, action: &menu_bar::OpenInIde, window_handle, cx| {
                window.perform(
                    Command::OpenInIde(action.bundle_identifier.clone()),
                    window_handle,
                    cx,
                );
            }),
        )
        .on_action(
            cx.listener(|_, _: &menu_bar::Minimize, window_handle: &mut Window, _| {
                window_handle.minimize_window();
            }),
        )
        .on_action(
            cx.listener(|_, _: &menu_bar::Zoom, window_handle: &mut Window, _| {
                window_handle.zoom_window();
            }),
        )
        .on_action(cx.listener(|window, _: &crate::keymap::CloseTab, _, cx| {
            window.close_active_tab(cx);
        }))
        .on_action(cx.listener(|window, _: &crate::keymap::ClosePane, _, cx| {
            window.close_active_tab(cx);
        }))
        .on_action(cx.listener(|window, _: &crate::keymap::RenameTab, _, cx| {
            window.rename_active_tab(cx);
        }))
        .on_action(
            cx.listener(|window, _: &crate::keymap::PinUnpinTab, _, cx| {
                window.toggle_active_tab_pinned(cx);
            }),
        )
        .on_action(cx.listener(|window, _: &crate::keymap::SplitRight, _, cx| {
            window.split_focused(muxy_core::workspace::Edge::Right, cx);
        }))
        .on_action(cx.listener(|window, _: &crate::keymap::SplitDown, _, cx| {
            window.split_focused(muxy_core::workspace::Edge::Bottom, cx);
        }))
        .on_action(cx.listener(|window, _: &crate::keymap::NextTab, _, cx| {
            window.select_relative_root(1, cx);
        }))
        .on_action(
            cx.listener(|window, _: &crate::keymap::PreviousTab, _, cx| {
                window.select_relative_root(-1, cx);
            }),
        )
        .on_action(cx.listener(
            |window, _: &crate::keymap::CycleNextTabAcrossPanes, _, cx| {
                window.cycle_pane(1, cx);
            },
        ))
        .on_action(cx.listener(
            |window, _: &crate::keymap::CyclePreviousTabAcrossPanes, _, cx| {
                window.cycle_pane(-1, cx);
            },
        ))
        .on_action(
            cx.listener(|window, _: &crate::keymap::FocusPaneLeft, _, cx| {
                window.focus_pane_direction(
                    muxy_core::workspace::Axis::Horizontal,
                    false,
                    false,
                    cx,
                );
            }),
        )
        .on_action(
            cx.listener(|window, _: &crate::keymap::FocusPaneRight, _, cx| {
                window.focus_pane_direction(
                    muxy_core::workspace::Axis::Horizontal,
                    true,
                    false,
                    cx,
                );
            }),
        )
        .on_action(
            cx.listener(|window, _: &crate::keymap::FocusPaneUp, _, cx| {
                window.focus_pane_direction(muxy_core::workspace::Axis::Vertical, false, false, cx);
            }),
        )
        .on_action(
            cx.listener(|window, _: &crate::keymap::FocusPaneDown, _, cx| {
                window.focus_pane_direction(muxy_core::workspace::Axis::Vertical, true, false, cx);
            }),
        )
        .on_action(
            cx.listener(|window, _: &crate::keymap::MovePaneLeft, _, cx| {
                window.focus_pane_direction(
                    muxy_core::workspace::Axis::Horizontal,
                    false,
                    true,
                    cx,
                );
            }),
        )
        .on_action(
            cx.listener(|window, _: &crate::keymap::MovePaneRight, _, cx| {
                window.focus_pane_direction(muxy_core::workspace::Axis::Horizontal, true, true, cx);
            }),
        )
        .on_action(cx.listener(|window, _: &crate::keymap::MovePaneUp, _, cx| {
            window.focus_pane_direction(muxy_core::workspace::Axis::Vertical, false, true, cx);
        }))
        .on_action(
            cx.listener(|window, _: &crate::keymap::MovePaneDown, _, cx| {
                window.focus_pane_direction(muxy_core::workspace::Axis::Vertical, true, true, cx);
            }),
        )
        .on_action(cx.listener(|window, _: &crate::keymap::SelectTab1, _, cx| {
            window.select_root_index(0, cx);
        }))
        .on_action(cx.listener(|window, _: &crate::keymap::SelectTab2, _, cx| {
            window.select_root_index(1, cx);
        }))
        .on_action(cx.listener(|window, _: &crate::keymap::SelectTab3, _, cx| {
            window.select_root_index(2, cx);
        }))
        .on_action(cx.listener(|window, _: &crate::keymap::SelectTab4, _, cx| {
            window.select_root_index(3, cx);
        }))
        .on_action(cx.listener(|window, _: &crate::keymap::SelectTab5, _, cx| {
            window.select_root_index(4, cx);
        }))
        .on_action(cx.listener(|window, _: &crate::keymap::SelectTab6, _, cx| {
            window.select_root_index(5, cx);
        }))
        .on_action(cx.listener(|window, _: &crate::keymap::SelectTab7, _, cx| {
            window.select_root_index(6, cx);
        }))
        .on_action(cx.listener(|window, _: &crate::keymap::SelectTab8, _, cx| {
            window.select_root_index(7, cx);
        }))
        .on_action(cx.listener(|window, _: &crate::keymap::SelectTab9, _, cx| {
            window.select_root_index(8, cx);
        }))
        .on_action(
            cx.listener(|window, _: &crate::keymap::ToggleMaximizePane, _, cx| {
                window.toggle_maximize(cx);
            }),
        )
        .on_action(
            cx.listener(|window, _: &crate::keymap::FindInTerminal, _, cx| {
                window.open_search(cx);
            }),
        )
        .on_action(cx.listener(
            |window, _: &crate::keymap::OpenProject, window_handle, cx| {
                window.perform(Command::OpenProjectPicker, window_handle, cx);
            },
        ))
        .on_action(
            cx.listener(|window, _: &crate::keymap::NextProject, _, cx| {
                window.select_project_relative(1, cx);
            }),
        )
        .on_action(
            cx.listener(|window, _: &crate::keymap::PreviousProject, _, cx| {
                window.select_project_relative(-1, cx);
            }),
        )
        .on_action(
            cx.listener(|window, _: &crate::keymap::NavigateBack, _, cx| {
                window.navigate(muxy_core::navigation::Direction::Back, cx);
            }),
        )
        .on_action(
            cx.listener(|window, _: &crate::keymap::NavigateForward, _, cx| {
                window.navigate(muxy_core::navigation::Direction::Forward, cx);
            }),
        )
        .on_action(
            cx.listener(|window, _: &crate::keymap::SelectProject1, _, cx| {
                window.select_project_index(0, cx);
            }),
        )
        .on_action(
            cx.listener(|window, _: &crate::keymap::SelectProject2, _, cx| {
                window.select_project_index(1, cx);
            }),
        )
        .on_action(
            cx.listener(|window, _: &crate::keymap::SelectProject3, _, cx| {
                window.select_project_index(2, cx);
            }),
        )
        .on_action(
            cx.listener(|window, _: &crate::keymap::SelectProject4, _, cx| {
                window.select_project_index(3, cx);
            }),
        )
        .on_action(
            cx.listener(|window, _: &crate::keymap::SelectProject5, _, cx| {
                window.select_project_index(4, cx);
            }),
        )
        .on_action(
            cx.listener(|window, _: &crate::keymap::SelectProject6, _, cx| {
                window.select_project_index(5, cx);
            }),
        )
        .on_action(
            cx.listener(|window, _: &crate::keymap::SelectProject7, _, cx| {
                window.select_project_index(6, cx);
            }),
        )
        .on_action(
            cx.listener(|window, _: &crate::keymap::SelectProject8, _, cx| {
                window.select_project_index(7, cx);
            }),
        )
        .on_action(
            cx.listener(|window, _: &crate::keymap::SelectProject9, _, cx| {
                window.select_project_index(8, cx);
            }),
        )
        .on_action(
            cx.listener(|window, _: &crate::keymap::ToggleSidebar, _, cx| {
                window.toggle_sidebar(cx);
            }),
        )
        .on_action(cx.listener(
            |_, _: &crate::keymap::ToggleFullScreen, window_handle: &mut Window, _| {
                window_handle.toggle_fullscreen();
            },
        ))
        .on_action(cx.listener(|window, _: &SearchPrevious, _, cx| {
            window.navigate_search(false, cx);
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|window: &mut MainWindow, _, _, cx| {
                window.close_floating_composer_from_outside(cx);
            }),
        )
        .on_mouse_down(
            MouseButton::Navigate(NavigationDirection::Back),
            cx.listener(handle_navigation_mouse_down),
        )
        .on_mouse_down(
            MouseButton::Navigate(NavigationDirection::Forward),
            cx.listener(handle_navigation_mouse_down),
        )
        .on_mouse_move(
            cx.listener(|window: &mut MainWindow, event: &MouseMoveEvent, _, cx| {
                window.handle_workspace_mouse_move(event, cx);
            }),
        )
        .capture_any_mouse_up(cx.listener(
            |window: &mut MainWindow, event: &MouseUpEvent, _, cx| {
                window.handle_workspace_mouse_up(event, cx);
            },
        ))
        .relative()
        .flex()
        .flex_row()
        .size_full()
        .text_color(theme.fg)
        .font_family(".SystemUIFont")
        .line_height(relative(SYSTEM_LINE_HEIGHT));

    if !layout.hidden_sidebar {
        columns = columns.child(sidebar_column);
    }

    columns = columns
        .child(main_region)
        .child(titlebar::nav_overlay(state, layout, cx))
        .child(sidebar_border);

    if let Some((bounds, zone)) = drop_highlight {
        columns = columns.child(workspace_view::drop_highlight(state, bounds, zone));
    }

    if overlay.is_open() {
        columns = columns.child(overlay::layer(
            overlay,
            state,
            repository_state,
            repository_mutation_busy,
            menu_focus,
            window,
            cx,
        ));
    }

    if let Some((content, generation)) = active_toast {
        let position =
            crate::toast::ToastPosition::from_setting(&muxy_core::prefs::settings::string_value(
                "muxy.notifications.toastPosition",
                "Top Center",
            ));
        columns = columns.child(toast::layer(
            content,
            generation,
            position,
            &state.theme,
            state.metrics,
            cx,
        ));
    }

    if let Some(popover) = extension_surfaces.active_popover()
        && let Some(layer) = crate::extensions::surface_view::render_popover(
            popover,
            extension_popover_anchor,
            extension_webviews,
            &state.theme,
            state.metrics,
            cx,
        )
    {
        columns = columns.child(layer);
    }

    if let Some(modal) = extension_surfaces.active_modal()
        && let Some(layer) = crate::extensions::surface_view::render_modal(
            modal,
            extension_webviews,
            &state.theme,
            state.metrics,
            cx,
        )
    {
        columns = columns.child(layer);
    }

    columns.into_any_element()
}

fn handle_navigation_mouse_down(
    window: &mut MainWindow,
    event: &MouseDownEvent,
    _: &mut Window,
    cx: &mut Context<MainWindow>,
) {
    if let Some(direction) = mouse_navigation_direction(event.button) {
        window.navigate(direction, cx);
    }
}

fn mouse_navigation_direction(button: MouseButton) -> Option<muxy_core::navigation::Direction> {
    match button {
        MouseButton::Navigate(NavigationDirection::Back) => {
            Some(muxy_core::navigation::Direction::Back)
        }
        MouseButton::Navigate(NavigationDirection::Forward) => {
            Some(muxy_core::navigation::Direction::Forward)
        }
        MouseButton::Left | MouseButton::Right | MouseButton::Middle => None,
    }
}

fn shows_welcome(workspace: Option<&muxy_core::workspace::WorkspaceState>) -> bool {
    workspace.is_none_or(|workspace| workspace.top_level_root.is_none())
}

fn open_configuration() {
    muxy_core::store::ghostty_conf::seed_if_needed();
    let path = muxy_core::store::ghostty_conf::path();
    let _ = std::process::Command::new("/usr/bin/open")
        .args(["-a", "/System/Applications/TextEdit.app"])
        .arg(path)
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxy_core::prefs::ScalePreset;
    use muxy_core::workspace::{Tab, TabKind, WorkspaceState};

    #[test]
    fn chrome_empty_project_uses_the_welcome_new_tab_surface() {
        let empty = WorkspaceState::new("project");
        let mut populated = WorkspaceState::new("project");
        populated.new_top_level_tab(Tab::new(TabKind::Terminal));

        assert!(shows_welcome(Some(&empty)));
        assert!(!shows_welcome(Some(&populated)));
        assert!(shows_welcome(None));
    }

    #[test]
    fn only_a_pinned_bottom_composer_merges_into_a_visible_status_bar() {
        use muxy_ui::panel::{PanelMode, PanelPlacement, PanelPosition};

        let pinned_bottom =
            PanelPlacement::new("composer", PanelPosition::Bottom, PanelMode::Pinned);
        let floating_bottom =
            PanelPlacement::new("composer", PanelPosition::Bottom, PanelMode::Floating);
        let pinned_right = PanelPlacement::new("composer", PanelPosition::Right, PanelMode::Pinned);

        assert!(merge_composer_footer_with_status_bar(
            true,
            Some(&pinned_bottom)
        ));
        assert!(!merge_composer_footer_with_status_bar(
            false,
            Some(&pinned_bottom)
        ));
        assert!(!merge_composer_footer_with_status_bar(
            true,
            Some(&floating_bottom)
        ));
        assert!(!merge_composer_footer_with_status_bar(
            true,
            Some(&pinned_right)
        ));
        assert!(!merge_composer_footer_with_status_bar(true, None));
    }

    #[test]
    fn navigation_mouse_buttons_map_to_history_directions() {
        assert_eq!(
            mouse_navigation_direction(gpui::MouseButton::Navigate(
                gpui::NavigationDirection::Back
            )),
            Some(muxy_core::navigation::Direction::Back)
        );
        assert_eq!(
            mouse_navigation_direction(gpui::MouseButton::Navigate(
                gpui::NavigationDirection::Forward
            )),
            Some(muxy_core::navigation::Direction::Forward)
        );
        assert_eq!(mouse_navigation_direction(gpui::MouseButton::Left), None);
    }

    #[test]
    fn sidebar_modes_preserve_hidden_icons_narrow_and_wide_layouts() {
        let metrics = Metrics::new(ScalePreset::Regular.multiplier());
        let mut prefs = Prefs {
            collapsed_style: CollapsedStyle::Hidden,
            ..Prefs::default()
        };

        let hidden = AppLayout::new(&prefs, metrics, false);
        assert!(hidden.hidden_sidebar);
        assert_eq!(hidden.sidebar_width, px(0.0));

        prefs.collapsed_style = CollapsedStyle::Icons;
        let icons = AppLayout::new(&prefs, metrics, false);
        assert!(!icons.hidden_sidebar);
        assert_eq!(icons.sidebar_width, metrics.sidebar_collapsed_width());

        prefs.expanded_style = ExpandedStyle::Icons;
        let narrow = AppLayout::new(&prefs, metrics, true);
        assert!(!narrow.wide_sidebar);
        assert_eq!(narrow.sidebar_width, metrics.sidebar_collapsed_width());

        prefs.expanded_style = ExpandedStyle::Wide;
        let wide = AppLayout::new(&prefs, metrics, true);
        assert!(wide.wide_sidebar);
        assert_eq!(wide.sidebar_width, metrics.sidebar_expanded_width());
    }

    #[test]
    fn custom_width_clamps_and_titlebar_overlap_scales() {
        let metrics = Metrics::new(ScalePreset::Huge.multiplier());
        let mut prefs = Prefs {
            expanded_style: ExpandedStyle::Wide,
            sidebar_expanded_custom_width: Some(1.0),
            ..Prefs::default()
        };
        let minimum = AppLayout::new(&prefs, metrics, true);
        assert_eq!(minimum.sidebar_width, metrics.sidebar_expanded_min_width());

        prefs.sidebar_expanded_custom_width = Some(10_000.0);
        let maximum = AppLayout::new(&prefs, metrics, true);
        assert_eq!(maximum.sidebar_width, metrics.sidebar_expanded_max_width());

        prefs.collapsed_style = CollapsedStyle::Hidden;
        let hidden = AppLayout::new(&prefs, metrics, false);
        assert_eq!(
            hidden.nav_overlay_width,
            metrics.traffic_light_width() + metrics.navigation_arrows_width()
        );
        assert_eq!(hidden.main_titlebar_leading_inset, hidden.nav_overlay_width);
        assert_eq!(
            hidden.sidebar_border_top_pad,
            metrics.title_bar_height() + px(1.0)
        );
    }
}
