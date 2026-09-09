use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    AnyElement, Bounds, Context, DispatchPhase, InteractiveElement, IntoElement, MouseButton,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, SharedString, Styled, canvas, div,
    px, relative,
};
use muxy_app_core::{Axis, Branch, Layout, TabId};

use crate::model::AppModel;

#[derive(Clone, Default)]
pub(crate) struct SplitResizeState(Rc<RefCell<Option<SplitResize>>>);

#[derive(Clone)]
struct SplitResize {
    tab: TabId,
    path: Vec<Branch>,
    axis: Axis,
    ratio: f32,
    pointer: Point<Pixels>,
    bounds: Bounds<Pixels>,
}

impl SplitResize {
    fn ratio_at(&self, pointer: Point<Pixels>) -> f32 {
        let (delta, extent) = match self.axis {
            Axis::Horizontal => (pointer.x - self.pointer.x, self.bounds.size.width),
            Axis::Vertical => (pointer.y - self.pointer.y, self.bounds.size.height),
        };
        (self.ratio + f32::from(delta) / (f32::from(extent) - 1.0).max(1.0)).clamp(0.15, 0.85)
    }
}

impl SplitResizeState {
    pub(crate) fn end(&self) -> bool {
        self.0.borrow_mut().take().is_some()
    }
}

pub(crate) fn render(model: &AppModel, cx: &mut Context<AppModel>) -> Option<AnyElement> {
    let tab = model
        .state
        .current_project()
        .tabs
        .iter()
        .find(|tab| Some(tab.id) == model.active_tab())?;
    if let Some(zoomed) = tab.zoomed {
        let pane = model.grids.get(&zoomed)?;
        return Some(
            div()
                .debug_selector(|| "zoomed-pane-frame".into())
                .flex()
                .size_full()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .p(model.metrics.spacing7())
                .bg(model.theme.bg)
                .child(
                    div()
                        .flex()
                        .size_full()
                        .min_w(px(0.0))
                        .min_h(px(0.0))
                        .rounded(model.metrics.radius_lg())
                        .border(px(1.0))
                        .border_color(model.theme.border)
                        .shadow_md()
                        .overflow_hidden()
                        .child(pane.view.clone()),
                )
                .into_any_element(),
        );
    }
    let content = node(&tab.layout, tab.id, Vec::new(), model);
    let state = model.split_resize.clone();
    let weak = cx.weak_entity();
    Some(
        div()
            .relative()
            .size_full()
            .child(content)
            .child(
                canvas(
                    |_, _, _| (),
                    move |_, (), window, _| {
                        let state_move = state.clone();
                        let model_move = weak.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if phase != DispatchPhase::Capture {
                                return;
                            }
                            let Some(resize) = state_move.0.borrow().clone() else {
                                return;
                            };
                            let _ = model_move.update(cx, |model, cx| {
                                if model.active_tab() == Some(resize.tab)
                                    && model
                                        .state
                                        .set_ratio(
                                            resize.tab,
                                            &resize.path,
                                            resize.ratio_at(event.position),
                                        )
                                        .is_ok()
                                {
                                    cx.notify();
                                } else {
                                    model.split_resize.end();
                                }
                            });
                            cx.stop_propagation();
                        });
                        let state_end = state.clone();
                        let model_end = weak.clone();
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                            if phase == DispatchPhase::Capture
                                && event.button == MouseButton::Left
                                && state_end.end()
                            {
                                let _ = model_end.update(cx, AppModel::save_split_resize);
                                cx.stop_propagation();
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
            .into_any_element(),
    )
}

fn node(layout: &Layout, tab: TabId, path: Vec<Branch>, model: &AppModel) -> AnyElement {
    let (axis, ratio, first, second) = match layout {
        Layout::Split {
            axis,
            ratio,
            first,
            second,
        } => (axis, ratio, first, second),
        Layout::Leaf(id) => {
            return model.grids.get(id).map_or_else(
                || div().size_full().into_any_element(),
                |pane| pane.view.clone().into_any_element(),
            );
        }
    };
    let mut first_path = path.clone();
    first_path.push(Branch::First);
    let mut second_path = path.clone();
    second_path.push(Branch::Second);
    let first = weighted(node(first, tab, first_path, model), *ratio);
    let second = weighted(node(second, tab, second_path, model), 1.0 - *ratio);
    let bounds = Rc::new(Cell::new(Bounds::default()));
    let measured = bounds.clone();
    let state = model.split_resize.clone();
    let axis = *axis;
    let ratio = *ratio;
    let selector = format!("split-divider-{path:?}");
    let hit = div()
        .id(SharedString::from(format!("split-{tab}-{path:?}")))
        .debug_selector(move || selector.clone())
        .absolute()
        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
            *state.0.borrow_mut() = Some(SplitResize {
                tab,
                path: path.clone(),
                axis,
                ratio,
                pointer: event.position,
                bounds: bounds.get(),
            });
            cx.stop_propagation();
        });
    let (container, divider, hit) = match axis {
        Axis::Horizontal => (
            div().flex().flex_row(),
            div().w(px(1.0)).h_full(),
            hit.left(relative(ratio))
                .ml(px(-ratio - 2.5))
                .top_0()
                .w(px(6.0))
                .h_full()
                .cursor_ew_resize(),
        ),
        Axis::Vertical => (
            div().flex().flex_col(),
            div().h(px(1.0)).w_full(),
            hit.top(relative(ratio))
                .mt(px(-ratio - 2.5))
                .left_0()
                .h(px(6.0))
                .w_full()
                .cursor_ns_resize(),
        ),
    };
    container
        .relative()
        .size_full()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .child(first)
        .child(divider.relative().flex_none().bg(model.theme.border))
        .child(second)
        .child(hit)
        .child(
            canvas(move |bounds, _, _| measured.set(bounds), |_, (), _, _| ())
                .absolute()
                .size_full(),
        )
        .into_any_element()
}

fn weighted(content: AnyElement, ratio: f32) -> gpui::Div {
    let mut view = div()
        .flex()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .flex_basis(px(0.0))
        .child(content);
    view.style().flex_grow = Some(ratio);
    view
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, size};

    #[test]
    fn divider_uses_parent_pixels_retains_grab_offset_and_clamps() {
        for axis in [Axis::Horizontal, Axis::Vertical] {
            let resize = SplitResize {
                tab: TabId::new(),
                path: vec![],
                axis,
                ratio: 0.5,
                pointer: point(px(500.0), px(300.0)),
                bounds: Bounds::new(point(px(200.0), px(100.0)), size(px(601.0), px(401.0))),
            };
            assert!((resize.ratio_at(resize.pointer) - 0.5).abs() < f32::EPSILON);
            assert!((resize.ratio_at(point(px(560.0), px(340.0))) - 0.6).abs() < f32::EPSILON);
            assert!((resize.ratio_at(point(px(-1000.0), px(-1000.0))) - 0.15).abs() < f32::EPSILON);
            assert!((resize.ratio_at(point(px(2000.0), px(2000.0))) - 0.85).abs() < f32::EPSILON);
        }
    }
}
