#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use super::loader::{LoadedExtension, load_extension};
use super::logs::{MAX_EXTENSION_AUDIT_BYTES, MAX_EXTENSION_LOG_BYTES};
use super::package::ExtensionPackagePublisher;
use super::paths::ExtensionPaths;
use super::state::{
    ExtensionGrantRule, ExtensionMigrationWarning, ExtensionShortcut, ExtensionStateStore,
    MAX_SETTING_VALUE_BYTES,
};
use crate::environment::BuildMode;
#[cfg(target_os = "macos")]
use crate::environment::StoragePathPolicy;
use crate::store::write_private;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

pub const EXTENSION_MIGRATION_SCHEMA_VERSION: u32 = 1;
const LEGACY_GRANTS_FILE: &str = "extension-grants.json";
const LEGACY_SHORTCUTS_FILE: &str = "extension-shortcuts.json";
const LEGACY_STORAGE_DIRECTORY: &str = "extension-storage";
const LEGACY_AUDIT_FILE: &str = "extension-audit.log";
const DEVELOPMENT_PATHS_KEY: &str = "muxy.ext.devPaths";
const ENABLED_PREFIX: &str = "muxy.ext.enabled.";
const SETTINGS_PREFIX: &str = "muxy.ext.";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionMigrationOutcome {
    Completed,
    SourceMissing,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExtensionMigrationState {
    pub schema_version: u32,
    pub outcome: ExtensionMigrationOutcome,
    pub imported: Vec<String>,
    pub existing: Vec<String>,
    pub warnings: Vec<ExtensionMigrationWarning>,
}

#[derive(Clone, Debug)]
pub struct SwiftExtensionSources {
    pub packages: PathBuf,
    pub app_support: PathBuf,
    pub preferences: Map<String, Value>,
}

#[derive(Debug, Error)]
pub enum ExtensionMigrationError {
    #[error("failed to prepare extension migration: {0}")]
    Prepare(#[source] std::io::Error),
    #[error("extension migration is already running")]
    Locked,
    #[error("failed to read extension migration marker: {0}")]
    MarkerRead(#[source] std::io::Error),
    #[error("extension migration marker is invalid: {0}")]
    MarkerInvalid(#[source] serde_json::Error),
    #[error("unsupported extension migration schema {0}")]
    UnsupportedSchema(u32),
    #[error("failed to access Rust extension state: {0}")]
    State(#[from] super::state::ExtensionStateError),
    #[error("failed to write extension migration marker: {0}")]
    MarkerWrite(#[source] std::io::Error),
}

#[derive(Default)]
struct MigrationRecords {
    imported: BTreeSet<String>,
    existing: BTreeSet<String>,
    warnings: Vec<ExtensionMigrationWarning>,
}

struct MigrationLock {
    path: PathBuf,
}

impl Drop for MigrationLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn run_startup(
    paths: &ExtensionPaths,
) -> Result<Option<ExtensionMigrationState>, ExtensionMigrationError> {
    if crate::build_mode!() != BuildMode::Production || !cfg!(target_os = "macos") {
        return Ok(None);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = paths;
        Ok(None)
    }
    #[cfg(target_os = "macos")]
    {
        let home = crate::prefs::home_dir();
        let sources = SwiftExtensionSources {
            packages: home.join(".config/muxy/extensions"),
            app_support: StoragePathPolicy::swift_source(&home),
            preferences: read_swift_extension_defaults().unwrap_or_default(),
        };
        run(paths, &sources).map(Some)
    }
}

pub fn run(
    paths: &ExtensionPaths,
    sources: &SwiftExtensionSources,
) -> Result<ExtensionMigrationState, ExtensionMigrationError> {
    std::fs::create_dir_all(&paths.state).map_err(ExtensionMigrationError::Prepare)?;
    std::fs::create_dir_all(&paths.staging).map_err(ExtensionMigrationError::Prepare)?;
    std::fs::create_dir_all(&paths.packages).map_err(ExtensionMigrationError::Prepare)?;
    let _lock = acquire_lock(paths)?;
    if let Some(marker) = read_marker(paths)? {
        return Ok(marker);
    }

    let mut store = ExtensionStateStore::open(paths.state_file())?;
    let mut state = store.state().clone();
    let mut records = MigrationRecords::default();
    let mut extension_ids = BTreeSet::new();
    let mut loaded_sources = Vec::new();
    let publisher = ExtensionPackagePublisher::new(paths.clone());

    import_packages(
        paths,
        sources,
        &publisher,
        &mut extension_ids,
        &mut loaded_sources,
        &mut records,
    );
    import_development_paths(
        sources,
        &mut state.development_paths,
        &mut extension_ids,
        &mut loaded_sources,
        &mut records.imported,
        &mut records.warnings,
    );
    import_preferences(
        &sources.preferences,
        &extension_ids,
        &loaded_sources,
        &mut state,
        &mut records.imported,
        &mut records.warnings,
    );
    import_grants_and_shortcuts(
        sources,
        &mut state,
        &mut records.imported,
        &mut records.existing,
        &mut records.warnings,
    );
    import_mutable_files(
        paths,
        sources,
        &extension_ids,
        &loaded_sources,
        &mut records.imported,
        &mut records.existing,
        &mut records.warnings,
    );

    for warning in &records.warnings {
        if !state.migration_warnings.contains(warning) {
            state.migration_warnings.push(warning.clone());
        }
    }
    store.replace(state)?;
    let has_source = sources.packages.exists()
        || sources.app_support.exists()
        || !sources.preferences.is_empty();
    let marker = ExtensionMigrationState {
        schema_version: EXTENSION_MIGRATION_SCHEMA_VERSION,
        outcome: if has_source {
            ExtensionMigrationOutcome::Completed
        } else {
            ExtensionMigrationOutcome::SourceMissing
        },
        imported: records.imported.into_iter().collect(),
        existing: records.existing.into_iter().collect(),
        warnings: records.warnings,
    };
    write_marker(paths, &marker)?;
    Ok(marker)
}

fn import_packages(
    paths: &ExtensionPaths,
    sources: &SwiftExtensionSources,
    publisher: &ExtensionPackagePublisher,
    extension_ids: &mut BTreeSet<String>,
    loaded_sources: &mut Vec<LoadedExtension>,
    records: &mut MigrationRecords,
) {
    let Ok(entries) = std::fs::read_dir(&sources.packages) else {
        return;
    };
    let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let relative = format!("packages/{}", entry.file_name().to_string_lossy());
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                warning(
                    &mut records.warnings,
                    "unsafe_package",
                    &path,
                    "package is not a real directory",
                );
                continue;
            }
            Err(error) => {
                warning(
                    &mut records.warnings,
                    "package_read",
                    &path,
                    &error.to_string(),
                );
                continue;
            }
        }
        let loaded = match load_extension(&path) {
            Ok(loaded) => loaded,
            Err(error) => {
                warning(
                    &mut records.warnings,
                    "invalid_package",
                    &path,
                    &error.to_string(),
                );
                continue;
            }
        };
        extension_ids.insert(loaded.id.clone());
        loaded_sources.push(loaded.clone());
        let destination = paths.packages.join(&loaded.id);
        if destination.exists() {
            records.existing.insert(relative);
            continue;
        }
        match publisher.publish(&path, false) {
            Ok(_) => {
                records.imported.insert(relative);
            }
            Err(error) => warning(
                &mut records.warnings,
                "package_publish",
                &path,
                &error.to_string(),
            ),
        }
    }
}

fn import_development_paths(
    sources: &SwiftExtensionSources,
    development_paths: &mut Vec<String>,
    extension_ids: &mut BTreeSet<String>,
    loaded_sources: &mut Vec<LoadedExtension>,
    imported: &mut BTreeSet<String>,
    warnings: &mut Vec<ExtensionMigrationWarning>,
) {
    let Some(Value::Array(paths)) = sources.preferences.get(DEVELOPMENT_PATHS_KEY) else {
        return;
    };
    for path in paths.iter().filter_map(Value::as_str) {
        let normalized = normalize_path(path);
        if !development_paths.contains(&normalized) {
            development_paths.push(normalized.clone());
            imported.insert(format!("preferences/{DEVELOPMENT_PATHS_KEY}"));
        }
        match load_extension(&normalized) {
            Ok(loaded) => {
                extension_ids.insert(loaded.id.clone());
                if !loaded_sources
                    .iter()
                    .any(|current| current.package_root == loaded.package_root)
                {
                    loaded_sources.push(loaded);
                }
            }
            Err(error) => warning(
                warnings,
                "invalid_development_package",
                Path::new(&normalized),
                &error.to_string(),
            ),
        }
    }
}

fn import_preferences(
    preferences: &Map<String, Value>,
    extension_ids: &BTreeSet<String>,
    loaded_sources: &[LoadedExtension],
    state: &mut super::state::ExtensionState,
    imported: &mut BTreeSet<String>,
    warnings: &mut Vec<ExtensionMigrationWarning>,
) {
    for (key, value) in preferences {
        if let Some(extension_id) = key.strip_prefix(ENABLED_PREFIX)
            && !extension_id.is_empty()
            && let Some(enabled) = value.as_bool()
            && let std::collections::btree_map::Entry::Vacant(entry) =
                state.enabled.entry(extension_id.to_owned())
        {
            entry.insert(enabled);
            imported.insert(format!("preferences/{key}"));
        }
    }
    for loaded in loaded_sources {
        if let Some(enabled) = loaded.legacy_enabled {
            state.enabled.entry(loaded.id.clone()).or_insert(enabled);
        }
    }
    let mut ids = extension_ids.iter().collect::<Vec<_>>();
    ids.sort_by_key(|id| std::cmp::Reverse(id.len()));
    for (storage_key, value) in preferences {
        if storage_key == DEVELOPMENT_PATHS_KEY || storage_key.starts_with(ENABLED_PREFIX) {
            continue;
        }
        let Some(suffix) = storage_key.strip_prefix(SETTINGS_PREFIX) else {
            continue;
        };
        let Some((extension_id, key)) = ids.iter().find_map(|extension_id| {
            suffix
                .strip_prefix(extension_id.as_str())?
                .strip_prefix('.')
                .filter(|key| !key.is_empty())
                .map(|key| ((*extension_id).clone(), key))
        }) else {
            continue;
        };
        if serde_json::to_vec(value).map_or(true, |encoded| encoded.len() > MAX_SETTING_VALUE_BYTES)
        {
            warning(
                warnings,
                "setting_too_large",
                Path::new(storage_key),
                "setting exceeds the Rust bridge limit",
            );
            continue;
        }
        let settings = state.settings.entry(extension_id.clone()).or_default();
        if !settings.contains_key(key) {
            settings.insert(key.to_owned(), value.clone());
            imported.insert(format!("preferences/{storage_key}"));
        }
    }
}

fn import_grants_and_shortcuts(
    sources: &SwiftExtensionSources,
    state: &mut super::state::ExtensionState,
    imported: &mut BTreeSet<String>,
    existing: &mut BTreeSet<String>,
    warnings: &mut Vec<ExtensionMigrationWarning>,
) {
    let grants_path = sources.app_support.join(LEGACY_GRANTS_FILE);
    if let Some(grants) = read_json::<Vec<ExtensionGrantRule>>(&grants_path, warnings) {
        for grant in grants {
            if state.grants.iter().any(|current| current.id == grant.id) {
                existing.insert(LEGACY_GRANTS_FILE.to_owned());
            } else {
                state.grants.push(grant);
                imported.insert(LEGACY_GRANTS_FILE.to_owned());
            }
        }
    }
    let shortcuts_path = sources.app_support.join(LEGACY_SHORTCUTS_FILE);
    if let Some(shortcuts) = read_json::<Vec<ExtensionShortcut>>(&shortcuts_path, warnings) {
        for shortcut in shortcuts {
            if state.shortcuts.iter().any(|current| {
                current.extension_id == shortcut.extension_id
                    && current.command_id == shortcut.command_id
            }) {
                existing.insert(LEGACY_SHORTCUTS_FILE.to_owned());
            } else {
                state.shortcuts.push(shortcut);
                imported.insert(LEGACY_SHORTCUTS_FILE.to_owned());
            }
        }
    }
}

fn import_mutable_files(
    paths: &ExtensionPaths,
    sources: &SwiftExtensionSources,
    extension_ids: &BTreeSet<String>,
    loaded_sources: &[LoadedExtension],
    imported: &mut BTreeSet<String>,
    existing: &mut BTreeSet<String>,
    warnings: &mut Vec<ExtensionMigrationWarning>,
) {
    for extension_id in extension_ids {
        let filename = format!(
            "{}.json",
            super::paths::safe_extension_filename(extension_id)
        );
        copy_bounded_if_absent(
            &sources
                .app_support
                .join(LEGACY_STORAGE_DIRECTORY)
                .join(&filename),
            &paths.storage_directory().join(&filename),
            super::storage::MAX_STORAGE_BYTES,
            &format!("{LEGACY_STORAGE_DIRECTORY}/{filename}"),
            imported,
            existing,
            warnings,
        );
    }
    copy_bounded_if_absent(
        &sources.app_support.join(LEGACY_AUDIT_FILE),
        &paths.audit_file(),
        MAX_EXTENSION_AUDIT_BYTES,
        LEGACY_AUDIT_FILE,
        imported,
        existing,
        warnings,
    );
    for loaded in loaded_sources {
        let source = loaded.resource_root.join("logs/output.log");
        let destination = paths.log_file(&loaded.id);
        copy_bounded_if_absent(
            &source,
            &destination,
            MAX_EXTENSION_LOG_BYTES,
            &format!("logs/{}.log", loaded.id),
            imported,
            existing,
            warnings,
        );
    }
}

fn copy_bounded_if_absent(
    source: &Path,
    destination: &Path,
    maximum: usize,
    label: &str,
    imported: &mut BTreeSet<String>,
    existing: &mut BTreeSet<String>,
    warnings: &mut Vec<ExtensionMigrationWarning>,
) {
    if destination.exists() {
        existing.insert(label.to_owned());
        return;
    }
    let metadata = match std::fs::symlink_metadata(source) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata,
        Ok(_) => {
            warning(
                warnings,
                "unsafe_mutable_file",
                source,
                "source is not a regular file",
            );
            return;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            warning(warnings, "mutable_file_read", source, &error.to_string());
            return;
        }
    };
    let mut contents = match std::fs::read(source) {
        Ok(contents) => contents,
        Err(error) => {
            warning(warnings, "mutable_file_read", source, &error.to_string());
            return;
        }
    };
    if metadata.len() as usize > maximum {
        let start = contents.len().saturating_sub(maximum);
        let start = contents[start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|offset| start + offset + 1)
            .unwrap_or(start);
        contents = contents[start..].to_vec();
    }
    match write_private(destination, &contents) {
        Ok(()) => {
            imported.insert(label.to_owned());
        }
        Err(error) => warning(
            warnings,
            "mutable_file_write",
            destination,
            &error.to_string(),
        ),
    }
}

fn read_json<T: serde::de::DeserializeOwned>(
    path: &Path,
    warnings: &mut Vec<ExtensionMigrationWarning>,
) -> Option<T> {
    let contents = match std::fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            warning(warnings, "json_read", path, &error.to_string());
            return None;
        }
    };
    match serde_json::from_slice(&contents) {
        Ok(value) => Some(value),
        Err(error) => {
            warning(warnings, "json_invalid", path, &error.to_string());
            None
        }
    }
}

