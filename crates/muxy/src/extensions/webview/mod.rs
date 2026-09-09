mod fixture;
mod lifecycle;

pub(crate) use fixture::{prepare_phase_6_fixture, provision_phase_6_fixture_tab};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod unsupported;

#[cfg(target_os = "macos")]
pub(crate) use macos::ExtensionWebViewRegistry;
#[cfg(not(target_os = "macos"))]
pub(crate) use unsupported::ExtensionWebViewRegistry;

pub(crate) use lifecycle::{
    ExtensionLifecycleCoordinator, ExtensionLifecycleVerdict, LIFECYCLE_ACKNOWLEDGEMENT_TIMEOUT,
};

use serde_json::Value;
use std::path::PathBuf;

use crate::extensions::surfaces::{ExtensionSurfaceKind, ExtensionSurfaceRegistry};

pub(crate) const WEBVIEW_MESSAGE_HANDLER_NAME: &str = "muxy";
pub(crate) const PHASE_6_FIXTURE_ENV: &str = "MUXY_EXTENSION_WEBVIEW_FIXTURE";

pub(crate) enum ExtensionWebViewEvent {
    Api(ExtensionWebViewApiCall),
    FocusRequested { surface_id: String },
}

pub(crate) struct ExtensionWebViewApiCall {
    pub extension_id: String,
    pub surface_id: String,
    pub message: Value,
    completion: Option<Box<dyn FnOnce(Value)>>,
}

impl ExtensionWebViewApiCall {
    pub(crate) fn new(
        extension_id: String,
        surface_id: String,
        message: Value,
        completion: impl FnOnce(Value) + 'static,
    ) -> Self {
        Self {
            extension_id,
            surface_id,
            message,
            completion: Some(Box::new(completion)),
        }
    }

