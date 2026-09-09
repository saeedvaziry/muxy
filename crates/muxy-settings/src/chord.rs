use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct KeyChord(String);

impl KeyChord {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn default_binding(value: &str) -> Self {
        value.parse().unwrap_or_else(|_| Self(value.into()))
    }
}

impl FromStr for KeyChord {
    type Err = Error;

    fn from_str(source: &str) -> Result<Self> {
        let invalid = || Error::new("chord", format!("invalid key chord {source:?}"));
        let mut rest = source;
        let mut modifiers = [false; 5];
        let names = ["cmd", "ctrl", "alt", "shift", "fn"];
        while let Some((component, tail)) = rest.split_once('-') {
            let Some(index) = names
                .iter()
                .position(|name| component.eq_ignore_ascii_case(name))
            else {
                break;
            };
            if modifiers[index] {
                return Err(invalid());
            }
            modifiers[index] = true;
            rest = tail;
        }
        let mut key = rest.to_ascii_lowercase();
        key = match key.as_str() {
            "plus" => "+".into(),
            "minus" => "-".into(),
            "return" => "enter".into(),
            "esc" => "escape".into(),
            _ => key,
        };
        if rest.len() == 1 && rest.as_bytes()[0].is_ascii_uppercase() {
            modifiers[3] = true;
        }
        if !valid_key(&key) {
            return Err(invalid());
        }
        let mut canonical = String::new();
        for (name, enabled) in names.into_iter().zip(modifiers) {
            if enabled {
                canonical.push_str(name);
                canonical.push('-');
            }
        }
        canonical.push_str(&key);
        Ok(Self(canonical))
    }
}

fn valid_key(key: &str) -> bool {
    (key.chars().count() == 1 && key.chars().all(|c| !c.is_whitespace() && !c.is_control()))
        || matches!(
            key,
            "enter"
                | "escape"
                | "space"
                | "tab"
                | "backspace"
                | "delete"
                | "insert"
                | "home"
                | "end"
                | "pageup"
                | "pagedown"
                | "up"
                | "down"
                | "left"
                | "right"
        )
        || key.strip_prefix('f').is_some_and(|number| {
            number
                .parse::<u8>()
                .is_ok_and(|value| (1..=24).contains(&value) && number == value.to_string())
        })
}

impl TryFrom<String> for KeyChord {
    type Error = Error;

    fn try_from(value: String) -> Result<Self> {
        value.parse()
    }
}

impl From<KeyChord> for String {
    fn from(value: KeyChord) -> Self {
        value.0
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
