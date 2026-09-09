use std::collections::HashSet;
use std::error::Error;

use muxy_app_core::{
    AppError, AppState, Color, PaneContent, PaneId, ProjectId, ServerId, TabId, WindowBounds,
};
use muxy_protocol::SessionId;
use serde_json::json;

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn bootstrap_has_only_home_with_no_tabs() -> TestResult {
    let state = AppState::bootstrap()?;
    let home = state.home();
    assert_eq!(state.version(), 1);
    assert_eq!(state.projects().len(), 1);
    assert_eq!(state.projects().first(), Some(home));
    assert_eq!(home.name, "Home");
    assert_eq!(home.server_id, ServerId::local());
    assert_eq!(Some(home.directory.clone()), std::env::home_dir());
    assert_eq!(home.color.as_str(), "#808080");
    assert!(home.icon.is_none());
    assert!(home.kind.is_none());
    assert!(home.parent_id.is_none());
    assert!(home.tabs.is_empty());
    assert_eq!(state.window().current_project, home.id);
    assert!(state.window().selected_tab.is_empty());
    assert!(state.window().bounds.is_none());
    assert_eq!(state.project(home.id), Some(home));
    assert!(state.project(ProjectId::new()).is_none());
    Ok(())
}

#[test]
fn generated_ids_are_unique_v4_hex_strings_and_round_trip() -> TestResult {
    let mut ids = HashSet::new();
    for _ in 0..64 {
        for id in [
            ServerId::new().to_string(),
            ProjectId::new().to_string(),
            TabId::new().to_string(),
            PaneId::new().to_string(),
        ] {
            assert_eq!(id.len(), 32);
            assert!(id.bytes().all(|byte| byte.is_ascii_hexdigit()));
            assert_eq!(
                uuid::Uuid::parse_str(&id)?.get_version(),
                Some(uuid::Version::Random)
            );
            assert!(ids.insert(id));
        }
    }
    let project = ProjectId::new();
    assert_eq!(project.to_string().parse::<ProjectId>()?, project);
    assert_eq!(serde_json::to_value(project)?, json!(project.to_string()));
    assert_eq!(
        serde_json::from_value::<ProjectId>(json!(project.to_string()))?,
        project
    );
    assert!("invalid".parse::<TabId>().is_err());
    assert_eq!(
        ServerId::local().to_string().parse::<ServerId>()?,
        ServerId::local()
    );
    Ok(())
}

#[test]
fn hex_colors_are_validated_and_normalized() -> TestResult {
    let color: Color = "#Aa12Ff".parse()?;
    assert_eq!(color.as_str(), "#aa12ff");
    assert_eq!(serde_json::to_value(&color)?, json!("#aa12ff"));
    assert_eq!(serde_json::from_value::<Color>(json!("#Aa12Ff"))?, color);
    for invalid in ["808080", "#fff", "#gggggg", "#ffffffff", "#💙00", ""] {
        assert!(invalid.parse::<Color>().is_err(), "{invalid}");
    }
    Ok(())
}

#[test]
fn opening_tabs_selects_one_terminal_pane_and_title_follows_it() -> TestResult {
    let mut state = AppState::bootstrap()?;
    let home = state.home().id;
    let first = state.open_terminal_tab(home)?;
    let second = state.open_terminal_tab(home)?;
    assert_ne!(first, second);
    assert_eq!(state.window().selected_tab.get(&home), Some(&second));
    assert_eq!(state.window().current_project, home);
    for tab in &state.home().tabs {
        assert_eq!(tab.panes.len(), 1);
        assert_eq!(tab.layout.leaves(), [tab.panes[0].id]);
        assert_eq!(tab.title(state.window().active_pane), "Terminal");
        assert_eq!(
            tab.panes[0].content,
            PaneContent::Terminal { session: None }
        );
    }
    let pane = state.home().tabs[0].panes[0].id;
    let session = SessionId::new(42).ok_or("nonzero session rejected")?;
    state.set_pane_session(pane, Some(session))?;
    state.set_pane_title(pane, "editor — résumé")?;
    assert_eq!(
        state.home().tabs[0].title(state.window().active_pane),
        "editor — résumé"
    );
    assert_eq!(
        state.home().tabs[0].panes[0].content,
        PaneContent::Terminal {
            session: Some(session)
        }
    );
    assert_eq!(
        state.home().tabs[1].title(state.window().active_pane),
        "Terminal"
    );
    state.set_pane_session(pane, None)?;
    assert_eq!(
        state.home().tabs[0].panes[0].content,
        PaneContent::Terminal { session: None }
    );
    Ok(())
}

