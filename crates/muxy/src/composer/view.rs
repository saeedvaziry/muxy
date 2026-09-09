use crate::composer::ComposerController;
use crate::views::window::MainWindow;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AppContext, Bounds, Context, ExternalPaths, FocusHandle, Focusable,
    FontWeight, InteractiveElement, IntoElement, ParentElement, Pixels, Point, SharedString,
    StatefulInteractiveElement, Styled, Window, canvas, div, point, px,
};
use muxy_core::prefs::{
    COMPOSER_BOTTOM_HEIGHT_MAX, COMPOSER_BOTTOM_HEIGHT_MIN, COMPOSER_RIGHT_WIDTH_MAX,
    COMPOSER_RIGHT_WIDTH_MIN, ComposerPanelMode, ComposerPanelPosition, ComposerPreferences,
};
use muxy_core::shortcuts::{
    COMMAND, CONTROL, KeyCombo, OPTION, SHIFT, ShortcutAction, ShortcutMap, canonical_key,
};
use muxy_ui::components::{IconButton, IconGlyph, Tooltip};
use muxy_ui::icon::Icon;
use muxy_ui::panel::{
    PanelAction, PanelChrome, PanelFrame, PanelId, PanelMode, PanelPlacement, PanelPosition,
    PanelSizeBounds, PanelSizing, PanelStyle,
};
use muxy_ui::theme::{Metrics, Theme};
use std::cell::Cell;
use std::rc::Rc;

pub struct RenderedComposerPanel {
    pub placement: PanelPlacement,
    pub element: AnyElement,
    pub merged_footer: Option<AnyElement>,
}

fn position_name(position: ComposerPanelPosition) -> &'static str {
    match position {
        ComposerPanelPosition::Right => "right",
        ComposerPanelPosition::Bottom => "bottom",
    }
}

fn mode_name(mode: ComposerPanelMode) -> &'static str {
    match mode {
        ComposerPanelMode::Pinned => "pinned",
        ComposerPanelMode::Floating => "floating",
    }
}

fn panel_dimension(preferences: &ComposerPreferences) -> f32 {
    match preferences.position {
        ComposerPanelPosition::Right => preferences.panel_width as f32,
        ComposerPanelPosition::Bottom => preferences.panel_height as f32,
    }
}

fn panel_bounds(position: ComposerPanelPosition) -> PanelSizeBounds {
    match position {
        ComposerPanelPosition::Right => PanelSizeBounds::new(
            COMPOSER_RIGHT_WIDTH_MIN as f32,
            COMPOSER_RIGHT_WIDTH_MAX as f32,
        ),
        ComposerPanelPosition::Bottom => PanelSizeBounds::new(
            COMPOSER_BOTTOM_HEIGHT_MIN as f32,
            COMPOSER_BOTTOM_HEIGHT_MAX as f32,
        ),
    }
}

fn status_proof(
    controller: &ComposerController,
    preferences: &ComposerPreferences,
    placement: &PanelPlacement,
    dimension: f32,
    cx: &Context<MainWindow>,
) -> Option<AnyElement> {
    let path = crate::views::window::composer::current_phase_4_status_path()?;
    let restore = controller.staged_restore()?;
    let target = controller.target()?;
    let input = controller.input()?;
    let value = serde_json::json!({
        "painted": true,
        "panelId": placement.id.as_str(),
        "position": position_name(preferences.position),
        "mode": mode_name(preferences.panel_mode),
        "dimension": dimension,
        "overlaysWorkspace": placement.mode == PanelMode::Floating,
        "broadcast": preferences.broadcast,
        "fontSize": preferences.font_size,
        "text": input.read(cx).text(),
        "fileAttachments": controller.file_attachments(),
        "target": {
            "projectId": target.project_id,
            "worktreeId": target.worktree_id,
            "paneId": target.pane_id,
        },
        "restoredBeforeEdit": {
            "text": restore.text,
            "position": position_name(restore.position),
            "mode": mode_name(restore.mode),
            "dimension": restore.dimension,
            "broadcast": restore.broadcast,
            "fontSize": restore.font_size,
        }
    });
    Some(
        canvas(
            |_, _, _| (),
            move |_, _, _, _| {
                if let Ok(contents) = serde_json::to_vec_pretty(&value)
                    && let Err(error) = muxy_core::store::write_private(&path, &contents)
                {
                    log::warn!("failed to write P7 Composer panel status: {error}");
                }
            },
        )
        .absolute()
        .size_full()
        .into_any_element(),
    )
}

