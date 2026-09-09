use super::contract::required_event_permission;
use super::loader::{LoadedExtension, load_extension};
use super::manifest::ExtensionPermission;
use super::paths::ExtensionPaths;
use super::state::{ExtensionGrantRule, ExtensionStateError, ExtensionStateStore};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use thiserror::Error;

pub const MAX_HOST_RESTART_ATTEMPTS: u8 = 5;
pub const HOST_RESTART_BACKOFF: Duration = Duration::from_secs(1);
pub const HOST_STABILITY_WINDOW: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtensionRuntimeStatus {
    Disabled,
    Ready,
    Loading,
    Running,
    Failed,
    Crashed,
    Updating,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExtensionSource {
    Installed,
    Development(PathBuf),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExtensionRuntimeRecord {
    pub extension: LoadedExtension,
    pub source: ExtensionSource,
    pub enabled: bool,
    pub status: ExtensionRuntimeStatus,
    pub last_error: Option<String>,
    token: Option<String>,
}

impl ExtensionRuntimeRecord {
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    pub fn has_background_host(&self) -> bool {
        self.extension.background_script_path().is_some()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionLoadFailure {
    pub path: PathBuf,
    pub development_path: Option<PathBuf>,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeSubscriptionAccess {
    Allowed,
    Denied(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionRuntimeSnapshotEntry {
    pub token: String,
    pub granted_permissions: BTreeSet<String>,
    pub subscription_access: BTreeMap<String, RuntimeSubscriptionAccess>,
    pub can_write_notifications: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExtensionRuntimeSnapshot {
    pub entries: BTreeMap<String, ExtensionRuntimeSnapshotEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionHostLaunch {
    pub extension_id: String,
    pub token: String,
    pub background_script: PathBuf,
    pub working_directory: PathBuf,
}

#[derive(Debug, Error)]
pub enum ExtensionRuntimeError {
    #[error("failed to prepare extension packages: {0}")]
    Packages(#[source] std::io::Error),
    #[error("failed to generate an extension authentication token: {0}")]
    Token(#[source] std::io::Error),
    #[error("failed to update extension state: {0}")]
    State(#[from] ExtensionStateError),
    #[error("unknown extension '{0}'")]
    UnknownExtension(String),
    #[error("setting '{key}' is not declared by extension '{extension_id}'")]
    UnknownSetting { extension_id: String, key: String },
}

#[derive(Clone, Debug)]
pub struct ExtensionRuntimeCatalog {
    paths: ExtensionPaths,
    state: ExtensionStateStore,
    records: BTreeMap<String, ExtensionRuntimeRecord>,
    failures: Vec<ExtensionLoadFailure>,
}

impl ExtensionRuntimeCatalog {
    pub fn load(paths: ExtensionPaths) -> Result<Self, ExtensionRuntimeError> {
        let state = ExtensionStateStore::open(paths.state_file())?;
        let mut catalog = Self {
            paths,
            state,
            records: BTreeMap::new(),
            failures: Vec::new(),
        };
        catalog.scan()?;
        Ok(catalog)
    }

    pub fn paths(&self) -> &ExtensionPaths {
        &self.paths
    }

    pub fn records(&self) -> &BTreeMap<String, ExtensionRuntimeRecord> {
        &self.records
    }

    pub fn failures(&self) -> &[ExtensionLoadFailure] {
        &self.failures
    }

    pub fn grants(&self) -> &[ExtensionGrantRule] {
        &self.state.state().grants
    }

    pub fn set_grants(
        &mut self,
        grants: Vec<ExtensionGrantRule>,
    ) -> Result<(), ExtensionRuntimeError> {
        self.state.set_grants(grants)?;
        Ok(())
    }

    pub fn effective_setting(
        &self,
        extension_id: &str,
        key: &str,
    ) -> Result<Option<Value>, ExtensionRuntimeError> {
        let record = self
            .records
            .get(extension_id)
            .ok_or_else(|| ExtensionRuntimeError::UnknownExtension(extension_id.to_owned()))?;
        let entry = record.extension.manifest.setting(key).ok_or_else(|| {
            ExtensionRuntimeError::UnknownSetting {
                extension_id: extension_id.to_owned(),
                key: key.to_owned(),
            }
        })?;
        Ok(self
            .state
            .setting(extension_id, key)
            .cloned()
            .or_else(|| entry.default_value.clone()))
    }

    pub fn set_setting(
        &mut self,
        extension_id: &str,
        key: &str,
        value: Value,
    ) -> Result<(), ExtensionRuntimeError> {
        let record = self
            .records
            .get(extension_id)
            .ok_or_else(|| ExtensionRuntimeError::UnknownExtension(extension_id.to_owned()))?;
        if record.extension.manifest.setting(key).is_none() {
            return Err(ExtensionRuntimeError::UnknownSetting {
                extension_id: extension_id.to_owned(),
                key: key.to_owned(),
            });
        }
        self.state
            .set_setting(extension_id.to_owned(), key.to_owned(), Some(value))?;
        Ok(())
    }

    pub fn snapshot(&self) -> ExtensionRuntimeSnapshot {
        let entries = self
            .records
            .iter()
            .filter(|(_, record)| record.enabled)
            .filter_map(|(extension_id, record)| {
                let token = record.token.clone()?;
                let granted_permissions = record
                    .extension
                    .manifest
                    .permissions
                    .iter()
                    .map(|permission| permission.as_str().to_owned())
                    .collect::<BTreeSet<_>>();
                let mut subscription_access = record
                    .extension
                    .manifest
                    .events
                    .iter()
                    .map(|event| {
                        let access = match required_event_permission(event) {
                            Some(permission)
                                if !record.extension.manifest.permissions.contains(&permission) =>
                            {
                                RuntimeSubscriptionAccess::Denied(format!(
                                    "permission denied ({})",
                                    permission.as_str()
                                ))
                            }
                            _ => RuntimeSubscriptionAccess::Allowed,
                        };
                        (event.clone(), access)
                    })
                    .collect::<BTreeMap<_, _>>();
                subscription_access.extend(
                    record
                        .extension
                        .manifest
                        .commands
                        .iter()
                        .map(|command| (command.event_name(), RuntimeSubscriptionAccess::Allowed)),
                );
                Some((
                    extension_id.clone(),
                    ExtensionRuntimeSnapshotEntry {
                        token,
                        can_write_notifications: record
                            .extension
                            .manifest
                            .permissions
                            .contains(&ExtensionPermission::NotificationsWrite),
                        granted_permissions,
                        subscription_access,
                    },
                ))
            })
            .collect();
        ExtensionRuntimeSnapshot { entries }
    }

    pub fn host_launches(&self) -> Vec<ExtensionHostLaunch> {
        self.records
            .values()
            .filter(|record| record.enabled)
            .filter_map(|record| {
                Some(ExtensionHostLaunch {
                    extension_id: record.extension.id.clone(),
                    token: record.token.clone()?,
                    background_script: record.extension.background_script_path()?,
                    working_directory: record.extension.resource_root.clone(),
                })
            })
            .collect()
    }

    pub fn set_enabled(
        &mut self,
        extension_id: &str,
        enabled: bool,
    ) -> Result<(), ExtensionRuntimeError> {
        if !self.records.contains_key(extension_id) {
            return Err(ExtensionRuntimeError::UnknownExtension(
                extension_id.to_owned(),
            ));
        }
        self.state
            .set_enabled(extension_id.to_owned(), Some(enabled))?;
        let record = self.records.get_mut(extension_id).unwrap();
        record.enabled = enabled;
        record.last_error = None;
        if enabled {
            record.token = Some(generate_token().map_err(ExtensionRuntimeError::Token)?);
            record.status = if record.has_background_host() {
                ExtensionRuntimeStatus::Loading
            } else {
                ExtensionRuntimeStatus::Ready
            };
        } else {
            record.token = None;
            record.status = ExtensionRuntimeStatus::Disabled;
        }
        Ok(())
    }

    pub fn set_status(
        &mut self,
        extension_id: &str,
        status: ExtensionRuntimeStatus,
        error: Option<String>,
    ) -> Result<(), ExtensionRuntimeError> {
        let record = self
            .records
            .get_mut(extension_id)
            .ok_or_else(|| ExtensionRuntimeError::UnknownExtension(extension_id.to_owned()))?;
        record.status = status;
        record.last_error = error;
        Ok(())
    }

    pub fn rescan(&mut self) -> Result<(), ExtensionRuntimeError> {
        self.state = ExtensionStateStore::open(self.paths.state_file())?;
        self.scan()
    }

    fn scan(&mut self) -> Result<(), ExtensionRuntimeError> {
        std::fs::create_dir_all(&self.paths.packages).map_err(ExtensionRuntimeError::Packages)?;
        self.records.clear();
        self.failures.clear();
        let mut installed = std::fs::read_dir(&self.paths.packages)
            .map_err(ExtensionRuntimeError::Packages)?
            .filter_map(Result::ok)
            .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
            .collect::<Vec<_>>();
        installed.sort_by_key(std::fs::DirEntry::file_name);
        for entry in installed {
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path);
            if !metadata
                .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
            {
                continue;
            }
            self.load_one(path, ExtensionSource::Installed, true)?;
        }
        let development_paths = self.state.state().development_paths.clone();
        for path in development_paths {
            let path = PathBuf::from(path);
            self.load_one(path.clone(), ExtensionSource::Development(path), false)?;
        }
        Ok(())
    }

    fn load_one(
        &mut self,
        path: PathBuf,
        source: ExtensionSource,
        enforce_directory_name: bool,
    ) -> Result<(), ExtensionRuntimeError> {
        let development_path = match &source {
            ExtensionSource::Installed => None,
            ExtensionSource::Development(path) => Some(path.clone()),
        };
        let extension = match load_extension(&path) {
            Ok(extension) => extension,
            Err(error) => {
                self.failures.push(ExtensionLoadFailure {
                    path,
                    development_path,
                    message: error.to_string(),
                });
                return Ok(());
            }
        };
        if enforce_directory_name
            && path.file_name().and_then(|name| name.to_str()) != Some(extension.id.as_str())
        {
            self.failures.push(ExtensionLoadFailure {
                path,
                development_path,
                message: format!(
                    "Extension name '{}' does not match its directory name",
                    extension.id
                ),
            });
            return Ok(());
        }
        if self.records.contains_key(&extension.id) {
            self.failures.push(ExtensionLoadFailure {
                path,
                development_path,
                message: format!("Duplicate extension name '{}'", extension.id),
            });
            return Ok(());
        }
        let enabled = self.state.enabled_override(&extension.id).unwrap_or(false);
        let token = enabled
            .then(generate_token)
            .transpose()
            .map_err(ExtensionRuntimeError::Token)?;
        let status = if !enabled {
            ExtensionRuntimeStatus::Disabled
        } else if extension.background_script_path().is_some() {
            ExtensionRuntimeStatus::Loading
        } else {
            ExtensionRuntimeStatus::Ready
        };
        self.records.insert(
            extension.id.clone(),
            ExtensionRuntimeRecord {
                extension,
                source,
                enabled,
                status,
                last_error: None,
                token,
            },
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostTermination {
    Intentional,
    Exited(i32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostLifecycleAction {
    None,
    RestartAfter(Duration),
}

#[derive(Clone, Debug)]
pub struct ExtensionHostLifecycle {
    pub status: ExtensionRuntimeStatus,
    pub restart_attempts: u8,
    generation: u64,
    running_since: Option<Instant>,
    restart_due: Option<Instant>,
}

impl Default for ExtensionHostLifecycle {
    fn default() -> Self {
        Self {
            status: ExtensionRuntimeStatus::Ready,
            restart_attempts: 0,
            generation: 0,
            running_since: None,
            restart_due: None,
        }
    }
}

impl ExtensionHostLifecycle {
    pub fn request_start(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1).max(1);
        self.status = ExtensionRuntimeStatus::Loading;
        self.running_since = None;
        self.restart_due = None;
        self.generation
    }

    pub fn started(&mut self, generation: u64, now: Instant) -> bool {
        if generation != self.generation {
            return false;
        }
        self.status = ExtensionRuntimeStatus::Running;
        self.running_since = Some(now);
        self.restart_due = None;
        true
    }

    pub fn start_failed(&mut self, generation: u64) -> bool {
        if generation != self.generation {
            return false;
        }
        self.status = ExtensionRuntimeStatus::Failed;
        self.running_since = None;
        self.restart_due = None;
        true
    }

    pub fn terminated(
        &mut self,
        generation: u64,
        termination: HostTermination,
        enabled: bool,
        now: Instant,
    ) -> HostLifecycleAction {
        if generation != self.generation {
            return HostLifecycleAction::None;
        }
        if self
            .running_since
            .is_some_and(|started| now.saturating_duration_since(started) >= HOST_STABILITY_WINDOW)
        {
            self.restart_attempts = 0;
        }
        self.running_since = None;
        self.restart_due = None;
        if !enabled || termination == HostTermination::Intentional {
            self.status = if enabled {
                ExtensionRuntimeStatus::Ready
            } else {
                ExtensionRuntimeStatus::Disabled
            };
            self.restart_attempts = 0;
            return HostLifecycleAction::None;
        }
        if termination == HostTermination::Exited(0) {
            self.status = ExtensionRuntimeStatus::Ready;
            return HostLifecycleAction::None;
        }
        if self.restart_attempts >= MAX_HOST_RESTART_ATTEMPTS {
            self.status = ExtensionRuntimeStatus::Failed;
            return HostLifecycleAction::None;
        }
        self.restart_attempts += 1;
        self.status = ExtensionRuntimeStatus::Crashed;
        self.restart_due = Some(now + HOST_RESTART_BACKOFF);
        HostLifecycleAction::RestartAfter(HOST_RESTART_BACKOFF)
    }

    pub fn restart_is_due(&self, now: Instant, enabled: bool) -> bool {
        enabled && self.restart_due.is_some_and(|due| now >= due)
    }

    pub fn disable(&mut self) {
        self.generation = self.generation.wrapping_add(1).max(1);
        self.status = ExtensionRuntimeStatus::Disabled;
        self.restart_attempts = 0;
        self.running_since = None;
        self.restart_due = None;
    }
}

fn generate_token() -> std::io::Result<String> {
    let mut bytes = [0_u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::{BuildMode, RuntimePathPolicy};
    use serde_json::json;
    use std::path::Path;

    fn paths(root: &Path) -> ExtensionPaths {
        ExtensionPaths::new(RuntimePathPolicy::new(BuildMode::Production), root)
    }

    fn package(root: &Path, id: &str, background: bool, events: &[&str], permissions: &[&str]) {
        std::fs::create_dir_all(root).unwrap();
        if background {
            std::fs::write(root.join("background.js"), b"muxy.log('started')").unwrap();
        }
        let mut body = json!({
            "events": events,
            "permissions": permissions
        });
        if background {
            body["background"] = json!("background.js");
        }
        std::fs::write(
            root.join("package.json"),
            serde_json::to_vec(&json!({
                "name": id,
                "version": "1.0.0",
                "muxy": body
            }))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn installed_extensions_win_over_duplicate_development_paths() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        package(&paths.packages.join("git"), "git", false, &[], &[]);
        let development = directory.path().join("development");
        package(&development, "git", false, &[], &[]);
        let mut state = ExtensionStateStore::open(paths.state_file()).unwrap();
        state
            .set_development_paths(vec![development.to_string_lossy().into_owned()])
            .unwrap();
        let catalog = ExtensionRuntimeCatalog::load(paths).unwrap();
        assert_eq!(catalog.records().len(), 1);
        assert_eq!(catalog.failures().len(), 1);
        assert_eq!(catalog.records()["git"].source, ExtensionSource::Installed);
    }

    #[test]
    fn snapshot_precedes_hosts_and_enforces_event_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        package(
            &paths.packages.join("git"),
            "git",
            true,
            &["agent.status", "theme.changed"],
            &["notifications:write"],
        );
        let mut state = ExtensionStateStore::open(paths.state_file()).unwrap();
        state.set_enabled("git", Some(true)).unwrap();
        let catalog = ExtensionRuntimeCatalog::load(paths).unwrap();
        let snapshot = catalog.snapshot();
        let launch = catalog.host_launches().pop().unwrap();
        let entry = &snapshot.entries["git"];
        assert_eq!(entry.token, launch.token);
        assert_eq!(entry.token.len(), 64);
        assert!(entry.can_write_notifications);
        assert_eq!(
            entry.subscription_access["agent.status"],
            RuntimeSubscriptionAccess::Denied("permission denied (agents:read)".to_owned())
        );
        assert_eq!(
            entry.subscription_access["theme.changed"],
            RuntimeSubscriptionAccess::Allowed
        );
    }

    #[test]
    fn disabled_and_invalid_extensions_are_absent_from_authentication_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        package(&paths.packages.join("disabled"), "disabled", true, &[], &[]);
        std::fs::create_dir_all(paths.packages.join("invalid")).unwrap();
        std::fs::write(paths.packages.join("invalid/package.json"), b"not json").unwrap();
        let catalog = ExtensionRuntimeCatalog::load(paths).unwrap();
        assert!(catalog.snapshot().entries.is_empty());
        assert_eq!(catalog.failures().len(), 1);
    }

    #[test]
    fn enabling_and_disabling_rotates_tokens_and_updates_launches() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        package(&paths.packages.join("git"), "git", true, &[], &[]);
        let mut catalog = ExtensionRuntimeCatalog::load(paths).unwrap();
        catalog.set_enabled("git", true).unwrap();
        let token = catalog.snapshot().entries["git"].token.clone();
        assert_eq!(catalog.host_launches().len(), 1);
        catalog.set_enabled("git", false).unwrap();
        assert!(catalog.snapshot().entries.is_empty());
        assert!(catalog.host_launches().is_empty());
        catalog.set_enabled("git", true).unwrap();
        assert_ne!(catalog.snapshot().entries["git"].token, token);
    }

    #[test]
    fn grants_and_effective_settings_persist_through_the_runtime_catalog() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let package_path = paths.packages.join("git");
        package(&package_path, "git", false, &[], &[]);
        let manifest_path = package_path.join("package.json");
        let mut manifest: Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        manifest["muxy"]["settings"] = json!([{
            "key": "branch",
            "title": "Branch",
            "type": "string",
            "defaultValue": "main"
        }]);
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let mut catalog = ExtensionRuntimeCatalog::load(paths.clone()).unwrap();
        assert_eq!(
            catalog.effective_setting("git", "branch").unwrap(),
            Some(json!("main"))
        );
        catalog
            .set_setting("git", "branch", json!("develop"))
            .unwrap();
        let grant = ExtensionGrantRule {
            id: "grant-1".to_owned(),
            extension_id: "git".to_owned(),
            verb: crate::extensions::state::ExtensionGatedVerb::GitWrite,
            match_rule: crate::extensions::state::ExtensionGrantMatch::GitOperationEquals {
                string: "commit".to_owned(),
            },
            decision: crate::extensions::state::ExtensionGrantDecision::Allow,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        catalog.set_grants(vec![grant.clone()]).unwrap();
        let loaded = ExtensionRuntimeCatalog::load(paths).unwrap();
        assert_eq!(
            loaded.effective_setting("git", "branch").unwrap(),
            Some(json!("develop"))
        );
        assert_eq!(loaded.grants(), [grant]);
    }

    #[test]
    fn lifecycle_restarts_five_times_and_resets_after_stability() {
        let start = Instant::now();
        let mut lifecycle = ExtensionHostLifecycle::default();
        for attempt in 1..=MAX_HOST_RESTART_ATTEMPTS {
            let generation = lifecycle.request_start();
            assert!(lifecycle.started(generation, start));
            assert_eq!(
                lifecycle.terminated(
                    generation,
                    HostTermination::Exited(1),
                    true,
                    start + Duration::from_millis(attempt.into()),
                ),
                HostLifecycleAction::RestartAfter(HOST_RESTART_BACKOFF)
            );
            assert_eq!(lifecycle.restart_attempts, attempt);
            assert!(lifecycle.restart_is_due(
                start + HOST_RESTART_BACKOFF + Duration::from_millis(attempt.into()),
                true
            ));
        }
        let generation = lifecycle.request_start();
        lifecycle.started(generation, start);
        assert_eq!(
            lifecycle.terminated(
                generation,
                HostTermination::Exited(1),
                true,
                start + Duration::from_secs(1),
            ),
            HostLifecycleAction::None
        );
        assert_eq!(lifecycle.status, ExtensionRuntimeStatus::Failed);

        lifecycle.restart_attempts = 3;
        let generation = lifecycle.request_start();
        lifecycle.started(generation, start);
        assert_eq!(
            lifecycle.terminated(
                generation,
                HostTermination::Exited(1),
                true,
                start + HOST_STABILITY_WINDOW,
            ),
            HostLifecycleAction::RestartAfter(HOST_RESTART_BACKOFF)
        );
        assert_eq!(lifecycle.restart_attempts, 1);
    }

    #[test]
    fn intentional_stops_and_stale_terminations_never_restart() {
        let start = Instant::now();
        let mut lifecycle = ExtensionHostLifecycle::default();
        let stale = lifecycle.request_start();
        let current = lifecycle.request_start();
        assert_eq!(
            lifecycle.terminated(stale, HostTermination::Exited(1), true, start,),
            HostLifecycleAction::None
        );
        lifecycle.started(current, start);
        assert_eq!(
            lifecycle.terminated(current, HostTermination::Intentional, false, start),
            HostLifecycleAction::None
        );
        assert_eq!(lifecycle.status, ExtensionRuntimeStatus::Disabled);
    }
}
