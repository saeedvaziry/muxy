use super::manifest::ExtensionPermission;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionApiReply {
    #[serde(rename = "requestID", skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ExtensionApiReply {
    pub fn success(request_id: Option<String>, value: Value) -> Self {
        Self {
            request_id,
            ok: true,
            value: Some(value),
            error: None,
        }
    }

    pub fn failure(request_id: Option<String>, error: impl Into<String>) -> Self {
        Self {
            request_id,
            ok: false,
            value: None,
            error: Some(error.into()),
        }
    }
}

pub const P9_BROWSER_API_METHODS: [&str; 36] = [
    "browser.open",
    "browser.navigate",
    "browser.list",
    "browser.read",
    "browser.close",
    "browser.eval",
    "browser.click",
    "browser.type",
    "browser.waitFor",
    "browser.getText",
    "browser.getHTML",
    "browser.getAttribute",
    "browser.reload",
    "browser.back",
    "browser.forward",
    "browser.waitForNavigation",
    "browser.screenshot",
    "browser.storage.get",
    "browser.storage.set",
    "browser.storage.clear",
    "browser.cookies.get",
    "browser.cookies.set",
    "browser.cookies.delete",
    "browser.cookies.clear",
    "browser.wait",
    "browser.fill",
    "browser.press",
    "browser.select",
    "browser.hover",
    "browser.scrollIntoView",
    "browser.setChecked",
    "browser.is",
    "browser.getValue",
    "browser.getCount",
    "browser.find",
    "browser.snapshot",
];

pub const P10_EXTENSION_API_METHODS: [&str; 96] = [
    "exec",
    "agents.list",
    "http.fetch",
    "dialog.confirm",
    "dialog.alert",
    "dialog.prompt",
    "dialog.pickFolder",
    "shortcuts.register",
    "shortcuts.unregister",
    "shortcuts.list",
    "storage.get",
    "storage.set",
    "storage.delete",
    "storage.keys",
    "modal.open",
    "modal.feed",
    "modal.finish",
    "modal.await",
    "modal.openWebview",
    "modal.awaitWebview",
    "modal.submitWebview",
    "modal.closeWebview",
    "extension.settings.get",
    "extension.settings.set",
    "extension.statusbar.set",
    "panel.open",
    "panel.close",
    "panel.toggle",
    "popover.close",
    "popover.resize",
    "topbar.set",
    "statusbar.set",
    "tabs.open",
    "projects.delete",
    "projects.add",
    "projects.create",
    "projects.rename",
    "projects.setColor",
    "projects.setIcon",
    "projects.setLogo",
    "projects.reorder",
    "projects.attach",
    "projects.detach",
    "workspaces.list",
    "workspaces.create",
    "workspaces.switch",
    "workspaces.rename",
    "workspaces.delete",
    "lifecycle.ackBeforeClose",
    "lifecycle.resolveBeforeClose",
    "lifecycle.closeSelf",
    "files.list",
    "files.read",
    "files.stat",
    "files.write",
    "files.mkdir",
    "files.rename",
    "files.move",
    "files.delete",
    "git.status",
    "git.diff",
    "git.repoInfo",
    "git.log",
    "git.branches",
    "git.currentBranch",
    "git.aheadBehind",
    "git.pr.info",
    "git.pr.number",
    "git.pr.diff",
    "git.pr.list",
    "git.worktrees",
    "git.init",
    "git.stage",
    "git.unstage",
    "git.discard",
    "git.commit",
    "git.push",
    "git.pull",
    "git.branch.create",
    "git.branch.switch",
    "git.pr.create",
    "git.pr.merge",
    "git.pr.close",
    "git.worktree.add",
    "git.worktree.remove",
    "git.worktree.switch",
    "git.remoteBranches",
    "git.branch.delete",
    "git.branch.deleteRemote",
    "git.checkout",
    "git.cherryPick",
    "git.revert",
    "git.tag.create",
    "git.pr.checkout",
    "git.pr.checkoutWorktree",
    "gh.user",
];

pub const WEBVIEW_CONTROL_METHODS: [&str; 23] = [
    "toast",
    "notifications.notify",
    "tabs.list",
    "tabs.switch",
    "tabs.new",
    "tabs.next",
    "tabs.previous",
    "tabs.setTitle",
    "tabs.setIcon",
    "panes.list",
    "panes.send",
    "panes.sendKeys",
    "panes.readScreen",
    "panes.close",
    "panes.rename",
    "projects.list",
    "projects.switch",
    "worktrees.list",
    "worktrees.switch",
    "worktrees.refresh",
    "events.subscribe",
    "events.unsubscribe",
    "events.emit",
];

pub const EXTENSION_EVENT_NAMES: [&str; 21] = [
    "pane.created",
    "pane.closed",
    "pane.focused",
    "worktree.offline",
    "tab.created",
    "tab.updated",
    "tab.closed",
    "tab.focused",
    "panel.opened",
    "panel.closed",
    "popover.opened",
    "popover.closed",
    "modal.opened",
    "modal.closed",
    "project.switched",
    "projects.changed",
    "worktree.switched",
    "worktree.headChanged",
    "notification.posted",
    "file.changed",
    "agent.status",
];

