use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use muxy_server_core::ServerSettings;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct SettingsFile {
    default_shell: Option<PathBuf>,
    history_budget_bytes: u64,
}

impl Default for SettingsFile {
    fn default() -> Self {
        let defaults = ServerSettings::default();
        Self {
            default_shell: defaults.default_shell,
            history_budget_bytes: defaults.history_budget_bytes,
        }
    }
}

pub(crate) fn load(path: &Path) -> io::Result<ServerSettings> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let defaults =
                toml::to_string_pretty(&SettingsFile::default()).map_err(io::Error::other)?;
            let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return load(path),
                Err(error) => return Err(error),
            };
            file.write_all(defaults.as_bytes())?;
            defaults
        }
        Err(error) => return Err(error),
    };
    let settings: SettingsFile = toml::from_str(&contents).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: {error}", path.display()),
        )
    })?;
    Ok(ServerSettings {
        default_shell: settings.default_shell,
        history_budget_bytes: settings.history_budget_bytes,
    })
}