fn shortcut_key_label(key: &str) -> String {
    let key = canonical_key(key);
    match key.as_str() {
        "return" => "Enter".to_owned(),
        "leftarrow" => "Left".to_owned(),
        "rightarrow" => "Right".to_owned(),
        "uparrow" => "Up".to_owned(),
        "downarrow" => "Down".to_owned(),
        "escape" => "Esc".to_owned(),
        "space" => "Space".to_owned(),
        _ => {
            let mut characters = key.chars();
            let Some(first) = characters.next() else {
                return String::new();
            };
            first.to_uppercase().chain(characters).collect()
        }
    }
}

fn composer_send_label(combo: &KeyCombo) -> SharedString {
    if !combo.is_assigned() {
        return SharedString::from("Send");
    }
    let mut parts = Vec::new();
    if cfg!(target_os = "macos") {
        if combo.modifiers & CONTROL != 0 {
            parts.push("Ctrl".to_owned());
        }
        if combo.modifiers & OPTION != 0 {
            parts.push("Option".to_owned());
        }
        if combo.modifiers & SHIFT != 0 {
            parts.push("Shift".to_owned());
        }
        if combo.modifiers & COMMAND != 0 {
            parts.push("Cmd".to_owned());
        }
    } else {
        if combo.modifiers & (CONTROL | COMMAND) != 0 {
            parts.push("Ctrl".to_owned());
        }
        if combo.modifiers & OPTION != 0 {
            parts.push("Alt".to_owned());
        }
        if combo.modifiers & SHIFT != 0 {
            parts.push("Shift".to_owned());
        }
    }
    parts.push(shortcut_key_label(&combo.key));
    SharedString::from(format!("{} to Send", parts.join("+")))
}

type ComposerFooterHandler = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)>;

struct ComposerFooterAction {
    id: &'static str,
    icon: Icon,
    label: Option<SharedString>,
    tooltip: SharedString,
    focus_handle: FocusHandle,
    shrink: bool,
}

impl ComposerFooterAction {
    fn new(
        id: &'static str,
        icon: Icon,
        label: Option<SharedString>,
        tooltip: impl Into<SharedString>,
        focus_handle: FocusHandle,
        shrink: bool,
    ) -> Self {
        Self {
            id,
            icon,
            label,
            tooltip: tooltip.into(),
            focus_handle,
            shrink,
        }
    }
}

fn composer_footer_action(
    action: ComposerFooterAction,
    theme: &Theme,
    metrics: Metrics,
    window: &Window,
    on_activate: impl Fn(Point<Pixels>, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let group = SharedString::from(format!("{}-group", action.id));
    let ComposerFooterAction {
        id,
        icon,
        label,
        tooltip,
        focus_handle,
        shrink,
    } = action;
    let tooltip_background = theme.raised();
    let tooltip_foreground = theme.fg;
    let tooltip_border = theme.border;
    let handler: ComposerFooterHandler = Rc::new(on_activate);
    let click_handler = handler.clone();
    let key_handler = handler;
    let bounds = Rc::new(Cell::new(None::<Bounds<Pixels>>));
    let bounds_recorder = bounds.clone();
    let key_bounds = bounds;
    let focused = focus_handle.is_focused(window);
    div()
        .id(id)
        .relative()
        .group(group.clone())
        .flex()
        .items_center()
        .gap(metrics.spacing2())
        .h_full()
        .min_w(px(0.0))
        .px(metrics.spacing4())
        .track_focus(&focus_handle)
        .text_size(metrics.font_footnote())
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.fg_muted)
        .cursor_pointer()
        .when(!shrink, |action| action.flex_none())
        .when(shrink, |action| {
            action.flex_shrink().min_w(metrics.control_small())
        })
        .when(focused, |style| style.bg(theme.accent_soft))
        .hover(|style| style.bg(theme.hover).text_color(theme.fg))
        .tooltip(move |_, cx| {
            cx.new(|_| {
                Tooltip::new(
                    tooltip.clone(),
                    tooltip_background,
                    tooltip_foreground,
                    tooltip_border,
                )
            })
            .into()
        })
        .on_click(move |event, window, cx| click_handler(event.position(), window, cx))
        .on_key_down(move |event, window, cx| {
            if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                if let Some(bounds) = key_bounds.get() {
                    key_handler(
                        point(bounds.origin.x + bounds.size.width, bounds.origin.y),
                        window,
                        cx,
                    );
                }
                cx.stop_propagation();
            }
        })
        .child(
            canvas(
                move |bounds, _, _| bounds_recorder.set(Some(bounds)),
                |_, _: (), _, _| {},
            )
            .absolute()
            .size_full(),
        )
        .child(
            IconGlyph::new(icon, metrics.font_caption(), theme.fg_muted)
                .hover_in_group(group, theme.fg),
        )
        .when_some(label, |action, label| {
            action.child(div().min_w(px(0.0)).truncate().child(label))
        })
        .into_any_element()
}

