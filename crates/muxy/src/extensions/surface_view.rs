use crate::extensions::surfaces::{ExtensionSurface, ExtensionSurfaceKind};
use crate::views::window::MainWindow;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, Context, ElementId, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Pixels, Point, SharedString, Styled, div, px,
};
use muxy_core::extensions::manifest::PanelHeaderControl;
use muxy_ui::panel::{
    PanelAction, PanelChrome, PanelFrame, PanelMode, PanelPlacement, PanelPosition,
    PanelResizeState, PanelSizeBounds, PanelSizing, PanelStyle,
};
use muxy_ui::{components::SymbolGlyph, icon::Icon};

pub(crate) struct RenderedExtensionPanel {
    pub placement: PanelPlacement,
    pub element: AnyElement,
}

pub(crate) fn render_panel(
    surface: &ExtensionSurface,
    placement: PanelPlacement,
    dimension: f64,
    resize_state: PanelResizeState,
    webviews: &crate::extensions::webview::ExtensionWebViewRegistry,
    theme: &muxy_ui::theme::Theme,
    metrics: muxy_ui::theme::Metrics,
    cx: &mut Context<MainWindow>,
) -> Option<RenderedExtensionPanel> {
    let ExtensionSurfaceKind::Panel(panel) = &surface.kind else {
        return None;
    };
    let position = placement.position;
    let mode = placement.mode;
    let style = PanelStyle::new(theme.clone(), metrics);
    let surface_id = surface.instance_id.clone();
    let move_view = cx.weak_entity();
    let move_id = surface_id.clone();
    let mode_view = cx.weak_entity();
    let mode_id = surface_id.clone();
    let close_view = cx.weak_entity();
    let close_id = surface_id.clone();
    let mut chrome = PanelChrome::new(
        panel.title.clone().unwrap_or_else(|| panel.id.clone()),
        panel.icon.as_ref().map(|icon| {
            extension_icon(
                icon,
                &surface.resource_root,
                metrics.icon_md(),
                theme.fg_muted,
            )
        }),
        cx.focus_handle(),
        PanelAction::icon(
            ElementId::Name(SharedString::from(format!(
                "extension-panel-move-{surface_id}"
            ))),
            "Move panel",
            match position {
                PanelPosition::Right => Icon::PanelBottom,
                PanelPosition::Bottom => Icon::PanelRight,
            },
            cx.focus_handle(),
            move |_, cx| {
                let _ = move_view.update(cx, |window, cx| {
                    window.move_extension_panel(&move_id, cx);
                });
            },
        ),
        PanelAction::icon(
            ElementId::Name(SharedString::from(format!(
                "extension-panel-mode-{surface_id}"
            ))),
            if mode == PanelMode::Pinned {
                "Float panel"
            } else {
                "Pin panel"
            },
            if mode == PanelMode::Pinned {
                Icon::PinOff
            } else {
                Icon::Pin
            },
            cx.focus_handle(),
            move |_, cx| {
                let _ = mode_view.update(cx, |window, cx| {
                    window.toggle_extension_panel_mode(&mode_id, cx);
                });
            },
        )
        .selected(mode == PanelMode::Pinned),
        PanelAction::icon(
            ElementId::Name(SharedString::from(format!(
                "extension-panel-close-{surface_id}"
            ))),
            "Close panel",
            Icon::X,
            cx.focus_handle(),
            move |_, cx| {
                let _ = close_view.update(cx, |window, cx| {
                    window.close_extension_surface(&close_id, cx);
                });
            },
        ),
        style.clone(),
    );
    if panel
        .hidden_controls
        .contains(&PanelHeaderControl::Position)
    {
        chrome = chrome.without_move_action();
    }
    if panel.hidden_controls.contains(&PanelHeaderControl::Pin) {
        chrome = chrome.without_mode_action();
    }
    if panel.hidden_controls.contains(&PanelHeaderControl::Close) {
        chrome = chrome.without_close_action();
    }
    for button in &panel.header_buttons {
        let extension_id = surface.extension_id.clone();
        let command = button.command.clone();
        let view = cx.weak_entity();
        let icon = extension_icon(
            &button.icon,
            &surface.resource_root,
            metrics.icon_sm(),
            theme.fg_muted,
        );
        chrome = chrome.with_trailing_action(PanelAction::element(
            ElementId::Name(SharedString::from(format!(
                "extension-panel-button-{surface_id}-{}",
                button.id
            ))),
            button.tooltip.clone().unwrap_or_else(|| button.id.clone()),
            icon,
            cx.focus_handle(),
            move |_, cx| {
                let _ = view.update(cx, |window, cx| {
                    window.run_extension_command(&extension_id, &command, None, cx);
                });
            },
        ));
    }
    let content = webviews.element(&surface.instance_id, true)?;
    let content = if panel.hide_topbar {
        content
    } else {
        div()
            .relative()
            .flex()
            .flex_grow()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .child(content)
            .into_any_element()
    };
    let bounds = match position {
        PanelPosition::Right => PanelSizeBounds::new(240.0, 720.0),
        PanelPosition::Bottom => PanelSizeBounds::new(160.0, 600.0),
    };
    let sizing = PanelSizing::new(&placement, dimension as f32, bounds, resize_state);
    let resize_view = cx.weak_entity();
    let resize_id = surface.instance_id.clone();
    let frame = PanelFrame::new(
        placement.clone(),
        sizing,
        if panel.hide_topbar {
            div().into_any_element()
        } else {
            chrome.into_any_element()
        },
        content,
        move |dimension, _, cx| {
            let _ = resize_view.update(cx, |window, cx| {
                window.resize_extension_panel(&resize_id, dimension, cx);
            });
        },
        style,
    )
    .into_any_element();
    Some(RenderedExtensionPanel {
        placement,
        element: frame,
    })
}

