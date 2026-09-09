use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub dark_theme: String,
    pub light_theme: String,
    pub sidebar_expanded: bool,
    pub status_bar_visible: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            dark_theme: "Muxy".into(),
            light_theme: "Muxy Light".into(),
            sidebar_expanded: false,
            status_bar_visible: true,
        }
    }
}

impl Appearance {
    pub fn load(path: &Path) -> Result<Self> {
        let document = read_document(path)?;
        document.get("appearance").cloned().map_or_else(
            || Ok(Self::default()),
            |value| value.try_into().map_err(Into::into),
        )
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        save_section(path, "appearance", self)
    }
}

pub(crate) fn save_section(path: &Path, section: &str, values: &impl Serialize) -> Result<()> {
    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
    let mut document = read_document(path)?;
    let values = toml::Value::try_from(values)?;
    let values = values
        .as_table()
        .ok_or_else(|| io::Error::other(format!("{section} must be a table")))?;
    let target = document
        .entry(section)
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    target
        .as_table_mut()
        .ok_or_else(|| io::Error::other(format!("{section} must be a table")))?
        .extend(values.clone());
    let source = toml::to_string_pretty(&document)?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("settings path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".settings-{}-{}.tmp",
        std::process::id(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| -> io::Result<()> {
        file.write_all(source.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(Into::into)
}

fn read_document(path: &Path) -> Result<toml::Table> {
    match fs::read_to_string(path) {
        Ok(source) => source.parse().map_err(Into::into),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(toml::Table::new()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_appearance_keeps_defaults() -> Result<()> {
        let appearance: Appearance = toml::from_str("dark_theme = 'Dracula'")?;
        assert_eq!(appearance.dark_theme, "Dracula");
        assert_eq!(appearance.light_theme, "Muxy Light");
        assert!(appearance.status_bar_visible);
        assert!(!appearance.sidebar_expanded);
        Ok(())
    }

    #[test]
    fn updating_appearance_preserves_other_settings() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("muxy-appearance-{}", std::process::id()));
        fs::create_dir_all(&directory)?;
        let path = directory.join("settings.toml");
        fs::write(
            &path,
            "[terminal]\nfont_size = 17\n[appearance]\nlight_theme = 'Solarized Light'\n",
        )?;
        let mut appearance = Appearance::load(&path)?;
        appearance.dark_theme = "Dracula".into();
        appearance.save(&path)?;
        assert_eq!(Appearance::load(&path)?, appearance);
        assert_eq!(
            read_document(&path)?["terminal"]["font_size"].as_integer(),
            Some(17)
        );
        fs::remove_dir_all(directory)?;
        Ok(())
    }
}
