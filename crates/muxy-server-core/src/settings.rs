use std::path::PathBuf;

const DEFAULT_HISTORY_BUDGET_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerSettings {
    pub default_shell: Option<PathBuf>,
    pub shell_integration: bool,
    pub history_budget_bytes: u64,
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            default_shell: None,
            shell_integration: true,
            history_budget_bytes: DEFAULT_HISTORY_BUDGET_BYTES,
        }
    }
}
