use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtensionBridgeSurface {
    Background,
    WebView,
}

pub fn extension_bridge_script(extension_id: &str, surface: ExtensionBridgeSurface) -> String {
    render_bridge(
        extension_id,
        matches!(surface, ExtensionBridgeSurface::Background),
        "",
        &Value::Null,
        &serde_json::json!({}),
        false,
    )
}

pub fn extension_webview_bridge_script(
    extension_id: &str,
    instance_id: &str,
    data: &Value,
    theme: &Value,
    focused: bool,
) -> String {
    render_bridge(extension_id, false, instance_id, data, theme, focused)
}

fn render_bridge(
    extension_id: &str,
    background: bool,
    instance_id: &str,
    data: &Value,
    theme: &Value,
    focused: bool,
) -> String {
    let string_literal =
        |value: &str| serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned());
    let value_literal = |value: &Value, fallback: &str| {
        serde_json::to_string(value).unwrap_or_else(|_| fallback.to_owned())
    };
    include_str!("bridge.js")
        .replace("__MUXY_EXTENSION_ID__", &string_literal(extension_id))
        .replace("__MUXY_INSTANCE_ID__", &string_literal(instance_id))
        .replace("__MUXY_DATA__", &value_literal(data, "null"))
        .replace("__MUXY_THEME__", &value_literal(theme, "{}"))
        .replace("__MUXY_FOCUSED__", if focused { "true" } else { "false" })
        .replace(
            "__MUXY_BACKGROUND__",
            if background { "true" } else { "false" },
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::contract::{
        P9_BROWSER_API_METHODS, P10_EXTENSION_API_METHODS, WEBVIEW_CONTROL_METHODS,
    };

    #[test]
    fn bridge_escapes_identity_and_selects_the_surface() {
        let background =
            extension_bridge_script("sample\"extension", ExtensionBridgeSurface::Background);
        assert!(background.contains("const background = true"));
        assert!(background.contains("const extensionID = \"sample\\\"extension\""));
        assert!(!background.contains("__MUXY_"));

        let webview = extension_webview_bridge_script(
            "sample",
            "instance\"one",
            &serde_json::json!({"number": 7}),
            &serde_json::json!({"colorScheme": "dark"}),
            true,
        );
        assert!(webview.contains("const background = false"));
        assert!(webview.contains("const tabInstanceID = \"instance\\\"one\""));
        assert!(webview.contains("let currentData = {\"number\":7}"));
        assert!(webview.contains("let currentTheme = Object.freeze({\"colorScheme\":\"dark\"})"));
        assert!(webview.contains("let currentFocus = true"));
        assert!(!webview.contains("__MUXY_"));
    }

    #[test]
    fn webview_bridge_accounts_for_every_public_catalog_method() {
        let bridge = extension_bridge_script("sample", ExtensionBridgeSurface::WebView);
        for method in P9_BROWSER_API_METHODS
            .into_iter()
            .chain(WEBVIEW_CONTROL_METHODS)
            .chain(P10_EXTENSION_API_METHODS.into_iter().filter(|method| {
                !matches!(
                    *method,
                    "extension.settings.get" | "extension.settings.set" | "extension.statusbar.set"
                )
            }))
        {
            assert!(
                bridge.contains(&format!("'{method}'")),
                "bridge is missing {method}"
            );
        }
    }

    #[test]
    fn socket_only_extension_methods_are_not_added_to_window_muxy() {
        let bridge = extension_bridge_script("sample", ExtensionBridgeSurface::WebView);
        for method in [
            "extension.settings.get",
            "extension.settings.set",
            "extension.statusbar.set",
        ] {
            assert!(!bridge.contains(&format!("'{method}'")), "{method}");
        }
    }
}
