use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildMode {
    Development,
    Production,
}

impl BuildMode {
    pub const fn from_debug_assertions(enabled: bool) -> Self {
        if enabled {
            Self::Development
        } else {
            Self::Production
        }
    }

    pub const fn is_development(self) -> bool {
        matches!(self, Self::Development)
    }

    pub const fn other(self) -> Self {
        match self {
            Self::Development => Self::Production,
            Self::Production => Self::Development,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoragePathPolicy {
    mode: BuildMode,
}

impl StoragePathPolicy {
    pub const fn new(mode: BuildMode) -> Self {
        Self { mode }
    }

    pub fn root(self, home: impl AsRef<Path>) -> PathBuf {
        match self.mode {
            BuildMode::Development => home.as_ref().join(".muxy-dev"),
            BuildMode::Production => home.as_ref().join(".muxy"),
        }
    }

    pub fn swift_source(home: impl AsRef<Path>) -> PathBuf {
        home.as_ref().join("Library/Application Support/Muxy")
    }
}

#[macro_export]
macro_rules! build_mode {
    () => {
        $crate::environment::BuildMode::from_debug_assertions(cfg!(debug_assertions))
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimePathPolicy {
    mode: BuildMode,
}

impl RuntimePathPolicy {
    pub const fn new(mode: BuildMode) -> Self {
        Self { mode }
    }

    pub const fn main_socket_filename(self) -> &'static str {
        match self.mode {
            BuildMode::Development => "muxy-dev.sock",
            BuildMode::Production => "muxy.sock",
        }
    }

    pub const fn hook_default_socket_filename(self) -> &'static str {
        match self.mode {
            BuildMode::Development => "muxy-dev.sock",
            BuildMode::Production => "muxy.sock",
        }
    }

    pub const fn session_directory_name(self) -> &'static str {
        match self.mode {
            BuildMode::Development => "sessions-dev",
            BuildMode::Production => "sessions",
        }
    }

    pub const fn session_socket_filename(self) -> &'static str {
        "control.sock"
    }

    pub fn fallback_session_directory_name(self, uid: u32) -> String {
        match self.mode {
            BuildMode::Development => format!("muxy-dev-{uid}"),
            BuildMode::Production => format!("muxy-{uid}"),
        }
    }

    pub const fn hook_staging_directory_name(self) -> &'static str {
        match self.mode {
            BuildMode::Development => "hooks-dev",
            BuildMode::Production => "hooks",
        }
    }

