use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, AppContext, Bounds, Context, DragMoveEvent, Empty, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Pixels, Point, SharedString, StatefulInteractiveElement, Styled,
    div, px,
};
use muxy_app_core::{Project, ProjectId, ProjectStatus};
use muxy_ui::components::{IconButton, IconGlyph, SymbolGlyph};
use muxy_ui::icon::Icon;
use muxy_ui::theme::{contrasting_foreground, parse_hex};

use crate::model::AppModel;

pub(crate) fn sidebar(model: &AppModel, cx: &mut Context<AppModel>) -> AnyElement {
    let m = model.metrics;
    let theme = &model.theme;
    let wide = model.appearance.sidebar_expanded;
    let header = header(model, cx);
    let rows = project_list(model, cx);
    div()
        .flex()
        .flex_col()
        .flex_none()
        .w(if wide {
            m.sidebar_expanded_width()
        } else {
            m.sidebar_collapsed_width()
        })
        .h_full()
        .min_h(px(0.0))
        .bg(theme.bg)
        .child(div().h(m.title_bar_height()).flex_none())
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .gap(m.spacing3())
                .child(header)
                .child(rows),
        )
        .child(footer(model, cx))
        .into_any_element()
}

fn footer(model: &AppModel, cx: &mut Context<AppModel>) -> AnyElement {
    let m = model.metrics;
    let theme = &model.theme;
    let toggle = IconButton::new(
        "toggle-sidebar",
        Icon::PanelLeft,
        m.scaled(13.0),
        m.control_medium(),
        theme.fg_muted,
        theme.fg,
    )
    .on_click(cx.listener(|model, _, window, cx| model.toggle_sidebar(window, cx)));
    let view = cx.weak_entity();
    let notifications = div()
        .flex()
        .flex_none()
        .child(
            IconButton::new(
                "notifications",
                Icon::Bell,
                m.scaled(13.0),
                m.control_medium(),
                theme.fg_muted,
                theme.fg,
            )
            .tooltip(
                "Notifications, no unread notifications",
                theme.raised(),
                theme.fg,
                theme.border,
            )
            .on_click(cx.listener(|model, _, window, cx| model.toggle_notifications(window, cx))),
        )
        .on_children_prepainted(move |bounds, _, cx| {
            if let Some(bounds) = bounds.first() {
                let _ = view.update(cx, |model, _| model.notification_anchor = Some(*bounds));
            }
        });
    let view = cx.weak_entity();
    let themes = div()
        .flex()
        .flex_none()
        .child(
            IconButton::new(
                "theme-picker",
                Icon::Palette,
                m.scaled(13.0),
                m.control_medium(),
                theme.fg_muted,
                theme.fg,
            )
            .on_click(cx.listener(|model, _, window, cx| model.open_theme_picker(window, cx))),
        )
        .on_children_prepainted(move |bounds, _, cx| {
            if let Some(bounds) = bounds.first() {
                let _ = view.update(cx, |model, _| model.theme_anchor = Some(*bounds));
            }
        });
    let footer = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(m.spacing2())
        .pb(m.spacing4());
    if model.appearance.sidebar_expanded {
        footer
            .px(m.spacing5())
            .child(toggle)
            .child(div().flex_grow())
            .child(notifications)
            .child(themes)
            .into_any_element()
    } else {
        footer
            .flex_col()
            .child(notifications)
            .child(themes)
            .child(toggle)
            .into_any_element()
    }
}

fn header(model: &AppModel, _: &mut Context<AppModel>) -> AnyElement {
    if !model.appearance.sidebar_expanded {
        return div().into_any_element();
    }
    div()
        .px(model.metrics.spacing6())
        .pt(model.metrics.spacing2())
        .text_size(model.metrics.font_caption())
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(model.theme.fg_muted)
        .child("All Projects")
        .into_any_element()
}

struct DraggedProject {
    id: ProjectId,
    last_target: Cell<Option<ProjectId>>,
}

impl DraggedProject {
    fn move_to(&self, target: Option<ProjectId>, model: &mut AppModel, cx: &mut Context<AppModel>) {
        if model.overlay.is_some() || model.close_prompt.is_some() {
            return;
        }
        let target = target.filter(|target| *target != self.id);
        if self.last_target.replace(target) != target
            && let Some(target) = target
        {
            model.move_project(self.id, target, cx);
        }
    }
}

#[derive(Default)]
struct ProjectRows(Vec<(ProjectId, Bounds<Pixels>)>);

impl ProjectRows {
    fn at(&self, position: Point<Pixels>) -> Option<ProjectId> {
        self.0
            .iter()
            .find_map(|(id, bounds)| bounds.contains(&position).then_some(*id))
    }
}

