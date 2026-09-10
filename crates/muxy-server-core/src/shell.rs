use std::ffi::OsString;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use muxy_pty::SpawnRequest;

/// Server-owned startup hooks, kept beside this server's socket.
#[derive(Clone, Debug)]
pub struct ShellIntegration {
    directory: PathBuf,
}

impl ShellIntegration {
    pub fn install(directory: &Path) -> io::Result<Self> {
        for path in [
            directory.to_path_buf(),
            directory.join("zsh"),
            directory.join("fish"),
            directory.join("fish/vendor_conf.d"),
        ] {
            match DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    if !fs::symlink_metadata(&path)?.is_dir() {
                        return Err(io::Error::other(
                            "shell integration directory is not a directory",
                        ));
                    }
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
                }
                Err(error) => return Err(error),
            }
        }
        for (name, contents) in [
            ("zsh/.zshenv", include_str!("../shell/.zshenv")),
            ("muxy.zsh", include_str!("../shell/muxy.zsh")),
            ("muxy.bash", include_str!("../shell/muxy.bash")),
            (
                "fish/vendor_conf.d/muxy.fish",
                include_str!("../shell/muxy.fish"),
            ),
        ] {
            let path = directory.join(name);
            let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            let result = file
                .write_all(contents.as_bytes())
                .and_then(|()| file.sync_all())
                .and_then(|()| fs::rename(&temporary, &path));
            if result.is_err() {
                let _ = fs::remove_file(&temporary);
            }
            result?;
        }
        Ok(Self {
            directory: directory.canonicalize()?,
        })
    }

    pub(crate) fn configure(&self, request: &mut SpawnRequest, enabled: bool) {
        set_env(
            request,
            "MUXY_SHELL_INTEGRATION",
            if enabled { "1" } else { "0" }.into(),
        );
        if !enabled {
            return;
        }
        set_env(
            request,
            "MUXY_SHELL_INTEGRATION_DIR",
            self.directory.as_os_str().to_owned(),
        );
        match request.program.file_name().and_then(|name| name.to_str()) {
            Some("zsh") => {
                let original = get_env(request, "ZDOTDIR").cloned();
                set_env(
                    request,
                    "MUXY_ZDOTDIR_SET",
                    if original.is_some() { "1" } else { "0" }.into(),
                );
                set_env(
                    request,
                    "MUXY_ORIGINAL_ZDOTDIR",
                    original.unwrap_or_default(),
                );
                set_env(
                    request,
                    "ZDOTDIR",
                    self.directory.join("zsh").into_os_string(),
                );
            }
            Some("fish") => {
                let mut paths = self.directory.as_os_str().to_owned();
                paths.push(":");
                paths.push(
                    get_env(request, "XDG_DATA_DIRS")
                        .filter(|value| !value.is_empty())
                        .cloned()
                        .unwrap_or_else(|| "/usr/local/share:/usr/share".into()),
                );
                set_env(request, "XDG_DATA_DIRS", paths);
            }
            _ => {}
        }
    }
}

fn get_env<'a>(request: &'a SpawnRequest, name: &str) -> Option<&'a OsString> {
    request
        .env
        .iter()
        .rev()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

fn set_env(request: &mut SpawnRequest, name: &str, value: OsString) {
    request.env.retain(|(key, _)| key != name);
    request.env.push((name.into(), value));
}

#[cfg(test)]
#[path = "shell/tests.rs"]
mod tests;
