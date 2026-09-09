#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ExtensionApiServiceGroup {
    Execution,
    Agents,
    Http,
    Dialogs,
    Shortcuts,
    Storage,
    Modals,
    Settings,
    Surfaces,
    Tabs,
    Projects,
    Workspaces,
    Lifecycle,
    Files,
    Git,
    Github,
    Browser,
    Notifications,
    Panes,
    Worktrees,
    Events,
}

impl ExtensionApiServiceGroup {
    pub const fn implementation_phase(self) -> u8 {
        match self {
            Self::Execution
            | Self::Agents
            | Self::Http
            | Self::Dialogs
            | Self::Storage
            | Self::Settings
            | Self::Projects
            | Self::Workspaces
            | Self::Files
            | Self::Git
            | Self::Github
            | Self::Panes
            | Self::Worktrees => 5,
            Self::Lifecycle => 6,
            Self::Shortcuts
            | Self::Modals
            | Self::Surfaces
            | Self::Tabs
            | Self::Notifications
            | Self::Events => 7,
            Self::Browser => 8,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Execution => "execution",
            Self::Agents => "agents",
            Self::Http => "http",
            Self::Dialogs => "dialogs",
            Self::Shortcuts => "shortcuts",
            Self::Storage => "storage",
            Self::Modals => "modals",
            Self::Settings => "settings",
            Self::Surfaces => "surfaces",
            Self::Tabs => "tabs",
            Self::Projects => "projects",
            Self::Workspaces => "workspaces",
            Self::Lifecycle => "lifecycle",
            Self::Files => "files",
            Self::Git => "git",
            Self::Github => "github",
            Self::Browser => "browser",
            Self::Notifications => "notifications",
            Self::Panes => "panes",
            Self::Worktrees => "worktrees",
            Self::Events => "events",
        }
    }
}

pub fn extension_api_service_group(method: &str) -> Option<ExtensionApiServiceGroup> {
    let group = match method {
        "exec" => ExtensionApiServiceGroup::Execution,
        "agents.list" => ExtensionApiServiceGroup::Agents,
        "http.fetch" => ExtensionApiServiceGroup::Http,
        "dialog.confirm" | "dialog.alert" | "dialog.prompt" | "dialog.pickFolder" => {
            ExtensionApiServiceGroup::Dialogs
        }
        "shortcuts.register" | "shortcuts.unregister" | "shortcuts.list" => {
            ExtensionApiServiceGroup::Shortcuts
        }
        "storage.get" | "storage.set" | "storage.delete" | "storage.keys" => {
            ExtensionApiServiceGroup::Storage
        }
        "modal.open"
        | "modal.feed"
        | "modal.finish"
        | "modal.await"
        | "modal.openWebview"
        | "modal.awaitWebview"
        | "modal.submitWebview"
        | "modal.closeWebview" => ExtensionApiServiceGroup::Modals,
        "extension.settings.get" | "extension.settings.set" | "extension.statusbar.set" => {
            ExtensionApiServiceGroup::Settings
        }
        "panel.open" | "panel.close" | "panel.toggle" | "popover.close" | "popover.resize"
        | "topbar.set" | "statusbar.set" => ExtensionApiServiceGroup::Surfaces,
        "tabs.open" | "tabs.list" | "tabs.switch" | "tabs.new" | "tabs.next" | "tabs.previous"
        | "tabs.setTitle" | "tabs.setIcon" => ExtensionApiServiceGroup::Tabs,
        "projects.delete" | "projects.add" | "projects.create" | "projects.rename"
        | "projects.setColor" | "projects.setIcon" | "projects.setLogo" | "projects.reorder"
        | "projects.attach" | "projects.detach" | "projects.list" | "projects.switch" => {
            ExtensionApiServiceGroup::Projects
        }
        "workspaces.list" | "workspaces.create" | "workspaces.switch" | "workspaces.rename"
        | "workspaces.delete" => ExtensionApiServiceGroup::Workspaces,
        "lifecycle.ackBeforeClose" | "lifecycle.resolveBeforeClose" | "lifecycle.closeSelf" => {
            ExtensionApiServiceGroup::Lifecycle
        }
        "files.list" | "files.read" | "files.stat" | "files.write" | "files.mkdir"
        | "files.rename" | "files.move" | "files.delete" => ExtensionApiServiceGroup::Files,
        method if method.starts_with("git.") => ExtensionApiServiceGroup::Git,
        "gh.user" => ExtensionApiServiceGroup::Github,
        method if method.starts_with("browser.") => ExtensionApiServiceGroup::Browser,
        "toast" | "notifications.notify" => ExtensionApiServiceGroup::Notifications,
        "panes.list" | "panes.send" | "panes.sendKeys" | "panes.readScreen" | "panes.close"
        | "panes.rename" => ExtensionApiServiceGroup::Panes,
        "worktrees.list" | "worktrees.switch" | "worktrees.refresh" => {
            ExtensionApiServiceGroup::Worktrees
        }
        "events.subscribe" | "events.unsubscribe" | "events.emit" => {
            ExtensionApiServiceGroup::Events
        }
        _ => return None,
    };
    Some(group)
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxy_core::extensions::contract::{
        P9_BROWSER_API_METHODS, P10_EXTENSION_API_METHODS, WEBVIEW_CONTROL_METHODS,
    };

    #[test]
    fn every_catalog_method_has_one_dependency_group() {
        for method in P9_BROWSER_API_METHODS
            .into_iter()
            .chain(P10_EXTENSION_API_METHODS)
            .chain(WEBVIEW_CONTROL_METHODS)
        {
            assert!(extension_api_service_group(method).is_some(), "{method}");
        }
        assert_eq!(extension_api_service_group("future.method"), None);
    }

    #[test]
    fn dependency_groups_identify_their_owning_phase() {
        assert_eq!(ExtensionApiServiceGroup::Storage.implementation_phase(), 5);
        assert_eq!(
            ExtensionApiServiceGroup::Lifecycle.implementation_phase(),
            6
        );
        assert_eq!(ExtensionApiServiceGroup::Surfaces.implementation_phase(), 7);
        assert_eq!(ExtensionApiServiceGroup::Browser.implementation_phase(), 8);
    }
}