fn composer_footer_separator(theme: &Theme) -> AnyElement {
    div()
        .w(px(1.0))
        .h_full()
        .flex_none()
        .bg(theme.border)
        .into_any_element()
}

fn composer_toolbar(
    preferences: &ComposerPreferences,
    shortcuts: &ShortcutMap,
    theme: &Theme,
    metrics: Metrics,
    merged: bool,
    window: &Window,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let view = cx.weak_entity();
    let attach = composer_footer_action(
        ComposerFooterAction::new(
            "composer-attach-files",
            Icon::Plus,
            None,
            "Attach File",
            cx.focus_handle().tab_stop(true),
            false,
        ),
        theme,
        metrics,
        window,
        move |_, _, cx| {
            let _ = view.update(cx, |window, cx| window.choose_composer_files(cx));
        },
    );
    let view = cx.weak_entity();
    let more = composer_footer_action(
        ComposerFooterAction::new(
            "composer-more-actions",
            Icon::Ellipsis,
            None,
            "More Composer Actions",
            cx.focus_handle().tab_stop(true),
            false,
        ),
        theme,
        metrics,
        window,
        move |position, window, cx| {
            let _ = view.update(cx, |main_window, cx| {
                main_window.open_composer_menu(position, window, cx);
            });
        },
    );
    let send_label = composer_send_label(shortcuts.combo(ShortcutAction::SubmitRichInput));
    let view = cx.weak_entity();
    let send = composer_footer_action(
        ComposerFooterAction::new(
            "composer-send",
            Icon::ArrowUp,
            Some(send_label.clone()),
            send_label,
            cx.focus_handle().tab_stop(true),
            true,
        ),
        theme,
        metrics,
        window,
        move |_, _, cx| {
            let _ = view.update(cx, |window, cx| window.submit_composer(true, cx));
        },
    );
    let target = div()
        .flex()
        .items_center()
        .h_full()
        .min_w(px(0.0))
        .when(!merged, |target| target.flex_grow())
        .when(merged, |target| target.flex_none())
        .px(metrics.spacing4())
        .text_size(metrics.font_footnote())
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.fg_dim)
        .truncate()
        .child(if preferences.broadcast {
            "All split panes"
        } else {
            "Active pane"
        });

    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .h(metrics.status_bar_height())
        .when(!merged, |toolbar| {
            toolbar
                .w_full()
                .bg(theme.bg)
                .border_t(px(1.0))
                .border_color(theme.border)
        })
        .child(attach)
        .child(composer_footer_separator(theme))
        .child(more)
        .child(composer_footer_separator(theme))
        .child(target)
        .child(composer_footer_separator(theme))
        .child(send)
        .into_any_element()
}

