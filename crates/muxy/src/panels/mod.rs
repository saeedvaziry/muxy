use gpui::{
    AnyElement, AppContext, Context, IntoElement, ParentElement, SharedString, Styled, canvas, div,
    px,
};
use muxy_ui::panel::{
    PanelAction, PanelChrome, PanelDisplacement, PanelFrame, PanelHost, PanelId, PanelMode,
    PanelPlacement, PanelPosition, PanelResizeState, PanelSizeBounds, PanelSizing, PanelStyle,
};
use muxy_ui::text_input::{InputStyle, TextInput};
use muxy_ui::theme::{Metrics, Theme};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Default)]
pub struct PanelRuntime {
    host: PanelHost,
    focused: Option<PanelId>,
    resize_states: BTreeMap<PanelId, PanelResizeState>,
}

impl PanelRuntime {
    pub fn host(&self) -> &PanelHost {
        &self.host
    }

    pub fn open(&mut self, placement: PanelPlacement) -> Option<PanelDisplacement> {
        let id = placement.id.clone();
        let displacement = self.host.place(placement);
        self.resize_states.entry(id).or_default();
        if let Some(result) = &displacement {
            self.resize_states.remove(&result.displaced.id);
            if self.focused.as_ref() == Some(&result.displaced.id) {
                self.focused = None;
            }
        }
        displacement
    }

    pub fn move_to(&mut self, id: &PanelId, position: PanelPosition) -> Option<PanelDisplacement> {
        let current = self.host.placement(id)?.clone();
        self.open(PanelPlacement::new(current.id, position, current.mode))
    }

    pub fn set_mode(&mut self, id: &PanelId, mode: PanelMode) -> Option<PanelDisplacement> {
        let current = self.host.placement(id)?.clone();
        self.open(PanelPlacement::new(current.id, current.position, mode))
    }

    pub fn close(&mut self, id: &PanelId) -> Option<PanelPlacement> {
        if self.focused.as_ref() == Some(id) {
            self.focused = None;
        }
        self.resize_states.remove(id);
        self.host.remove(id)
    }

    pub fn resize_state(&self, id: &PanelId) -> Option<PanelResizeState> {
        self.resize_states.get(id).cloned()
    }

    pub fn report_focus(&mut self, id: &PanelId, focused: bool) {
        if focused && self.host.placement(id).is_some() {
            self.focused = Some(id.clone());
        } else if !focused && self.focused.as_ref() == Some(id) {
            self.focused = None;
        }
    }

    pub fn focused_panel(&self) -> Option<&PanelId> {
        self.focused.as_ref()
    }

    pub fn report_outside_click(&mut self, id: &PanelId, clicked_inside: bool) -> bool {
        if clicked_inside
            || self
                .host
                .placement(id)
                .is_none_or(|placement| placement.mode != PanelMode::Floating)
        {
            return false;
        }
        self.close(id).is_some()
    }
}

impl crate::views::window::MainWindow {
    pub(crate) fn place_panel(&mut self, placement: PanelPlacement) {
        let Some(displacement) = self.panels.open(placement) else {
            return;
        };
        if displacement.displaced.id.as_str() == muxy_core::composer::PANEL_ID {
            self.panels.close(&displacement.replacement.id);
            self.panels.open(displacement.displaced);
            return;
        }
        let changes = self
            .extension_surfaces
            .close(displacement.displaced.id.as_str());
        self.publish_extension_surface_changes(&changes);
    }
}

fn phase_3_status_path(
    is_test_process: bool,
    case_name: Option<&str>,
    app_support: &Path,
    injected_app_support: Option<&Path>,
    home: &Path,
) -> Option<PathBuf> {
    if !is_test_process
        || case_name != Some("phase-3")
        || injected_app_support != Some(app_support)
        || !app_support.is_absolute()
        || app_support.starts_with(home)
        || app_support
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        || std::fs::symlink_metadata(app_support)
            .ok()
            .is_none_or(|metadata| !metadata.is_dir() || metadata.file_type().is_symlink())
        || std::fs::canonicalize(app_support).ok().as_deref() != Some(app_support)
    {
        return None;
    }
    Some(app_support.join(".muxy-p7-components-status.json"))
}

fn current_phase_3_status_path() -> Option<PathBuf> {
    let app_support = muxy_core::prefs::app_support_dir();
    let injected = std::env::var_os("MUXY_TEST_APPLICATION_SUPPORT_DIRECTORY").map(PathBuf::from);
    let case_name = std::env::var("MUXY_TEST_P7_COMPOSER_CASE").ok();
    phase_3_status_path(
        muxy_core::prefs::is_test_process(),
        case_name.as_deref(),
        &app_support,
        injected.as_deref(),
        &muxy_core::prefs::home_dir(),
    )
}