#[test]
fn closing_selected_tabs_chooses_right_then_left_then_none() -> TestResult {
    let mut state = AppState::bootstrap()?;
    let home = state.home().id;
    let first = state.open_terminal_tab(home)?;
    let middle = state.open_terminal_tab(home)?;
    let last = state.open_terminal_tab(home)?;
    state.select_tab(home, middle)?;
    state.close_tab(home, middle)?;
    assert_eq!(state.window().selected_tab.get(&home), Some(&last));
    state.close_tab(home, last)?;
    assert_eq!(state.window().selected_tab.get(&home), Some(&first));
    state.close_tab(home, first)?;
    assert!(state.home().tabs.is_empty());
    assert!(state.window().selected_tab.is_empty());
    assert_eq!(state.window().current_project, home);
    assert_eq!(state.home().id, home);
    Ok(())
}

#[test]
fn closing_unselected_tab_preserves_selection_and_last_pane_closes_its_tab() -> TestResult {
    let mut state = AppState::bootstrap()?;
    let home = state.home().id;
    let first = state.open_terminal_tab(home)?;
    let last = state.open_terminal_tab(home)?;
    state.close_tab(home, first)?;
    assert_eq!(state.window().selected_tab.get(&home), Some(&last));
    let pane = state.home().tabs[0].panes[0].id;
    state.close_pane(pane)?;
    assert!(state.home().tabs.is_empty());
    assert!(state.window().selected_tab.is_empty());
    Ok(())
}

#[test]
fn moving_tabs_in_both_directions_preserves_selection_and_contents() -> TestResult {
    let mut state = AppState::bootstrap()?;
    let home = state.home().id;
    let first = state.open_terminal_tab(home)?;
    let middle = state.open_terminal_tab(home)?;
    let last = state.open_terminal_tab(home)?;
    let original = state.clone();
    state.move_tab(home, 0, 2)?;
    assert_eq!(
        state
            .home()
            .tabs
            .iter()
            .map(|tab| tab.id)
            .collect::<Vec<_>>(),
        [middle, last, first]
    );
    assert_eq!(state.window().selected_tab.get(&home), Some(&last));
    state.move_tab(home, 2, 0)?;
    assert_eq!(state, original);
    state.move_tab(home, 1, 1)?;
    assert_eq!(state, original);
    Ok(())
}

#[test]
fn rejected_operations_leave_state_unchanged() -> TestResult {
    let mut state = AppState::bootstrap()?;
    let home = state.home().id;
    let tab = state.open_terminal_tab(home)?;
    let before = state.clone();
    let unknown_project = ProjectId::new();
    let unknown_tab = TabId::new();
    let unknown_pane = PaneId::new();
    assert!(matches!(
        state.open_terminal_tab(unknown_project),
        Err(AppError::UnknownProject(_))
    ));
    assert!(state.close_tab(unknown_project, tab).is_err());
    assert!(state.close_tab(home, unknown_tab).is_err());
    assert!(state.select_tab(home, unknown_tab).is_err());
    assert!(state.select_tab(unknown_project, tab).is_err());
    assert!(state.move_tab(home, 0, 1).is_err());
    assert!(state.move_tab(home, usize::MAX, 0).is_err());
    assert!(state.move_tab(unknown_project, 0, 0).is_err());
    assert!(state.set_pane_session(unknown_pane, None).is_err());
    assert!(state.set_pane_title(unknown_pane, "missing").is_err());
    assert!(state.close_pane(unknown_pane).is_err());
    assert_eq!(state, before);
    Ok(())
}

