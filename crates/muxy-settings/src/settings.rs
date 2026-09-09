use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Appearance, Error, Keymap, Result};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub appearance: Appearance,
    pub window: WindowSettings,
    pub keymap: Keymap,
    pub projects: ProjectSettings,
    pub panes: PaneSettings,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PaneSettings {
    pub new_pane_directory: NewPaneDirectory,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NewPaneDirectory {
    #[default]
    Project,
    Current,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectSettings {
    pub search_root: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WindowSettings {
    pub default_size: [f32; 2],
    pub confirm_running_process: bool,
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            default_size: [1200.0, 800.0],
            confirm_running_process: true,
        }
    }
}

impl Settings {
    pub fn set_confirm_running_process(&mut self, enabled: bool, path: &Path) -> Result<()> {
        let values = toml::Table::from_iter([(
            "confirm_running_process".into(),
            toml::Value::Boolean(enabled),
        )]);
        crate::appearance::save_section(path, "window", &values)
            .map_err(|error| Error::new("window.confirm_running_process", error))?;
        self.window.confirm_running_process = enabled;
        Ok(())
    }

    pub fn set_project_search_root(&mut self, root: PathBuf, path: &Path) -> Result<()> {
        let values = toml::Table::from_iter([(
            "search_root".into(),
            toml::Value::String(root.to_string_lossy().into_owned()),
        )]);
        crate::appearance::save_section(path, "projects", &values)
            .map_err(|error| Error::new("projects.search_root", error))?;
        self.projects.search_root = Some(root);
        Ok(())
    }

    pub fn default_path() -> Result<PathBuf> {
        muxy_core::dirs::muxy_dir()
            .map(|directory| directory.join("settings.toml"))
            .map_err(|error| Error::new("settings path", error))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let source = read_or_create(
            path,
            &toml::to_string_pretty(&Self::default())
                .map_err(|error| Error::new("settings defaults", error))?,
        )?;
        let settings: Self = toml::from_str(&source)
            .map_err(|error| Error::new(path.display().to_string(), error))?;
        for (name, value, minimum) in [
            ("width", settings.window.default_size[0], 640.0),
            ("height", settings.window.default_size[1], 400.0),
        ] {
            if !value.is_finite() || !(minimum..=16384.0).contains(&value) {
                return Err(Error::new(
                    format!("window.default_size.{name}"),
                    format!("must be between {minimum} and 16384"),
                ));
            }
        }
        Ok(settings)
    }
}

pub(crate) fn read_or_create(path: &Path, defaults: &str) -> Result<String> {
    let context = || path.display().to_string();
    match fs::read_to_string(path) {
        Ok(source) => return Ok(source),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(Error::new(context(), error)),
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| Error::new(context(), error))?;
    }
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => {
            file.write_all(defaults.as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|error| Error::new(context(), error))?;
            Ok(defaults.into())
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            fs::read_to_string(path).map_err(|error| Error::new(context(), error))
        }
        Err(error) => Err(Error::new(context(), error)),
    }
}
