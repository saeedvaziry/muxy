use crate::command_popover::CommandPopover;
use crate::components::SymbolGlyph;
use crate::text_input::TextInput;
use crate::theme::{Metrics, Theme};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Bounds, ClickEvent, Corner, Entity, FontWeight, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, Point, SharedString,
    StatefulInteractiveElement, Styled, Window, anchored, canvas, deferred, div, point, px,
};
use std::cell::Cell;
use std::rc::Rc;

pub const CONTROL_WIDTH: f32 = 210.0;
pub const SLIDER_WIDTH: f32 = 220.0;

#[derive(Clone, Copy, Debug)]
pub struct Style<'a> {
    pub theme: &'a Theme,
    pub metrics: &'a Metrics,
}

#[derive(Clone, Debug)]
pub struct Choice {
    pub value: String,
    pub label: String,
    pub enabled: bool,
}

impl Choice {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            enabled: true,
        }
    }
}

pub fn section(
    style: Style,
    title: &str,
    footer: Option<&str>,
    shows_divider: bool,
    children: Vec<AnyElement>,
) -> AnyElement {
    let Style { theme, metrics } = style;
    let mut block = div()
        .flex()
        .flex_col()
        .child(
            div()
                .px(metrics.spacing6())
                .pt(metrics.spacing5())
                .pb(metrics.spacing2())
                .text_size(metrics.font_footnote())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg_muted)
                .child(SharedString::from(title.to_owned())),
        )
        .children(children);

    if let Some(footer) = footer {
        block = block.child(
            div()
                .px(metrics.spacing6())
                .pt(metrics.spacing3())
                .pb(metrics.spacing5())
                .text_size(metrics.font_footnote())
                .text_color(theme.fg_muted)
                .child(SharedString::from(footer.to_owned())),
        );
    }

    if shows_divider {
        block = block.child(
            div()
                .mx(metrics.spacing6())
                .h(px(1.0))
                .flex_none()
                .bg(theme.border),
        );
    }

    block.into_any_element()
}

pub fn row(style: Style, label: &str, control: AnyElement) -> AnyElement {
    let Style { theme, metrics } = style;
    div()
        .flex()
        .flex_row()
        .items_start()
        .px(metrics.spacing6())
        .py(metrics.spacing3())
        .child(
            div()
                .flex_shrink()
                .min_w(px(0.0))
                .py(metrics.spacing1())
                .text_size(metrics.font_body())
                .text_color(theme.fg)
                .child(SharedString::from(label.to_owned())),
        )
        .child(div().flex_grow().min_w(metrics.spacing6()))
        .child(div().flex_none().child(control))
        .into_any_element()
}

pub fn toggle(
    style: Style,
    id: &str,
    value: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let Style { theme, metrics } = style;
    let knob = div()
        .size(metrics.scaled(12.0))
        .flex_none()
        .rounded_full()
        .bg(if value {
            theme.accent_foreground
        } else {
            theme.fg_muted
        });

    div()
        .id(SharedString::from(format!("settings-toggle-{id}")))
        .flex()
        .flex_row()
        .items_center()
        .flex_none()
        .w(metrics.scaled(28.0))
        .h(metrics.scaled(16.0))
        .px(metrics.scaled(2.0))
        .rounded_full()
        .cursor_pointer()
        .bg(if value { theme.accent } else { theme.surface })
        .border_1()
        .border_color(if value { theme.accent } else { theme.border })
        .when(value, Styled::justify_end)
        .child(knob)
        .on_click(on_click)
        .into_any_element()
}

pub fn button(
    style: Style,
    id: &str,
    label: &str,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let Style { theme, metrics } = style;
    div()
        .id(SharedString::from(format!("settings-button-{id}")))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(metrics.control_medium())
        .px(metrics.spacing5())
        .rounded(metrics.radius_sm())
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border)
        .text_size(metrics.font_footnote())
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.fg)
        .when(!enabled, |element| element.opacity(0.4))
        .when(enabled, |element| {
            element
                .cursor_pointer()
                .hover(|hover| hover.bg(theme.hover))
                .on_click(on_click)
        })
        .child(SharedString::from(label.to_owned()))
        .into_any_element()
}

pub fn picker(
    style: Style,
    id: &str,
    choices: &[Choice],
    selected: &str,
    popover: Option<Entity<CommandPopover>>,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let metrics = style.metrics;
    let label = choices
        .iter()
        .find(|choice| choice.value == selected)
        .map(|choice| choice.label.clone())
        .unwrap_or_default();
    let field = picker_trigger(style, id, &label, popover.is_some(), on_toggle);

    let Some(popover) = popover else {
        return div()
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .child(field)
            .into_any_element();
    };

    div()
        .relative()
        .flex()
        .flex_col()
        .flex_none()
        .child(field)
        .child(
            deferred(
                anchored()
                    .anchor(Corner::TopLeft)
                    .offset(point(
                        px(0.0),
                        metrics.control_medium() + metrics.spacing1(),
                    ))
                    .snap_to_window_with_margin(px(8.0))
                    .child(popover),
            )
            .with_priority(1),
        )
        .into_any_element()
}

