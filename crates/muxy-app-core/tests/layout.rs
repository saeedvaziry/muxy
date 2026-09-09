use muxy_app_core::{AppState, Axis, Branch, Direction, Layout, PaneId, restore};
use muxy_protocol::{ServerPath, SessionId, SessionInfo};

type Result = std::result::Result<(), Box<dyn std::error::Error>>;

#[test]
fn nested_splits_focus_zoom_and_collapse_survive_roundtrip() -> Result {
    let mut state = AppState::bootstrap()?;
    let tab = state.open_terminal_tab(state.home().id)?;
    let left = state.home().tabs[0].panes[0].id;
    let right = state.split_pane(left, Direction::Right)?;
    let bottom = state.split_pane(right, Direction::Down)?;
    assert_eq!(state.home().tabs[0].layout.leaves(), [left, right, bottom]);
    assert_eq!(state.neighbor(left, Direction::Right), Some(right));
    assert_eq!(state.neighbor(right, Direction::Down), Some(bottom));
    assert_eq!(state.neighbor(bottom, Direction::Up), Some(right));
    assert_eq!(state.neighbor(bottom, Direction::Left), Some(left));
    assert_eq!(state.neighbor(left, Direction::Left), None);
    assert_eq!(state.neighbor(right, Direction::Right), None);
    state.set_ratio(tab, &[], 0.6)?;
    state.set_ratio(tab, &[Branch::Second], 0.3)?;
    state.focus_pane(right)?;
    state.set_pane_title(right, "editor")?;
    assert_eq!(
        state.home().tabs[0].title(state.window().active_pane),
        "editor"
    );
    state.toggle_zoom(right)?;
    assert_eq!(state.home().tabs[0].visible_panes(), [right]);
    let restored: AppState = serde_json::from_value(serde_json::to_value(&state)?)?;
    assert_eq!(restored, state);
    state.focus_pane(bottom)?;
    assert_eq!(state.home().tabs[0].visible_panes(), [bottom]);
    state.toggle_zoom(bottom)?;
    assert_eq!(state.home().tabs[0].visible_panes(), [left, right, bottom]);
    state.close_pane(right)?;
    assert_eq!(state.home().tabs[0].layout.leaves(), [left, bottom]);
    assert_eq!(state.window().active_pane, Some(bottom));
    state.toggle_zoom(bottom)?;
    state.close_pane(bottom)?;
    assert_eq!(state.home().tabs[0].layout, Layout::Leaf(left));
    assert_eq!(state.window().active_pane, Some(left));
    assert_eq!(state.home().tabs[0].zoomed, None);
    state.close_pane(left)?;
    assert!(state.home().tabs.is_empty());
    assert!(state.window().selected_tab.is_empty());
    Ok(())
}

#[test]
fn splits_before_the_source_and_ratio_errors_leave_valid_trees() -> Result {
    let mut state = AppState::bootstrap()?;
    let tab = state.open_terminal_tab(state.home().id)?;
    let original = state.home().tabs[0].panes[0].id;
    let left = state.split_pane(original, Direction::Left)?;
    let top = state.split_pane(left, Direction::Up)?;
    assert_eq!(state.home().tabs[0].layout.leaves(), [top, left, original]);
    state.set_ratio(tab, &[], -1.0)?;
    state.set_ratio(tab, &[Branch::First], 2.0)?;
    let Layout::Split { ratio, first, .. } = &state.home().tabs[0].layout else {
        return Err("split expected".into());
    };
    assert!((*ratio - 0.15).abs() < f32::EPSILON);
    assert!(
        matches!(first.as_ref(), Layout::Split { ratio, axis: Axis::Vertical, .. } if (*ratio - 0.85).abs() < f32::EPSILON)
    );
    let before = state.clone();
    assert!(state.set_ratio(tab, &[], f32::NAN).is_err());
    assert!(state.set_ratio(tab, &[Branch::Second], 0.5).is_err());
    assert!(state.split_pane(PaneId::new(), Direction::Right).is_err());
    assert!(state.focus_pane(PaneId::new()).is_err());
    assert!(state.toggle_zoom(PaneId::new()).is_err());
    assert_eq!(state, before);
    Ok(())
}

#[test]
fn m1_tabs_load_and_malformed_layouts_are_rejected() -> Result {
    let mut state = AppState::bootstrap()?;
    state.open_terminal_tab(state.home().id)?;
    let mut legacy = serde_json::to_value(&state)?;
    let tab = legacy["projects"][0]["tabs"][0]
        .as_object_mut()
        .ok_or("tab")?;
    tab.remove("layout");
    tab.remove("zoomed");
    assert_eq!(serde_json::from_value::<AppState>(legacy)?, state);
    let first = state.home().tabs[0].panes[0].id;
    state.split_pane(first, Direction::Right)?;
    let valid = serde_json::to_value(&state)?;
    for (pointer, replacement) in [
        (
            "/layout/Split/first",
            serde_json::to_value(Layout::Leaf(PaneId::new()))?,
        ),
        (
            "/layout/Split/second",
            serde_json::to_value(Layout::Leaf(first))?,
        ),
        ("/layout/Split/ratio", serde_json::json!(1.5)),
        ("/zoomed", serde_json::to_value(PaneId::new())?),
        ("/panes", serde_json::json!([])),
    ] {
        let mut invalid = valid.clone();
        *invalid["projects"][0]["tabs"][0]
            .pointer_mut(pointer)
            .ok_or("pointer")? = replacement;
        assert!(
            serde_json::from_value::<AppState>(invalid).is_err(),
            "{pointer}"
        );
    }
    Ok(())
}