fn warning(
    warnings: &mut Vec<ExtensionMigrationWarning>,
    category: &str,
    path: &Path,
    message: &str,
) {
    warnings.push(ExtensionMigrationWarning {
        category: category.to_owned(),
        path: path.to_string_lossy().into_owned(),
        message: message.to_owned(),
    });
}

fn normalize_path(path: &str) -> String {
    let path = Path::new(path);
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized.to_string_lossy().into_owned()
}

fn acquire_lock(paths: &ExtensionPaths) -> Result<MigrationLock, ExtensionMigrationError> {
    let path = paths.migration_lock_file();
    let result = OpenOptions::new().write(true).create_new(true).open(&path);
    match result {
        Ok(_) => Ok(MigrationLock { path }),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(ExtensionMigrationError::Locked)
        }
        Err(error) => Err(ExtensionMigrationError::Prepare(error)),
    }
}

fn read_marker(
    paths: &ExtensionPaths,
) -> Result<Option<ExtensionMigrationState>, ExtensionMigrationError> {
    let contents = match std::fs::read(paths.migration_file()) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ExtensionMigrationError::MarkerRead(error)),
    };
    let marker: ExtensionMigrationState =
        serde_json::from_slice(&contents).map_err(ExtensionMigrationError::MarkerInvalid)?;
    if marker.schema_version != EXTENSION_MIGRATION_SCHEMA_VERSION {
        return Err(ExtensionMigrationError::UnsupportedSchema(
            marker.schema_version,
        ));
    }
    Ok(Some(marker))
}

