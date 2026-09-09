use super::*;
use gpui::{Bounds, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point};

fn projects() -> (AppState, [ProjectId; 4]) {
    let mut state = AppState::bootstrap().expect("state");
    let home = state.home().id;
    let first = state.add_project(std::env::temp_dir()).expect("first");
    let second = state.add_project(std::env::temp_dir()).expect("second");
    let third = state.add_project(std::env::temp_dir()).expect("third");
    state.select_project(second).expect("select second");
    (state, [home, first, second, third])
}

fn row(cx: &mut VisualTestContext, index: usize) -> Bounds<Pixels> {
    cx.debug_bounds(
        [
            "project-row-0",
            "project-row-1",
            "project-row-2",
            "project-row-3",
        ][index],
    )
    .expect("project row")
}

fn pointer(cx: &mut VisualTestContext, position: Point<Pixels>, dragging: bool) {
    cx.simulate_event(MouseMoveEvent {
        position,
        pressed_button: dragging.then_some(MouseButton::Left),
        ..Default::default()
    });
    cx.run_until_parked();
}

fn start(cx: &mut VisualTestContext, position: Point<Pixels>) {
    pointer(cx, position, false);
    cx.simulate_event(MouseDownEvent {
        position,
        button: MouseButton::Left,
        click_count: 1,
        ..Default::default()
    });
    pointer(cx, position + gpui::point(px(7.0), px(0.0)), true);
}

fn release(cx: &mut VisualTestContext, position: Point<Pixels>) {
    cx.simulate_event(MouseUpEvent {
        position,
        button: MouseButton::Left,
        ..Default::default()
    });
    cx.run_until_parked();
}

fn order(view: &Entity<AppModel>, cx: &VisualTestContext) -> Vec<ProjectId> {
    view.read_with(cx, |model, _| {
        model
            .state
            .projects()
            .iter()
            .map(|project| project.id)
            .collect()
    })
}

#[gpui::test]
fn dragging_projects_reorders_in_place_and_persists_without_selecting(cx: &mut TestAppContext) {
    for wide in [true, false] {
        let (state, [home, first, second, third]) = projects();
        let (mut boot, _requests) = stub_boot(state);
        boot.settings.appearance.sidebar_expanded = wide;
        let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
        cx.run_until_parked();
        let from = row(cx, 1).center();
        let to = row(cx, 3).center();
        start(cx, from);
        pointer(cx, to, true);
        assert_eq!(
            order(&view, cx),
            [home, second, third, first],
            "reorder before releasing, expanded={wide}"
        );
        pointer(cx, to, true);
        assert_eq!(order(&view, cx), [home, second, third, first]);
        let back = row(cx, 1).center();
        pointer(cx, back, true);
        assert_eq!(order(&view, cx), [home, first, second, third]);
        release(cx, back);
        assert!(!cx.update(|_, cx| cx.has_active_drag()));
        view.read_with(cx, |model, _| {
            assert_eq!(model.state.current_project().id, second);
            assert!(model.error.is_none());
            let saved = store::load(&model.path).expect("persisted order");
            assert_eq!(
                saved
                    .projects()
                    .iter()
                    .map(|project| project.id)
                    .collect::<Vec<_>>(),
                [home, first, second, third]
            );
        });
    }
}

#[gpui::test]
fn project_reordering_handles_a_fast_drop_and_release_outside_the_sidebar(cx: &mut TestAppContext) {
    for wide in [true, false] {
        let (state, [home, first, second, third]) = projects();
        let (mut boot, _requests) = stub_boot(state);
        boot.settings.appearance.sidebar_expanded = wide;
        let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
        cx.run_until_parked();
        let from = row(cx, 1).center();
        let to = row(cx, 3).center();
        pointer(cx, from, false);
        cx.simulate_event(MouseDownEvent {
            position: from,
            button: MouseButton::Left,
            click_count: 1,
            ..Default::default()
        });
        pointer(cx, to, true);
        release(cx, to);
        assert_eq!(order(&view, cx), [home, second, third, first]);
        view.read_with(cx, |model, _| {
            let saved = store::load(&model.path).expect("saved order");
            assert_eq!(
                saved
                    .projects()
                    .iter()
                    .map(|project| project.id)
                    .collect::<Vec<_>>(),
                [home, second, third, first]
            );
        });

        let from = row(cx, 3).center();
        let outside = gpui::point(px(500.0), row(cx, 1).center().y);
        start(cx, from);
        pointer(cx, outside, true);
        release(cx, outside);
        assert_eq!(order(&view, cx), [home, second, third, first]);
        assert!(!cx.update(|_, cx| cx.has_active_drag()));
        let first_row = row(cx, 1).center();
        pointer(cx, first_row, false);
        assert_eq!(order(&view, cx), [home, second, third, first]);
    }
}

