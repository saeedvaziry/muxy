use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::Value;
use std::collections::BTreeMap;

pub type ExtensionJson = Value;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum ExtensionPermission {
    #[serde(rename = "panes:read")]
    PanesRead,
    #[serde(rename = "panes:write")]
    PanesWrite,
    #[serde(rename = "tabs:read")]
    TabsRead,
    #[serde(rename = "tabs:write")]
    TabsWrite,
    #[serde(rename = "browser:read")]
    BrowserRead,
    #[serde(rename = "browser:write")]
    BrowserWrite,
    #[serde(rename = "projects:read")]
    ProjectsRead,
    #[serde(rename = "projects:write")]
    ProjectsWrite,
    #[serde(rename = "projects:delete")]
    ProjectsDelete,
    #[serde(rename = "worktrees:read")]
    WorktreesRead,
    #[serde(rename = "worktrees:write")]
    WorktreesWrite,
    #[serde(rename = "agents:read")]
    AgentsRead,
    #[serde(rename = "git:read")]
    GitRead,
    #[serde(rename = "git:write")]
    GitWrite,
    #[serde(rename = "files:read")]
    FilesRead,
    #[serde(rename = "files:write")]
    FilesWrite,
    #[serde(rename = "storage:read")]
    StorageRead,
    #[serde(rename = "storage:write")]
    StorageWrite,
    #[serde(rename = "notifications:write")]
    NotificationsWrite,
    #[serde(rename = "panels:write")]
    PanelsWrite,
    #[serde(rename = "commands:run-script")]
    CommandsRunScript,
    #[serde(rename = "commands:exec")]
    CommandsExec,
    #[serde(rename = "shortcuts:register")]
    ShortcutsRegister,
    #[serde(rename = "remote:serve")]
    RemoteServe,
    #[serde(rename = "gh:read")]
    GhRead,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionKind {
    Read,
    Write,
    Action,
}

impl ExtensionPermission {
    pub const ALL: [Self; 25] = [
        Self::PanesRead,
        Self::PanesWrite,
        Self::TabsRead,
        Self::TabsWrite,
        Self::BrowserRead,
        Self::BrowserWrite,
        Self::ProjectsRead,
        Self::ProjectsWrite,
        Self::ProjectsDelete,
        Self::WorktreesRead,
        Self::WorktreesWrite,
        Self::AgentsRead,
        Self::GitRead,
        Self::GitWrite,
        Self::FilesRead,
        Self::FilesWrite,
        Self::StorageRead,
        Self::StorageWrite,
        Self::NotificationsWrite,
        Self::PanelsWrite,
        Self::CommandsRunScript,
        Self::CommandsExec,
        Self::ShortcutsRegister,
        Self::RemoteServe,
        Self::GhRead,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PanesRead => "panes:read",
            Self::PanesWrite => "panes:write",
            Self::TabsRead => "tabs:read",
            Self::TabsWrite => "tabs:write",
            Self::BrowserRead => "browser:read",
            Self::BrowserWrite => "browser:write",
            Self::ProjectsRead => "projects:read",
            Self::ProjectsWrite => "projects:write",
            Self::ProjectsDelete => "projects:delete",
            Self::WorktreesRead => "worktrees:read",
            Self::WorktreesWrite => "worktrees:write",
            Self::AgentsRead => "agents:read",
            Self::GitRead => "git:read",
            Self::GitWrite => "git:write",
            Self::FilesRead => "files:read",
            Self::FilesWrite => "files:write",
            Self::StorageRead => "storage:read",
            Self::StorageWrite => "storage:write",
            Self::NotificationsWrite => "notifications:write",
            Self::PanelsWrite => "panels:write",
            Self::CommandsRunScript => "commands:run-script",
            Self::CommandsExec => "commands:exec",
            Self::ShortcutsRegister => "shortcuts:register",
            Self::RemoteServe => "remote:serve",
            Self::GhRead => "gh:read",
        }
    }

    pub const fn kind(self) -> PermissionKind {
        match self {
            Self::PanesRead
            | Self::TabsRead
            | Self::BrowserRead
            | Self::ProjectsRead
            | Self::WorktreesRead
            | Self::AgentsRead
            | Self::GitRead
            | Self::GhRead
            | Self::FilesRead
            | Self::StorageRead => PermissionKind::Read,
            Self::PanesWrite
            | Self::TabsWrite
            | Self::BrowserWrite
            | Self::ProjectsWrite
            | Self::ProjectsDelete
            | Self::WorktreesWrite
            | Self::GitWrite
            | Self::FilesWrite
            | Self::StorageWrite
            | Self::NotificationsWrite
            | Self::PanelsWrite => PermissionKind::Write,
            Self::CommandsRunScript
            | Self::CommandsExec
            | Self::ShortcutsRegister
            | Self::RemoteServe => PermissionKind::Action,
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::RemoteServe => "remote-api",
            permission => permission.as_str(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExtensionIcon {
    Symbol(String),
    Svg(String),
}

impl<'de> Deserialize<'de> for ExtensionIcon {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match value {
            Value::String(symbol) if !symbol.is_empty() => Ok(Self::Symbol(symbol)),
            Value::String(_) => Err(de::Error::custom("icon symbol must not be empty")),
            Value::Object(values) if values.len() == 1 => {
                if let Some(Value::String(symbol)) = values.get("symbol")
                    && !symbol.is_empty()
                {
                    return Ok(Self::Symbol(symbol.clone()));
                }
                if let Some(Value::String(svg)) = values.get("svg")
                    && !svg.is_empty()
                {
                    return Ok(Self::Svg(svg.clone()));
                }
                Err(de::Error::custom(
                    "icon requires one non-empty 'symbol' or 'svg' field",
                ))
            }
            Value::Object(_) => Err(de::Error::custom(
                "icon requires exactly one 'symbol' or 'svg' field",
            )),
            _ => Err(de::Error::custom("icon must be a string or object")),
        }
    }
}

impl Serialize for ExtensionIcon {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let values = match self {
            Self::Symbol(value) => BTreeMap::from([("symbol", value)]),
            Self::Svg(value) => BTreeMap::from([("svg", value)]),
        };
        values.serialize(serializer)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionTabType {
    pub id: String,
    pub title: String,
    pub entry: String,
    #[serde(default)]
    pub default_data: Option<ExtensionJson>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionHomeView {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub icon: Option<ExtensionIcon>,
    pub entry: String,
    #[serde(default)]
    pub default_data: Option<ExtensionJson>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PanelPosition {
    #[default]
    Right,
    Bottom,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PanelMode {
    Pinned,
    #[default]
    Floating,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PanelHeaderControl {
    Icon,
    Title,
    Close,
    Pin,
    Position,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionPanelHeaderButton {
    pub id: String,
    pub icon: ExtensionIcon,
    #[serde(default)]
    pub tooltip: Option<String>,
    pub command: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionPanel {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub icon: Option<ExtensionIcon>,
    pub entry: String,
    #[serde(default)]
    pub position: PanelPosition,
    #[serde(default)]
    pub mode: PanelMode,
    #[serde(default)]
    pub hidden_controls: Vec<PanelHeaderControl>,
    #[serde(default)]
    pub header_buttons: Vec<ExtensionPanelHeaderButton>,
    #[serde(default)]
    pub hide_topbar: bool,
    #[serde(default)]
    pub default_data: Option<ExtensionJson>,
}

fn default_popover_width() -> f64 {
    320.0
}

fn default_popover_height() -> f64 {
    360.0
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionPopover {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    pub entry: String,
    #[serde(default = "default_popover_width")]
    pub width: f64,
    #[serde(default = "default_popover_height")]
    pub height: f64,
    #[serde(default)]
    pub default_data: Option<ExtensionJson>,
}

impl ExtensionPopover {
    pub const DEFAULT_WIDTH: f64 = 320.0;
    pub const DEFAULT_HEIGHT: f64 = 360.0;
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionSidebar {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub icon: Option<ExtensionIcon>,
    pub entry: String,
    #[serde(default)]
    pub default_data: Option<ExtensionJson>,
}

fn default_patterns() -> Vec<String> {
    vec!["*".to_owned()]
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionFileOpener {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    pub tab_type: String,
    #[serde(default = "default_patterns")]
    pub patterns: Vec<String>,
    #[serde(default = "default_true")]
    pub singleton: bool,
}

impl ExtensionFileOpener {
    pub fn matches(&self, relative_path: &str) -> bool {
        let default_pattern = ["*".to_owned()];
        let patterns = if self.patterns.is_empty() {
            default_pattern.as_slice()
        } else {
            self.patterns.as_slice()
        };
        patterns
            .iter()
            .any(|pattern| wildcard_matches(pattern, relative_path))
    }
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.to_lowercase().chars().collect::<Vec<_>>();
    let value = value.to_lowercase().chars().collect::<Vec<_>>();
    let mut row = vec![false; value.len() + 1];
    row[0] = true;
    for token in pattern {
        let mut next = vec![false; value.len() + 1];
        if token == '*' {
            next[0] = row[0];
            for index in 1..=value.len() {
                next[index] = row[index] || next[index - 1];
            }
        } else {
            for index in 1..=value.len() {
                next[index] = row[index - 1] && (token == '?' || token == value[index - 1]);
            }
        }
        row = next;
    }
    row[value.len()]
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionLocalization {
    pub id: String,
    pub language: String,
    pub title: String,
    pub bundle: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionTopbarItem {
    pub id: String,
    pub icon: ExtensionIcon,
    #[serde(default)]
    pub tooltip: Option<String>,
    pub command: String,
    #[serde(default = "default_true")]
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StatusBarSide {
    Left,
    Right,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionStatusBarItem {
    pub id: String,
    pub icon: ExtensionIcon,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub tooltip: Option<String>,
    pub side: StatusBarSide,
    pub command: String,
    #[serde(default = "default_true")]
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtensionSettingType {
    String,
    Bool,
    Number,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionSettingEntry {
    pub key: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub setting_type: ExtensionSettingType,
    #[serde(default)]
    pub default_value: Option<ExtensionJson>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ExtensionCommandAction {
    #[default]
    Event,
    OpenTab {
        #[serde(rename = "tabType")]
        tab_type: String,
        #[serde(default)]
        data: Option<ExtensionJson>,
    },
    TogglePanel {
        panel: String,
    },
    OpenPopover {
        popover: String,
    },
    OpenModal {
        entry: String,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        height: Option<f64>,
        #[serde(default = "default_true")]
        dismiss_on_outside_click: bool,
        #[serde(default)]
        data: Option<ExtensionJson>,
    },
    RunScript {
        script: String,
    },
}

impl ExtensionCommandAction {
    pub const fn is_anchored(&self) -> bool {
        matches!(self, Self::OpenPopover { .. })
    }

    pub const fn is_rail_eligible(&self) -> bool {
        matches!(self, Self::TogglePanel { .. })
    }

    pub const fn required_permission(&self) -> Option<ExtensionPermission> {
        match self {
            Self::Event => None,
            Self::OpenTab { .. } => Some(ExtensionPermission::TabsWrite),
            Self::TogglePanel { .. } | Self::OpenPopover { .. } | Self::OpenModal { .. } => {
                Some(ExtensionPermission::PanelsWrite)
            }
            Self::RunScript { .. } => Some(ExtensionPermission::CommandsRunScript),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionPaletteCommand {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub action: ExtensionCommandAction,
    #[serde(default)]
    pub default_shortcut: Option<String>,
}

impl ExtensionPaletteCommand {
    pub fn event_name(&self) -> String {
        format!("command.{}", self.id)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionRemoteMethod {
    pub id: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MuxyManifestBody {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub commands: Vec<ExtensionPaletteCommand>,
    #[serde(default)]
    pub tab_types: Vec<ExtensionTabType>,
    #[serde(default)]
    pub home_views: Vec<ExtensionHomeView>,
    #[serde(default)]
    pub panels: Vec<ExtensionPanel>,
    #[serde(default)]
    pub popovers: Vec<ExtensionPopover>,
    #[serde(default)]
    pub sidebar: Option<ExtensionSidebar>,
    #[serde(default)]
    pub file_openers: Vec<ExtensionFileOpener>,
    #[serde(default)]
    pub localizations: Vec<ExtensionLocalization>,
    #[serde(default)]
    pub permissions: Vec<ExtensionPermission>,
    #[serde(default)]
    pub topbar_items: Vec<ExtensionTopbarItem>,
    #[serde(default)]
    pub status_bar_items: Vec<ExtensionStatusBarItem>,
    #[serde(default)]
    pub settings: Vec<ExtensionSettingEntry>,
    #[serde(default)]
    pub remote_methods: Vec<ExtensionRemoteMethod>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PackageManifest {
    pub name: String,
    pub version: String,
    pub muxy: MuxyManifestBody,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionManifest {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub background: Option<String>,
    pub events: Vec<String>,
    pub commands: Vec<ExtensionPaletteCommand>,
    pub tab_types: Vec<ExtensionTabType>,
    pub home_views: Vec<ExtensionHomeView>,
    pub panels: Vec<ExtensionPanel>,
    pub popovers: Vec<ExtensionPopover>,
    pub sidebar: Option<ExtensionSidebar>,
    pub file_openers: Vec<ExtensionFileOpener>,
    pub localizations: Vec<ExtensionLocalization>,
    pub permissions: Vec<ExtensionPermission>,
    pub topbar_items: Vec<ExtensionTopbarItem>,
    pub status_bar_items: Vec<ExtensionStatusBarItem>,
    pub settings: Vec<ExtensionSettingEntry>,
    pub remote_methods: Vec<ExtensionRemoteMethod>,
}

impl From<PackageManifest> for ExtensionManifest {
    fn from(package: PackageManifest) -> Self {
        let body = package.muxy;
        Self {
            name: package.name,
            version: package.version,
            description: body.description,
            background: body.background,
            events: body.events,
            commands: body.commands,
            tab_types: body.tab_types,
            home_views: body.home_views,
            panels: body.panels,
            popovers: body.popovers,
            sidebar: body.sidebar,
            file_openers: body.file_openers,
            localizations: body.localizations,
            permissions: body.permissions,
            topbar_items: body.topbar_items,
            status_bar_items: body.status_bar_items,
            settings: body.settings,
            remote_methods: body.remote_methods,
        }
    }
}

impl ExtensionManifest {
    pub fn tab_type(&self, id: &str) -> Option<&ExtensionTabType> {
        self.tab_types.iter().find(|item| item.id == id)
    }

    pub fn home_view(&self, id: &str) -> Option<&ExtensionHomeView> {
        self.home_views.iter().find(|item| item.id == id)
    }

    pub fn panel(&self, id: &str) -> Option<&ExtensionPanel> {
        self.panels.iter().find(|item| item.id == id)
    }

    pub fn popover(&self, id: &str) -> Option<&ExtensionPopover> {
        self.popovers.iter().find(|item| item.id == id)
    }

    pub fn file_opener(&self, id: &str) -> Option<&ExtensionFileOpener> {
        self.file_openers.iter().find(|item| item.id == id)
    }

    pub fn localization(&self, id: &str) -> Option<&ExtensionLocalization> {
        self.localizations.iter().find(|item| item.id == id)
    }

    pub fn setting(&self, key: &str) -> Option<&ExtensionSettingEntry> {
        self.settings.iter().find(|item| item.key == key)
    }

    pub fn status_bar_item(&self, id: &str) -> Option<&ExtensionStatusBarItem> {
        self.status_bar_items.iter().find(|item| item.id == id)
    }

    pub fn topbar_item(&self, id: &str) -> Option<&ExtensionTopbarItem> {
        self.topbar_items.iter().find(|item| item.id == id)
    }

    pub fn remote_method(&self, id: &str) -> Option<&ExtensionRemoteMethod> {
        self.remote_methods.iter().find(|item| item.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_manifest_defaults_match_swift() {
        let package: PackageManifest =
            serde_json::from_str(r#"{"name":"minimal","version":"1.0.0","muxy":{}}"#).unwrap();
        let manifest = ExtensionManifest::from(package);
        assert!(manifest.events.is_empty());
        assert!(manifest.permissions.is_empty());
        assert!(manifest.commands.is_empty());
        assert!(manifest.remote_methods.is_empty());
    }

    #[test]
    fn icons_accept_swift_forms_and_encode_as_objects() {
        let symbol: ExtensionIcon = serde_json::from_str(r#""bolt""#).unwrap();
        let svg: ExtensionIcon = serde_json::from_str(r#"{"svg":"icon.svg"}"#).unwrap();
        assert_eq!(symbol, ExtensionIcon::Symbol("bolt".to_owned()));
        assert_eq!(svg, ExtensionIcon::Svg("icon.svg".to_owned()));
        assert_eq!(
            serde_json::to_string(&symbol).unwrap(),
            r#"{"symbol":"bolt"}"#
        );
        assert!(serde_json::from_str::<ExtensionIcon>(r#"{"symbol":"a","svg":"b"}"#).is_err());
        assert!(serde_json::from_str::<ExtensionIcon>(r#""""#).is_err());
    }

    #[test]
    fn defaults_and_command_permissions_match_swift() {
        let body: MuxyManifestBody = serde_json::from_str(
            r#"{
                "panels":[{"id":"p","entry":"panel.html"}],
                "popovers":[{"id":"o","entry":"popover.html"}],
                "fileOpeners":[{"id":"f","tabType":"tab"}],
                "commands":[
                    {"id":"event","title":"Event"},
                    {"id":"tab","title":"Tab","action":{"kind":"openTab","tabType":"tab"}},
                    {"id":"panel","title":"Panel","action":{"kind":"togglePanel","panel":"p"}},
                    {"id":"script","title":"Script","action":{"kind":"runScript","script":"script.js"}},
                    {"id":"modal","title":"Modal","action":{"kind":"openModal","entry":"modal.html"}}
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(body.panels[0].position, PanelPosition::Right);
        assert_eq!(body.panels[0].mode, PanelMode::Floating);
        assert_eq!(body.popovers[0].width, ExtensionPopover::DEFAULT_WIDTH);
        assert_eq!(body.popovers[0].height, ExtensionPopover::DEFAULT_HEIGHT);
        assert_eq!(body.file_openers[0].patterns, ["*"]);
        assert!(body.file_openers[0].singleton);
        assert_eq!(body.commands[0].action.required_permission(), None);
        assert_eq!(
            body.commands[1].action.required_permission(),
            Some(ExtensionPermission::TabsWrite)
        );
        assert_eq!(
            body.commands[2].action.required_permission(),
            Some(ExtensionPermission::PanelsWrite)
        );
        assert_eq!(
            body.commands[3].action.required_permission(),
            Some(ExtensionPermission::CommandsRunScript)
        );
        let ExtensionCommandAction::OpenModal {
            dismiss_on_outside_click,
            ..
        } = body.commands[4].action
        else {
            panic!("expected modal action");
        };
        assert!(dismiss_on_outside_click);
    }

    #[test]
    fn file_opener_wildcards_are_case_insensitive() {
        let opener = ExtensionFileOpener {
            id: "code".to_owned(),
            title: None,
            tab_type: "editor".to_owned(),
            patterns: vec!["src/*.RS".to_owned(), "README.?d".to_owned()],
            singleton: true,
        };
        assert!(opener.matches("src/main.rs"));
        assert!(opener.matches("readme.md"));
        assert!(!opener.matches("notes.txt"));
    }

    #[test]
    fn permission_inventory_is_complete_and_classified() {
        assert_eq!(ExtensionPermission::ALL.len(), 25);
        assert_eq!(ExtensionPermission::PanesRead.kind(), PermissionKind::Read);
        assert_eq!(ExtensionPermission::GitWrite.kind(), PermissionKind::Write);
        assert_eq!(
            ExtensionPermission::RemoteServe.kind(),
            PermissionKind::Action
        );
        assert_eq!(
            ExtensionPermission::RemoteServe.display_name(),
            "remote-api"
        );
        for permission in ExtensionPermission::ALL {
            let encoded = serde_json::to_string(&permission).unwrap();
            let decoded: ExtensionPermission = serde_json::from_str(&encoded).unwrap();
            assert_eq!(decoded, permission);
        }
    }
}
