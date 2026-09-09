use std::error::Error;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use muxy_app_core::{AppError, AppState, PaneId, ProjectId, TabId, WindowBounds, store};
use muxy_protocol::SessionId;
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn Error>>;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        let directory = std::env::temp_dir().join(format!("muxy-app-core-{}", ProjectId::new()));
        fs::create_dir(&directory)?;
        Ok(Self(directory))
    }

    fn path(&self) -> PathBuf {
        self.0.join("state.json")
    }

    fn temporary(&self) -> PathBuf {
        self.0.join("state.json.tmp")
    }

    fn write(&self, value: &Value) -> TestResult {
        fs::write(self.path(), serde_json::to_vec(value)?)?;
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn populated() -> Result<AppState, Box<dyn Error>> {
    let mut state = AppState::bootstrap()?;
    let home = state.home().id;
    let first = state.open_terminal_tab(home)?;
    state.open_terminal_tab(home)?;
    state.select_tab(home, first)?;
    let pane = state.home().tabs[0].panes[0].id;
    state.set_pane_title(pane, "日本語 — editor")?;
    state.set_pane_session(pane, SessionId::new(u64::MAX))?;
    state.set_window_bounds(Some(WindowBounds {
        x: -1400.0,
        y: 40.0,
        width: 1200.0,
        height: 900.0,
    }))?;
    Ok(state)
}

#[test]
fn missing_file_bootstraps_without_writing() -> TestResult {
    let fixture = Fixture::new()?;
    let state = store::load(fixture.path())?;
    assert_eq!(state.home().name, "Home");
    assert!(state.home().tabs.is_empty());
    assert!(!fixture.path().exists());
    assert!(!fixture.temporary().exists());
    Ok(())
}

#[test]
fn saved_json_is_readable_and_round_trips_every_field() -> TestResult {
    let fixture = Fixture::new()?;
    let state = populated()?;
    store::save(fixture.path(), &state)?;
    let bytes = fs::read_to_string(fixture.path())?;
    assert!(bytes.contains("\n  \"version\": 1,"));
    assert!(bytes.ends_with('\n'));
    assert_eq!(
        serde_json::from_str::<Value>(&bytes)?,
        serde_json::to_value(&state)?
    );
    assert_eq!(store::load(fixture.path())?, state);
    assert!(!fixture.temporary().exists());
    assert_eq!(
        fs::metadata(fixture.path())?.permissions().mode() & 0o777,
        0o600
    );
    Ok(())
}

#[test]
fn save_replaces_file_atomically_and_reuses_abandoned_temporary_file() -> TestResult {
    let fixture = Fixture::new()?;
    let mut state = populated()?;
    store::save(fixture.path(), &state)?;
    let old_bytes = fs::read_to_string(fixture.path())?;
    let mut old_file = File::open(fixture.path())?;
    let home = state.home().id;
    state.open_terminal_tab(home)?;
    fs::write(fixture.temporary(), "interrupted write")?;
    store::save(fixture.path(), &state)?;
    let mut still_old = String::new();
    old_file.read_to_string(&mut still_old)?;
    assert_eq!(still_old, old_bytes);
    assert_ne!(fs::read_to_string(fixture.path())?, old_bytes);
    assert_eq!(store::load(fixture.path())?, state);
    assert!(!fixture.temporary().exists());
    Ok(())
}

#[test]
fn corrupt_json_is_an_error_naming_the_path_and_is_not_overwritten() -> TestResult {
    let fixture = Fixture::new()?;
    fs::write(fixture.path(), b"{ broken json")?;
    let error = store::load(fixture.path())
        .err()
        .ok_or("corruption accepted")?;
    assert!(matches!(error, AppError::Json { .. }));
    assert!(
        error
            .to_string()
            .contains(&fixture.path().display().to_string())
    );
    assert!(error.source().is_some());
    assert_eq!(fs::read(fixture.path())?, b"{ broken json");
    Ok(())
}

#[test]
fn load_restores_missing_home_and_clears_its_old_selection() -> TestResult {
    let fixture = Fixture::new()?;
    let mut value = serde_json::to_value(populated()?)?;
    value["projects"] = json!([]);
    fixture.write(&value)?;
    let state = store::load(fixture.path())?;
    assert_eq!(state.projects().len(), 1);
    assert_eq!(state.home().name, "Home");
    assert_eq!(state.window().current_project, state.home().id);
    assert!(state.window().selected_tab.is_empty());
    store::save(fixture.path(), &state)?;
    assert_eq!(store::load(fixture.path())?, state);
    Ok(())
}

#[test]
fn load_repairs_stale_or_missing_window_selection_without_losing_tabs() -> TestResult {
    let fixture = Fixture::new()?;
    let state = populated()?;
    let home = state.home().id;
    for selections in [
        json!({}),
        json!({home.to_string(): TabId::new(), ProjectId::new().to_string(): TabId::new()}),
    ] {
        let mut value = serde_json::to_value(&state)?;
        value["window"]["selected_tab"] = selections;
        value["window"]["current_project"] = json!(ProjectId::new());
        fixture.write(&value)?;
        assert_eq!(store::load(fixture.path())?, state);
    }
    Ok(())
}

#[test]
fn malformed_domain_state_is_rejected_with_the_path() -> TestResult {
    let fixture = Fixture::new()?;
    let valid = serde_json::to_value(populated()?)?;
    let mut invalid_states = Vec::new();
    for version in [0, 2] {
        let mut value = valid.clone();
        value["version"] = json!(version);
        invalid_states.push(value);
    }
    for panes in [
        json!([]),
        json!(vec![valid["projects"][0]["tabs"][0]["panes"][0].clone(); 2]),
    ] {
        let mut value = valid.clone();
        value["projects"][0]["tabs"][0]["panes"] = panes;
        invalid_states.push(value);
    }
    for (pointer, replacement) in [
        ("/window/active_pane", json!(PaneId::new())),
        (
            "/projects/0/tabs/0/id",
            valid["projects"][0]["tabs"][1]["id"].clone(),
        ),
        (
            "/projects/0/tabs/0/panes/0/content",
            json!({"type": "unknown"}),
        ),
        ("/projects/0/tabs/0/panes/0/content/session", json!(0)),
        ("/projects/0/id", json!("invalid")),
        ("/projects/0/color", json!("blue")),
        ("/window/bounds/width", json!(-1)),
    ] {
        let mut value = valid.clone();
        *value.pointer_mut(pointer).ok_or("missing fixture field")? = replacement;
        invalid_states.push(value);
    }
    let mut duplicate_pane = valid.clone();
    let pane = duplicate_pane["projects"][0]["tabs"][0]["panes"][0]["id"].clone();
    duplicate_pane["projects"][0]["tabs"][1]["active_pane"] = pane.clone();
    duplicate_pane["projects"][0]["tabs"][1]["panes"][0]["id"] = pane;
    invalid_states.push(duplicate_pane);
    for value in invalid_states {
        fixture.write(&value)?;
        let error = store::load(fixture.path())
            .err()
            .ok_or_else(|| format!("invalid state accepted: {value}"))?;
        assert!(
            error
                .to_string()
                .contains(&fixture.path().display().to_string())
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(fixture.path())?)?,
            value
        );
    }
    Ok(())
}