fn project_list(model: &AppModel, cx: &mut Context<AppModel>) -> AnyElement {
    let m = model.metrics;
    let wide = model.appearance.sidebar_expanded;
    let targets = Rc::new(RefCell::new(ProjectRows::default()));
    let measured = targets.clone();
    let moving = targets.clone();
    let projects = model.state.projects();
    let ids: Vec<_> = projects
        .iter()
        .map(|project| {
            (!project.home && project.status() == ProjectStatus::Available).then_some(project.id)
        })
        .collect();
    div()
        .id("projects-scroll")
        .debug_selector(|| "projects-scroll".into())
        .flex_1()
        .min_h(px(0.0))
        .overflow_y_scroll()
        .on_drag_move(cx.listener(
            move |model, event: &DragMoveEvent<DraggedProject>, window, cx| {
                if event.event.pressed_button != Some(gpui::MouseButton::Left) {
                    cx.stop_active_drag(window);
                    return;
                }
                let target = event
                    .bounds
                    .contains(&event.event.position)
                    .then(|| moving.borrow().at(event.event.position))
                    .flatten();
                let drag = event.dragged_item().downcast_ref::<DraggedProject>();
                if let Some(drag) = drag {
                    drag.move_to(target, model, cx);
                }
            },
        ))
        .on_drop(
            cx.listener(move |model, drag: &DraggedProject, window, cx| {
                drag.move_to(targets.borrow().at(window.mouse_position()), model, cx);
            }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(m.spacing3())
                .px(if wide { m.spacing3() } else { m.spacing4() })
                .pt(if wide { px(0.0) } else { m.spacing2() })
                .pb(m.spacing2())
                .when(!wide, Styled::items_center)
                .children(
                    projects
                        .iter()
                        .enumerate()
                        .map(|(index, project)| project_row(project, index, model, cx)),
                )
                .child(add_project_button(model, cx))
                .on_children_prepainted(move |bounds, _, _| {
                    measured.borrow_mut().0 = ids
                        .iter()
                        .zip(bounds)
                        .filter_map(|(id, bounds)| id.map(|id| (id, bounds)))
                        .collect();
                }),
        )
        .into_any_element()
}

fn project_row(
    project: &Project,
    index: usize,
    model: &AppModel,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let m = model.metrics;
    let theme = &model.theme;
    let wide = model.appearance.sidebar_expanded;
    let id = project.id;
    let missing = project.status() == ProjectStatus::Missing;
    let active = model.state.current_project().id == id;
    let group = SharedString::from(format!("project-{id}"));
    let tile = project_tile(project, model, group.clone());
    let drag = DraggedProject {
        id,
        last_target: Cell::new(None),
    };
    div()
        .id(group.clone())
        .debug_selector(|| format!("project-row-{index}"))
        .group(group)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .when(wide, |row| {
            row.p(m.spacing2())
                .gap(m.spacing4())
                .rounded(m.radius_lg())
                .when(active, |row| row.bg(theme.surface))
                .hover(|style| style.bg(theme.hover))
        })
        .when(!wide, |row| row.justify_center().size(m.scaled(34.0)))
        .when(missing, |row| row.opacity(0.5))
        .when(!missing, |row| {
            row.cursor_pointer()
                .on_click(cx.listener(move |model, _, window, cx| {
                    model.select_project(id, cx);
                    model.focus_active(window, cx);
                }))
        })
        .on_mouse_down(
            gpui::MouseButton::Right,
            cx.listener(move |model, event: &gpui::MouseDownEvent, window, cx| {
                if let Some(project) = model.state.project(id) {
                    model.open_menu(
                        super::project_menu::items(project),
                        event.position,
                        window,
                        cx,
                    );
                }
            }),
        )
        .when(!project.home && !missing, |row| {
            row.on_drag(drag, |_, _, _, cx| cx.new(|_| Empty))
        })
        .child(tile)
        .when(wide, |row| {
            row.child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(m.font_emphasis())
                    .font_weight(if active {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::MEDIUM
                    })
                    .text_color(theme.fg)
                    .child(project.name.clone()),
            )
        })
        .when(!wide && active, |row| {
            row.child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(m.scaled(9.0))
                    .border(m.scaled(1.5))
                    .border_color(theme.accent),
            )
        })
        .when(missing, |row| {
            row.child(div().absolute().top_0().right_0().child(SymbolGlyph::new(
                "exclamationmark.triangle.fill",
                m.font_caption(),
                theme.warning,
            )))
        })
        .into_any_element()
}

fn add_project_button(model: &AppModel, cx: &mut Context<AppModel>) -> AnyElement {
    let m = model.metrics;
    let theme = &model.theme;
    let wide = model.appearance.sidebar_expanded;
    div()
        .id("add-project")
        .flex()
        .flex_none()
        .items_center()
        .cursor_pointer()
        .when(wide, |row| {
            row.p(m.spacing2())
                .gap(m.spacing4())
                .rounded(m.radius_lg())
                .hover(|style| style.bg(theme.hover))
        })
        .when(!wide, |row| row.justify_center().size(m.scaled(34.0)))
        .on_click(cx.listener(|model, _, window, cx| model.open_project_picker(window, cx)))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(m.icon_xxl())
                .rounded(m.radius_md())
                .bg(theme.surface)
                .child(IconGlyph::new(
                    Icon::Plus,
                    m.font_emphasis(),
                    theme.fg_muted,
                )),
        )
        .when(wide, |row| {
            row.child(
                div()
                    .text_size(m.font_body())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.fg_muted)
                    .child("Add Project"),
            )
        })
        .into_any_element()
}

fn project_tile(project: &Project, model: &AppModel, group: SharedString) -> AnyElement {
    let m = model.metrics;
    let color = parse_hex(project.color.as_str()).unwrap_or(gpui::rgb(0x80_80_80));
    let foreground = contrasting_foreground(color);
    let glyph = if let Some(icon) = &project.icon {
        div()
            .text_size(m.font_title_large())
            .child(icon.clone())
            .into_any_element()
    } else if project.home {
        SymbolGlyph::new("house.fill", m.font_title_large(), foreground.into()).into_any_element()
    } else {
        div()
            .text_size(m.font_emphasis())
            .font_weight(FontWeight::BOLD)
            .text_color(foreground)
            .child(project.initial().to_owned())
            .into_any_element()
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(m.icon_xxl())
        .rounded(m.radius_md())
        .bg(color)
        .group_hover(group, |style| style.opacity(0.85))
        .child(glyph)
        .into_any_element()
}