    pub(crate) fn complete(mut self, reply: Value) {
        if let Some(completion) = self.completion.take() {
            completion(reply);
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ExtensionWebViewDescriptor {
    pub extension_id: String,
    pub instance_id: String,
    pub entry_url: String,
    pub resource_root: PathBuf,
    pub data: Value,
    pub kind: ExtensionSurfaceKind,
}

pub(crate) fn extension_webview_descriptors(
    registry: &ExtensionSurfaceRegistry,
) -> Vec<ExtensionWebViewDescriptor> {
    let mut descriptors = registry
        .webview_surfaces()
        .map(|surface| ExtensionWebViewDescriptor {
            extension_id: surface.extension_id.clone(),
            instance_id: surface.instance_id.clone(),
            entry_url: surface.entry_url.clone(),
            resource_root: surface.resource_root.clone(),
            data: surface.data.clone(),
            kind: surface.kind.clone(),
        })
        .collect::<Vec<_>>();
    descriptors.sort_by(|left, right| left.instance_id.cmp(&right.instance_id));
    descriptors
}

fn handle_lifecycle_message(
    lifecycle: &mut ExtensionLifecycleCoordinator,
    surface_id: &str,
    request: &muxy_api::extensions::ExtensionApiRequest,
) -> Option<Result<Value, muxy_api::extensions::ExtensionApiError>> {
    match request.method.as_str() {
        "lifecycle.ackBeforeClose" => {
            if let Some(call_id) = request.args.get("callID").and_then(Value::as_str) {
                lifecycle.acknowledge(surface_id, call_id);
            }
            Some(Ok(Value::Null))
        }
        "lifecycle.resolveBeforeClose" => {
            if let Some(call_id) = request.args.get("callID").and_then(Value::as_str) {
                let prevent = request
                    .args
                    .get("prevent")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                lifecycle.resolve(surface_id, call_id, prevent);
            }
            Some(Ok(Value::Null))
        }
        "lifecycle.closeSelf" => Some(Ok(Value::Null)),
        _ => None,
    }
}

pub(crate) fn extension_theme_snapshot(
    theme: &muxy_ui::theme::Theme,
    appearance: muxy_ui::theme::Appearance,
    topbar_height: gpui::Pixels,
) -> Value {
    let hex = |color: gpui::Hsla| {
        let rgba: gpui::Rgba = color.into();
        let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        if rgba.a >= 0.999 {
            format!(
                "#{:02x}{:02x}{:02x}",
                channel(rgba.r),
                channel(rgba.g),
                channel(rgba.b)
            )
        } else {
            format!(
                "#{:02x}{:02x}{:02x}{:02x}",
                channel(rgba.r),
                channel(rgba.g),
                channel(rgba.b),
                channel(rgba.a)
            )
        }
    };
    serde_json::json!({
        "background": hex(theme.bg),
        "foreground": hex(theme.fg),
        "foregroundMuted": hex(theme.fg_muted),
        "surface": hex(theme.surface),
        "surfaceSolid": hex(theme.raised()),
        "border": hex(theme.border),
        "hover": hex(theme.hover),
        "accent": hex(theme.accent),
        "accentForeground": hex(theme.accent_foreground),
        "accentSoft": hex(theme.accent_soft),
        "diffAdd": hex(theme.diff_add),
        "diffRemove": hex(theme.diff_remove),
        "diffHunk": hex(theme.diff_hunk),
        "colorScheme": match appearance {
            muxy_ui::theme::Appearance::Light => "light",
            muxy_ui::theme::Appearance::Dark => "dark",
        },
        "topbarHeight": format!("{}px", f32::from(topbar_height).round() as i32),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxy_core::environment::{BuildMode, RuntimePathPolicy};
    use muxy_core::extensions::paths::ExtensionPaths;
    use muxy_core::extensions::runtime::ExtensionRuntimeCatalog;
    use muxy_core::extensions::state::ExtensionStateStore;
    use muxy_core::workspace::{Tab, TabKind};
    use muxy_core::workspace_store::WorkspaceStore;
    use std::fs;

    #[test]
    fn descriptors_resolve_enabled_manifest_tab_types_and_default_data() {
        let root = tempfile::tempdir().unwrap();
        let paths = ExtensionPaths::new(RuntimePathPolicy::new(BuildMode::Production), root.path());
        let package = paths.packages.join("fixture");
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("index.html"), "fixture").unwrap();
        fs::write(
            package.join("package.json"),
            r#"{"name":"fixture","version":"1.0.0","muxy":{"tabTypes":[{"id":"main","title":"Main","entry":"index.html","defaultData":{"source":"default"}}]}}"#,
        )
        .unwrap();
        let mut state = ExtensionStateStore::open(paths.state_file()).unwrap();
        state.set_enabled("fixture", Some(true)).unwrap();
        let catalog = ExtensionRuntimeCatalog::load(paths).unwrap();
        let mut store = WorkspaceStore::load_from(root.path().join("workspaces.json"));
        let workspace = store.ensure_project("project", "/project");
        let mut tab = Tab::new(TabKind::ExtensionWebView);
        tab.id = "instance".to_owned();
        tab.extension_id = Some("fixture".to_owned());
        tab.extension_web_view_id = Some("main".to_owned());
        workspace.new_top_level_tab(tab);

        let mut registry = ExtensionSurfaceRegistry::default();
        registry.sync_tabs(&store, &catalog);
        let descriptors = extension_webview_descriptors(&registry);
        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].extension_id, "fixture");
        assert_eq!(descriptors[0].instance_id, "instance");
        assert_eq!(descriptors[0].entry_url, "muxy-ext://fixture/index.html");
        assert_eq!(
            descriptors[0].data,
            serde_json::json!({"source": "default"})
        );
    }

    #[test]
    fn theme_snapshot_has_the_swift_bridge_keys() {
        let scheme = muxy_ui::theme::ColorScheme::default();
        let theme = muxy_ui::theme::Theme::from_scheme(&scheme);
        let snapshot =
            extension_theme_snapshot(&theme, muxy_ui::theme::Appearance::Dark, gpui::px(32.0));
        for key in [
            "background",
            "foreground",
            "foregroundMuted",
            "surface",
            "surfaceSolid",
            "border",
            "hover",
            "accent",
            "accentForeground",
            "accentSoft",
            "diffAdd",
            "diffRemove",
            "diffHunk",
            "colorScheme",
            "topbarHeight",
        ] {
            assert!(snapshot.get(key).is_some(), "missing {key}");
        }
        assert_eq!(snapshot["colorScheme"], "dark");
    }
}
