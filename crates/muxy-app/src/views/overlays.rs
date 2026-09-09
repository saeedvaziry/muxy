use gpui::{
    AnyElement, AppContext, Bounds, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Pixels, Point, Size, Styled, Window, div, point, px, size,
};
use muxy_ui::components::SymbolGlyph;

use super::{
    menu::{self, Item, Menu},
    theme_picker::{ThemeEvent, ThemePicker},
};
use crate::model::AppModel;

pub(crate) enum Overlay {
    Menu(Menu),
    ProjectEditor(super::project_editor::Editor),
    ProjectColors(super::project_editor::Colors),
    Projects(Entity<super::project_picker::ProjectPicker>),
    Themes {
        picker: Entity<ThemePicker>,
        anchor: Option<Bounds<Pixels>>,
    },
    Notifications {
        anchor: Option<Bounds<Pixels>>,
    },
}

impl AppModel {
    pub(crate) fn dismiss_overlay(&mut self, cx: &mut Context<Self>) {
        self.overlay = None;
        self.overlay_subscription = None;
        self.focus_requested = true;
        cx.notify();
    }

    pub(crate) fn open_menu(
        &mut self,
        items: Vec<Item>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.overlay_subscription = None;
        self.overlay = Some(Overlay::Menu(Menu::new(items, position)));
        self.overlay_focus.focus(window);
        cx.notify();
    }

    pub(crate) fn open_theme_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.overlay, Some(Overlay::Themes { .. })) {
            self.dismiss_overlay(cx);
            return;
        }
        self.reload_themes(cx);
        let active = self.themes.active_name(&self.appearance, self.dark);
        let picker = cx.new(|cx| {
            ThemePicker::new(
                self.themes.entries.clone(),
                active,
                self.theme.clone(),
                self.metrics,
                cx,
            )
        });
        self.overlay_subscription =
            Some(cx.subscribe(&picker, |model, _, event, cx| match event {
                ThemeEvent::Selected(name) => {
                    if model.dark {
                        model.appearance.dark_theme.clone_from(name);
                    } else {
                        model.appearance.light_theme.clone_from(name);
                    }
                    model.refresh_theme(cx);
                    model.save_appearance(cx);
                }
                ThemeEvent::Dismiss => model.dismiss_overlay(cx),
            }));
        self.overlay = Some(Overlay::Themes {
            picker,
            anchor: self.theme_anchor,
        });
        self.overlay_focus.focus(window);
        cx.notify();
    }

    pub(crate) fn toggle_notifications(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.overlay, Some(Overlay::Notifications { .. })) {
            self.dismiss_overlay(cx);
            return;
        }
        self.overlay_subscription = None;
        self.overlay = Some(Overlay::Notifications {
            anchor: self.notification_anchor,
        });
        self.overlay_focus.focus(window);
        cx.notify();
    }
}

pub(crate) fn clamp(
    origin: Point<Pixels>,
    panel: Size<Pixels>,
    viewport: Size<Pixels>,
) -> Point<Pixels> {
    point(
        origin
            .x
            .max(px(8.0))
            .min((viewport.width - panel.width - px(8.0)).max(px(8.0))),
        origin
            .y
            .max(px(8.0))
            .min((viewport.height - panel.height - px(8.0)).max(px(8.0))),
    )
}

pub(crate) fn layer(model: &AppModel, window: &Window, cx: &mut Context<AppModel>) -> AnyElement {
    let viewport = window.viewport_size();
    let content = match &model.overlay {
        None => return div().into_any_element(),
        Some(Overlay::Menu(menu)) => menu::render(menu, model, window, cx),
        Some(Overlay::ProjectEditor(editor)) => {
            super::project_editor::render(editor, model, window, cx)
        }
        Some(Overlay::ProjectColors(colors)) => {
            super::project_editor::render_colors(colors, model, window, cx)
        }
        Some(Overlay::Projects(picker)) => picker.clone().into_any_element(),
        Some(Overlay::Themes { picker, anchor }) => {
            let origin = anchor.map_or(point(px(8.0), viewport.height - px(12.0)), |anchor| {
                anchor.origin
            });
            let left = clamp(origin, size(px(340.0), px(0.0)), viewport).x;
            let bottom = (viewport.height - origin.y + px(4.0))
                .max(px(8.0))
                .min(viewport.height - px(8.0));
            div()
                .absolute()
                .left(left)
                .bottom(bottom)
                .child(picker.clone())
                .into_any_element()
        }
        Some(Overlay::Notifications { anchor }) => notifications(*anchor, model, window, cx),
    };
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|model, _, _, cx| {
                        model.dismiss_overlay(cx);
                        cx.stop_propagation();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|model, _, _, cx| {
                        model.dismiss_overlay(cx);
                        cx.stop_propagation();
                    }),
                ),
        )
        .child(content)
        .into_any_element()
}

fn notifications(
    anchor: Option<Bounds<Pixels>>,
    model: &AppModel,
    window: &Window,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let theme = &model.theme;
    let m = model.metrics;
    let viewport = window.viewport_size();
    let height = px(400.0).min(viewport.height - px(16.0));
    let origin = anchor.map_or(
        point(px(8.0), viewport.height - height - px(8.0)),
        |anchor| point(anchor.origin.x, anchor.origin.y - height - px(4.0)),
    );
    let origin = clamp(origin, size(px(320.0), height), viewport);
    div()
        .key_context("Menu")
        .track_focus(&model.overlay_focus)
        .on_action(cx.listener(|model, _: &menu::DismissMenu, _, cx| model.dismiss_overlay(cx)))
        .absolute()
        .left(origin.x)
        .top(origin.y)
        .w(px(320.0))
        .h(height)
        .flex()
        .flex_col()
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .rounded(m.radius_lg())
        .bg(theme.raised())
        .border_1()
        .border_color(theme.border)
        .shadow_lg()
        .child(
            div()
                .px(m.spacing5())
                .py(m.spacing4())
                .border_b_1()
                .border_color(theme.border)
                .text_size(m.font_body())
                .font_weight(FontWeight::SEMIBOLD)
                .child("Notifications"),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(m.spacing4())
                .child(SymbolGlyph::new("bell.slash", m.icon_xl(), theme.fg_dim))
                .child(
                    div()
                        .text_size(m.font_body())
                        .text_color(theme.fg_muted)
                        .child("No notifications"),
                ),
        )
        .into_any_element()
}