fn attachment_rows(
    controller: &ComposerController,
    theme: &Theme,
    metrics: Metrics,
    cx: &mut Context<MainWindow>,
) -> Vec<AnyElement> {
    controller
        .file_attachments()
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let remove_path = path.clone();
            let view = cx.weak_entity();
            let remove = IconButton::new(
                SharedString::from(format!("composer-remove-file-{index}")),
                Icon::X,
                metrics.font_footnote(),
                metrics.control_small(),
                theme.fg_muted,
                theme.fg,
            )
            .tooltip(
                SharedString::from(format!("Remove {path}")),
                theme.raised(),
                theme.fg,
                theme.border_solid(),
            )
            .on_click(move |_, _, cx| {
                let remove_path = remove_path.clone();
                let _ = view.update(cx, |window, cx| {
                    window.remove_composer_file(&remove_path, cx);
                });
            });
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(metrics.spacing2())
                .w_full()
                .min_w(px(0.0))
                .px(metrics.spacing3())
                .py(metrics.spacing1())
                .rounded(metrics.radius_sm())
                .bg(theme.raised())
                .child(
                    div()
                        .min_w(px(0.0))
                        .flex_grow()
                        .truncate()
                        .text_size(metrics.font_caption())
                        .text_color(theme.fg_muted)
                        .child(path.clone()),
                )
                .child(remove)
                .into_any_element()
        })
        .collect()
}