fn write_marker(
    paths: &ExtensionPaths,
    marker: &ExtensionMigrationState,
) -> Result<(), ExtensionMigrationError> {
    let contents = serde_json::to_vec_pretty(marker)
        .map_err(|error| ExtensionMigrationError::MarkerWrite(std::io::Error::other(error)))?;
    write_private(&paths.migration_file(), &contents).map_err(ExtensionMigrationError::MarkerWrite)
}

#[cfg(target_os = "macos")]
fn read_swift_extension_defaults() -> Result<Map<String, Value>, String> {
    use objc2_foundation::{NSJSONSerialization, NSJSONWritingOptions, NSString, NSUserDefaults};

    let defaults = NSUserDefaults::standardUserDefaults();
    let domain_name = NSString::from_str("com.muxy.app");
    let Some(domain) = defaults.persistentDomainForName(&domain_name) else {
        return Ok(Map::new());
    };
    let (keys, values) = domain.to_vecs();
    let mut result = Map::new();
    for (key, value) in keys.into_iter().zip(values) {
        let Ok(key) = key.downcast::<NSString>() else {
            continue;
        };
        let data = unsafe {
            NSJSONSerialization::dataWithJSONObject_options_error(
                &value,
                NSJSONWritingOptions::FragmentsAllowed,
            )
        };
        let Ok(data) = data else {
            continue;
        };
        if let Ok(value) = serde_json::from_slice(&data.to_vec()) {
            result.insert(key.to_string(), value);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::{BuildMode, RuntimePathPolicy};
    use serde_json::json;

    fn paths(root: &Path) -> ExtensionPaths {
        ExtensionPaths::new(RuntimePathPolicy::new(BuildMode::Production), root)
    }

    fn package(root: &Path, version: &str, enabled: Option<bool>) {
        std::fs::create_dir_all(root.join("logs")).unwrap();
        let enabled = enabled
            .map(|enabled| format!(r#","enabled":{enabled}"#))
            .unwrap_or_default();
        std::fs::write(
            root.join("package.json"),
            format!(r#"{{"name":"git","version":"{version}","muxy":{{}}{enabled}}}"#),
        )
        .unwrap();
        std::fs::write(root.join("logs/output.log"), b"legacy log\n").unwrap();
    }

    fn sources(root: &Path) -> SwiftExtensionSources {
        SwiftExtensionSources {
            packages: root.join("legacy-packages"),
            app_support: root.join("legacy-support"),
            preferences: json!({
                "muxy.ext.devPaths": [],
                "muxy.ext.enabled.git": false,
                "muxy.ext.git.branch": "main"
            })
            .as_object()
            .unwrap()
            .clone(),
        }
    }

    #[test]
    fn migration_imports_all_swift_extension_state_once() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(&directory.path().join("rust"));
        let sources = sources(directory.path());
        package(&sources.packages.join("git"), "1.16.0", Some(true));
        std::fs::create_dir_all(sources.app_support.join(LEGACY_STORAGE_DIRECTORY)).unwrap();
        let filename = format!(
            "{}.json",
            super::super::paths::safe_extension_filename("git")
        );
        std::fs::write(
            sources
                .app_support
                .join(LEGACY_STORAGE_DIRECTORY)
                .join(filename),
            br#"{"token":"value"}"#,
        )
        .unwrap();
        std::fs::write(
            sources.app_support.join(LEGACY_GRANTS_FILE),
            br#"[{"id":"rule","extensionID":"git","verb":"git.write","match":{"kind":"any"},"decision":"allow","createdAt":"2026-09-02T12:00:00Z"}]"#,
        )
        .unwrap();
        std::fs::write(
            sources.app_support.join(LEGACY_SHORTCUTS_FILE),
            br#"[{"extensionID":"git","commandID":"open","combo":{"key":"g","modifiers":1}}]"#,
        )
        .unwrap();
        std::fs::write(sources.app_support.join(LEGACY_AUDIT_FILE), b"audit\n").unwrap();

        let first = run(&paths, &sources).unwrap();
        assert_eq!(first.outcome, ExtensionMigrationOutcome::Completed);
        assert!(paths.packages.join("git/package.json").exists());
        let store = ExtensionStateStore::open(paths.state_file()).unwrap();
        assert_eq!(store.enabled_override("git"), Some(false));
        assert_eq!(store.setting("git", "branch"), Some(&json!("main")));
        assert_eq!(store.state().grants.len(), 1);
        assert_eq!(store.state().shortcuts.len(), 1);
        assert_eq!(
            std::fs::read(paths.log_file("git")).unwrap(),
            b"legacy log\n"
        );
        assert_eq!(std::fs::read(paths.audit_file()).unwrap(), b"audit\n");

        std::fs::write(sources.app_support.join(LEGACY_AUDIT_FILE), b"changed\n").unwrap();
        let second = run(&paths, &sources).unwrap();
        assert_eq!(second, first);
        assert_eq!(std::fs::read(paths.audit_file()).unwrap(), b"audit\n");
    }

    #[test]
    fn migration_preserves_newer_rust_values_and_packages() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(&directory.path().join("rust"));
        let sources = sources(directory.path());
        package(&sources.packages.join("git"), "1.0.0", Some(true));
        package(&paths.packages.join("git"), "2.0.0", None);
        let mut store = ExtensionStateStore::open(paths.state_file()).unwrap();
        store.set_enabled("git", Some(true)).unwrap();
        store
            .set_setting("git", "branch", Some(json!("rust")))
            .unwrap();

        run(&paths, &sources).unwrap();
        let loaded = load_extension(paths.packages.join("git")).unwrap();
        assert_eq!(loaded.manifest.version, "2.0.0");
        let store = ExtensionStateStore::open(paths.state_file()).unwrap();
        assert_eq!(store.enabled_override("git"), Some(true));
        assert_eq!(store.setting("git", "branch"), Some(&json!("rust")));
    }

    #[test]
    fn invalid_packages_create_visible_warnings_without_mutation() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(&directory.path().join("rust"));
        let sources = sources(directory.path());
        std::fs::create_dir_all(sources.packages.join("broken")).unwrap();
        std::fs::write(sources.packages.join("broken/package.json"), b"not json").unwrap();
        let marker = run(&paths, &sources).unwrap();
        assert_eq!(marker.warnings.len(), 1);
        assert!(!paths.packages.join("broken").exists());
        let state = ExtensionStateStore::open(paths.state_file()).unwrap();
        assert_eq!(state.state().migration_warnings, marker.warnings);
    }

    #[test]
    fn missing_sources_are_terminal_and_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(&directory.path().join("rust"));
        let sources = SwiftExtensionSources {
            packages: directory.path().join("missing-packages"),
            app_support: directory.path().join("missing-support"),
            preferences: Map::new(),
        };
        let first = run(&paths, &sources).unwrap();
        assert_eq!(first.outcome, ExtensionMigrationOutcome::SourceMissing);
        let second = run(&paths, &sources).unwrap();
        assert_eq!(second, first);
    }
}
