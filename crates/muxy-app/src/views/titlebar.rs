use gpui::StatefulInteractiveElement;
use gpui::{AnyElement, Context, InteractiveElement, IntoElement, ParentElement, Styled, div, px};
use muxy_ui::components::IconGlyph;
use muxy_ui::icon::Icon;

use super::menu::{Command, Item};
use crate::model::AppModel;

pub(crate) fn navigation(
    model: &AppModel,
    sidebar_width: f32,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let m = model.metrics;
    let theme = &model.theme;
    div()
        .occlude()
        .absolute()
        .top_0()
        .left_0()
        .w(px(sidebar_width.max(153.0)))
        .h(m.title_bar_height())
        .flex()
        .items_center()
        .justify_end()
        .pr(m.spacing4())
        .gap(m.spacing1())
        .bg(theme.bg)
        .border_r_1()
        .border_color(theme.border)
        .child(arrow(model, false, cx))
        .child(arrow(model, true, cx))
        .child(
            div()
                .id("layout-menu")
                .group("layout-menu")
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(m.scaled(22.0))
                .cursor_pointer()
                .on_click(cx.listener(|model, event: &gpui::ClickEvent, window, cx| {
                    model.open_menu(
                        vec![
                            Item::action("Project Focused", Command::Dismiss).checked(),
                            Item::action("Tab Focused", Command::Dismiss).disabled(),
                            Item::action("Agents Focused", Command::Dismiss).disabled(),
                        ],
                        event.position(),
                        window,
                        cx,
                    );
                }))
                .child(
                    IconGlyph::new(Icon::Grid, m.font_body(), theme.fg_muted)
                        .hover_in_group("layout-menu", theme.fg),
                ),
        )
        .into_any_element()
}

fn arrow(model: &AppModel, forward: bool, cx: &mut Context<AppModel>) -> AnyElement {
    let m = model.metrics;
    let theme = &model.theme;
    let enabled = model.can_navigate(forward);
    let id = if forward { "nav-forward" } else { "nav-back" };
    let color = if enabled {
        theme.fg_muted
    } else {
        gpui::Hsla {
            a: theme.fg_muted.a * 0.35,
            ..theme.fg_muted
        }
    };
    let glyph = IconGlyph::new(
        if forward {
            Icon::ChevronRight
        } else {
            Icon::ChevronLeft
        },
        m.font_body(),
        color,
    );
    let arrow = div()
        .id(id)
        .debug_selector(move || id.into())
        .group(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(m.scaled(22.0));
    if enabled {
        arrow
            .cursor_pointer()
            .on_click(cx.listener(move |model, _, _, cx| model.navigate(forward, cx)))
            .child(glyph.hover_in_group(id, theme.fg))
            .into_any_element()
    } else {
        arrow.child(glyph).into_any_element()
    }
}
