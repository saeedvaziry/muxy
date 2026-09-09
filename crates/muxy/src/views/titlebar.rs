use crate::command::Command;
use crate::state::AppState;
use crate::views::app::AppLayout;
use crate::views::menu::Item;
use crate::views::window::MainWindow;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, AppContext, ClickEvent, Context, FontWeight, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use muxy_ui::components::{IconButton, IconGlyph};
use muxy_ui::icon::Icon;

pub fn nav_overlay(
    state: &AppState,
    layout: AppLayout,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let metrics = &state.metrics;
    let theme = &state.theme;

    div()
        .absolute()
        .top_0()
        .left_0()
        .w(layout.nav_overlay_width)
        .h(metrics.title_bar_height())
        .flex()
        .flex_row()
        .items_center()
        .justify_end()
        .pr(metrics.spacing4())
        .gap(metrics.spacing1())
        .bg(theme.bg)
        .border_r(px(1.0))
        .border_color(theme.border)
        .child(nav_arrow(
            state,
            "nav-back",
            Icon::ChevronLeft,
            muxy_core::navigation::Direction::Back,
            cx,
        ))
        .child(nav_arrow(
            state,
            "nav-forward",
            Icon::ChevronRight,
            muxy_core::navigation::Direction::Forward,
            cx,
        ))
        .child(layout_menu(state, cx))
        .into_any_element()
}

fn nav_arrow(
    state: &AppState,
    id: &'static str,
    icon: Icon,
    direction: muxy_core::navigation::Direction,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let metrics = &state.metrics;
    let theme = &state.theme;
    let enabled = state.can_navigate(direction);
    let color = if enabled {
        theme.fg_muted
    } else {
        let mut disabled = theme.fg_muted;
        disabled.a *= 0.35;
        disabled
    };

    let glyph = IconGlyph::new(icon, metrics.font_body(), color);
    let mut arrow = div()
        .id(id)
        .group(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(metrics.scaled(22.0));

    if enabled {
        arrow = arrow
            .cursor_pointer()
            .on_click(cx.listener(move |window: &mut MainWindow, _, _, cx| {
                window.navigate(direction, cx);
            }))
            .child(glyph.hover_in_group(id, theme.fg));
    } else {
        arrow = arrow.child(glyph);
    }
    arrow.into_any_element()
}

fn layout_menu(state: &AppState, cx: &mut Context<MainWindow>) -> AnyElement {
    let metrics = &state.metrics;
    let theme = &state.theme;

    div()
        .id("layout-menu")
        .group("layout-menu")
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(metrics.scaled(22.0))
        .cursor_pointer()
        .on_click(
            cx.listener(|window: &mut MainWindow, event: &ClickEvent, view, cx| {
                window.open_layout_menu(event.position(), view, cx);
            }),
        )
        .child(
            IconGlyph::new(Icon::Grid, metrics.font_body(), theme.fg_muted)
                .hover_in_group("layout-menu", theme.fg),
        )
        .into_any_element()
}

pub fn main_titlebar(
    state: &AppState,
    layout: AppLayout,
    extension_items: &[crate::extensions::ExtensionTopbarBinding],
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let metrics = &state.metrics;
    let theme = &state.theme;

    let mut bar = div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .h(metrics.title_bar_height())
        .pl(layout.main_titlebar_leading_inset)
        .bg(theme.bg);

    if let Some(project) = state.active_project() {
        bar = bar.child(
            div()
                .flex_grow()
                .min_w(px(0.0))
                .pl(metrics.spacing4())
                .text_size(metrics.font_body())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg_muted)
                .truncate()
                .child(SharedString::from(project.name.clone())),
        );
    } else {
        bar = bar.child(div().flex_grow());
    }

    if state.prefs.show_topbar_actions && state.active_project().is_some() {
        bar = bar
            .child(extension_actions(state, extension_items, cx))
            .child(open_project_control(state, cx))
            .child(pane_actions(state, cx));
    }

    bar.into_any_element()
}

pub(crate) fn extension_actions(
    state: &AppState,
    items: &[crate::extensions::ExtensionTopbarBinding],
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let mut actions = div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(state.metrics.spacing1());
    for item in items {
        let extension_id = item.extension_id.clone();
        let command = item.command.clone();
        let icon = crate::extensions::surface_view::extension_icon(
            &item.icon,
            &item.resource_root,
            state.metrics.scaled(13.0),
            state.theme.fg_muted,
        );
        let label = item
            .tooltip
            .clone()
            .unwrap_or_else(|| format!("{} extension action", item.item_id));
        actions = actions.child(
            div()
                .id(SharedString::from(format!(
                    "extension-topbar-{}-{}",
                    item.extension_id, item.item_id
                )))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(state.metrics.control_medium())
                .rounded(state.metrics.radius_sm())
                .cursor_pointer()
                .hover(|style| style.bg(state.theme.hover))
                .tooltip({
                    let label = SharedString::from(label);
                    let background = state.theme.raised();
                    let foreground = state.theme.fg;
                    let border = state.theme.border;
                    move |_, cx| {
                        cx.new(|_| {
                            muxy_ui::components::Tooltip::new(
                                label.clone(),
                                background,
                                foreground,
                                border,
                            )
                        })
                        .into()
                    }
                })
                .on_click(
                    cx.listener(move |window: &mut MainWindow, event: &ClickEvent, _, cx| {
                        window.run_extension_command(
                            &extension_id,
                            &command,
                            Some(event.position()),
                            cx,
                        );
                    }),
                )
                .child(icon),
        );
    }
    actions.into_any_element()
}