pub fn render(
    controller: &ComposerController,
    panels: &crate::panels::PanelRuntime,
    preferences: &ComposerPreferences,
    shortcuts: &ShortcutMap,
    style: PanelStyle,
    merge_footer: bool,
    window: &Window,
    cx: &mut Context<MainWindow>,
) -> Option<RenderedComposerPanel> {
    let theme = &style.theme;
    let metrics = style.metrics;
    let placement = panels
        .host()
        .placement(&PanelId::from(muxy_core::composer::PANEL_ID))?
        .clone();
    let merge_footer = merge_footer
        && placement.position == PanelPosition::Bottom
        && placement.mode == PanelMode::Pinned;
    let input = controller.input()?.clone();
    let dimension = panel_dimension(preferences);
    let resize_state = panels.resize_state(&PanelId::from(muxy_core::composer::PANEL_ID))?;
    let view = cx.weak_entity();
    let move_action = PanelAction::icon(
        "composer-move",
        match preferences.position {
            ComposerPanelPosition::Right => "Move Rich Input to bottom",
            ComposerPanelPosition::Bottom => "Move Rich Input to right",
        },
        match preferences.position {
            ComposerPanelPosition::Right => Icon::PanelBottom,
            ComposerPanelPosition::Bottom => Icon::PanelRight,
        },
        cx.focus_handle(),
        move |_, cx| {
            let _ = view.update(cx, |window, cx| window.move_composer_panel(cx));
        },
    );
    let view = cx.weak_entity();
    let mode_action = PanelAction::icon(
        "composer-mode",
        match preferences.panel_mode {
            ComposerPanelMode::Floating => "Pin Rich Input",
            ComposerPanelMode::Pinned => "Unpin Rich Input",
        },
        match preferences.panel_mode {
            ComposerPanelMode::Floating => Icon::Pin,
            ComposerPanelMode::Pinned => Icon::PinOff,
        },
        cx.focus_handle(),
        move |_, cx| {
            let _ = view.update(cx, |window, cx| window.toggle_composer_panel_mode(cx));
        },
    )
    .selected(preferences.panel_mode == ComposerPanelMode::Pinned);
    let view = cx.weak_entity();
    let close_action = PanelAction::icon(
        "composer-close",
        "Close Rich Input",
        Icon::X,
        cx.focus_handle(),
        move |_, cx| {
            let _ = view.update(cx, |window, cx| window.close_composer(cx));
        },
    );
    let view = cx.weak_entity();
    let broadcast_action = PanelAction::icon(
        "composer-broadcast",
        if preferences.broadcast {
            "Broadcast On, send to all split panes"
        } else {
            "Broadcast Off, send to active pane"
        },
        if preferences.broadcast {
            Icon::Broadcast
        } else {
            Icon::BroadcastOff
        },
        cx.focus_handle(),
        move |_, cx| {
            let _ = view.update(cx, |window, cx| window.toggle_composer_broadcast(cx));
        },
    )
    .selected(preferences.broadcast);
    let chrome = PanelChrome::new(
        "Rich Input",
        Some(
            IconGlyph::new(Icon::Keyboard, metrics.font_footnote(), theme.fg_muted)
                .into_any_element(),
        ),
        cx.focus_handle(),
        move_action,
        mode_action,
        close_action,
        style.clone(),
    )
    .with_trailing_action(broadcast_action);
    let attachments = attachment_rows(controller, theme, metrics, cx);
    let toolbar = composer_toolbar(
        preferences,
        shortcuts,
        theme,
        metrics,
        merge_footer,
        window,
        cx,
    );
    let (embedded_toolbar, merged_footer) = if merge_footer {
        (None, Some(toolbar))
    } else {
        (Some(toolbar), None)
    };
    let proof = status_proof(controller, preferences, &placement, dimension, cx);
    let drop_view = cx.weak_entity();
    let shortcut_view = cx.weak_entity();
    let shortcut_input = input.clone();
    let body = div()
        .flex()
        .flex_col()
        .flex_grow()
        .w_full()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .p(metrics.scaled(14.0))
        .gap(metrics.spacing3())
        .child(
            div()
                .flex()
                .flex_col()
                .flex_grow()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .bg(theme.bg)
                .child(input),
        )
        .when(!attachments.is_empty(), |body| {
            body.child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap(metrics.spacing2())
                    .children(attachments),
            )
        });
    let content = div()
        .relative()
        .drag_over::<ExternalPaths>({
            let accent_soft = theme.accent_soft;
            move |style, _, _, _| style.bg(accent_soft)
        })
        .on_drop(move |paths: &ExternalPaths, _, cx| {
            let _ = drop_view.update(cx, |window, cx| {
                window.handle_composer_drop(paths.paths(), cx);
            });
        })
        .on_key_down(move |event, window, cx| {
            if !shortcut_input.read(cx).focus_handle(cx).is_focused(window) {
                return;
            }
            let Some(delta) = crate::views::window::composer::composer_font_shortcut_delta(
                &event.keystroke.key,
                event.keystroke.modifiers.platform,
            ) else {
                return;
            };
            let _ = shortcut_view.update(cx, |main_window, cx| {
                main_window.change_composer_font_size(delta, cx);
            });
            cx.stop_propagation();
        })
        .flex()
        .flex_col()
        .size_full()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .child(body)
        .when_some(embedded_toolbar, |content, toolbar| content.child(toolbar))
        .when_some(proof, |content, proof| content.child(proof));
    let sizing = PanelSizing::new(
        &placement,
        dimension,
        panel_bounds(preferences.position),
        resize_state,
    );
    let view = cx.weak_entity();
    let frame = PanelFrame::new(
        placement.clone(),
        sizing,
        chrome,
        content,
        move |dimension, _, cx| {
            let _ = view.update(cx, |window, cx| {
                window.resize_composer_panel(dimension, cx);
            });
        },
        style,
    );
    Some(RenderedComposerPanel {
        placement,
        element: frame.into_any_element(),
        merged_footer,
    })
}

#[cfg(test)]
mod tests {
    use super::{composer_send_label, shortcut_key_label};
    use muxy_core::shortcuts::{COMMAND, KeyCombo, OPTION, SHIFT};

    #[test]
    fn composer_send_label_uses_platform_words_and_the_actual_binding() {
        let command_enter = composer_send_label(&KeyCombo::new("return", COMMAND));
        let expected = if cfg!(target_os = "macos") {
            "Cmd+Enter to Send"
        } else {
            "Ctrl+Enter to Send"
        };
        assert_eq!(command_enter.as_ref(), expected);

        let alternate = composer_send_label(&KeyCombo::new("return", OPTION | SHIFT));
        let expected = if cfg!(target_os = "macos") {
            "Option+Shift+Enter to Send"
        } else {
            "Alt+Shift+Enter to Send"
        };
        assert_eq!(alternate.as_ref(), expected);
        assert_eq!(composer_send_label(&KeyCombo::new("", 0)).as_ref(), "Send");
        assert_eq!(shortcut_key_label("f12"), "F12");
    }
}