pub(crate) fn render_popover(
    surface: &ExtensionSurface,
    anchor: Option<Point<Pixels>>,
    webviews: &crate::extensions::webview::ExtensionWebViewRegistry,
    theme: &muxy_ui::theme::Theme,
    metrics: muxy_ui::theme::Metrics,
    cx: &mut Context<MainWindow>,
) -> Option<AnyElement> {
    let ExtensionSurfaceKind::Popover(popover) = &surface.kind else {
        return None;
    };
    let content = webviews.element(&surface.instance_id, true)?;
    let anchor = anchor.unwrap_or(Point::new(px(24.0), metrics.title_bar_height()));
    let width = px(popover.width as f32);
    let height = px(popover.height as f32);
    let surface_id = surface.instance_id.clone();
    Some(
        div()
            .absolute()
            .inset_0()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |window: &mut MainWindow, _, _, cx| {
                    window.close_extension_surface(&surface_id, cx);
                }),
            )
            .child(
                div()
                    .absolute()
                    .top(anchor.y + metrics.spacing3())
                    .right(metrics.spacing4())
                    .w(width)
                    .h(height)
                    .max_w(px(720.0))
                    .max_h(px(760.0))
                    .rounded(metrics.radius_lg())
                    .border_1()
                    .border_color(theme.border_solid())
                    .bg(theme.bg)
                    .shadow_lg()
                    .overflow_hidden()
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(content),
            )
            .into_any_element(),
    )
}

pub(crate) fn render_modal(
    surface: &ExtensionSurface,
    webviews: &crate::extensions::webview::ExtensionWebViewRegistry,
    theme: &muxy_ui::theme::Theme,
    metrics: muxy_ui::theme::Metrics,
    cx: &mut Context<MainWindow>,
) -> Option<AnyElement> {
    let ExtensionSurfaceKind::Modal {
        width,
        height,
        dismiss_on_outside_click,
        ..
    } = &surface.kind
    else {
        return None;
    };
    let content = webviews.element(&surface.instance_id, true)?;
    let surface_id = surface.instance_id.clone();
    Some(
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_start()
            .justify_center()
            .pt(metrics.scaled(72.0))
            .bg(gpui::black().opacity(0.32))
            .when(*dismiss_on_outside_click, |layer| {
                layer.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |window: &mut MainWindow, _, _, cx| {
                        window.close_extension_surface(&surface_id, cx);
                    }),
                )
            })
            .child(
                div()
                    .relative()
                    .w(px(*width as f32))
                    .h(px(*height as f32))
                    .max_w(px(900.0))
                    .max_h(px(760.0))
                    .rounded(metrics.radius_lg())
                    .border_1()
                    .border_color(theme.border_solid())
                    .bg(theme.bg)
                    .shadow_lg()
                    .overflow_hidden()
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(content),
            )
            .into_any_element(),
    )
}

pub(crate) fn extension_icon(
    icon: &muxy_core::extensions::manifest::ExtensionIcon,
    resource_root: &std::path::Path,
    size: Pixels,
    color: gpui::Hsla,
) -> AnyElement {
    match icon {
        muxy_core::extensions::manifest::ExtensionIcon::Symbol(symbol) => {
            SymbolGlyph::new(symbol.clone(), size, color).into_any_element()
        }
        muxy_core::extensions::manifest::ExtensionIcon::Svg(path) => gpui::svg()
            .path(resource_root.join(path).to_string_lossy().into_owned())
            .size(size)
            .flex_none()
            .text_color(color)
            .into_any_element(),
    }
}

impl MainWindow {
    pub(crate) fn move_extension_panel(&mut self, surface_id: &str, cx: &mut Context<Self>) {
        let changes = self.extension_surfaces.move_panel(surface_id);
        self.apply_extension_surface_changes(changes, cx);
        if !self.activate_extension_panel(surface_id, cx) {
            self.extension_surfaces.move_panel(surface_id);
            self.activate_extension_panel(surface_id, cx);
        }
        cx.notify();
    }

    pub(crate) fn toggle_extension_panel_mode(&mut self, surface_id: &str, cx: &mut Context<Self>) {
        let changes = self.extension_surfaces.toggle_panel_mode(surface_id);
        self.apply_extension_surface_changes(changes, cx);
        if !self.activate_extension_panel(surface_id, cx) {
            self.extension_surfaces.toggle_panel_mode(surface_id);
            self.activate_extension_panel(surface_id, cx);
        }
        cx.notify();
    }

    pub(crate) fn resize_extension_panel(
        &mut self,
        surface_id: &str,
        dimension: f32,
        cx: &mut Context<Self>,
    ) {
        if self
            .extension_surfaces
            .resize_panel(surface_id, dimension as f64)
        {
            cx.notify();
        }
    }
}