pub fn required_permission(method: &str) -> Option<ExtensionPermission> {
    match method {
        "panes.list" | "panes.readScreen" => Some(ExtensionPermission::PanesRead),
        "panes.send" | "panes.sendKeys" | "panes.close" | "panes.rename" => {
            Some(ExtensionPermission::PanesWrite)
        }
        "tabs.list" => Some(ExtensionPermission::TabsRead),
        "tabs.switch" | "tabs.new" | "tabs.next" | "tabs.previous" | "tabs.open"
        | "tabs.setTitle" | "tabs.setIcon" => Some(ExtensionPermission::TabsWrite),
        "browser.list"
        | "browser.read"
        | "browser.waitFor"
        | "browser.getText"
        | "browser.getHTML"
        | "browser.getAttribute"
        | "browser.waitForNavigation"
        | "browser.screenshot"
        | "browser.storage.get"
        | "browser.cookies.get"
        | "browser.wait"
        | "browser.is"
        | "browser.getValue"
        | "browser.getCount"
        | "browser.find"
        | "browser.snapshot" => Some(ExtensionPermission::BrowserRead),
        "browser.open"
        | "browser.navigate"
        | "browser.close"
        | "browser.eval"
        | "browser.click"
        | "browser.type"
        | "browser.reload"
        | "browser.back"
        | "browser.forward"
        | "browser.storage.set"
        | "browser.storage.clear"
        | "browser.cookies.set"
        | "browser.cookies.delete"
        | "browser.cookies.clear"
        | "browser.fill"
        | "browser.press"
        | "browser.select"
        | "browser.hover"
        | "browser.scrollIntoView"
        | "browser.setChecked" => Some(ExtensionPermission::BrowserWrite),
        "projects.list" | "workspaces.list" => Some(ExtensionPermission::ProjectsRead),
        "projects.switch" | "projects.add" | "projects.create" | "projects.rename"
        | "projects.setColor" | "projects.setIcon" | "projects.setLogo" | "projects.reorder"
        | "projects.attach" | "projects.detach" | "workspaces.create" | "workspaces.switch"
        | "workspaces.rename" | "workspaces.delete" => Some(ExtensionPermission::ProjectsWrite),
        "projects.delete" => Some(ExtensionPermission::ProjectsDelete),
        "worktrees.list" => Some(ExtensionPermission::WorktreesRead),
        "worktrees.switch" | "worktrees.refresh" => Some(ExtensionPermission::WorktreesWrite),
        "agents.list" => Some(ExtensionPermission::AgentsRead),
        "git.status" | "git.diff" | "git.repoInfo" | "git.log" | "git.branches"
        | "git.remoteBranches" | "git.currentBranch" | "git.aheadBehind" | "git.pr.info"
        | "git.pr.number" | "git.pr.diff" | "git.pr.list" | "git.worktrees" => {
            Some(ExtensionPermission::GitRead)
        }
        "git.init"
        | "git.stage"
        | "git.unstage"
        | "git.discard"
        | "git.commit"
        | "git.push"
        | "git.pull"
        | "git.branch.create"
        | "git.branch.switch"
        | "git.pr.create"
        | "git.pr.merge"
        | "git.pr.close"
        | "git.worktree.add"
        | "git.worktree.remove"
        | "git.worktree.switch"
        | "git.branch.delete"
        | "git.branch.deleteRemote"
        | "git.checkout"
        | "git.cherryPick"
        | "git.revert"
        | "git.tag.create"
        | "git.pr.checkout"
        | "git.pr.checkoutWorktree" => Some(ExtensionPermission::GitWrite),
        "files.list" | "files.read" | "files.stat" => Some(ExtensionPermission::FilesRead),
        "files.write" | "files.mkdir" | "files.rename" | "files.move" | "files.delete" => {
            Some(ExtensionPermission::FilesWrite)
        }
        "storage.get" | "storage.keys" => Some(ExtensionPermission::StorageRead),
        "storage.set" | "storage.delete" => Some(ExtensionPermission::StorageWrite),
        "toast" | "notifications.notify" => Some(ExtensionPermission::NotificationsWrite),
        "panel.open"
        | "panel.close"
        | "panel.toggle"
        | "popover.close"
        | "popover.resize"
        | "modal.openWebview"
        | "modal.submitWebview"
        | "modal.closeWebview"
        | "topbar.set"
        | "statusbar.set" => Some(ExtensionPermission::PanelsWrite),
        "exec" => Some(ExtensionPermission::CommandsExec),
        "shortcuts.register" | "shortcuts.unregister" => {
            Some(ExtensionPermission::ShortcutsRegister)
        }
        "gh.user" => Some(ExtensionPermission::GhRead),
        _ => None,
    }
}

