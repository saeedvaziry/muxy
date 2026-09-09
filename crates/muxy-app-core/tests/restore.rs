use std::error::Error;
use std::fs;

use muxy_app_core::{AppState, ProjectId, WindowBounds, restore, store};
use muxy_protocol::{ServerPath, SessionId, SessionInfo};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn closing_offline_persists_cleanup_without_restoring_the_tab() -> TestResult {
    let path = std::env::temp_dir().join(format!("muxy-close-{}.json", ProjectId::new()));
    let mut state = AppState::bootstrap()?;
    let tab = state.open_terminal_tab(state.home().id)?;
    let session = SessionId::new(42).ok_or("zero ID")?;
    state.set_pane_session(state.home().tabs[0].panes[0].id, Some(session))?;
    state.queue_discard(session);
    state.queue_discard(session);
    state.close_tab(state.home().id, tab)?;
    store::save(&path, &state)?;
    let mut loaded = store::load(&path)?;
    assert!(loaded.home().tabs.is_empty());
    assert_eq!(loaded.pending_discards(), &[session]);
    assert_eq!(restore::plan(&loaded, &[]), restore::RestorePlan::default());
    loaded.complete_discard(session);
    store::save(&path, &loaded)?;
    assert!(store::load(&path)?.pending_discards().is_empty());
    fs::remove_file(path)?;
    Ok(())
}

#[test]
fn existing_state_files_load_without_pending_cleanup() -> TestResult {
    let state = AppState::bootstrap()?;
    let mut value = serde_json::to_value(&state)?;
    value
        .as_object_mut()
        .ok_or("not an object")?
        .remove("pending_discards");
    let loaded: AppState = serde_json::from_value(value)?;
    assert_eq!(loaded, state);
    assert!(loaded.pending_discards().is_empty());
    Ok(())
}

#[test]
fn mixed_restore_preserves_every_tab_and_its_selection() -> TestResult {
    let mut state = AppState::bootstrap()?;
    let home = state.home().id;
    for _ in 0..3 {
        state.open_terminal_tab(home)?;
    }
    let panes: Vec<_> = state
        .home()
        .tabs
        .iter()
        .map(|tab| tab.panes[0].id)
        .collect();
    let live = SessionId::new(11).ok_or("zero ID")?;
    let missing = SessionId::new(22).ok_or("zero ID")?;
    state.set_pane_session(panes[0], Some(live))?;
    state.set_pane_session(panes[1], Some(missing))?;
    state.select_tab(home, state.home().tabs[1].id)?;
    let before = state.clone();
    let plan = restore::plan(
        &state,
        &[SessionInfo {
            id: live,
            directory: ServerPath(b"/tmp".to_vec()),
        }],
    );
    assert_eq!(plan.attach, vec![(panes[0], live)]);
    assert_eq!(plan.retain, vec![(panes[1], missing)]);
    assert_eq!(plan.create, vec![panes[2]]);
    assert_eq!(state, before);
    let all_missing = restore::plan(&state, &[]);
    assert_eq!(
        all_missing.retain,
        vec![(panes[0], live), (panes[1], missing)]
    );
    assert!(all_missing.attach.is_empty());
    assert_eq!(state.home().tabs.len(), 3);
    Ok(())
}

#[test]
fn empty_restore_does_not_create_a_tab_or_adopt_an_unreferenced_session() -> TestResult {
    let state = AppState::bootstrap()?;
    let live = SessionInfo {
        id: SessionId::new(1).ok_or("zero ID")?,
        directory: ServerPath(b"/tmp".to_vec()),
    };
    assert_eq!(
        restore::plan(&state, &[live]),
        restore::RestorePlan::default()
    );
    assert!(state.home().tabs.is_empty());
    Ok(())
}

#[test]
fn saved_bounds_and_ended_session_references_round_trip() -> TestResult {
    let path = std::env::temp_dir().join(format!("muxy-restore-{}.json", ProjectId::new()));
    let mut state = AppState::bootstrap()?;
    state.open_terminal_tab(state.home().id)?;
    let pane = state.home().tabs[0].panes[0].id;
    let session = SessionId::new(u64::MAX).ok_or("zero ID")?;
    state.set_pane_session(pane, Some(session))?;
    state.set_window_bounds(Some(WindowBounds {
        x: -800.0,
        y: 75.0,
        width: 1100.0,
        height: 700.0,
    }))?;
    store::save(&path, &state)?;
    let loaded = store::load(&path)?;
    fs::remove_file(path)?;
    assert_eq!(loaded, state);
    let plan = restore::plan(&loaded, &[]);
    assert_eq!(plan.retain, vec![(pane, session)]);
    assert!(plan.attach.is_empty());
    assert!(plan.create.is_empty());
    Ok(())
}

#[test]
fn restore_covers_hidden_projects_without_creating_tabs_for_empty_projects() -> TestResult {
    let mut state = AppState::bootstrap()?;
    let live = SessionId::new(24).ok_or("session")?;
    let ended = SessionId::new(25).ok_or("session")?;
    let mut panes = Vec::new();
    for session in [Some(live), Some(ended), None] {
        let project = state.add_project(std::env::temp_dir())?;
        state.open_terminal_tab(project)?;
        let pane = state.current_project().tabs[0].panes[0].id;
        state.set_pane_session(pane, session)?;
        panes.push(pane);
    }
    state.add_project(std::env::temp_dir())?;
    let loaded: AppState = serde_json::from_value(serde_json::to_value(&state)?)?;
    let plan = restore::plan(
        &loaded,
        &[SessionInfo {
            id: live,
            directory: ServerPath(b"/tmp".to_vec()),
        }],
    );
    assert_eq!(plan.attach, [(panes[0], live)]);
    assert_eq!(plan.retain, [(panes[1], ended)]);
    assert_eq!(plan.create, [panes[2]]);
    assert!(loaded.current_project().tabs.is_empty());
    Ok(())
}