#[gpui::test]
fn home_and_missing_projects_cannot_be_dragged_or_used_as_reorder_targets(cx: &mut TestAppContext) {
    let mut state = AppState::bootstrap().expect("state");
    let home = state.home().id;
    let first = state.add_project(std::env::temp_dir()).expect("first");
    let missing = std::env::temp_dir().join(format!("muxy-missing-drag-{}", ProjectId::new()));
    std::fs::create_dir(&missing).expect("temporary directory");
    let second = state.add_project(missing.clone()).expect("second");
    std::fs::remove_dir(missing).expect("remove empty test directory");
    let third = state.add_project(std::env::temp_dir()).expect("third");
    state.refresh_project_statuses();
    let ids = [home, first, second, third];
    let (boot, _requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.run_until_parked();
    for index in [0, 2] {
        let from = row(cx, index).center();
        let to = row(cx, 3).center();
        start(cx, from);
        pointer(cx, to, true);
        assert!(!cx.update(|_, cx| cx.has_active_drag()));
        release(cx, to);
        assert_eq!(order(&view, cx), ids);
    }
    let from = row(cx, 3).center();
    start(cx, from);
    for index in [0, 2] {
        let to = row(cx, index).center();
        pointer(cx, to, true);
        assert_eq!(order(&view, cx), ids);
    }
    release(cx, from);
    assert!(view.read_with(cx, |model, _| model.error.is_none()));
}

#[gpui::test]
fn project_clicks_still_select_and_a_lost_mouse_release_stops_reordering(cx: &mut TestAppContext) {
    let (state, ids) = projects();
    let (boot, _requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.run_until_parked();
    let from = row(cx, 1).center();
    pointer(cx, from, false);
    cx.simulate_event(MouseDownEvent {
        position: from,
        button: MouseButton::Left,
        click_count: 1,
        ..Default::default()
    });
    pointer(cx, from + gpui::point(px(1.0), px(0.0)), true);
    release(cx, from);
    assert_eq!(
        view.read_with(cx, |model, _| model.state.current_project().id),
        ids[1]
    );
    assert_eq!(order(&view, cx), ids);

    start(cx, from);
    assert!(cx.update(|_, cx| cx.has_active_drag()));
    let to = row(cx, 3).center();
    pointer(cx, to, false);
    assert!(!cx.update(|_, cx| cx.has_active_drag()));
    assert_eq!(order(&view, cx), ids);
}

#[gpui::test]
fn project_dragging_uses_scrolled_row_bounds_and_ignores_clipped_rows(cx: &mut TestAppContext) {
    for wide in [true, false] {
        let mut state = AppState::bootstrap().expect("state");
        for _ in 0..24 {
            state.add_project(std::env::temp_dir()).expect("project");
        }
        let original: Vec<_> = state.projects().iter().map(|project| project.id).collect();
        let (mut boot, _requests) = stub_boot(state);
        boot.settings.appearance.sidebar_expanded = wide;
        let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
        cx.simulate_resize(size(px(700.0), px(440.0)));
        cx.run_until_parked();
        let viewport = cx.debug_bounds("projects-scroll").expect("viewport");
        pointer(cx, viewport.center(), false);
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: viewport.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(-300.0))),
            ..Default::default()
        });
        cx.run_until_parked();
        let from = cx.debug_bounds("project-row-10").expect("source").center();
        let to = cx.debug_bounds("project-row-12").expect("target").center();
        assert!(
            viewport.contains(&from) && viewport.contains(&to),
            "scrolled rows visible, expanded={wide}"
        );
        start(cx, from);
        let header = gpui::point(from.x, viewport.origin.y - px(5.0));
        pointer(cx, header, true);
        assert_eq!(order(&view, cx), original);
        pointer(cx, to, true);
        let mut expected = original;
        let dragged = expected.remove(10);
        expected.insert(12, dragged);
        assert_eq!(order(&view, cx), expected);
        release(cx, to);
        assert_eq!(order(&view, cx), expected);
    }
}

#[gpui::test]
fn an_open_overlay_prevents_project_reordering(cx: &mut TestAppContext) {
    let (state, ids) = projects();
    let (boot, _requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.run_until_parked();
    let from = row(cx, 1).center();
    let to = row(cx, 3).center();
    start(cx, from);
    cx.update(|window, cx| {
        view.update(cx, |model, cx| {
            let project = model.state.project(ids[1]).expect("project");
            model.open_menu(crate::views::project_menu::items(project), from, window, cx);
        });
    });
    pointer(cx, to, true);
    assert_eq!(order(&view, cx), ids);
    release(cx, to);
    assert_eq!(order(&view, cx), ids);
}
