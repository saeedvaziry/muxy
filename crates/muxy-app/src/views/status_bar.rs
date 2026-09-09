use gpui::StatefulInteractiveElement;
use gpui::prelude::FluentBuilder;
use gpui::{Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Styled, div, px};
use muxy_ui::components::IconGlyph;
use muxy_ui::icon::Icon;

use super::menu::{Command, Item};
use crate::model::AppModel;

pub(crate) fn status_bar(model: &AppModel, cx: &mut Context<AppModel>) -> impl IntoElement {
    let m = model.metrics;
    let theme = &model.theme;
    let project = model.state.current_project();
    let id = project.id;
    let available = project.status() == muxy_app_core::ProjectStatus::Available;
    div()
        .debug_selector(|| "project-status-bar".into())
        .flex()
        .flex_none()
        .items_center()
        .h(m.status_bar_height())
        .bg(theme.bg)
        .border_t_1()
        .border_color(theme.border)
        .px(m.spacing5())
        .gap(m.spacing4())
        .child(
            div()
                .id("status-path")
                .flex()
                .flex_1()
                .min_w(px(0.0))
                .items_center()
                .gap(m.spacing2())
                .h_full()
                .text_color(theme.fg_muted)
                .when(available, |path| {
                    path.cursor_pointer()
                        .on_click(cx.listener(|model, _, _, cx| {
                            cx.reveal_path(&model.state.current_project().directory);
                        }))
                        .on_mouse_down(
                            gpui::MouseButton::Right,
                            cx.listener(move |model, event: &gpui::MouseDownEvent, window, cx| {
                                model.open_menu(
                                    vec![
                                        Item::action("Copy Path", Command::CopyPath(id)),
                                        Item::action("Reveal in Finder", Command::RevealPath(id)),
                                    ],
                                    event.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                })
                .child(IconGlyph::new(
                    Icon::Folder,
                    m.font_caption(),
                    theme.fg_muted,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .text_size(m.font_footnote())
                        .font_weight(FontWeight::MEDIUM)
                        .child(super::project_picker::display_path(&project.directory)),
                ),
        )
        .children(super::disconnected::status(model, cx).map(|status| {
            div()
                .debug_selector(|| "project-connection-status".into())
                .flex_none()
                .child(status)
        }))
}
