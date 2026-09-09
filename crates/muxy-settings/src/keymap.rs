use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, de};

use crate::{Error, KeyChord, Result};
use muxy_core::shortcuts::{self, Shortcut, ShortcutSettings};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    NewHomeTab,
    ToggleSidebar,
    ToggleFullScreen,
    ToggleThemePicker,
    NavigateBack,
    NavigateForward,
    Quit,
    HideApp,
    HideOthers,
    Minimize,

    NewTab,
    CloseTab,
    SplitRight,
    SplitDown,
    FocusPaneLeft,
    FocusPaneRight,
    FocusPaneUp,
    FocusPaneDown,
    ToggleZoomPane,
    ClosePane,

    NextTab,
    PreviousTab,
    PreviousProject,
    NextProject,
    AddProject,
    SelectTab1,
    SelectTab2,
    SelectTab3,
    SelectTab4,
    SelectTab5,
    SelectTab6,
    SelectTab7,
    SelectTab8,
    SelectTab9,
    Copy,
    Paste,
    Find,
    FindNext,
    FindPrevious,
    ScrollToBottom,
    IncreaseFontSize,
    DecreaseFontSize,
}

impl Action {
    pub const ALL: [Self; 42] = [
        Self::NewHomeTab,
        Self::ToggleSidebar,
        Self::ToggleFullScreen,
        Self::ToggleThemePicker,
        Self::NavigateBack,
        Self::NavigateForward,
        Self::Quit,
        Self::HideApp,
        Self::HideOthers,
        Self::Minimize,
        Self::NewTab,
        Self::CloseTab,
        Self::SplitRight,
        Self::SplitDown,
        Self::FocusPaneLeft,
        Self::FocusPaneRight,
        Self::FocusPaneUp,
        Self::FocusPaneDown,
        Self::ToggleZoomPane,
        Self::ClosePane,
        Self::NextTab,
        Self::PreviousTab,
        Self::PreviousProject,
        Self::NextProject,
        Self::AddProject,
        Self::SelectTab1,
        Self::SelectTab2,
        Self::SelectTab3,
        Self::SelectTab4,
        Self::SelectTab5,
        Self::SelectTab6,
        Self::SelectTab7,
        Self::SelectTab8,
        Self::SelectTab9,
        Self::Copy,
        Self::Paste,
        Self::Find,
        Self::FindNext,
        Self::FindPrevious,
        Self::ScrollToBottom,
        Self::IncreaseFontSize,
        Self::DecreaseFontSize,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::NewHomeTab => "new_home_tab",
            Self::ToggleSidebar => "toggle_sidebar",
            Self::ToggleFullScreen => "toggle_full_screen",
            Self::ToggleThemePicker => "toggle_theme_picker",
            Self::NavigateBack => "navigate_back",
            Self::NavigateForward => "navigate_forward",
            Self::Quit => "quit",
            Self::HideApp => "hide_app",
            Self::HideOthers => "hide_others",
            Self::Minimize => "minimize",
            Self::NewTab => "new_tab",
            Self::CloseTab => "close_tab",
            Self::SplitRight => "split_right",
            Self::SplitDown => "split_down",
            Self::FocusPaneLeft => "focus_pane_left",
            Self::FocusPaneRight => "focus_pane_right",
            Self::FocusPaneUp => "focus_pane_up",
            Self::FocusPaneDown => "focus_pane_down",
            Self::ToggleZoomPane => "toggle_zoom_pane",
            Self::ClosePane => "close_pane",

            Self::NextTab => "next_tab",
            Self::PreviousTab => "previous_tab",
            Self::PreviousProject => "previous_project",
            Self::NextProject => "next_project",
            Self::AddProject => "add_project",
            Self::SelectTab1 => "select_tab1",
            Self::SelectTab2 => "select_tab2",
            Self::SelectTab3 => "select_tab3",
            Self::SelectTab4 => "select_tab4",
            Self::SelectTab5 => "select_tab5",
            Self::SelectTab6 => "select_tab6",
            Self::SelectTab7 => "select_tab7",
            Self::SelectTab8 => "select_tab8",
            Self::SelectTab9 => "select_tab9",
            Self::Copy => "copy",
            Self::Paste => "paste",
            Self::Find => "find",
            Self::FindNext => "find_next",
            Self::FindPrevious => "find_previous",
            Self::ScrollToBottom => "scroll_to_bottom",
            Self::IncreaseFontSize => "increase_font_size",
            Self::DecreaseFontSize => "decrease_font_size",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Keymap(BTreeMap<String, KeyChord>);

impl Default for Keymap {
    fn default() -> Self {
        Self(
            shortcuts::ALL
                .iter()
                .filter_map(|shortcut| {
                    shortcut
                        .keys
                        .first()
                        .map(|key| (shortcut.id.to_owned(), KeyChord::default_binding(key)))
                })
                .collect(),
        )
    }
}

impl Keymap {
    pub fn chord(&self, action: Action) -> Option<&KeyChord> {
        self.0.get(action.name())
    }

    pub fn action(&self, chord: &KeyChord) -> Option<Action> {
        Action::ALL
            .into_iter()
            .find(|action| self.chord(*action) == Some(chord))
    }

    fn from_overrides(overrides: BTreeMap<String, String>) -> Result<Self> {
        let mut keymap = Self::default();
        let mut explicit = BTreeMap::new();
        for (name, value) in overrides {
            let key = format!("keymap.{name}");
            let shortcut =
                shortcuts::find(&name).ok_or_else(|| Error::new(&key, "unknown action"))?;
            let chord = value
                .parse()
                .map_err(|error: Error| Error::new(&key, error))?;
            explicit.insert(shortcut.id.to_owned(), chord);
        }
        for action in [
            Action::NewHomeTab,
            Action::ToggleSidebar,
            Action::ToggleFullScreen,
            Action::ToggleThemePicker,
            Action::NavigateBack,
            Action::NavigateForward,
            Action::Quit,
            Action::HideApp,
            Action::HideOthers,
            Action::Minimize,
            Action::Find,
            Action::FindNext,
            Action::FindPrevious,
            Action::PreviousProject,
            Action::NextProject,
            Action::AddProject,
            Action::CloseTab,
            Action::SplitRight,
            Action::SplitDown,
            Action::FocusPaneLeft,
            Action::FocusPaneRight,
            Action::FocusPaneUp,
            Action::FocusPaneDown,
            Action::ToggleZoomPane,
            Action::ClosePane,
        ] {
            if !explicit.contains_key(action.name())
                && explicit.iter().any(|(id, chord)| {
                    Some(chord) == keymap.chord(action) && same_scope(id, action.name())
                })
            {
                keymap.0.remove(action.name());
            }
        }
        keymap.0.extend(explicit);
        let bindings: Vec<_> = keymap.0.iter().collect();
        for (index, (id, chord)) in bindings.iter().enumerate() {
            for (other, other_chord) in &bindings[..index] {
                if chord == other_chord && same_scope(id, other) {
                    return Err(Error::new(
                        format!("keymap.{id}"),
                        format!("{chord} is also bound to keymap.{other}"),
                    ));
                }
            }
        }
        Ok(keymap)
    }
}

impl<'de> Deserialize<'de> for Keymap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        Self::from_overrides(BTreeMap::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

fn overlaps(left: &Shortcut, right: &Shortcut) -> bool {
    left.contexts.iter().any(|context| {
        right
            .contexts
            .iter()
            .any(|other| context_overlap(*context, *other))
    })
}

fn same_scope(left: &str, right: &str) -> bool {
    shortcuts::find(left)
        .zip(shortcuts::find(right))
        .is_some_and(|(left, right)| overlaps(left, right))
}

impl ShortcutSettings for Keymap {
    fn keys(&self, id: &str, context: Option<&str>) -> Vec<String> {
        let Some(primary) = self.0.get(id) else {
            return Vec::new();
        };
        let Some(shortcut) = shortcuts::find(id) else {
            return Vec::new();
        };
        let is_default = shortcut.keys.first().is_some_and(|key| {
            key.parse::<KeyChord>()
                .is_ok_and(|default| default == *primary)
        });
        if !is_default {
            return vec![primary.as_str().to_owned()];
        }
        shortcut
            .keys
            .iter()
            .zip(shortcut.key_contexts)
            .filter(|(_, scopes)| scopes.contains(&context))
            .filter_map(|(key, _)| key.parse::<KeyChord>().ok())
            .filter(|key| {
                !self.0.iter().any(|(other, chord)| {
                    other != id && chord == key && primary_applies(other, chord, context)
                })
            })
            .map(|key| key.as_str().to_owned())
            .collect()
    }
}

fn context_overlap(left: Option<&str>, right: Option<&str>) -> bool {
    fn scope(context: Option<&str>) -> Option<&str> {
        context.map(|context| {
            if context == shortcuts::WORKSPACE_CLIPBOARD_CONTEXT {
                "WorkspaceTabs"
            } else {
                context
            }
        })
    }
    left.is_none() || right.is_none() || scope(left) == scope(right)
}

fn primary_applies(id: &str, chord: &KeyChord, context: Option<&str>) -> bool {
    let Some(shortcut) = shortcuts::find(id) else {
        return false;
    };
    let is_default = shortcut.keys.first().is_some_and(|key| {
        key.parse::<KeyChord>()
            .is_ok_and(|default| default == *chord)
    });
    let contexts = if is_default {
        shortcut.key_contexts[0]
    } else {
        shortcut.contexts
    };
    contexts
        .iter()
        .any(|other| context_overlap(context, *other))
}