pub fn picker_trigger(
    style: Style,
    id: &str,
    label: &str,
    open: bool,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let Style { theme, metrics } = style;
    div()
        .id(SharedString::from(format!("settings-picker-{id}")))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(metrics.spacing3())
        .w(metrics.scaled(CONTROL_WIDTH))
        .h(metrics.control_medium())
        .px(metrics.spacing4())
        .rounded(metrics.radius_sm())
        .cursor_pointer()
        .bg(theme.surface)
        .border_1()
        .border_color(if open { theme.accent } else { theme.border })
        .hover(|hover| hover.bg(theme.hover))
        .child(
            div()
                .flex_grow()
                .min_w(px(0.0))
                .truncate()
                .text_size(metrics.font_footnote())
                .text_color(theme.fg)
                .child(SharedString::from(label.to_owned())),
        )
        .child(SymbolGlyph::new(
            "chevron.up.chevron.down",
            metrics.font_caption(),
            theme.fg_muted,
        ))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(on_toggle)
        .into_any_element()
}

pub fn segmented(
    style: Style,
    id: &str,
    choices: &[Choice],
    selected: &str,
    on_select: impl Fn(&SharedString, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let Style { theme, metrics } = style;
    let handler = Rc::new(on_select);
    let mut group = div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(px(1.0))
        .p(px(1.0))
        .rounded(metrics.radius_sm())
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border);

    for choice in choices {
        let is_selected = choice.value == selected;
        let handler = handler.clone();
        let value = SharedString::from(choice.value.clone());
        group = group.child(
            div()
                .id(SharedString::from(format!(
                    "settings-segment-{id}-{}",
                    choice.value
                )))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .h(metrics.control_medium() - px(4.0))
                .px(metrics.spacing4())
                .rounded(metrics.radius_sm())
                .text_size(metrics.font_footnote())
                .font_weight(if is_selected {
                    FontWeight::MEDIUM
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if is_selected {
                    theme.accent_foreground
                } else {
                    theme.fg
                })
                .when(is_selected, |element| element.bg(theme.accent))
                .when(!choice.enabled, |element| element.opacity(0.4))
                .when(choice.enabled && !is_selected, |element| {
                    element
                        .cursor_pointer()
                        .hover(|hover| hover.bg(theme.hover))
                })
                .when(choice.enabled, |element| {
                    element.on_click(move |_, window, cx| handler(&value, window, cx))
                })
                .child(SharedString::from(choice.label.clone())),
        );
    }

    group.into_any_element()
}

#[derive(Debug)]
pub struct Grab {
    pub bounds: Bounds<Pixels>,
    pub position: Point<Pixels>,
}

pub fn fraction_at(bounds: Bounds<Pixels>, position: Point<Pixels>) -> f32 {
    let width = f32::from(bounds.size.width);
    if width <= 0.0 {
        return 0.0;
    }
    (f32::from(position.x - bounds.origin.x) / width).clamp(0.0, 1.0)
}

pub fn slider(
    style: Style,
    id: &str,
    value: f32,
    range: (f32, f32),
    on_grab: impl Fn(&Grab, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let Style { theme, metrics } = style;
    let span = (range.1 - range.0).max(f32::EPSILON);
    let fraction = ((value - range.0) / span).clamp(0.0, 1.0);
    let track: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
    let recorder = track.clone();
    let width = metrics.scaled(SLIDER_WIDTH);
    let knob = metrics.scaled(12.0);
    let rail = metrics.scaled(4.0);

    div()
        .id(SharedString::from(format!("settings-slider-{id}")))
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .w(width)
        .h(metrics.control_medium())
        .cursor_pointer()
        .child(
            canvas(
                move |bounds, _, _| recorder.set(Some(bounds)),
                |_, (): (), _, _| {},
            )
            .absolute()
            .size_full(),
        )
        .child(
            div()
                .absolute()
                .left_0()
                .w_full()
                .h(rail)
                .rounded_full()
                .bg(theme.border),
        )
        .child(
            div()
                .absolute()
                .left_0()
                .w(width * fraction)
                .h(rail)
                .rounded_full()
                .bg(theme.accent),
        )
        .child(
            div()
                .absolute()
                .left((width - knob) * fraction)
                .size(knob)
                .rounded_full()
                .bg(theme.accent_foreground)
                .border_1()
                .border_color(theme.border),
        )
        .on_mouse_down(
            MouseButton::Left,
            move |event: &MouseDownEvent, window, cx| {
                let Some(bounds) = track.get() else {
                    return;
                };
                on_grab(
                    &Grab {
                        bounds,
                        position: event.position,
                    },
                    window,
                    cx,
                );
            },
        )
        .into_any_element()
}

pub fn text_area(
    style: Style,
    id: &str,
    input: &Entity<TextInput>,
    content_height: Option<f32>,
) -> AnyElement {
    let Style { theme, metrics } = style;
    div()
        .id(SharedString::from(format!("settings-area-{id}")))
        .flex()
        .flex_col()
        .min_h(px(0.0))
        .p(metrics.spacing4())
        .rounded(metrics.radius_sm())
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border)
        .overflow_hidden()
        .map(|element| match content_height {
            Some(height) => element
                .flex_none()
                .h(metrics.scaled(height) + metrics.spacing4() * 2.0 + px(2.0)),
            None => element.flex_grow(),
        })
        .child(input.clone())
        .into_any_element()
}

pub fn text_field(
    style: Style,
    id: &str,
    input: &Entity<TextInput>,
    width: Option<f32>,
) -> AnyElement {
    let Style { theme, metrics } = style;
    div()
        .id(SharedString::from(format!("settings-field-{id}")))
        .flex()
        .flex_row()
        .items_center()
        .h(metrics.control_medium())
        .px(metrics.spacing4())
        .rounded(metrics.radius_sm())
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border)
        .map(|element| match width {
            Some(width) => element.flex_none().w(metrics.scaled(width)),
            None => element.flex_grow().min_w(px(0.0)),
        })
        .on_mouse_down_out(|_, window, _| window.blur())
        .child(crate::text_input::growing_input(input))
        .into_any_element()
}