#[test]
fn window_bounds_round_trip_and_reject_invalid_dimensions() -> TestResult {
    let mut state = AppState::bootstrap()?;
    let bounds = WindowBounds {
        x: -1440.0,
        y: 32.0,
        width: 1280.0,
        height: 800.0,
    };
    state.set_window_bounds(Some(bounds))?;
    assert_eq!(state.window().bounds, Some(bounds));
    let json = serde_json::to_string(&state)?;
    assert_eq!(serde_json::from_str::<AppState>(&json)?, state);
    for invalid in [
        WindowBounds {
            width: 0.0,
            ..bounds
        },
        WindowBounds {
            height: -1.0,
            ..bounds
        },
        WindowBounds {
            x: f64::NAN,
            ..bounds
        },
        WindowBounds {
            y: f64::INFINITY,
            ..bounds
        },
    ] {
        assert!(state.set_window_bounds(Some(invalid)).is_err());
        assert_eq!(state.window().bounds, Some(bounds));
    }
    state.set_window_bounds(None)?;
    assert!(state.window().bounds.is_none());
    Ok(())
}

#[test]
fn loading_repairs_missing_home_and_dangling_window_references() -> TestResult {
    let mut state = AppState::bootstrap()?;
    let home = state.home().id;
    state.open_terminal_tab(home)?;
    let mut json = serde_json::to_value(&state)?;
    json["projects"] = json!([]);
    let repaired: AppState = serde_json::from_value(json)?;
    assert_eq!(repaired.projects().len(), 1);
    assert_eq!(repaired.home().name, "Home");
    assert_ne!(repaired.home().id, home);
    assert_eq!(repaired.window().current_project, repaired.home().id);
    assert!(repaired.window().selected_tab.is_empty());
    assert!(repaired.home().tabs.is_empty());
    Ok(())
}

#[test]
fn loading_preserves_home_identity_and_restores_current_os_directory() -> TestResult {
    let state = AppState::bootstrap()?;
    let mut json = serde_json::to_value(&state)?;
    json["projects"][0]["directory"] = json!("/old/home");
    let repaired: AppState = serde_json::from_value(json)?;
    assert_eq!(repaired, state);
    Ok(())
}

#[test]
fn projects_at_the_same_location_keep_independent_identity_and_tabs() -> TestResult {
    let mut state = AppState::bootstrap()?;
    let home = state.home().id;
    let directory = std::env::temp_dir();
    let first = state.add_project(directory.clone())?;
    assert!(state.current_project().tabs.is_empty());
    let first_tab = state.open_terminal_tab(first)?;
    let second = state.add_project(directory)?;
    assert_ne!(first, second);
    assert!(state.current_project().tabs.is_empty());
    let second_tab = state.open_terminal_tab(second)?;
    state.select_project(first)?;
    assert_eq!(state.window().selected_tab.get(&first), Some(&first_tab));
    assert_eq!(state.window().selected_tab.get(&second), Some(&second_tab));
    state.move_project(second, 1)?;
    assert_eq!(
        state
            .projects()
            .iter()
            .map(|project| project.id)
            .collect::<Vec<_>>(),
        [home, second, first]
    );
    let before = state.clone();
    assert!(state.move_project(home, 1).is_err());
    assert!(state.move_project(first, 0).is_err());
    assert!(state.move_project(first, 3).is_err());
    assert!(state.remove_project(home).is_err());
    assert_eq!(state, before);
    assert_eq!(
        serde_json::from_value::<AppState>(serde_json::to_value(&state)?)?,
        state
    );
    Ok(())
}

#[test]
fn legacy_home_migrates_and_renamed_home_wins_over_another_home_name() -> TestResult {
    let mut state = AppState::bootstrap()?;
    let home = state.home().id;
    let tab = state.open_terminal_tab(home)?;
    let mut legacy = serde_json::to_value(&state)?;
    legacy["projects"][0]
        .as_object_mut()
        .ok_or("project object")?
        .remove("home");
    let mut migrated: AppState = serde_json::from_value(legacy)?;
    assert!(migrated.home().home);
    assert_eq!(migrated.home().id, home);
    assert_eq!(migrated.home().tabs[0].id, tab);
    migrated.rename_project(home, "Personal")?;
    let other = migrated.add_project(std::env::temp_dir())?;
    migrated.rename_project(other, "Home")?;
    let restored: AppState = serde_json::from_value(serde_json::to_value(&migrated)?)?;
    assert_eq!(restored.home().id, home);
    assert_eq!(restored.home().name, "Personal");
    assert_eq!(restored.current_project().id, other);
    assert_eq!(restored, migrated);
    Ok(())
}