#[test]
fn read_and_write_io_errors_name_the_path() -> TestResult {
    let fixture = Fixture::new()?;
    fs::create_dir(fixture.path())?;
    let read_error = store::load(fixture.path())
        .err()
        .ok_or("directory read succeeded")?;
    assert!(
        read_error
            .to_string()
            .contains(&fixture.path().display().to_string())
    );
    let write_error = store::save(fixture.path(), &populated()?)
        .err()
        .ok_or("rename over directory succeeded")?;
    assert!(
        write_error
            .to_string()
            .contains(&fixture.path().display().to_string())
    );
    assert!(fixture.path().is_dir());
    assert!(!fixture.temporary().exists());
    Ok(())
}

#[test]
fn temporary_file_failure_preserves_previous_saved_state() -> TestResult {
    let fixture = Fixture::new()?;
    let mut state = populated()?;
    store::save(fixture.path(), &state)?;
    let old_bytes = fs::read(fixture.path())?;
    fs::create_dir(fixture.temporary())?;
    let home = state.home().id;
    state.open_terminal_tab(home)?;
    let error = store::save(fixture.path(), &state)
        .err()
        .ok_or("temporary directory accepted")?;
    assert!(
        error
            .to_string()
            .contains(&fixture.temporary().display().to_string())
    );
    assert_eq!(fs::read(fixture.path())?, old_bytes);
    assert!(fixture.temporary().is_dir());
    Ok(())
}

#[test]
fn overlapping_save_preserves_the_active_writer_and_previous_state() -> TestResult {
    let fixture = Fixture::new()?;
    let state = populated()?;
    store::save(fixture.path(), &state)?;
    let old_bytes = fs::read(fixture.path())?;
    let active_writer = File::create(fixture.temporary())?;
    active_writer.lock()?;
    fs::write(fixture.temporary(), "active write")?;
    let error = store::save(fixture.path(), &state)
        .err()
        .ok_or("overlapping save was accepted")?;
    assert!(
        matches!(error, AppError::Io { source, .. } if source.kind() == std::io::ErrorKind::WouldBlock)
    );
    assert_eq!(fs::read(fixture.temporary())?, b"active write");
    assert_eq!(fs::read(fixture.path())?, old_bytes);
    drop(active_writer);
    store::save(fixture.path(), &state)?;
    assert_eq!(store::load(fixture.path())?, state);
    Ok(())
}
