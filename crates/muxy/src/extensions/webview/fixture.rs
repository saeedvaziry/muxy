use crate::state::AppState;
use muxy_core::extensions::loader::load_extension;
use muxy_core::extensions::paths::ExtensionPaths;
use muxy_core::extensions::state::ExtensionStateStore;
use muxy_core::workspace::{Tab, TabKind, WorkspaceState};
use std::path::{Path, PathBuf};

const FIXTURE_EXTENSION_ID: &str = "muxy-phase6-webview-fixture";
const FIXTURE_TAB_TYPE_ID: &str = "main";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Phase6Fixture {
    extension_id: String,
    tab_type_id: String,
    title: String,
}

pub(crate) fn prepare_phase_6_fixture(
    paths: &ExtensionPaths,
) -> Result<Option<Phase6Fixture>, String> {
    let Some(value) = std::env::var_os(super::PHASE_6_FIXTURE_ENV) else {
        return Ok(None);
    };
    if !cfg!(debug_assertions) {
        return Err(format!(
            "{} is available only in debug builds",
            super::PHASE_6_FIXTURE_ENV
        ));
    }
    let source = if value == "1" {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/extensions/phase6-webview")
    } else {
        PathBuf::from(value)
    };
    prepare_phase_6_fixture_at(paths, &source).map(Some)
}

fn prepare_phase_6_fixture_at(
    paths: &ExtensionPaths,
    source: &Path,
) -> Result<Phase6Fixture, String> {
    let source = std::fs::canonicalize(source)
        .map_err(|error| format!("failed to resolve extension webview fixture: {error}"))?;
    let loaded = load_extension(&source)
        .map_err(|error| format!("failed to load extension webview fixture: {error}"))?;
    if loaded.id != FIXTURE_EXTENSION_ID {
        return Err(format!(
            "extension webview fixture must be named '{FIXTURE_EXTENSION_ID}'"
        ));
    }
    let tab_type = loaded
        .manifest
        .tab_types
        .iter()
        .find(|tab_type| tab_type.id == FIXTURE_TAB_TYPE_ID)
        .ok_or_else(|| {
            format!("extension webview fixture must declare tab type '{FIXTURE_TAB_TYPE_ID}'")
        })?;
    let mut state = ExtensionStateStore::open(paths.state_file())
        .map_err(|error| format!("failed to open extension fixture state: {error}"))?;
    let mut next = state.state().clone();
    let source = source.to_string_lossy().into_owned();
    if !next.development_paths.contains(&source) {
        next.development_paths.push(source);
    }
    next.enabled.insert(loaded.id.clone(), true);
    state
        .replace(next)
        .map_err(|error| format!("failed to enable extension webview fixture: {error}"))?;
    Ok(Phase6Fixture {
        extension_id: loaded.id,
        tab_type_id: tab_type.id.clone(),
        title: tab_type.title.clone(),
    })
}

pub(crate) fn provision_phase_6_fixture_tab(
    state: &mut AppState,
    fixture: &Phase6Fixture,
) -> Result<(), String> {
    let project_path = state.active_project().map(|project| project.path.clone());
    let workspace = state
        .active_tab_workspace_mut()
        .ok_or_else(|| "no active workspace is available for the extension fixture".to_owned())?;
    ensure_fixture_tab(workspace, fixture, project_path);
    state
        .persist_tab_workspaces()
        .map_err(|error| format!("failed to persist extension fixture tab: {error}"))
}

fn ensure_fixture_tab(
    workspace: &mut WorkspaceState,
    fixture: &Phase6Fixture,
    project_path: Option<String>,
) -> String {
    if let Some(tab_id) = workspace.root.as_ref().and_then(|root| {
        root.tabs()
            .into_iter()
            .find(|tab| {
                tab.kind == TabKind::ExtensionWebView
                    && tab.extension_id.as_deref() == Some(&fixture.extension_id)
                    && tab.extension_web_view_id.as_deref() == Some(&fixture.tab_type_id)
            })
            .map(|tab| tab.id.clone())
    }) {
        let root_id = workspace
            .root_id_for_tab(&tab_id)
            .map(str::to_owned)
            .unwrap_or_else(|| tab_id.clone());
        workspace.select_root_tab(&root_id);
        if let Some(area_id) = workspace
            .area_containing_tab(&tab_id)
            .map(|area| area.id.clone())
        {
            workspace.select_tab(&area_id, &tab_id);
        }
        return tab_id;
    }
    let mut tab = Tab::new(TabKind::ExtensionWebView);
    tab.project_path = project_path;
    tab.static_title = Some(fixture.title.clone());
    tab.extension_id = Some(fixture.extension_id.clone());
    tab.extension_web_view_id = Some(fixture.tab_type_id.clone());
    tab.extension_data = Some(serde_json::json!({
        "source": "rust-phase-6",
        "message": "Workspace data reached the extension webview"
    }));
    let tab_id = tab.id.clone();
    workspace.new_top_level_tab(tab);
    tab_id
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxy_core::environment::{BuildMode, RuntimePathPolicy};

    fn fixture() -> Phase6Fixture {
        Phase6Fixture {
            extension_id: FIXTURE_EXTENSION_ID.to_owned(),
            tab_type_id: FIXTURE_TAB_TYPE_ID.to_owned(),
            title: "Extension WebView Fixture".to_owned(),
        }
    }

    #[test]
    fn preparation_registers_and_enables_the_validated_development_fixture() {
        let root = tempfile::tempdir().unwrap();
        let paths =
            ExtensionPaths::new(RuntimePathPolicy::new(BuildMode::Development), root.path());
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/extensions/phase6-webview");

        let prepared = prepare_phase_6_fixture_at(&paths, &source).unwrap();
        assert_eq!(prepared, fixture());

        let state = ExtensionStateStore::open(paths.state_file()).unwrap();
        assert_eq!(state.enabled_override(FIXTURE_EXTENSION_ID), Some(true));
        assert_eq!(state.state().development_paths.len(), 1);
        assert_eq!(
            PathBuf::from(&state.state().development_paths[0]),
            std::fs::canonicalize(source).unwrap()
        );
    }

    #[test]
    fn tab_provisioning_is_idempotent_and_selects_the_fixture() {
        let fixture = fixture();
        let mut workspace = WorkspaceState::new("project");
        workspace.new_top_level_tab(Tab::new(TabKind::Terminal));

        let first = ensure_fixture_tab(&mut workspace, &fixture, Some("/project".to_owned()));
        let second = ensure_fixture_tab(&mut workspace, &fixture, Some("/project".to_owned()));

        assert_eq!(first, second);
        assert_eq!(
            workspace
                .root
                .as_ref()
                .unwrap()
                .tabs()
                .into_iter()
                .filter(|tab| tab.kind == TabKind::ExtensionWebView)
                .count(),
            1
        );
        assert_eq!(workspace.focused_root_tab_id(), Some(first.as_str()));
        let tab = workspace.tab(&first).unwrap();
        assert_eq!(tab.extension_id.as_deref(), Some(FIXTURE_EXTENSION_ID));
        assert_eq!(
            tab.extension_web_view_id.as_deref(),
            Some(FIXTURE_TAB_TYPE_ID)
        );
        assert_eq!(
            tab.extension_data.as_ref().unwrap()["source"],
            "rust-phase-6"
        );
    }
}