pub fn required_permission_for_args(
    method: &str,
    args: &serde_json::Map<String, Value>,
) -> Option<ExtensionPermission> {
    if method == "browser.wait"
        && args
            .get("function")
            .and_then(Value::as_str)
            .is_some_and(|function| !function.is_empty())
    {
        return Some(ExtensionPermission::BrowserWrite);
    }
    required_permission(method)
}

pub fn required_event_permission(event: &str) -> Option<ExtensionPermission> {
    match event {
        "agent.status" => Some(ExtensionPermission::AgentsRead),
        "file.changed" => Some(ExtensionPermission::FilesRead),
        "projects.changed" => Some(ExtensionPermission::ProjectsRead),
        "worktree.headChanged" => Some(ExtensionPermission::WorktreesRead),
        _ => None,
    }
}

pub fn is_known_api_method(method: &str) -> bool {
    P9_BROWSER_API_METHODS.contains(&method)
        || P10_EXTENSION_API_METHODS.contains(&method)
        || WEBVIEW_CONTROL_METHODS.contains(&method)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn api_reply_shape_matches_swift_bridges() {
        let success =
            ExtensionApiReply::success(Some("call-1".to_owned()), serde_json::json!({"value": 1}));
        assert_eq!(
            serde_json::to_value(success).unwrap(),
            serde_json::json!({
                "requestID": "call-1",
                "ok": true,
                "value": {"value": 1}
            })
        );
        let failure = ExtensionApiReply::failure(None, "permission denied (git:write)");
        assert_eq!(
            serde_json::to_value(failure).unwrap(),
            serde_json::json!({
                "ok": false,
                "error": "permission denied (git:write)"
            })
        );
    }

    #[test]
    fn phase_catalogs_are_unique_and_frozen() {
        assert_eq!(P9_BROWSER_API_METHODS.len(), 36);
        assert_eq!(P10_EXTENSION_API_METHODS.len(), 96);
        assert_eq!(WEBVIEW_CONTROL_METHODS.len(), 23);
        assert_eq!(
            P9_BROWSER_API_METHODS
                .into_iter()
                .collect::<BTreeSet<_>>()
                .len(),
            36
        );
        assert_eq!(
            P10_EXTENSION_API_METHODS
                .into_iter()
                .collect::<BTreeSet<_>>()
                .len(),
            96
        );
        assert_eq!(
            WEBVIEW_CONTROL_METHODS
                .into_iter()
                .collect::<BTreeSet<_>>()
                .len(),
            23
        );
        assert!(
            P9_BROWSER_API_METHODS
                .into_iter()
                .all(|method| !P10_EXTENSION_API_METHODS.contains(&method))
        );
    }

    #[test]
    fn every_browser_method_has_a_permission() {
        for method in P9_BROWSER_API_METHODS {
            assert!(required_permission(method).is_some(), "{method}");
        }
        assert_eq!(
            required_permission("browser.wait"),
            Some(ExtensionPermission::BrowserRead)
        );
        let read_args = serde_json::from_value::<serde_json::Map<String, Value>>(
            serde_json::json!({"function": ""}),
        )
        .unwrap();
        let write_args = serde_json::from_value::<serde_json::Map<String, Value>>(
            serde_json::json!({"function": "document.readyState"}),
        )
        .unwrap();
        assert_eq!(
            required_permission_for_args("browser.wait", &read_args),
            Some(ExtensionPermission::BrowserRead)
        );
        assert_eq!(
            required_permission_for_args("browser.wait", &write_args),
            Some(ExtensionPermission::BrowserWrite)
        );
    }

    #[test]
    fn every_method_has_an_explicit_contract_decision() {
        let intentionally_ungated = BTreeSet::from([
            "http.fetch",
            "dialog.confirm",
            "dialog.alert",
            "dialog.prompt",
            "dialog.pickFolder",
            "shortcuts.list",
            "modal.open",
            "modal.feed",
            "modal.finish",
            "modal.await",
            "modal.awaitWebview",
            "extension.settings.get",
            "extension.settings.set",
            "extension.statusbar.set",
            "lifecycle.ackBeforeClose",
            "lifecycle.resolveBeforeClose",
            "lifecycle.closeSelf",
            "events.subscribe",
            "events.unsubscribe",
            "events.emit",
        ]);
        for method in P10_EXTENSION_API_METHODS
            .into_iter()
            .chain(WEBVIEW_CONTROL_METHODS)
        {
            assert!(
                required_permission(method).is_some() || intentionally_ungated.contains(method),
                "{method}"
            );
        }
    }

    #[test]
    fn event_catalog_and_gates_match_swift() {
        assert_eq!(EXTENSION_EVENT_NAMES.len(), 21);
        assert_eq!(
            EXTENSION_EVENT_NAMES
                .into_iter()
                .collect::<BTreeSet<_>>()
                .len(),
            21
        );
        assert_eq!(
            required_event_permission("agent.status"),
            Some(ExtensionPermission::AgentsRead)
        );
        assert_eq!(required_event_permission("pane.focused"), None);
    }
}
