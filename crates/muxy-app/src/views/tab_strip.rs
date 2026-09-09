use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, AppContext, Context, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use muxy_app_core::{Tab, TabId};
use muxy_settings::Action;

use super::workspace::Zoom;
use crate::model::AppModel;
use muxy_ui::components::{IconButton, IconGlyph};
use muxy_ui::icon::Icon;
use muxy_ui::theme::Theme;

#[derive(Clone)]
struct DraggedTab {
    id: TabId,
    title: String,
    theme: Theme,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(12.0))
            .h(px(32.0))
            .bg(self.theme.bg)
            .text_color(self.theme.fg)
            .text_size(px(12.0))
            .border(px(1.0))
            .border_color(self.theme.accent)
            .child(self.title.clone())
    }
}

pub(crate) fn tab_strip(
    model: &AppModel,
    sidebar_width: f32,
    window: &Window,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let strip = div()
        .id("tab-strip")
        .debug_selector(|| "tab-strip".into())
        .h(px(32.0))
        .flex_none()
        .on_click(|event, window, cx| {
            if event.click_count() == 2 {
                cx.stop_propagation();
                window.dispatch_action(Box::new(Zoom), cx);
            }
        });
    if model.state.current_project().status() == muxy_app_core::ProjectStatus::Missing {
        return strip.into_any_element();
    }
    let theme = &model.theme;
    let zoom_tab = model
        .state
        .current_project()
        .tabs
        .iter()
        .find(|tab| Some(tab.id) == model.active_tab())
        .filter(|tab| tab.zoomed.is_some() || tab.panes.len() > 1);
    let zoom_width = zoom_tab.map_or(0.0, |_| {
        f32::from(model.metrics.control_medium() + model.metrics.spacing2())
    });
    let leading = (153.0 - sidebar_width).max(0.0);
    let available =
        (f32::from(window.viewport_size().width) - sidebar_width - leading - 28.0 - zoom_width)
            .max(0.0);
    let count = u16::try_from(model.state.current_project().tabs.len()).unwrap_or(u16::MAX);
    let ideal_width = available / f32::from(count.max(1));
    let width = ideal_width.clamp(44.0, 200.0);
    let mut cells = div().flex().flex_none().h_full();
    for tab in &model.state.current_project().tabs {
        cells = cells.child(tab_cell(
            tab,
            tab.title(model.state.window().active_pane),
            model.active_tab() == Some(tab.id),
            tab.panes.iter().any(|pane| {
                model
                    .grids
                    .get(&pane.id)
                    .is_some_and(|pane| pane.view.read(cx).bell_flashing)
            }),
            width,
            theme,
            cx,
        ));
    }
    let new_button = div()
        .debug_selector(|| "new-tab-button".into())
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .pl(px(4.0))
        .w(px(28.0))
        .h_full()
        .child(
            IconButton::new(
                "new-tab",
                Icon::Plus,
                px(13.0),
                px(24.0),
                theme.fg_muted,
                theme.fg,
            )
            .tooltip("New Tab (⌘T)", theme.raised(), theme.fg, theme.border)
            .on_click(cx.listener(|model, _, _, cx| {
                cx.stop_propagation();
                model.new_tab(cx);
            })),
        );
    let pinned_button = if ideal_width < 44.0 {
        Some(new_button)
    } else {
        cells = cells.child(new_button);
        None
    };
    strip
        .flex()
        .items_center()
        .pl(px(leading))
        .child(
            div()
                .id("tabs-scroll")
                .flex()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .overflow_x_scroll()
                .child(cells),
        )
        .children(pinned_button)
        .children(zoom_tab.map(|tab| zoom_control(tab.zoomed.is_some(), model, cx)))
        .into_any_element()
}

fn zoom_control(zoomed: bool, model: &AppModel, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = &model.theme;
    let label = if zoomed {
        "Restore Pane"
    } else {
        "Maximize Pane"
    };
    let tooltip = model
        .settings
        .keymap
        .chord(Action::ToggleZoomPane)
        .map_or_else(|| label.to_owned(), |chord| format!("{label} ({chord})"));
    div()
        .debug_selector(move || {
            if zoomed {
                "restore-pane"
            } else {
                "maximize-pane"
            }
            .into()
        })
        .flex()
        .flex_none()
        .items_center()
        .h_full()
        .pr(model.metrics.spacing2())
        .child(
            IconButton::new(
                "toggle-zoom-pane",
                if zoomed {
                    Icon::Restore
                } else {
                    Icon::Maximize
                },
                model.metrics.scaled(13.0),
                model.metrics.control_medium(),
                theme.fg_muted,
                theme.fg,
            )
            .tooltip(tooltip, theme.raised(), theme.fg, theme.border)
            .on_click(cx.listener(|model, _, window, cx| {
                cx.stop_propagation();
                model.toggle_zoom_pane(cx);
                model.focus_active(window, cx);
            })),
        )
        .into_any_element()
}

