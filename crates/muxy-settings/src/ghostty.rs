use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use crate::{Error, Result, settings::read_or_create};

const DEFAULT_CONFIG: &str = "font-family = Menlo\nfont-size = 13\nadjust-cell-height = 0\n";

#[derive(Clone, Debug, PartialEq)]
pub struct TerminalSettings {
    pub font_families: Vec<String>,
    pub font_size: f32,
    pub cell_height: CellHeight,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum CellHeight {
    #[default]
    Natural,
    Pixels(i16),
    Percent(f32),
}

impl CellHeight {
    pub fn apply(self, natural: f32, scale: f32) -> f32 {
        match self {
            Self::Natural => natural,
            Self::Pixels(amount) => natural + f32::from(amount) / scale,
            Self::Percent(amount) => natural * (1.0 + amount / 100.0),
        }
        .ceil()
        .max(1.0)
    }
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font_families: vec!["Menlo".into()],
            font_size: 13.0,
            cell_height: CellHeight::Natural,
        }
    }
}

impl TerminalSettings {
    pub fn load(path: &Path) -> Result<Self> {
        let seed =
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/ghostty/config"));
        Self::load_with_seed(path, seed.as_deref())
    }

    pub fn load_with_seed(path: &Path, seed: Option<&Path>) -> Result<Self> {
        if !path.exists() {
            let source = seed.map(fs::read_to_string).transpose();
            let defaults = match source {
                Ok(source) => source.unwrap_or_else(|| DEFAULT_CONFIG.into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => DEFAULT_CONFIG.into(),
                Err(error) => return Err(Error::new("Ghostty seed config", error)),
            };
            read_or_create(path, &defaults)?;
        }
        let mut settings = Self::default();
        let mut families = Vec::new();
        let mut pending = VecDeque::from([(path.to_owned(), false, 0)]);
        let mut loaded = HashSet::new();
        while let Some((path, optional, depth)) = pending.pop_front() {
            if optional && path.try_exists().is_ok_and(|exists| !exists) {
                continue;
            }
            let canonical = fs::canonicalize(&path)
                .map_err(|error| Error::new(path.display().to_string(), error))?;
            if !loaded.insert(canonical) || depth >= 32 {
                return Err(Error::new(
                    path.display().to_string(),
                    "config-file cycle or nesting exceeds 32 files",
                ));
            }
            pending.extend(
                settings
                    .read(&path, &mut families)?
                    .into_iter()
                    .map(|(path, optional)| (path, optional, depth + 1)),
            );
        }
        if !families.is_empty() {
            settings.font_families = families;
        }
        Ok(settings)
    }

    fn read(&mut self, path: &Path, families: &mut Vec<String>) -> Result<Vec<(PathBuf, bool)>> {
        let source = fs::read_to_string(path)
            .map_err(|error| Error::new(path.display().to_string(), error))?;
        let mut includes = Vec::new();
        for (index, line) in source.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let context = format!("{}:{}", path.display(), index + 1);
            let (key, value) = line
                .split_once('=')
                .map_or((line, None), |(key, value)| (key, Some(value)));
            let key = key.trim();
            if !matches!(
                key,
                "font-family" | "font-size" | "adjust-cell-height" | "config-file"
            ) {
                continue;
            }
            let context = format!("{context} {key}");
            let value = value
                .ok_or_else(|| Error::new(&context, "expected key = value"))?
                .trim();
            let optional = key == "config-file" && value.starts_with('?');
            let value = if optional { &value[1..] } else { value };
            let value = config_value(value).map_err(|error| Error::new(&context, error))?;
            match key {
                "font-family" if value.is_empty() => families.clear(),
                "font-family" => families.push(value.into()),
                "font-size" => {
                    let size = if value.is_empty() {
                        13.0
                    } else {
                        value
                            .parse::<f32>()
                            .map_err(|error| Error::new(&context, error))?
                    };
                    if !size.is_finite() || !(1.0..=256.0).contains(&size) {
                        return Err(Error::new(context, "must be between 1 and 256 points"));
                    }
                    self.font_size = size;
                }
                "adjust-cell-height" => {
                    self.cell_height =
                        parse_height(value).map_err(|error| Error::new(&context, error))?;
                }
                "config-file" if value.is_empty() => includes.clear(),
                "config-file" => {
                    let target = if let Some(relative) = value.strip_prefix("~/") {
                        PathBuf::from(
                            std::env::var_os("HOME")
                                .ok_or_else(|| Error::new(&context, "HOME is not set"))?,
                        )
                        .join(relative)
                    } else {
                        path.parent().unwrap_or_else(|| Path::new(".")).join(value)
                    };
                    includes.push((target, optional));
                }
                _ => {}
            }
        }
        Ok(includes)
    }

    pub fn zoom(&mut self, delta: f32) {
        self.font_size = (self.font_size + delta).clamp(1.0, 256.0);
    }
}

fn config_value(value: &str) -> Result<&str> {
    if let Some(quoted) = value.strip_prefix('"') {
        quoted
            .strip_suffix('"')
            .ok_or_else(|| Error::new("value", "unterminated quote"))
    } else {
        Ok(value)
    }
}

fn parse_height(value: &str) -> Result<CellHeight> {
    if value.is_empty() || value == "0" {
        return Ok(CellHeight::Natural);
    }
    if let Some(percentage) = value.strip_suffix('%') {
        let amount = percentage
            .parse::<f32>()
            .map_err(|error| Error::new("adjust-cell-height", error))?;
        if !amount.is_finite() || !(-99.0..=1000.0).contains(&amount) {
            return Err(Error::new(
                "adjust-cell-height",
                "percentage is outside the supported range",
            ));
        }
        return Ok(CellHeight::Percent(amount));
    }
    let amount = value
        .parse::<i16>()
        .map_err(|error| Error::new("adjust-cell-height", error))?;
    if !(-4096..=4096).contains(&amount) {
        return Err(Error::new(
            "adjust-cell-height",
            "adjustment is outside the supported range",
        ));
    }
    Ok(CellHeight::Pixels(amount))
}
