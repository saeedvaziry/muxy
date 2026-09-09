use crate::environment::RuntimePathPolicy;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const STATE_FILE_NAME: &str = "state.json";
pub const MIGRATION_FILE_NAME: &str = "swift-extension-migration.json";
pub const MIGRATION_LOCK_FILE_NAME: &str = "swift-extension-migration.lock";
pub const AUDIT_FILE_NAME: &str = "audit.log";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionPaths {
    pub packages: PathBuf,
    pub state: PathBuf,
    pub staging: PathBuf,
}

impl ExtensionPaths {
    pub fn new(policy: RuntimePathPolicy, app_support_root: impl AsRef<Path>) -> Self {
        let app_support_root = app_support_root.as_ref();
        Self {
            packages: policy.extension_packages_path(app_support_root),
            state: policy.extension_state_path(app_support_root),
            staging: policy.extension_staging_path(app_support_root),
        }
    }

    pub fn state_file(&self) -> PathBuf {
        self.state.join(STATE_FILE_NAME)
    }

    pub fn migration_file(&self) -> PathBuf {
        self.state.join(MIGRATION_FILE_NAME)
    }

    pub fn migration_lock_file(&self) -> PathBuf {
        self.state.join(MIGRATION_LOCK_FILE_NAME)
    }

    pub fn storage_directory(&self) -> PathBuf {
        self.state.join("storage")
    }

    pub fn storage_file(&self, extension_id: &str) -> PathBuf {
        self.storage_directory()
            .join(format!("{}.json", safe_extension_filename(extension_id)))
    }

    pub fn logs_directory(&self) -> PathBuf {
        self.state.join("logs")
    }

    pub fn log_file(&self, extension_id: &str) -> PathBuf {
        self.logs_directory()
            .join(format!("{}.log", safe_extension_filename(extension_id)))
    }

    pub fn audit_file(&self) -> PathBuf {
        self.state.join(AUDIT_FILE_NAME)
    }
}

pub fn safe_extension_filename(extension_id: &str) -> String {
    let slug = extension_id
        .chars()
        .take(64)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let slug = if slug.is_empty() { "_" } else { &slug };
    let digest = Sha256::digest(extension_id.as_bytes());
    let suffix = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{slug}-{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::BuildMode;

    #[test]
    fn extension_paths_isolate_build_modes() {
        let root = Path::new("/profile");
        let development = ExtensionPaths::new(RuntimePathPolicy::new(BuildMode::Development), root);
        let production = ExtensionPaths::new(RuntimePathPolicy::new(BuildMode::Production), root);
        assert_eq!(development.packages, root.join("extensions-dev"));
        assert_eq!(development.state, root.join("extension-state-dev"));
        assert_eq!(development.staging, root.join("extension-staging-dev"));
        assert_eq!(production.packages, root.join("extensions"));
        assert_eq!(production.state, root.join("extension-state"));
        assert_eq!(production.staging, root.join("extension-staging"));
    }

    #[test]
    fn storage_filenames_match_the_swift_slug_and_digest_contract() {
        assert_eq!(safe_extension_filename("git"), "git-9a881b9b9f238494");
        assert_eq!(
            safe_extension_filename("müxy/extension"),
            "m_xy_extension-57685a2a03e018b3"
        );
    }
}
