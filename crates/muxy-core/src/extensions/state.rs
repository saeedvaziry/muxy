use crate::store::write_private;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const STATE_SCHEMA_VERSION: u32 = 1;
pub const MAX_SETTING_VALUE_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtensionGrantDecision {
    Allow,
    Deny,
    Blocked,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum ExtensionGatedVerb {
    #[serde(rename = "exec")]
    Exec,
    #[serde(rename = "panes.send")]
    PanesSend,
    #[serde(rename = "panes.sendKeys")]
    PanesSendKeys,
    #[serde(rename = "panes.readScreen")]
    PanesReadScreen,
    #[serde(rename = "tabs.openForeign")]
    TabsOpenForeign,
    #[serde(rename = "tabs.runCommand")]
    TabsRunCommand,
    #[serde(rename = "remote.invoke")]
    RemoteInvoke,
    #[serde(rename = "git.write")]
    GitWrite,
    #[serde(rename = "files.write")]
    FilesWrite,
    #[serde(rename = "http.fetch")]
    HttpFetch,
    #[serde(rename = "projects.delete")]
    ProjectsDelete,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind")]
pub enum ExtensionGrantMatch {
    #[serde(rename = "any")]
    Any,
    #[serde(rename = "argvExact")]
    ArgvExact { value: Vec<String> },
    #[serde(rename = "argvPrefix")]
    ArgvPrefix { value: Vec<String> },
    #[serde(rename = "shellExact")]
    ShellExact { string: String },
    #[serde(rename = "paneEquals")]
    PaneEquals { string: String },
    #[serde(rename = "foreignTabEquals")]
    ForeignTabEquals { target: String, string: String },
    #[serde(rename = "remoteActionEquals")]
    RemoteActionEquals { string: String },
    #[serde(rename = "gitOperationEquals")]
    GitOperationEquals { string: String },
    #[serde(rename = "fileOperationEquals")]
    FileOperationEquals { string: String },
    #[serde(rename = "hostEquals")]
    HostEquals { string: String },
    #[serde(rename = "projectNameEquals")]
    ProjectNameEquals { string: String },
}

impl ExtensionGrantMatch {
    pub fn specificity(&self) -> usize {
        match self {
            Self::Any => 0,
            Self::ArgvPrefix { value } => 50 + value.len(),
            Self::ShellExact { .. } | Self::PaneEquals { .. } => 100,
            Self::HostEquals { .. } => 110,
            Self::RemoteActionEquals { .. } => 120,
            Self::GitOperationEquals { .. } => 130,
            Self::FileOperationEquals { .. } => 135,
            Self::ProjectNameEquals { .. } => 140,
            Self::ForeignTabEquals { .. } => 150,
            Self::ArgvExact { value } => 200 + value.len(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionGrantRule {
    pub id: String,
    #[serde(rename = "extensionID")]
    pub extension_id: String,
    pub verb: ExtensionGatedVerb,
    #[serde(rename = "match")]
    pub match_rule: ExtensionGrantMatch,
    pub decision: ExtensionGrantDecision,
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtensionShortcutSource {
    #[default]
    Manifest,
    Runtime,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionKeyCombo {
    pub key: String,
    pub modifiers: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionShortcut {
    #[serde(rename = "extensionID")]
    pub extension_id: String,
    #[serde(rename = "commandID")]
    pub command_id: String,
    pub combo: ExtensionKeyCombo,
    #[serde(default)]
    pub source: ExtensionShortcutSource,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExtensionMigrationWarning {
    pub category: String,
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ExtensionState {
    pub schema_version: u32,
    #[serde(default)]
    pub enabled: BTreeMap<String, bool>,
    #[serde(default)]
    pub settings: BTreeMap<String, BTreeMap<String, Value>>,
    #[serde(default)]
    pub development_paths: Vec<String>,
    #[serde(default)]
    pub grants: Vec<ExtensionGrantRule>,
    #[serde(default)]
    pub shortcuts: Vec<ExtensionShortcut>,
    #[serde(default)]
    pub migration_warnings: Vec<ExtensionMigrationWarning>,
}

impl Default for ExtensionState {
    fn default() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            enabled: BTreeMap::new(),
            settings: BTreeMap::new(),
            development_paths: Vec::new(),
            grants: Vec::new(),
            shortcuts: Vec::new(),
            migration_warnings: Vec::new(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ExtensionStateError {
    #[error("failed to read extension state: {0}")]
    Read(#[source] std::io::Error),
    #[error("extension state is invalid: {0}")]
    Invalid(#[source] serde_json::Error),
    #[error("unsupported extension state schema {0}")]
    UnsupportedSchema(u32),
    #[error("failed to encode extension state: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("failed to write extension state: {0}")]
    Write(#[source] std::io::Error),
    #[error("setting value exceeds {MAX_SETTING_VALUE_BYTES} bytes")]
    SettingValueTooLarge,
}

#[derive(Clone, Debug)]
pub struct ExtensionStateStore {
    path: PathBuf,
    state: ExtensionState,
}

impl ExtensionStateStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ExtensionStateError> {
        let path = path.as_ref().to_path_buf();
        let state = match std::fs::read(&path) {
            Ok(contents) => serde_json::from_slice::<ExtensionState>(&contents)
                .map_err(ExtensionStateError::Invalid)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => ExtensionState::default(),
            Err(error) => return Err(ExtensionStateError::Read(error)),
        };
        if state.schema_version != STATE_SCHEMA_VERSION {
            return Err(ExtensionStateError::UnsupportedSchema(state.schema_version));
        }
        Ok(Self { path, state })
    }

    pub fn state(&self) -> &ExtensionState {
        &self.state
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn replace(&mut self, state: ExtensionState) -> Result<(), ExtensionStateError> {
        if state.schema_version != STATE_SCHEMA_VERSION {
            return Err(ExtensionStateError::UnsupportedSchema(state.schema_version));
        }
        self.persist(&state)?;
        self.state = state;
        Ok(())
    }

    pub fn enabled_override(&self, extension_id: &str) -> Option<bool> {
        self.state.enabled.get(extension_id).copied()
    }

    pub fn set_enabled(
        &mut self,
        extension_id: impl Into<String>,
        enabled: Option<bool>,
    ) -> Result<(), ExtensionStateError> {
        let extension_id = extension_id.into();
        self.update(|state| match enabled {
            Some(enabled) => {
                state.enabled.insert(extension_id, enabled);
            }
            None => {
                state.enabled.remove(&extension_id);
            }
        })
    }

    pub fn setting(&self, extension_id: &str, key: &str) -> Option<&Value> {
        self.state.settings.get(extension_id)?.get(key)
    }

    pub fn set_setting(
        &mut self,
        extension_id: impl Into<String>,
        key: impl Into<String>,
        value: Option<Value>,
    ) -> Result<(), ExtensionStateError> {
        if let Some(value) = value.as_ref()
            && serde_json::to_vec(value)
                .map_err(ExtensionStateError::Encode)?
                .len()
                > MAX_SETTING_VALUE_BYTES
        {
            return Err(ExtensionStateError::SettingValueTooLarge);
        }
        let extension_id = extension_id.into();
        let key = key.into();
        self.update(|state| match value {
            Some(value) => {
                state
                    .settings
                    .entry(extension_id)
                    .or_default()
                    .insert(key, value);
            }
            None => {
                if let Some(settings) = state.settings.get_mut(&extension_id) {
                    settings.remove(&key);
                    if settings.is_empty() {
                        state.settings.remove(&extension_id);
                    }
                }
            }
        })
    }

    pub fn set_development_paths(
        &mut self,
        development_paths: Vec<String>,
    ) -> Result<(), ExtensionStateError> {
        self.update(|state| state.development_paths = development_paths)
    }

    pub fn set_grants(
        &mut self,
        grants: Vec<ExtensionGrantRule>,
    ) -> Result<(), ExtensionStateError> {
        self.update(|state| state.grants = grants)
    }

    pub fn set_shortcuts(
        &mut self,
        shortcuts: Vec<ExtensionShortcut>,
    ) -> Result<(), ExtensionStateError> {
        self.update(|state| state.shortcuts = shortcuts)
    }

    pub fn clear_extension(&mut self, extension_id: &str) -> Result<(), ExtensionStateError> {
        self.update(|state| {
            state.enabled.remove(extension_id);
            state.settings.remove(extension_id);
            state
                .grants
                .retain(|rule| rule.extension_id != extension_id);
            state
                .shortcuts
                .retain(|shortcut| shortcut.extension_id != extension_id);
        })
    }

    fn update(
        &mut self,
        edit: impl FnOnce(&mut ExtensionState),
    ) -> Result<(), ExtensionStateError> {
        let mut state = self.state.clone();
        edit(&mut state);
        self.persist(&state)?;
        self.state = state;
        Ok(())
    }

    fn persist(&self, state: &ExtensionState) -> Result<(), ExtensionStateError> {
        let contents = serde_json::to_vec_pretty(state).map_err(ExtensionStateError::Encode)?;
        write_private(&self.path, &contents).map_err(ExtensionStateError::Write)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn state_updates_are_atomic_and_preserve_absent_overrides() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let mut store = ExtensionStateStore::open(&path).unwrap();
        assert_eq!(store.enabled_override("git"), None);
        store.set_enabled("git", Some(true)).unwrap();
        store
            .set_setting("git", "branch", Some(json!("main")))
            .unwrap();
        let loaded = ExtensionStateStore::open(&path).unwrap();
        assert_eq!(loaded.enabled_override("git"), Some(true));
        assert_eq!(loaded.setting("git", "branch"), Some(&json!("main")));
        store.set_enabled("git", None).unwrap();
        assert_eq!(store.enabled_override("git"), None);
    }

    #[test]
    fn malformed_or_future_state_is_never_replaced_during_open() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        std::fs::write(&path, b"not json").unwrap();
        assert!(matches!(
            ExtensionStateStore::open(&path),
            Err(ExtensionStateError::Invalid(_))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"not json");
        std::fs::write(&path, br#"{"schema_version":2}"#).unwrap();
        assert!(matches!(
            ExtensionStateStore::open(&path),
            Err(ExtensionStateError::UnsupportedSchema(2))
        ));
    }

    #[test]
    fn swift_grant_and_shortcut_shapes_round_trip() {
        let grant = json!({
            "id": "A4F9D93A-2BA7-4787-B83D-A87C006E7267",
            "extensionID": "git",
            "verb": "git.write",
            "match": { "kind": "gitOperationEquals", "string": "commit" },
            "decision": "allow",
            "createdAt": "2026-09-02T12:00:00Z"
        });
        let decoded: ExtensionGrantRule = serde_json::from_value(grant.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), grant);
        let shortcut = json!({
            "extensionID": "git",
            "commandID": "open",
            "combo": { "key": "g", "modifiers": 1048576 },
            "source": "manifest"
        });
        let decoded: ExtensionShortcut = serde_json::from_value(shortcut.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), shortcut);
    }

    #[test]
    fn setting_values_enforce_the_swift_socket_limit() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = ExtensionStateStore::open(directory.path().join("state.json")).unwrap();
        let value = Value::String("x".repeat(MAX_SETTING_VALUE_BYTES));
        assert!(matches!(
            store.set_setting("git", "large", Some(value)),
            Err(ExtensionStateError::SettingValueTooLarge)
        ));
        assert!(store.setting("git", "large").is_none());
    }

    #[test]
    fn grant_specificity_matches_swift_ordering() {
        assert_eq!(ExtensionGrantMatch::Any.specificity(), 0);
        assert_eq!(
            ExtensionGrantMatch::ArgvPrefix {
                value: vec!["git".to_owned(), "status".to_owned()]
            }
            .specificity(),
            52
        );
        assert_eq!(
            ExtensionGrantMatch::ArgvExact {
                value: vec!["git".to_owned()]
            }
            .specificity(),
            201
        );
    }
}
