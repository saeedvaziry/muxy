use std::path::PathBuf;
use thiserror::Error;

#[cfg(target_os = "macos")]
mod runtime;

#[derive(Clone, Debug, Eq, PartialEq)]
struct HostConfig {
    script_path: PathBuf,
    socket_path: PathBuf,
    extension_id: String,
    token: String,
    oneshot: bool,
}

#[derive(Debug, Error)]
enum HostConfigError {
    #[error("missing background script path argument")]
    ScriptArgument,
    #[error("missing MUXY_SOCKET_PATH")]
    SocketEnvironment,
    #[error("missing MUXY_EXTENSION_ID")]
    ExtensionIdEnvironment,
}

impl HostConfig {
    fn from_process() -> Result<Self, HostConfigError> {
        Self::from_values(
            std::env::args_os().nth(1).map(PathBuf::from),
            std::env::var_os("MUXY_SOCKET_PATH").map(PathBuf::from),
            std::env::var("MUXY_EXTENSION_ID").ok(),
            std::env::var("MUXY_EXTENSION_TOKEN").unwrap_or_default(),
            matches!(std::env::var("MUXY_EXTENSION_ONESHOT").as_deref(), Ok("1")),
        )
    }

    fn from_values(
        script_path: Option<PathBuf>,
        socket_path: Option<PathBuf>,
        extension_id: Option<String>,
        token: String,
        oneshot: bool,
    ) -> Result<Self, HostConfigError> {
        Ok(Self {
            script_path: script_path.ok_or(HostConfigError::ScriptArgument)?,
            socket_path: socket_path.ok_or(HostConfigError::SocketEnvironment)?,
            extension_id: extension_id.ok_or(HostConfigError::ExtensionIdEnvironment)?,
            token,
            oneshot,
        })
    }
}

fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("[muxy-extension-host] {message}");
    std::process::exit(1)
}

#[cfg(target_os = "macos")]
fn main() {
    let config = HostConfig::from_process().unwrap_or_else(|error| fail(error));
    runtime::monitor_parent();
    runtime::run(config).unwrap_or_else(|error| fail(error));
}

#[cfg(not(target_os = "macos"))]
fn main() {
    let _ = HostConfig::from_process().unwrap_or_else(|error| fail(error));
    fail("extension background hosts are unsupported on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_requires_script_socket_and_extension_identity() {
        let script = Some(PathBuf::from("background.js"));
        let socket = Some(PathBuf::from("main.sock"));
        assert!(matches!(
            HostConfig::from_values(
                None,
                socket.clone(),
                Some("sample".to_owned()),
                String::new(),
                false
            ),
            Err(HostConfigError::ScriptArgument)
        ));
        assert!(matches!(
            HostConfig::from_values(
                script.clone(),
                None,
                Some("sample".to_owned()),
                String::new(),
                false
            ),
            Err(HostConfigError::SocketEnvironment)
        ));
        assert!(matches!(
            HostConfig::from_values(script.clone(), socket.clone(), None, String::new(), false),
            Err(HostConfigError::ExtensionIdEnvironment)
        ));
        assert_eq!(
            HostConfig::from_values(
                script,
                socket,
                Some("sample".to_owned()),
                "token".to_owned(),
                false
            )
            .unwrap(),
            HostConfig {
                script_path: PathBuf::from("background.js"),
                socket_path: PathBuf::from("main.sock"),
                extension_id: "sample".to_owned(),
                token: "token".to_owned(),
                oneshot: false,
            }
        );
    }
}