fn pane_actions(state: &AppState, cx: &mut Context<MainWindow>) -> AnyElement {
    let metrics = &state.metrics;
    let theme = &state.theme;
    let glyph = metrics.scaled(13.0);
    let box_size = metrics.control_medium();

    let mut actions = div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .pr(metrics.spacing2())
        .when(!state.layouts().is_empty(), |element| {
            element.child(
                IconButton::new(
                    SharedString::from("titlebar-apply-layout"),
                    Icon::LayoutSplit,
                    glyph,
                    box_size,
                    theme.fg_muted,
                    theme.fg,
                )
                .tooltip("Apply Layout", theme.raised(), theme.fg, theme.border)
                .on_click(cx.listener(
                    move |window: &mut MainWindow, event: &gpui::ClickEvent, view, cx| {
                        window.open_terminal_layout_menu(event.position(), view, cx);
                    },
                )),
            )
        });
    for action in TITLEBAR_ACTIONS {
        actions = actions.child(action_button(state, action, glyph, box_size, cx));
    }
    actions.into_any_element()
}

#[derive(Clone, Copy)]
enum TitlebarAction {
    SplitRight,
    SplitDown,
    NewTab,
}

const TITLEBAR_ACTIONS: [TitlebarAction; 3] = [
    TitlebarAction::SplitRight,
    TitlebarAction::SplitDown,
    TitlebarAction::NewTab,
];

impl TitlebarAction {
    fn id(self) -> &'static str {
        match self {
            Self::SplitRight => "split-right",
            Self::SplitDown => "split-down",
            Self::NewTab => "new-tab",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::SplitRight => Icon::Columns,
            Self::SplitDown => Icon::Rows,
            Self::NewTab => Icon::Plus,
        }
    }
}

#[cfg(test)]
fn titlebar_action_ids() -> Vec<&'static str> {
    TITLEBAR_ACTIONS.map(TitlebarAction::id).to_vec()
}

fn action_button(
    state: &AppState,
    action: TitlebarAction,
    glyph: gpui::Pixels,
    box_size: gpui::Pixels,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let theme = &state.theme;
    IconButton::new(
        action.id(),
        action.icon(),
        glyph,
        box_size,
        theme.fg_muted,
        theme.fg,
    )
    .on_click(
        cx.listener(move |window: &mut MainWindow, _, _, cx| match action {
            TitlebarAction::SplitRight => {
                window.split_focused(muxy_core::workspace::Edge::Right, cx)
            }
            TitlebarAction::SplitDown => {
                window.split_focused(muxy_core::workspace::Edge::Bottom, cx)
            }
            TitlebarAction::NewTab => window.new_terminal_tab(cx),
        }),
    )
    .into_any_element()
}

fn ide_menu_items() -> Vec<Item> {
    let finder = muxy_api::ide::finder();
    let mut items = vec![
        Item::action(
            finder.display_name,
            Command::OpenInIde(finder.bundle_identifier),
        ),
        Item::Separator,
    ];
    let installed = muxy_api::ide::installed();
    if installed.is_empty() {
        items.push(Item::label("No supported IDEs found"));
        return items;
    }
    items.extend(installed.into_iter().map(|entry| {
        Item::action(
            entry.display_name,
            Command::OpenInIde(entry.bundle_identifier),
        )
    }));
    items
}

fn open_project_control(state: &AppState, cx: &mut Context<MainWindow>) -> AnyElement {
    let metrics = &state.metrics;
    let theme = &state.theme;
    let selected = state
        .prefs
        .ide_bundle_identifier
        .clone()
        .unwrap_or_default();

    div()
        .flex()
        .flex_none()
        .flex_row()
        .items_center()
        .mr(metrics.spacing2())
        .rounded(metrics.radius_md())
        .bg(theme.surface)
        .border(px(1.0))
        .border_color(theme.border)
        .overflow_hidden()
        .child(
            div()
                .id("open-project")
                .flex()
                .flex_row()
                .items_center()
                .gap(metrics.spacing3())
                .px(metrics.spacing4())
                .h(metrics.control_small())
                .text_color(theme.fg_muted)
                .hover(|style| style.bg(theme.hover))
                .cursor_pointer()
                .child(IconGlyph::new(
                    Icon::Code,
                    metrics.font_footnote(),
                    theme.fg_muted,
                ))
                .child(
                    div()
                        .text_size(metrics.font_body())
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(SharedString::from(
                            state
                                .ide_name
                                .clone()
                                .unwrap_or_else(|| "Open Project".into()),
                        )),
                )
                .on_click(cx.listener(
                    move |window: &mut MainWindow, _, window_handle: &mut Window, cx| {
                        window.perform(Command::OpenInIde(selected.clone()), window_handle, cx);
                    },
                )),
        )
        .child(
            div()
                .w(px(1.0))
                .h(metrics.scaled(14.0))
                .flex_none()
                .bg(theme.border),
        )
        .child(
            div()
                .id("open-project-menu")
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(metrics.control_small())
                .h(metrics.control_small())
                .hover(|style| style.bg(theme.hover))
                .cursor_pointer()
                .child(IconGlyph::new(
                    Icon::ChevronDown,
                    metrics.font_caption(),
                    theme.fg_muted,
                ))
                .on_click(
                    cx.listener(|window: &mut MainWindow, event: &ClickEvent, _, cx| {
                        window.open_menu(ide_menu_items(), event.position(), cx);
                    }),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_titlebar_contract_excludes_browser_and_keeps_backed_actions() {
        assert_eq!(
            titlebar_action_ids(),
            vec!["split-right", "split-down", "new-tab"]
        );
        assert!(!titlebar_action_ids().contains(&"new-browser-tab"));
    }
}