pub(crate) fn with_phase_3_component_proof(
    app: AnyElement,
    theme: &Theme,
    metrics: Metrics,
    cx: &mut Context<crate::views::window::MainWindow>,
) -> AnyElement {
    let Some(status_path) = current_phase_3_status_path() else {
        return app;
    };
    let placement = PanelPlacement::new(
        "phase-3-component-proof",
        PanelPosition::Right,
        PanelMode::Floating,
    );
    let style = PanelStyle::new(theme.clone(), metrics);
    let input = cx.new(|cx| {
        let mut input = TextInput::new(InputStyle::field(theme, &metrics), cx)
            .multiline()
            .with_text("Unicode e\u{301} input");
        let dynamic_style = InputStyle {
            font_size: px(14.0),
            line_height: px(22.0),
            ..InputStyle::field(theme, &metrics)
        };
        input.set_style(dynamic_style, cx);
        input.set_font_family(Some(SharedString::from(".SystemUIFontMonospaced")), cx);
        input.set_paste_delegate(|_, _| false);
        input.insert_at_selection(" proof", cx);
        input
    });
    let chrome = PanelChrome::new(
        "Component proof",
        None,
        cx.focus_handle(),
        PanelAction::new(
            "phase-3-move",
            "Move panel",
            "M",
            cx.focus_handle(),
            |_, _| {},
        ),
        PanelAction::new(
            "phase-3-mode",
            "Pin panel",
            "P",
            cx.focus_handle(),
            |_, _| {},
        ),
        PanelAction::new(
            "phase-3-close",
            "Close panel",
            "C",
            cx.focus_handle(),
            |_, _| {},
        ),
        style.clone(),
    )
    .with_trailing_action(PanelAction::new(
        "phase-3-custom",
        "Custom action",
        "A",
        cx.focus_handle(),
        |_, _| {},
    ));
    let proof = canvas(
        |_, _, _| (),
        move |_, _, _, _| {
            let value = serde_json::json!({
                "painted": true,
                "panelId": "phase-3-component-proof",
                "position": "right",
                "mode": "floating",
                "dimension": 320.0,
                "overlaysWorkspace": true,
                "chromeActions": ["move", "mode", "close", "custom"],
                "textInput": {
                    "multiline": true,
                    "fontFamily": ".SystemUIFontMonospaced",
                    "fontSize": 14.0,
                    "lineHeight": 22.0,
                    "pasteDelegate": "deferred"
                }
            });
            if let Ok(contents) = serde_json::to_vec_pretty(&value)
                && let Err(error) = muxy_core::store::write_private(&status_path, &contents)
            {
                log::warn!("failed to write P7 component status: {error}");
            }
        },
    )
    .absolute()
    .size_full();
    let content = div()
        .relative()
        .flex()
        .flex_col()
        .size_full()
        .child(input)
        .child(proof);
    let sizing = PanelSizing::new(
        &placement,
        320.0,
        PanelSizeBounds::new(100.0, 500.0),
        PanelResizeState::default(),
    );
    let panel = PanelFrame::new(placement, sizing, chrome, content, |_, _, _| {}, style);
    div()
        .relative()
        .size_full()
        .child(app)
        .child(panel)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{PanelRuntime, phase_3_status_path};
    use gpui::Point;
    use muxy_ui::panel::{
        PanelId, PanelMode, PanelPlacement, PanelPosition, PanelResize, PanelSizeBounds,
    };

    #[test]
    fn component_status_requires_the_exact_isolated_phase_3_case() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let support = root.join("support");
        let home = root.join("home");
        std::fs::create_dir_all(&support).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        assert!(
            phase_3_status_path(false, Some("phase-3"), &support, Some(&support), &home).is_none()
        );
        assert!(
            phase_3_status_path(true, Some("other"), &support, Some(&support), &home).is_none()
        );
        assert!(phase_3_status_path(true, Some("phase-3"), &support, Some(&home), &home).is_none());
        assert_eq!(
            phase_3_status_path(true, Some("phase-3"), &support, Some(&support), &home),
            Some(support.join(".muxy-p7-components-status.json"))
        );
    }

    #[test]
    fn app_runtime_preserves_identity_across_move_and_mode_changes() {
        let mut runtime = PanelRuntime::default();
        let id = PanelId::from("panel");
        runtime.open(PanelPlacement::new(
            id.clone(),
            PanelPosition::Right,
            PanelMode::Floating,
        ));
        let resize_state = runtime.resize_state(&id).unwrap();
        resize_state.begin(PanelResize::new(
            PanelPosition::Right,
            300.0,
            Point::new(500.0, 400.0),
            PanelSizeBounds::new(100.0, 500.0),
        ));
        runtime.move_to(&id, PanelPosition::Bottom);
        runtime.set_mode(&id, PanelMode::Pinned);
        assert!(runtime.resize_state(&id).unwrap().is_active());
        assert_eq!(
            runtime.host().placement(&id),
            Some(&PanelPlacement::new(
                id,
                PanelPosition::Bottom,
                PanelMode::Pinned,
            ))
        );
    }

    #[test]
    fn only_floating_panels_close_on_reported_outside_click() {
        let mut runtime = PanelRuntime::default();
        let floating = PanelId::from("floating");
        let pinned = PanelId::from("pinned");
        runtime.open(PanelPlacement::new(
            floating.clone(),
            PanelPosition::Right,
            PanelMode::Floating,
        ));
        runtime.open(PanelPlacement::new(
            pinned.clone(),
            PanelPosition::Bottom,
            PanelMode::Pinned,
        ));
        assert!(!runtime.report_outside_click(&floating, true));
        assert!(runtime.report_outside_click(&floating, false));
        assert!(!runtime.report_outside_click(&pinned, false));
        assert!(runtime.host().placement(&pinned).is_some());
    }

    #[test]
    fn displacement_and_close_clear_panel_focus() {
        let mut runtime = PanelRuntime::default();
        let first = PanelId::from("first");
        let second = PanelId::from("second");
        runtime.open(PanelPlacement::new(
            first.clone(),
            PanelPosition::Right,
            PanelMode::Floating,
        ));
        runtime.report_focus(&first, true);
        assert_eq!(runtime.focused_panel(), Some(&first));

        let displacement = runtime
            .open(PanelPlacement::new(
                second.clone(),
                PanelPosition::Right,
                PanelMode::Floating,
            ))
            .unwrap();
        assert_eq!(displacement.displaced.id, first);
        assert!(runtime.focused_panel().is_none());

        runtime.report_focus(&second, true);
        runtime.close(&second);
        assert!(runtime.focused_panel().is_none());
    }
}