    pub const fn extension_packages_directory_name(self) -> &'static str {
        match self.mode {
            BuildMode::Development => "extensions-dev",
            BuildMode::Production => "extensions",
        }
    }

    pub const fn extension_state_directory_name(self) -> &'static str {
        match self.mode {
            BuildMode::Development => "extension-state-dev",
            BuildMode::Production => "extension-state",
        }
    }

    pub const fn extension_staging_directory_name(self) -> &'static str {
        match self.mode {
            BuildMode::Development => "extension-staging-dev",
            BuildMode::Production => "extension-staging",
        }
    }

    pub fn main_socket_path(self, app_support_root: impl AsRef<Path>) -> PathBuf {
        app_support_root.as_ref().join(self.main_socket_filename())
    }

    pub fn hook_default_socket_path(self, app_support_root: impl AsRef<Path>) -> PathBuf {
        app_support_root
            .as_ref()
            .join(self.hook_default_socket_filename())
    }

    pub fn preferred_session_socket_path(self, app_support_root: impl AsRef<Path>) -> PathBuf {
        app_support_root
            .as_ref()
            .join(self.session_directory_name())
            .join(self.session_socket_filename())
    }

    pub fn fallback_session_socket_path(
        self,
        fallback_root: impl AsRef<Path>,
        uid: u32,
    ) -> PathBuf {
        fallback_root
            .as_ref()
            .join(self.fallback_session_directory_name(uid))
            .join(self.session_socket_filename())
    }

    pub fn hook_staging_path(self, app_support_root: impl AsRef<Path>) -> PathBuf {
        app_support_root
            .as_ref()
            .join(self.hook_staging_directory_name())
    }

    pub fn extension_packages_path(self, app_support_root: impl AsRef<Path>) -> PathBuf {
        app_support_root
            .as_ref()
            .join(self.extension_packages_directory_name())
    }

    pub fn extension_state_path(self, app_support_root: impl AsRef<Path>) -> PathBuf {
        app_support_root
            .as_ref()
            .join(self.extension_state_directory_name())
    }

    pub fn extension_staging_path(self, app_support_root: impl AsRef<Path>) -> PathBuf {
        app_support_root
            .as_ref()
            .join(self.extension_staging_directory_name())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MobileSettingsKeys {
    pub enabled: &'static str,
    pub port: &'static str,
    pub scrollback_cap: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MobileSettingsPolicy {
    mode: BuildMode,
}

impl MobileSettingsPolicy {
    pub const fn new(mode: BuildMode) -> Self {
        Self { mode }
    }

    pub const fn keys(self) -> MobileSettingsKeys {
        match self.mode {
            BuildMode::Development => MobileSettingsKeys {
                enabled: "app.muxy.mobile.serverEnabled.dev",
                port: "app.muxy.mobile.serverPort.dev",
                scrollback_cap: "app.muxy.mobile.scrollbackCap.dev",
            },
            BuildMode::Production => MobileSettingsKeys {
                enabled: "app.muxy.mobile.serverEnabled",
                port: "app.muxy.mobile.serverPort",
                scrollback_cap: "app.muxy.mobile.scrollbackCap",
            },
        }
    }

    pub const fn settings_enabled_default(self) -> bool {
        false
    }

    pub const fn force_server_enabled_on_startup(self) -> bool {
        self.mode.is_development()
    }

    pub const fn default_port(self) -> u16 {
        match self.mode {
            BuildMode::Development => 4866,
            BuildMode::Production => 4865,
        }
    }

    pub const fn default_scrollback_cap(self) -> i64 {
        8
    }

    pub const fn minimum_port(self) -> u16 {
        1024
    }

    pub const fn maximum_port(self) -> u16 {
        65535
    }

    pub fn valid_port_range(self) -> RangeInclusive<u16> {
        self.minimum_port()..=self.maximum_port()
    }

    pub const fn is_valid_port(self, port: u16) -> bool {
        port >= self.minimum_port() && port <= self.maximum_port()
    }

    pub const fn minimum_scrollback_cap(self) -> i64 {
        1
    }

    pub const fn maximum_scrollback_cap(self) -> i64 {
        128
    }

    pub fn valid_scrollback_cap_range(self) -> RangeInclusive<i64> {
        self.minimum_scrollback_cap()..=self.maximum_scrollback_cap()
    }

    pub const fn is_valid_scrollback_cap(self, cap: i64) -> bool {
        cap >= self.minimum_scrollback_cap() && cap <= self.maximum_scrollback_cap()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderConfigMutationSource {
    AiNotificationsProviderToggle,
    AiNotificationsRefresh,
    AutomaticStartup,
    WatcherReconciliation,
    JsonSettingsApplication,
    SettingsOrBackupImport,
    Cli,
    OtherNonUi,
}

pub const fn provider_config_mutation_allowed(
    mode: BuildMode,
    source: ProviderConfigMutationSource,
) -> bool {
    match mode {
        BuildMode::Production => true,
        BuildMode::Development => matches!(
            source,
            ProviderConfigMutationSource::AiNotificationsProviderToggle
                | ProviderConfigMutationSource::AiNotificationsRefresh
        ),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderConfigMutationPermit {
    mode: BuildMode,
    source: ProviderConfigMutationSource,
}

impl ProviderConfigMutationPermit {
    pub const fn mode(self) -> BuildMode {
        self.mode
    }

    pub const fn source(self) -> ProviderConfigMutationSource {
        self.source
    }

    pub fn matches_mode(self, mode: BuildMode) -> bool {
        self.mode == mode
    }
}

#[doc(hidden)]
pub fn __authorize_provider_config_mutation(
    caller_mode: BuildMode,
    source: ProviderConfigMutationSource,
) -> Option<ProviderConfigMutationPermit> {
    let artifact_mode = BuildMode::from_debug_assertions(cfg!(debug_assertions));
    if caller_mode != artifact_mode || !provider_config_mutation_allowed(artifact_mode, source) {
        return None;
    }
    Some(ProviderConfigMutationPermit {
        mode: artifact_mode,
        source,
    })
}

#[macro_export]
macro_rules! authorize_provider_config_mutation {
    ($source:expr) => {
        $crate::environment::__authorize_provider_config_mutation($crate::build_mode!(), $source)
    };
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        BuildMode, MobileSettingsPolicy, ProviderConfigMutationPermit,
        ProviderConfigMutationSource, RuntimePathPolicy, StoragePathPolicy,
        provider_config_mutation_allowed,
    };

    const SOURCES: [ProviderConfigMutationSource; 8] = [
        ProviderConfigMutationSource::AiNotificationsProviderToggle,
        ProviderConfigMutationSource::AiNotificationsRefresh,
        ProviderConfigMutationSource::AutomaticStartup,
        ProviderConfigMutationSource::WatcherReconciliation,
        ProviderConfigMutationSource::JsonSettingsApplication,
        ProviderConfigMutationSource::SettingsOrBackupImport,
        ProviderConfigMutationSource::Cli,
        ProviderConfigMutationSource::OtherNonUi,
    ];

    #[test]
    fn explicit_build_mode_conversion_covers_both_values() {
        assert_eq!(
            BuildMode::from_debug_assertions(true),
            BuildMode::Development
        );
        assert_eq!(
            BuildMode::from_debug_assertions(false),
            BuildMode::Production
        );
        assert!(BuildMode::Development.is_development());
        assert!(!BuildMode::Production.is_development());
        assert_eq!(BuildMode::Development.other(), BuildMode::Production);
        assert_eq!(BuildMode::Production.other(), BuildMode::Development);
    }

    #[test]
    fn macro_selects_the_current_test_artifact() {
        assert_eq!(
            crate::build_mode!(),
            BuildMode::from_debug_assertions(cfg!(debug_assertions))
        );
    }

    #[test]
    fn storage_path_policy_separates_release_and_development_roots() {
        let home = Path::new("/Users/example");
        assert_eq!(
            StoragePathPolicy::new(BuildMode::Production).root(home),
            home.join(".muxy")
        );
        assert_eq!(
            StoragePathPolicy::new(BuildMode::Development).root(home),
            home.join(".muxy-dev")
        );
        assert_eq!(
            StoragePathPolicy::swift_source(home),
            home.join("Library/Application Support/Muxy")
        );
    }

    #[test]
    fn runtime_paths_match_both_swift_contracts() {
        let app_support = Path::new("/Users/example/Library/Application Support/Muxy");
        let fallback_root = Path::new("/tmp");
        let development = RuntimePathPolicy::new(BuildMode::Development);
        let production = RuntimePathPolicy::new(BuildMode::Production);

        assert_eq!(development.main_socket_filename(), "muxy-dev.sock");
        assert_eq!(production.main_socket_filename(), "muxy.sock");
        assert_eq!(development.hook_default_socket_filename(), "muxy-dev.sock");
        assert_eq!(production.hook_default_socket_filename(), "muxy.sock");
        assert_eq!(development.session_directory_name(), "sessions-dev");
        assert_eq!(production.session_directory_name(), "sessions");
        assert_eq!(development.session_socket_filename(), "control.sock");
        assert_eq!(production.session_socket_filename(), "control.sock");
        assert_eq!(
            development.fallback_session_directory_name(501),
            "muxy-dev-501"
        );
        assert_eq!(production.fallback_session_directory_name(501), "muxy-501");
        assert_eq!(development.hook_staging_directory_name(), "hooks-dev");
        assert_eq!(production.hook_staging_directory_name(), "hooks");
        assert_eq!(
            development.extension_packages_directory_name(),
            "extensions-dev"
        );
        assert_eq!(production.extension_packages_directory_name(), "extensions");
        assert_eq!(
            development.extension_state_directory_name(),
            "extension-state-dev"
        );
        assert_eq!(
            production.extension_state_directory_name(),
            "extension-state"
        );
        assert_eq!(
            development.extension_staging_directory_name(),
            "extension-staging-dev"
        );
        assert_eq!(
            production.extension_staging_directory_name(),
            "extension-staging"
        );

        assert_eq!(
            development.main_socket_path(app_support),
            app_support.join("muxy-dev.sock")
        );
        assert_eq!(
            production.main_socket_path(app_support),
            app_support.join("muxy.sock")
        );
        assert_eq!(
            development.hook_default_socket_path(app_support),
            app_support.join("muxy-dev.sock")
        );
        assert_eq!(
            production.hook_default_socket_path(app_support),
            app_support.join("muxy.sock")
        );
        assert_eq!(
            development.preferred_session_socket_path(app_support),
            app_support.join("sessions-dev/control.sock")
        );
        assert_eq!(
            production.preferred_session_socket_path(app_support),
            app_support.join("sessions/control.sock")
        );
        assert_eq!(
            development.fallback_session_socket_path(fallback_root, 501),
            fallback_root.join("muxy-dev-501/control.sock")
        );
        assert_eq!(
            production.fallback_session_socket_path(fallback_root, 501),
            fallback_root.join("muxy-501/control.sock")
        );
        assert_eq!(
            development.hook_staging_path(app_support),
            app_support.join("hooks-dev")
        );
        assert_eq!(
            production.hook_staging_path(app_support),
            app_support.join("hooks")
        );
        assert_eq!(
            development.extension_packages_path(app_support),
            app_support.join("extensions-dev")
        );
        assert_eq!(
            production.extension_packages_path(app_support),
            app_support.join("extensions")
        );
        assert_eq!(
            development.extension_state_path(app_support),
            app_support.join("extension-state-dev")
        );
        assert_eq!(
            production.extension_state_path(app_support),
            app_support.join("extension-state")
        );
        assert_eq!(
            development.extension_staging_path(app_support),
            app_support.join("extension-staging-dev")
        );
        assert_eq!(
            production.extension_staging_path(app_support),
            app_support.join("extension-staging")
        );
    }

    #[test]
    fn mobile_settings_match_both_swift_contracts() {
        let development = MobileSettingsPolicy::new(BuildMode::Development);
        let production = MobileSettingsPolicy::new(BuildMode::Production);

        assert_eq!(
            development.keys().enabled,
            "app.muxy.mobile.serverEnabled.dev"
        );
        assert_eq!(development.keys().port, "app.muxy.mobile.serverPort.dev");
        assert_eq!(
            development.keys().scrollback_cap,
            "app.muxy.mobile.scrollbackCap.dev"
        );
        assert_eq!(production.keys().enabled, "app.muxy.mobile.serverEnabled");
        assert_eq!(production.keys().port, "app.muxy.mobile.serverPort");
        assert_eq!(
            production.keys().scrollback_cap,
            "app.muxy.mobile.scrollbackCap"
        );

        for policy in [development, production] {
            assert!(!policy.settings_enabled_default());
            assert_eq!(policy.default_scrollback_cap(), 8);
            assert_eq!(policy.valid_port_range(), 1024..=65535);
            assert_eq!(policy.valid_scrollback_cap_range(), 1..=128);
            assert!(policy.is_valid_port(1024));
            assert!(policy.is_valid_port(65535));
            assert!(!policy.is_valid_port(1023));
            assert!(policy.is_valid_scrollback_cap(1));
            assert!(policy.is_valid_scrollback_cap(128));
            assert!(!policy.is_valid_scrollback_cap(0));
            assert!(!policy.is_valid_scrollback_cap(129));
        }

        assert!(development.force_server_enabled_on_startup());
        assert!(!production.force_server_enabled_on_startup());
        assert_eq!(development.default_port(), 4866);
        assert_eq!(production.default_port(), 4865);
    }

    #[test]
    fn provider_policy_matrix_covers_every_source_in_both_modes() {
        for source in SOURCES {
            let development_allowed = matches!(
                source,
                ProviderConfigMutationSource::AiNotificationsProviderToggle
                    | ProviderConfigMutationSource::AiNotificationsRefresh
            );
            let explicit_result: bool =
                provider_config_mutation_allowed(BuildMode::Development, source);
            assert_eq!(explicit_result, development_allowed);
            assert!(provider_config_mutation_allowed(
                BuildMode::Production,
                source
            ));
        }
    }

    #[test]
    fn runtime_permits_match_the_current_artifact_and_source() {
        for source in SOURCES {
            let permit = crate::authorize_provider_config_mutation!(source);
            let expected = provider_config_mutation_allowed(crate::build_mode!(), source);
            assert_eq!(permit.is_some(), expected);
            if let Some(permit) = permit {
                assert_eq!(permit.mode(), crate::build_mode!());
                assert_eq!(permit.source(), source);
                assert!(permit.matches_mode(crate::build_mode!()));
            }
        }
    }

    #[test]
    fn debug_artifact_rejects_every_non_ui_source() {
        if crate::build_mode!().is_development() {
            for source in SOURCES.into_iter().skip(2) {
                assert!(crate::authorize_provider_config_mutation!(source).is_none());
            }
        }
    }

    #[test]
    fn permit_rejects_a_mutator_mode_mismatch() {
        let permit = ProviderConfigMutationPermit {
            mode: crate::build_mode!().other(),
            source: ProviderConfigMutationSource::AiNotificationsProviderToggle,
        };
        assert!(!permit.matches_mode(crate::build_mode!()));
    }
}