#[test]
fn restore_accounts_for_each_leaf_even_when_zoomed() -> Result {
    let mut state = AppState::bootstrap()?;
    state.open_terminal_tab(state.home().id)?;
    let first = state.home().tabs[0].panes[0].id;
    let second = state.split_pane(first, Direction::Right)?;
    let third = state.split_pane(second, Direction::Down)?;
    let live = SessionId::new(23).ok_or("session")?;
    let ended = SessionId::new(24).ok_or("session")?;
    state.set_pane_session(first, Some(live))?;
    state.set_pane_session(second, Some(ended))?;
    state.toggle_zoom(third)?;
    let plan = restore::plan(
        &state,
        &[SessionInfo {
            id: live,
            directory: ServerPath(b"/tmp".to_vec()),
        }],
    );
    assert_eq!(plan.attach, [(first, live)]);
    assert_eq!(plan.retain, [(second, ended)]);
    assert_eq!(plan.create, [third]);
    Ok(())
}

#[test]
fn window_owns_focus_and_closing_chooses_neighbors_without_stealing_focus() -> Result {
    let mut state = AppState::bootstrap()?;
    let home = state.home().id;
    let first_tab = state.open_terminal_tab(home)?;
    let first = state.window().active_pane.ok_or("focus")?;
    let middle = state.split_pane(first, Direction::Right)?;
    let last = state.split_pane(middle, Direction::Right)?;
    state.close_pane(last)?;
    assert_eq!(state.window().active_pane, Some(middle));
    let second_tab = state.open_terminal_tab(home)?;
    let second_first = state.window().active_pane.ok_or("focus")?;
    let second_last = state.split_pane(second_first, Direction::Right)?;
    state.toggle_zoom(second_last)?;
    state.select_tab(home, first_tab)?;
    state.focus_pane(middle)?;
    state.close_tab(home, first_tab)?;
    assert_eq!(state.window().active_pane, Some(second_first));
    assert_eq!(
        state.home().tabs[0].visible_panes(),
        [second_first, second_last]
    );
    let third_tab = state.open_terminal_tab(home)?;
    let third_first = state.window().active_pane.ok_or("focus")?;
    state.close_pane(second_last)?;
    assert_eq!(state.window().active_pane, Some(third_first));
    state.close_tab(home, second_tab)?;
    assert_eq!(state.window().active_pane, Some(third_first));
    state.close_tab(home, third_tab)?;
    assert_eq!(state.window().active_pane, None);
    Ok(())
}

#[test]
fn legacy_tab_focus_migrates_once_into_the_window() -> Result {
    let mut state = AppState::bootstrap()?;
    state.open_terminal_tab(state.home().id)?;
    let first = state.window().active_pane.ok_or("focus")?;
    let second = state.split_pane(first, Direction::Right)?;
    let mut saved = serde_json::to_value(&state)?;
    saved["window"]
        .as_object_mut()
        .ok_or("window")?
        .remove("active_pane");
    saved["projects"][0]["tabs"][0]["active_pane"] = serde_json::to_value(second)?;
    let restored: AppState = serde_json::from_value(saved)?;
    assert_eq!(restored, state);
    let migrated = serde_json::to_value(restored)?;
    assert_eq!(
        migrated["window"]["active_pane"],
        serde_json::to_value(second)?
    );
    assert!(
        migrated["projects"][0]["tabs"][0]
            .get("active_pane")
            .is_none()
    );
    Ok(())
}

#[test]
fn closing_tabs_in_a_missing_project_keeps_saved_window_state_loadable() -> Result {
    let mut state = AppState::bootstrap()?;
    let directory = std::env::temp_dir().join(format!("muxy-missing-{}", PaneId::new()));
    std::fs::create_dir(&directory)?;
    let project = state.add_project(directory.clone())?;
    let first = state.open_terminal_tab(project)?;
    let second = state.open_terminal_tab(project)?;
    state.select_tab(project, first)?;
    std::fs::remove_dir(&directory)?;
    state.refresh_project_statuses();
    state.close_tab(project, first)?;
    let restored: AppState = serde_json::from_value(serde_json::to_value(&state)?)?;
    assert_eq!(restored, state);
    state.close_tab(project, second)?;
    assert_eq!(state.window().active_pane, None);
    let restored: AppState = serde_json::from_value(serde_json::to_value(&state)?)?;
    assert_eq!(restored, state);
    Ok(())
}
