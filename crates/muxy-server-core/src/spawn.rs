use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use muxy_protocol::ErrorCode;
use muxy_pty::{Pty, PtySize, SpawnRequest};

use crate::error::ServerError;
use crate::settings::ServerSettings;

const FALLBACK_SHELL: &str = "/bin/zsh";
const LOGIN_FLAG: &str = "-l";
const TERMINAL_ENV: [(&str, &str); 3] = [
    ("TERM", "xterm-256color"),
    ("COLORTERM", "truecolor"),
    ("TERM_PROGRAM", "muxy"),
];

pub(crate) fn spawn_shell(
    settings: &ServerSettings,
    integration: Option<&crate::ShellIntegration>,
    directory: &Path,
    size: PtySize,
) -> Result<Pty, ServerError> {
    if !directory.is_dir() {
        return Err(ServerError::new(
            ErrorCode::BadPath,
            format!("{} is not a directory", directory.display()),
        ));
    }
    let mut request = SpawnRequest {
        program: resolve_shell(settings),
        args: vec![OsString::from(LOGIN_FLAG)],
        cwd: directory.to_path_buf(),
        env: environment(),
        size,
    };
    if let Some(integration) = integration {
        integration.configure(&mut request, settings.shell_integration);
    }
    Pty::spawn(request).map_err(ServerError::spawn_failed)
}

fn resolve_shell(settings: &ServerSettings) -> PathBuf {
    settings
        .default_shell
        .clone()
        .or_else(|| {
            env::var_os("SHELL")
                .filter(|shell| !shell.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| PathBuf::from(FALLBACK_SHELL))
}

fn environment() -> Vec<(OsString, OsString)> {
    let mut env: Vec<_> = env::vars_os().collect();
    env.extend(
        TERMINAL_ENV
            .iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value))),
    );
    env
}
