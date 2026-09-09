use muxy_core::extensions::contract::{
    P9_BROWSER_API_METHODS, P10_EXTENSION_API_METHODS, WEBVIEW_CONTROL_METHODS, is_known_api_method,
};
use muxy_core::extensions::loader::{ExtensionLoadError, load_extension};
use muxy_core::extensions::manifest::{
    ExtensionCommandAction, ExtensionPermission, ExtensionSettingType, PanelMode, PanelPosition,
    StatusBarSide,
};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/extensions")
        .join(name)
}

fn quoted_calls(source: &'static str, function: &str) -> BTreeSet<&'static str> {
    let marker = format!("{function}('");
    let mut remaining = source;
    let mut methods = BTreeSet::new();
    while let Some(offset) = remaining.find(&marker) {
        remaining = &remaining[offset + marker.len()..];
        let Some(end) = remaining.find('\'') else {
            break;
        };
        methods.insert(&remaining[..end]);
        remaining = &remaining[end + 1..];
    }
    methods
}

#[test]
fn full_manifest_fixture_loads_every_supported_field() {
    let extension = load_extension(fixture("full-root")).unwrap();
    let manifest = &extension.manifest;
    assert_eq!(extension.id, "full-root");
    assert_eq!(extension.legacy_enabled, Some(false));
    assert_eq!(manifest.version, "1.16.0");
    assert_eq!(manifest.events.len(), 21);
    assert_eq!(manifest.commands.len(), 6);
    assert_eq!(manifest.tab_types.len(), 1);
    assert_eq!(manifest.home_views.len(), 1);
    assert_eq!(manifest.panels.len(), 1);
    assert_eq!(manifest.popovers.len(), 1);
    assert!(manifest.sidebar.is_some());
    assert_eq!(manifest.file_openers.len(), 1);
    assert_eq!(manifest.localizations.len(), 1);
    assert_eq!(manifest.permissions.len(), ExtensionPermission::ALL.len());
    assert_eq!(manifest.topbar_items.len(), 1);
    assert_eq!(manifest.status_bar_items.len(), 1);
    assert_eq!(manifest.settings.len(), 3);
    assert_eq!(manifest.remote_methods.len(), 1);
    assert_eq!(manifest.panels[0].position, PanelPosition::Bottom);
    assert_eq!(manifest.panels[0].mode, PanelMode::Pinned);
    assert_eq!(manifest.status_bar_items[0].side, StatusBarSide::Right);
    assert_eq!(
        manifest.settings[0].setting_type,
        ExtensionSettingType::String
    );
    assert_eq!(
        manifest.settings[1].setting_type,
        ExtensionSettingType::Bool
    );
    assert_eq!(
        manifest.settings[2].setting_type,
        ExtensionSettingType::Number
    );
    assert!(matches!(
        manifest.commands[1].action,
        ExtensionCommandAction::OpenTab { .. }
    ));
    assert!(matches!(
        manifest.commands[2].action,
        ExtensionCommandAction::TogglePanel { .. }
    ));
    assert!(matches!(
        manifest.commands[3].action,
        ExtensionCommandAction::OpenPopover { .. }
    ));
    assert!(matches!(
        manifest.commands[4].action,
        ExtensionCommandAction::OpenModal { .. }
    ));
    assert!(matches!(
        manifest.commands[5].action,
        ExtensionCommandAction::RunScript { .. }
    ));
    let encoded = serde_json::to_vec(manifest).unwrap();
    let decoded = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(*manifest, decoded);
}

#[test]
fn dist_and_invalid_package_fixtures_have_frozen_results() {
    let built = load_extension(fixture("full-dist")).unwrap();
    assert_eq!(built.id, "full-dist");
    assert!(built.resource_root.ends_with("full-dist/dist"));
    assert!(matches!(
        load_extension(fixture("invalid-traversal")),
        Err(ExtensionLoadError::BackgroundScriptOutsideDirectory(_))
    ));
    assert_eq!(
        load_extension(fixture("invalid-duplicates")),
        Err(ExtensionLoadError::DuplicateTabType("same".to_owned()))
    );
    assert_eq!(
        load_extension(fixture("invalid-references")),
        Err(ExtensionLoadError::CommandReferencesUnknownTabType {
            command_id: "open-missing".to_owned(),
            tab_type: "missing".to_owned(),
        })
    );
}

#[test]
fn schema_permission_inventory_matches_the_runtime_model() {
    let schema: Value = serde_json::from_str(include_str!(
        "../../../docs/extensions/schema/manifest.schema.json"
    ))
    .unwrap();
    let schema_permissions = schema["$defs"]["permission"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<BTreeSet<_>>();
    let runtime_permissions = ExtensionPermission::ALL
        .into_iter()
        .map(ExtensionPermission::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(schema_permissions, runtime_permissions);
}

#[test]
fn swift_bridge_verbs_are_present_in_the_authoritative_catalog() {
    let webview = include_str!("../../../Muxy/Services/Extensions/ExtensionWebBridge.swift");
    let shared = include_str!("../../../MuxyShared/ExtensionBridgeJS.swift");
    let webview_methods = quoted_calls(webview, "send");
    let shared_methods = quoted_calls(shared, "dispatch");
    assert!(webview_methods.len() >= 120);
    assert!(shared_methods.len() >= 115);
    for method in webview_methods.union(&shared_methods) {
        assert!(
            is_known_api_method(method),
            "missing bridge method {method}"
        );
    }
    for method in P9_BROWSER_API_METHODS {
        assert!(
            shared_methods.contains(method),
            "missing browser bridge method {method}"
        );
    }
    let socket_only = BTreeSet::from([
        "extension.settings.get",
        "extension.settings.set",
        "extension.statusbar.set",
    ]);
    for method in P10_EXTENSION_API_METHODS {
        assert!(
            webview_methods.contains(method)
                || shared_methods.contains(method)
                || socket_only.contains(method),
            "catalog method has no Swift bridge or socket owner {method}"
        );
    }
    assert_eq!(WEBVIEW_CONTROL_METHODS.len(), 23);
}
