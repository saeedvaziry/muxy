use super::*;
use crate::views::workspace::Zoom;
use gpui::{
    InteractiveElement, IntoElement, MouseButton, ParentElement, Render, Styled, div, point,
};

struct WindowZoomObserver {
    model: Entity<AppModel>,
    zoom_requests: usize,
}

impl Render for WindowZoomObserver {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .capture_action(cx.listener(|observer, _: &Zoom, _, cx| {
                observer.zoom_requests += 1;
                cx.stop_propagation();
            }))
            .child(self.model.clone())
    }
}

fn observe_window_zoom(
    state: AppState,
    cx: &mut TestAppContext,
) -> (Entity<WindowZoomObserver>, &mut VisualTestContext) {
    let (boot, _requests) = stub_boot(state);
    let result = cx.add_window_view(|window, cx| WindowZoomObserver {
        model: cx.new(|cx| AppModel::new(boot, window, cx)),
        zoom_requests: 0,
    });
    result.1.simulate_resize(size(px(1000.0), px(600.0)));
    result.1.run_until_parked();
    result
}

fn click(
    cx: &mut VisualTestContext,
    position: gpui::Point<gpui::Pixels>,
    button: MouseButton,
    click_count: usize,
) {
    cx.simulate_event(gpui::MouseDownEvent {
        position,
        button,
        click_count,
        ..Default::default()
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position,
        button,
        click_count,
        ..Default::default()
    });
    cx.run_until_parked();
}

#[gpui::test]
fn tab_strip_background_double_click_requests_native_zoom_each_time(cx: &mut TestAppContext) {
    for has_tabs in [false, true] {
        let mut state = AppState::bootstrap().expect("state");
        if has_tabs {
            state.open_terminal_tab(state.home().id).expect("tab");
        }
        let (observer, cx) = observe_window_zoom(state, cx);
        let model = observer.read_with(cx, |observer, _| observer.model.clone());
        for expanded in [true, false] {
            model.update(cx, |model, cx| {
                model.appearance.sidebar_expanded = expanded;
                cx.notify();
            });
            cx.run_until_parked();
            let strip = cx.debug_bounds("tab-strip").expect("tab strip");
            let background = point(strip.right() - px(16.0), strip.center().y);
            let before = observer.read_with(cx, |observer, _| observer.zoom_requests);
            for (button, count) in [
                (MouseButton::Left, 1),
                (MouseButton::Right, 2),
                (MouseButton::Middle, 2),
                (MouseButton::Left, 3),
            ] {
                click(cx, background, button, count);
                assert_eq!(
                    observer.read_with(cx, |observer, _| observer.zoom_requests),
                    before
                );
            }
            for expected in [before + 1, before + 2] {
                click(cx, background, MouseButton::Left, 1);
                click(cx, background, MouseButton::Left, 2);
                assert_eq!(
                    observer.read_with(cx, |observer, _| observer.zoom_requests),
                    expected
                );
            }
        }
    }
}

#[gpui::test]
fn tab_strip_tabs_and_controls_do_not_zoom_the_window(cx: &mut TestAppContext) {
    let mut state = AppState::bootstrap().expect("state");
    let first = state.open_terminal_tab(state.home().id).expect("first tab");
    let second = state
        .open_terminal_tab(state.home().id)
        .expect("second tab");
    let pane = state.home().tabs[1].panes[0].id;
    state.split_pane(pane, Direction::Right).expect("split");
    state.select_tab(state.home().id, first).expect("select");
    let (observer, cx) = observe_window_zoom(state, cx);
    let model = observer.read_with(cx, |observer, _| observer.model.clone());
    for count in [1, 2] {
        let tab = cx.debug_bounds("tab-terminal").expect("second tab icon");
        click(cx, tab.center(), MouseButton::Left, count);
        assert_eq!(
            model.read_with(cx, |model, _| model.active_tab()),
            Some(second)
        );
    }
    for (selector, count, zoomed) in [("maximize-pane", 1, true), ("restore-pane", 2, false)] {
        let control = cx.debug_bounds(selector).expect("pane zoom control");
        click(cx, control.center(), MouseButton::Left, count);
        assert_eq!(
            model.read_with(cx, |model, _| model.state.home().tabs[1].zoomed.is_some()),
            zoomed
        );
    }
    for (selector, expected_counts) in [("new-tab-button", [3, 4]), ("close-tab-button", [3, 2])] {
        for (count, expected) in [1, 2].into_iter().zip(expected_counts) {
            let button = cx.debug_bounds(selector).expect("tab control");
            click(cx, button.center(), MouseButton::Left, count);
            assert_eq!(
                model.read_with(cx, |model, _| model.state.home().tabs.len()),
                expected
            );
        }
    }
    assert_eq!(
        observer.read_with(cx, |observer, _| observer.zoom_requests),
        0
    );
}

#[gpui::test]
fn collapsed_titlebar_navigation_does_not_click_through_to_tab_strip(cx: &mut TestAppContext) {
    let mut state = AppState::bootstrap().expect("state");
    let first = state.open_terminal_tab(state.home().id).expect("first tab");
    let second = state
        .open_terminal_tab(state.home().id)
        .expect("second tab");
    let third = state.open_terminal_tab(state.home().id).expect("third tab");
    let (observer, cx) = observe_window_zoom(state, cx);
    let model = observer.read_with(cx, |observer, _| observer.model.clone());
    model.update(cx, |model, cx| {
        model.appearance.sidebar_expanded = false;
        assert!(!model.can_navigate(false) && !model.can_navigate(true));
        cx.notify();
    });
    cx.run_until_parked();
    for selector in ["nav-back", "nav-forward"] {
        let arrow = cx
            .debug_bounds(selector)
            .expect("disabled navigation arrow");
        for count in [1, 2] {
            click(cx, arrow.center(), MouseButton::Left, count);
        }
        assert_eq!(
            observer.read_with(cx, |observer, _| observer.zoom_requests),
            0
        );
        assert_eq!(
            model.read_with(cx, |model, _| model.active_tab()),
            Some(third)
        );
    }
    model.update(cx, |model, cx| {
        model.select_tab(second, cx);
        model.select_tab(first, cx);
    });
    cx.run_until_parked();
    for (selector, targets) in [
        ("nav-back", [second, third]),
        ("nav-forward", [second, first]),
    ] {
        for (count, target) in [1, 2].into_iter().zip(targets) {
            let arrow = cx.debug_bounds(selector).expect("enabled navigation arrow");
            click(cx, arrow.center(), MouseButton::Left, count);
            assert_eq!(
                model.read_with(cx, |model, _| model.active_tab()),
                Some(target)
            );
            assert_eq!(
                observer.read_with(cx, |observer, _| observer.zoom_requests),
                0
            );
        }
    }
}