fn close_control(
    id: TabId,
    active: bool,
    shows_title: bool,
    group: SharedString,
    theme: &Theme,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    div()
        .id(SharedString::from(format!("close-{id}")))
        .debug_selector(|| "close-tab-button".into())
        .absolute()
        .top_0()
        .flex()
        .items_center()
        .justify_center()
        .h_full()
        .when(shows_title, |button| button.right(px(10.0)).w(px(14.0)))
        .when(!shows_title, |button| button.left_0().right_0().w_full())
        .opacity(if active { 1.0 } else { 0.0 })
        .group_hover(group, |style| style.opacity(1.0))
        .cursor_pointer()
        .on_click(cx.listener(move |model, _, window, cx| {
            cx.stop_propagation();
            model.close_tab(id, cx);
            model.focus_active(window, cx);
        }))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(14.0))
                .rounded(px(4.0))
                .hover(|style| style.bg(theme.hover))
                .child(IconGlyph::new(Icon::X, px(10.0), theme.fg_muted)),
        )
        .into_any_element()
}

fn tab_cell(
    tab: &Tab,
    title: &str,
    active: bool,
    bell: bool,
    width: f32,
    theme: &Theme,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let shows_title = width >= 80.0;
    let id = tab.id;
    let group = SharedString::from(format!("tab-{id}"));
    let drag = DraggedTab {
        id,
        title: title.to_owned(),
        theme: theme.clone(),
    };
    let close = close_control(
        id,
        active && shows_title,
        shows_title,
        group.clone(),
        theme,
        cx,
    );
    let foreground = if active { theme.fg } else { theme.fg_muted };
    let surface = theme.surface;
    div()
        .id(group.clone())
        .group(group.clone())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .w(px(width))
        .h_full()
        .overflow_hidden()
        .border_r(px(1.0))
        .border_color(theme.border)
        .cursor_pointer()
        .text_size(px(12.0))
        .text_color(foreground)
        .when(active, |tab| tab.bg(theme.surface))
        .on_click(cx.listener(move |model, _, window, cx| {
            cx.stop_propagation();
            model.select_tab(id, cx);
            model.focus_active(window, cx);
        }))
        .on_mouse_down(
            MouseButton::Middle,
            cx.listener(move |model, _, window, cx| {
                cx.stop_propagation();
                model.close_tab(id, cx);
                model.focus_active(window, cx);
            }),
        )
        .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
        .drag_over::<DraggedTab>(move |style, _, _, _| style.bg(surface))
        .on_drop(cx.listener(move |model, dragged: &DraggedTab, _, cx| {
            model.move_tab(dragged.id, id, cx);
        }))
        .child(tab_label(
            title,
            shows_title,
            active,
            group,
            foreground,
            bell.then_some(theme.accent),
        ))
        .child(close)
        .into_any_element()
}

fn tab_label(
    title: &str,
    shows_title: bool,
    active: bool,
    group: SharedString,
    foreground: gpui::Hsla,
    bell: Option<gpui::Hsla>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_1()
        .min_w(px(0.0))
        .items_center()
        .gap(px(6.0))
        .h_full()
        .when(shows_title, |row| row.pl(px(12.0)).pr(px(28.0)))
        .when(!shows_title, Styled::justify_center)
        .child(
            div()
                .flex()
                .flex_none()
                .when(!shows_title, |icon| {
                    icon.group_hover(group, |style| style.opacity(0.0))
                })
                .debug_selector(|| {
                    if bell.is_some() {
                        "tab-bell".into()
                    } else {
                        "tab-terminal".into()
                    }
                })
                .child(IconGlyph::new(
                    if bell.is_some() {
                        Icon::Bell
                    } else {
                        Icon::Terminal
                    },
                    px(14.0),
                    bell.unwrap_or(foreground),
                )),
        )
        .when(shows_title, |row| {
            row.child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .font_weight(if active {
                        FontWeight::MEDIUM
                    } else {
                        FontWeight::NORMAL
                    })
                    .truncate()
                    .child(title.to_owned()),
            )
        })
}