#[test]
fn project_customization_accepts_one_grapheme_and_cycles_approved_colors() -> TestResult {
    let mut state = AppState::bootstrap()?;
    for index in 0..10 {
        let project = state.add_project(std::env::temp_dir())?;
        assert_eq!(
            state.current_project().color.as_str(),
            muxy_app_core::PROJECT_COLORS[index % 8].1
        );
        for icon in ["👩🏽‍💻", "🇩🇪", "e\u{301}"] {
            state.set_project_icon(project, Some(icon.into()))?;
            assert_eq!(state.current_project().icon.as_deref(), Some(icon));
        }
        let before = state.clone();
        for icon in ["", " ", "💙💛", "ab"] {
            assert!(state.set_project_icon(project, Some(icon.into())).is_err());
            assert_eq!(state, before);
        }
        assert!(state.rename_project(project, "  ").is_err());
        state.set_project_icon(project, None)?;
        state.rename_project(project, " Release ")?;
        state.set_project_color(project, "#123456".parse()?)?;
        assert_eq!(state.current_project().name, "Release");
    }
    Ok(())
}

#[test]
fn missing_projects_preserve_tabs_and_only_allow_removal() -> TestResult {
    let directory = std::env::temp_dir().join(format!("muxy-project-{}", ProjectId::new()));
    std::fs::create_dir(&directory)?;
    let marker = directory.join("keep.txt");
    std::fs::write(&marker, "keep")?;
    let mut state = AppState::bootstrap()?;
    let home = state.home().id;
    let project = state.add_project(directory.clone())?;
    let tab = state.open_terminal_tab(project)?;
    let session = SessionId::new(24).ok_or("session")?;
    state.set_pane_session(state.current_project().tabs[0].panes[0].id, Some(session))?;
    let moved = directory.with_extension("moved");
    std::fs::rename(&directory, &moved)?;
    state.refresh_project_statuses();
    assert_eq!(
        state.current_project().status(),
        muxy_app_core::ProjectStatus::Missing
    );
    assert!(state.open_terminal_tab(project).is_err());
    assert!(state.select_project(project).is_err());
    assert!(state.select_tab(project, tab).is_err());
    assert!(state.rename_project(project, "Gone").is_err());
    assert!(state.set_project_icon(project, None).is_err());
    assert!(state.set_project_color(project, Color::default()).is_err());
    assert!(state.move_project(project, 1).is_err());
    let json = serde_json::to_value(&state)?;
    assert!(json["projects"][1].get("status").is_none());
    let mut loaded: AppState = serde_json::from_value(json)?;
    assert_eq!(
        loaded.current_project().status(),
        muxy_app_core::ProjectStatus::Missing
    );
    assert_eq!(loaded.remove_project(project)?, [session]);
    assert_eq!(loaded.current_project().id, home);
    assert!(!loaded.window().selected_tab.contains_key(&project));
    assert_eq!(std::fs::read_to_string(moved.join("keep.txt"))?, "keep");
    std::fs::remove_dir_all(moved)?;
    Ok(())
}

#[test]
fn duplicate_projects_and_cross_project_tab_or_pane_ids_are_rejected() -> TestResult {
    let mut state = AppState::bootstrap()?;
    state.open_terminal_tab(state.home().id)?;
    let project = state.add_project(std::env::temp_dir())?;
    state.open_terminal_tab(project)?;
    let valid = serde_json::to_value(state)?;
    for (pointer, replacement) in [
        ("/projects/1/id", valid["projects"][0]["id"].clone()),
        ("/projects/1/home", json!(true)),
        (
            "/projects/1/tabs/0/id",
            valid["projects"][0]["tabs"][0]["id"].clone(),
        ),
        ("/projects/1/kind", json!("worktree")),
        ("/projects/1/parent_id", valid["projects"][0]["id"].clone()),
    ] {
        let mut invalid = valid.clone();
        *invalid.pointer_mut(pointer).ok_or("pointer")? = replacement;
        assert!(
            serde_json::from_value::<AppState>(invalid).is_err(),
            "{pointer}"
        );
    }
    let mut invalid = valid.clone();
    invalid["projects"][1]["tabs"][0]["panes"][0]["id"] =
        valid["projects"][0]["tabs"][0]["panes"][0]["id"].clone();
    invalid["projects"][1]["tabs"][0]["active_pane"] =
        valid["projects"][0]["tabs"][0]["panes"][0]["id"].clone();
    assert!(serde_json::from_value::<AppState>(invalid).is_err());
    Ok(())
}
