use crate::state::AppState;
use crate::terminal::{
    PointerButton, PointerInput, PointerModifiers, SurfaceProgress, SurfaceProgressKind,
    TerminalSurfaces, match_display,
};
use crate::views::app::AppLayout;
use crate::views::swatches::icon_color;
use crate::views::window::MainWindow;
use gpui::Entity;
use gpui::prelude::FluentBuilder;
use gpui::{
    Animation, AnimationExt as _, AnyElement, Bounds, Context, ElementId, FontWeight, Hsla,
    InteractiveElement, IntoElement, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, PathBuilder, Pixels, Point, SharedString,
    StatefulInteractiveElement, Styled, canvas, div, percentage, point, px, svg,
};
use muxy_core::workspace::{
    Axis, Edge, SplitNode, Tab, TabArea, TabKind, TopLevelTabNode, WorkspaceState,
};
use muxy_ui::components::{IconButton, IconGlyph, SymbolGlyph};
use muxy_ui::icon::Icon;
use muxy_ui::scrollbar::{
    MINIMUM_THUMB_LENGTH as SCROLLBAR_MIN_THUMB, ScrollbarRevealState,
    TRACK_INSET as SCROLLBAR_TRACK_INSET, ThumbGeometry, WIDTH as SCROLLBAR_WIDTH,
};
use muxy_ui::text_input::TextInput;
use muxy_ui::theme::Theme;

const TAB_MIN_WIDTH: f32 = 44.0;
const DROP_FILL_ALPHA: f32 = 0.15;
const DROP_BORDER_ALPHA: f32 = 0.40;
const SEARCH_BAR_WIDTH: f32 = 280.0;

pub struct Panes<'a> {
    pub terminals: &'a TerminalSurfaces,
    pub extension_webviews: &'a crate::extensions::webview::ExtensionWebViewRegistry,
    pub area_bounds: &'a std::collections::HashMap<String, Bounds<Pixels>>,
    pub search_inputs: &'a std::collections::HashMap<String, Entity<TextInput>>,
    pub reveal: &'a std::collections::HashMap<String, ScrollbarRevealState>,
    pub attention: &'a std::collections::HashSet<String>,
    pub bell_flashes: &'a std::collections::HashMap<String, std::time::Duration>,
    pub notifications: &'a muxy_core::notifications::NotificationStore,
    pub drag: Option<(&'a str, f64)>,
    pub now: std::time::Duration,
}

impl Panes<'_> {
    fn element(&self, tab: &Tab, visible: bool) -> Option<AnyElement> {
        match tab.kind {
            TabKind::Terminal => self.terminals.element(&tab.id, visible),
            TabKind::ExtensionWebView => self.extension_webviews.element(&tab.id, visible),
            TabKind::Browser => None,
        }
    }

    fn thumb(&self, tab_id: &str, area_id: &str) -> Option<(ThumbGeometry, f64)> {
        if self.terminals.has_native_scrollbar(tab_id) {
            return None;
        }
        if !self
            .reveal
            .get(tab_id)
            .is_some_and(|reveal| reveal.allows_hit(self.now))
        {
            return None;
        }
        let metrics = self.terminals.handle(tab_id)?.metadata().scrollbar;
        let bounds = self.area_bounds.get(area_id)?;
        let track = scrollbar_track_length(*bounds);
        let mut geometry = ThumbGeometry::from_lengths(
            metrics.total as f64,
            metrics.visible as f64,
            metrics.offset as f64,
            track,
            SCROLLBAR_MIN_THUMB,
        )?;
        if let Some((dragged, origin)) = self.drag
            && dragged == tab_id
        {
            geometry.origin = origin;
        }
        Some((geometry, track))
    }

    fn search(&self, tab_id: &str) -> Option<(&Entity<TextInput>, String)> {
        let input = self.search_inputs.get(tab_id)?;
        let totals = self.terminals.handle(tab_id)?.metadata().search_totals;
        Some((input, match_display(totals.total, totals.selected)))
    }

    fn indicator(
        &self,
        workspace: &WorkspaceState,
        root_tab_id: &str,
        is_active: bool,
    ) -> TabIndicator {
        let focused_tab_id = workspace
            .focused_area_id
            .as_deref()
            .and_then(|area_id| workspace.area(area_id))
            .and_then(|area| area.active_tab_id.as_deref());
        let Some(layout) = workspace.visible_layout(root_tab_id) else {
            return TabIndicator::default();
        };
        let mut indicator = TabIndicator::default();
        for tab in layout.tabs() {
            if tab.kind != TabKind::Terminal {
                continue;
            }
            let Some(handle) = self.terminals.handle(&tab.id) else {
                continue;
            };
            let metadata = handle.metadata();
            if !indicator.progress.is_active() && metadata.progress.is_active() {
                indicator.progress = metadata.progress;
            }
            if self
                .bell_flashes
                .get(&tab.id)
                .is_some_and(|deadline| *deadline > self.now)
            {
                indicator.bell_flashing = true;
            }
            if shows_terminal_attention(
                is_active,
                focused_tab_id,
                &tab.id,
                self.attention.contains(&tab.id),
            ) {
                indicator.shows_attention = true;
            }
        }
        if let Some(worktree_id) = workspace.worktree_id.as_deref() {
            indicator.unread =
                self.notifications
                    .unread_tab(&workspace.project_id, worktree_id, root_tab_id)
                    > 0;
        }
        indicator
    }
}

fn shows_terminal_attention(
    root_is_active: bool,
    focused_tab_id: Option<&str>,
    tab_id: &str,
    has_attention: bool,
) -> bool {
    has_attention && (!root_is_active || focused_tab_id != Some(tab_id))
}

#[derive(Clone, Copy, Default)]
struct TabIndicator {
    progress: SurfaceProgress,
    shows_attention: bool,
    bell_flashing: bool,
    unread: bool,
}

fn shows_tab_indicator_dot(indicator: TabIndicator) -> bool {
    indicator.shows_attention
        || (indicator.unread && !indicator.progress.is_active() && !indicator.bell_flashing)
}

pub fn titlebar_tab_strip(
    state: &AppState,
    layout: AppLayout,
    panes: &Panes,
    workspace: &WorkspaceState,
    is_window_titlebar: bool,
    available_width: f32,
    extension_items: &[crate::extensions::ExtensionTopbarBinding],
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    match workspace.top_level_root.as_ref() {
        Some(TopLevelTabNode::Group {
            id,
            tab_ids,
            active_tab_id,
        }) => tab_strip(
            state,
            layout.main_titlebar_leading_inset,
            panes,
            workspace,
            TabStripSpec {
                group_id: id,
                tab_ids,
                active_tab_id: active_tab_id.as_deref(),
                is_window_titlebar,
                available_width,
                extension_items,
            },
            cx,
        ),
        Some(TopLevelTabNode::Split { .. }) => div()
            .flex()
            .flex_none()
            .h(state.metrics.title_bar_height())
            .w_full()
            .bg(state.theme.bg)
            .into_any_element(),
        None => tab_strip(
            state,
            layout.main_titlebar_leading_inset,
            panes,
            workspace,
            TabStripSpec {
                group_id: "empty-workspace",
                tab_ids: &[],
                active_tab_id: None,
                is_window_titlebar,
                available_width,
                extension_items,
            },
            cx,
        ),
    }
}

pub fn workspace_content(
    state: &AppState,
    panes: &Panes,
    workspace: &WorkspaceState,
    available_width: f32,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let content = match workspace.top_level_root.as_ref() {
        Some(TopLevelTabNode::Group {
            tab_ids,
            active_tab_id,
            ..
        }) => active_tab_id
            .as_deref()
            .or_else(|| tab_ids.first().map(String::as_str))
            .map(|root_tab_id| render_group_body(state, panes, workspace, root_tab_id, cx))
            .unwrap_or_else(|| empty_workspace(state)),
        Some(root @ TopLevelTabNode::Split { .. }) => {
            render_outer_node(state, panes, workspace, root, available_width, cx)
        }
        None => empty_workspace(state),
    };

    div()
        .flex()
        .flex_col()
        .flex_grow()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .size_full()
        .child(content)
        .into_any_element()
}

pub fn drop_highlight(
    state: &AppState,
    bounds: Bounds<Pixels>,
    zone: muxy_core::workspace::DropZone,
) -> AnyElement {
    let inset = px(4.0);
    let gap = px(4.0);
    let mut left = bounds.origin.x + inset;
    let mut top = bounds.origin.y + inset;
    let mut width = bounds.size.width - inset * 2.0;
    let mut height = bounds.size.height - inset * 2.0;
    match zone {
        muxy_core::workspace::DropZone::Left => width = width / 2.0 - gap / 2.0,
        muxy_core::workspace::DropZone::Right => {
            width = width / 2.0 - gap / 2.0;
            left = bounds.origin.x + bounds.size.width / 2.0 + gap / 2.0;
        }
        muxy_core::workspace::DropZone::Top => height = height / 2.0 - gap / 2.0,
        muxy_core::workspace::DropZone::Bottom => {
            height = height / 2.0 - gap / 2.0;
            top = bounds.origin.y + bounds.size.height / 2.0 + gap / 2.0;
        }
        muxy_core::workspace::DropZone::Center => {}
    }
    div()
        .absolute()
        .left(left)
        .top(top)
        .w(width.max(px(0.0)))
        .h(height.max(px(0.0)))
        .rounded(state.metrics.radius_md())
        .bg(with_alpha(state.theme.accent, DROP_FILL_ALPHA))
        .border(px(2.0))
        .border_color(with_alpha(state.theme.accent, DROP_BORDER_ALPHA))
        .into_any_element()
}

fn render_outer_node(
    state: &AppState,
    panes: &Panes,
    workspace: &WorkspaceState,
    node: &TopLevelTabNode,
    available_width: f32,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    match node {
        TopLevelTabNode::Group {
            id,
            tab_ids,
            active_tab_id,
        } => render_outer_group(
            state,
            panes,
            workspace,
            id,
            tab_ids,
            active_tab_id.as_deref(),
            available_width,
            cx,
        ),
        TopLevelTabNode::Split {
            id,
            axis,
            ratio,
            first,
            second,
        } => {
            let (first_width, second_width) = match axis {
                Axis::Horizontal => (
                    (available_width * *ratio - 0.5).max(0.0),
                    (available_width * (1.0 - *ratio) - 0.5).max(0.0),
                ),
                Axis::Vertical => (available_width, available_width),
            };
            let first = render_outer_node(state, panes, workspace, first, first_width, cx);
            let second = render_outer_node(state, panes, workspace, second, second_width, cx);
            split_container(
                state,
                id,
                true,
                *axis,
                *ratio,
                SplitChildren { first, second },
                cx,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_outer_group(
    state: &AppState,
    panes: &Panes,
    workspace: &WorkspaceState,
    group_id: &str,
    tab_ids: &[String],
    active_tab_id: Option<&str>,
    available_width: f32,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let body = active_tab_id
        .or_else(|| tab_ids.first().map(String::as_str))
        .map(|root_tab_id| render_group_body(state, panes, workspace, root_tab_id, cx))
        .unwrap_or_else(|| empty_workspace(state));
    let strip = tab_strip(
        state,
        px(0.0),
        panes,
        workspace,
        TabStripSpec {
            group_id,
            tab_ids,
            active_tab_id,
            is_window_titlebar: false,
            available_width,
            extension_items: &[],
        },
        cx,
    );
    let group_id = group_id.to_owned();
    let view = cx.weak_entity();

    div()
        .flex()
        .flex_col()
        .flex_grow()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .size_full()
        .child(strip)
        .child(
            div()
                .h(px(1.0))
                .w_full()
                .flex_none()
                .bg(state.theme.border_solid()),
        )
        .child(
            div()
                .flex()
                .flex_grow()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .child(body),
        )
        .on_children_prepainted(move |bounds, _, cx| {
            let Some(bounds) = union_bounds(&bounds) else {
                return;
            };
            let _ = view.update(cx, |window, _| {
                window.record_group_bounds(&group_id, bounds);
            });
        })
        .into_any_element()
}

fn render_group_body(
    state: &AppState,
    panes: &Panes,
    workspace: &WorkspaceState,
    root_tab_id: &str,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let Some(layout) = workspace.visible_layout(root_tab_id) else {
        return empty_workspace(state);
    };
    let is_maximized = matches!(
        &layout,
        SplitNode::Area { area }
            if workspace.maximized_area_id.as_deref() == Some(area.id.as_str())
    );
    let content = render_physical_node(state, panes, &layout, cx);

    if !is_maximized {
        return content;
    }

    div()
        .relative()
        .flex()
        .flex_grow()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .size_full()
        .p(state.metrics.spacing7())
        .child(
            div()
                .absolute()
                .inset_0()
                .border(state.metrics.spacing7())
                .border_color(state.theme.bg),
        )
        .child(
            div()
                .flex()
                .flex_grow()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .size_full()
                .rounded(state.metrics.radius_lg())
                .border(px(1.0))
                .border_color(state.theme.border)
                .shadow_md()
                .overflow_hidden()
                .child(content),
        )
        .into_any_element()
}

fn render_physical_node(
    state: &AppState,
    panes: &Panes,
    node: &SplitNode,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    match node {
        SplitNode::Area { area } => render_physical_area(state, panes, area, cx),
        SplitNode::Split {
            id,
            axis,
            ratio,
            first,
            second,
        } => {
            let first = render_physical_node(state, panes, first, cx);
            let second = render_physical_node(state, panes, second, cx);
            split_container(
                state,
                id,
                false,
                *axis,
                *ratio,
                SplitChildren { first, second },
                cx,
            )
        }
    }
}

fn render_physical_area(
    state: &AppState,
    panes: &Panes,
    area: &TabArea,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let area_id = area.id.clone();
    let view = cx.weak_entity();
    let content = area
        .active_tab()
        .map(|tab| pane_content(state, panes, tab, &area.id, cx))
        .unwrap_or_else(|| empty_pane(state));

    div()
        .relative()
        .flex()
        .flex_grow()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .size_full()
        .child(content)
        .on_children_prepainted(move |bounds, _, cx| {
            let Some(bounds) = union_bounds(&bounds) else {
                return;
            };
            let _ = view.update(cx, |window, _| {
                window.record_area_bounds(&area_id, bounds);
            });
        })
        .into_any_element()
}

fn pane_content(
    state: &AppState,
    panes: &Panes,
    tab: &Tab,
    area_id: &str,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let surface = panes.element(tab, true);
    let Some(surface) = surface else {
        return pane_placeholder(state, tab, area_id, cx);
    };
    let tab_id = tab.id.clone();
    let area = area_id.to_owned();

    let mut pane = div()
        .id(ElementId::Name(SharedString::from(format!(
            "pane-surface-{}",
            tab.id
        ))))
        .relative()
        .flex()
        .flex_grow()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .size_full()
        .on_any_mouse_down(cx.listener({
            let tab_id = tab_id.clone();
            let area = area.clone();
            move |window: &mut MainWindow, event: &MouseDownEvent, window_handle, cx| {
                window.focus_pane(&tab_id, &area, cx);
                let forwarded = window.forward_pane_pointer(
                    &tab_id,
                    &area,
                    {
                        let (x, y) = pointer_xy(event.position);
                        PointerInput::Down {
                            x,
                            y,
                            button: pointer_button(event.button),
                            modifiers: pointer_modifiers(event.modifiers),
                            click_count: event.click_count,
                        }
                    },
                    cx,
                );
                if forwarded {
                    return;
                }
                match event.button {
                    MouseButton::Left if event.modifiers.platform => {
                        window.begin_pane_drag(tab_id.clone(), area.clone(), event.position, true);
                    }
                    MouseButton::Right => {
                        window.open_terminal_menu(&tab_id, event.position, window_handle, cx);
                    }
                    _ => {}
                }
            }
        }))
        .capture_any_mouse_up(cx.listener({
            let tab_id = tab_id.clone();
            let area = area.clone();
            move |window: &mut MainWindow, event: &MouseUpEvent, _, cx| {
                window.forward_pane_pointer(
                    &tab_id,
                    &area,
                    {
                        let (x, y) = pointer_xy(event.position);
                        PointerInput::Up {
                            x,
                            y,
                            button: pointer_button(event.button),
                            modifiers: pointer_modifiers(event.modifiers),
                        }
                    },
                    cx,
                );
            }
        }))
        .on_mouse_move(cx.listener({
            let tab_id = tab_id.clone();
            let area = area.clone();
            move |window: &mut MainWindow, event: &MouseMoveEvent, _, cx| {
                window.forward_pane_pointer(
                    &tab_id,
                    &area,
                    {
                        let (x, y) = pointer_xy(event.position);
                        PointerInput::Moved {
                            x,
                            y,
                            modifiers: pointer_modifiers(event.modifiers),
                        }
                    },
                    cx,
                );
            }
        }))
        .child(surface);

    if let Some((geometry, track)) = panes.thumb(&tab.id, area_id) {
        pane = pane.child(pane_scrollbar(state, &tab_id, &area, geometry, track, cx));
    }
    if let Some((input, count)) = panes.search(&tab.id) {
        pane = pane.child(pane_search_bar(state, &tab_id, input, count, cx));
    }

    pane.into_any_element()
}

fn pointer_xy(position: Point<Pixels>) -> (f64, f64) {
    (f64::from(position.x), f64::from(position.y))
}

fn pointer_button(button: MouseButton) -> PointerButton {
    match button {
        MouseButton::Left => PointerButton::Left,
        MouseButton::Right => PointerButton::Right,
        MouseButton::Middle => PointerButton::Middle,
        _ => PointerButton::Other,
    }
}

fn pointer_modifiers(modifiers: Modifiers) -> PointerModifiers {
    PointerModifiers {
        shift: modifiers.shift,
        control: modifiers.control,
        alt: modifiers.alt,
        platform: modifiers.platform,
    }
}

pub fn scrollbar_track_length(bounds: Bounds<Pixels>) -> f64 {
    (f64::from(bounds.size.height) - f64::from(SCROLLBAR_TRACK_INSET) * 2.0).max(0.0)
}

fn pane_scrollbar(
    state: &AppState,
    tab_id: &str,
    area_id: &str,
    geometry: ThumbGeometry,
    track: f64,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let tab_id = tab_id.to_owned();
    let area_id = area_id.to_owned();
    div()
        .absolute()
        .right(px(2.0))
        .top(px(SCROLLBAR_TRACK_INSET))
        .w(px(SCROLLBAR_WIDTH))
        .h(px(track as f32))
        .child(
            div()
                .id(ElementId::Name(SharedString::from(format!(
                    "pane-scrollbar-{tab_id}"
                ))))
                .absolute()
                .top(px(geometry.origin as f32))
                .w_full()
                .h(px(geometry.length as f32))
                .rounded_full()
                .bg(with_alpha(state.theme.fg, 0.35))
                .cursor_pointer()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(
                        move |window: &mut MainWindow, event: &MouseDownEvent, _, cx| {
                            window.begin_scrollbar_drag(&tab_id, &area_id, event.position, cx);
                            cx.stop_propagation();
                        },
                    ),
                ),
        )
        .into_any_element()
}

fn pane_search_bar(
    state: &AppState,
    tab_id: &str,
    input: &Entity<TextInput>,
    count: String,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let metrics = &state.metrics;
    let theme = &state.theme;
    let previous_tab = tab_id.to_owned();
    let next_tab = tab_id.to_owned();
    let close_tab = tab_id.to_owned();

    div()
        .absolute()
        .top(metrics.spacing5())
        .right(metrics.spacing6())
        .w(metrics.scaled(SEARCH_BAR_WIDTH))
        .h(metrics.control_large())
        .px(metrics.spacing4())
        .flex()
        .flex_row()
        .items_center()
        .gap(metrics.spacing3())
        .rounded(metrics.radius_md())
        .bg(theme.raised())
        .border_1()
        .border_color(theme.border)
        .shadow_md()
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(muxy_ui::text_input::growing_input(input))
        .child(
            div()
                .flex_none()
                .text_size(metrics.font_caption())
                .text_color(theme.fg_muted)
                .child(SharedString::from(count)),
        )
        .child(
            search_button("pane-search-previous", "chevron.up", state).on_click(cx.listener(
                move |window: &mut MainWindow, _, _, cx| {
                    let _ = &previous_tab;
                    window.navigate_search(false, cx);
                },
            )),
        )
        .child(
            search_button("pane-search-next", "chevron.down", state).on_click(cx.listener(
                move |window: &mut MainWindow, _, _, cx| {
                    let _ = &next_tab;
                    window.navigate_search(true, cx);
                },
            )),
        )
        .child(
            search_button("pane-search-close", "xmark", state).on_click(cx.listener(
                move |window: &mut MainWindow, _, _, cx| {
                    window.close_search(&close_tab, cx);
                },
            )),
        )
        .into_any_element()
}

fn search_button(
    id: &'static str,
    symbol: &'static str,
    state: &AppState,
) -> gpui::Stateful<gpui::Div> {
    let metrics = &state.metrics;
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(metrics.control_small())
        .rounded(metrics.radius_sm())
        .cursor_pointer()
        .hover(|style| style.bg(state.theme.hover))
        .child(SymbolGlyph::new(
            symbol,
            metrics.font_caption(),
            state.theme.fg_muted,
        ))
}

fn pane_placeholder(
    state: &AppState,
    tab: &Tab,
    area_id: &str,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let tab_id = tab.id.clone();
    let dragged_tab_id = tab.id.clone();
    let dragged_area_id = area_id.to_owned();
    let focused_area_id = area_id.to_owned();
    let color = icon_color(tab.color_id.as_deref())
        .map(Hsla::from)
        .unwrap_or(state.theme.fg_dim);

    div()
        .id(ElementId::Name(SharedString::from(format!(
            "pane-placeholder-{tab_id}"
        ))))
        .flex()
        .flex_col()
        .flex_grow()
        .items_center()
        .justify_center()
        .bg(state.theme.bg)
        .gap(state.metrics.spacing4())
        .min_w(px(0.0))
        .min_h(px(0.0))
        .size_full()
        .cursor_grab()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(
                move |window: &mut MainWindow, event: &MouseDownEvent, view, cx| {
                    window.focus_workspace(view);
                    window.focus_area(&focused_area_id, cx);
                    window.begin_pane_drag(
                        dragged_tab_id.clone(),
                        dragged_area_id.clone(),
                        event.position,
                        event.modifiers.platform,
                    );
                },
            ),
        )
        .child(tab_glyph(tab, state.metrics.icon_xxl(), color))
        .child(
            div()
                .text_size(state.metrics.font_body())
                .font_weight(FontWeight::MEDIUM)
                .text_color(state.theme.fg_dim)
                .child(SharedString::from(kind_title(tab.kind))),
        )
        .into_any_element()
}

fn empty_workspace(state: &AppState) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_grow()
        .items_center()
        .justify_center()
        .gap(state.metrics.spacing4())
        .min_w(px(0.0))
        .min_h(px(0.0))
        .size_full()
        .bg(state.theme.bg)
        .child(IconGlyph::new(
            Icon::Terminal,
            state.metrics.icon_xxl(),
            state.theme.fg_dim,
        ))
        .child(
            div()
                .text_size(state.metrics.font_body())
                .text_color(state.theme.fg_dim)
                .child(SharedString::from("No tabs")),
        )
        .into_any_element()
}

fn empty_pane(state: &AppState) -> AnyElement {
    div()
        .flex()
        .flex_grow()
        .items_center()
        .justify_center()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .size_full()
        .bg(state.theme.bg)
        .into_any_element()
}

struct TabStripSpec<'a> {
    group_id: &'a str,
    tab_ids: &'a [String],
    active_tab_id: Option<&'a str>,
    is_window_titlebar: bool,
    available_width: f32,
    extension_items: &'a [crate::extensions::ExtensionTopbarBinding],
}

fn tab_strip(
    state: &AppState,
    leading_inset: Pixels,
    panes: &Panes,
    workspace: &WorkspaceState,
    spec: TabStripSpec<'_>,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let TabStripSpec {
        group_id,
        tab_ids,
        active_tab_id,
        is_window_titlebar,
        available_width,
        extension_items,
    } = spec;
    let tabs: Vec<&Tab> = tab_ids
        .iter()
        .filter_map(|tab_id| workspace.tab(tab_id))
        .collect();
    let target_root_tab_id = active_tab_id.or_else(|| tab_ids.first().map(String::as_str));
    let show_maximize = workspace.maximized_area_id.is_some()
        || target_root_tab_id
            .and_then(|tab_id| workspace.visible_layout(tab_id))
            .is_some_and(|layout| layout.area_ids().len() > 1);
    let actions_visible = !is_window_titlebar || state.prefs.show_topbar_actions;
    let action_count = if actions_visible {
        2 + extension_items.len()
            + usize::from(show_maximize)
            + usize::from(state.prefs.browser_enabled)
            + usize::from(!state.layouts().is_empty())
    } else {
        0
    };
    let action_width = action_count as f32 * f32::from(state.metrics.control_medium())
        + if actions_visible {
            f32::from(state.metrics.spacing6())
        } else {
            0.0
        };
    let leading = if is_window_titlebar {
        leading_inset
    } else {
        px(0.0)
    };
    let strip_width = (available_width - f32::from(leading) - action_width).max(0.0);
    let pinned_count = tabs.iter().take_while(|tab| tab.pinned).count();
    let layout = muxy_core::workspace::TabStripLayout::calculate(
        muxy_core::workspace::Rect::new(0.0, 0.0, strip_width, 32.0),
        tabs.len(),
        pinned_count,
        muxy_core::workspace::TabStripMetrics {
            max_tab_width: state.prefs.tab_max_width,
        },
    );
    let recorded_tab_ids: Vec<String> = tabs.iter().map(|tab| tab.id.clone()).collect();
    let view = cx.weak_entity();
    let mut cells = div()
        .flex()
        .flex_row()
        .flex_nowrap()
        .flex_none()
        .min_w(px(0.0))
        .h_full();

    for tab in tabs {
        let is_active = active_tab_id == Some(tab.id.as_str());
        cells = cells.child(tab_cell(
            state,
            workspace,
            tab,
            TabCellSpec {
                group_id,
                is_active,
                tab_width: layout.tab_width,
                shows_title: layout.shows_titles,
                indicator: panes.indicator(workspace, &tab.id, is_active),
            },
            cx,
        ));
    }

    let cells = cells.on_children_prepainted(move |bounds, _, cx| {
        let _ = view.update(cx, |window, _| {
            for (tab_id, bounds) in recorded_tab_ids.iter().zip(bounds) {
                window.record_tab_bounds(tab_id, bounds);
            }
        });
    });
    let new_button = new_tab_button(state, group_id, target_root_tab_id, cx);
    let mut scroll_row = cells;
    let mut pinned_new_button = None;
    if layout.pins_new_tab_button {
        pinned_new_button = Some(new_button);
    } else {
        scroll_row = scroll_row.child(new_button);
    }

    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .w_full()
        .h(state.metrics.title_bar_height())
        .pl(leading)
        .bg(state.theme.bg)
        .child(
            div()
                .id(ElementId::Name(SharedString::from(format!(
                    "workspace-tabs-scroll-{group_id}"
                ))))
                .flex()
                .flex_grow()
                .min_w(px(0.0))
                .h_full()
                .overflow_x_scroll()
                .child(scroll_row),
        )
        .children(pinned_new_button)
        .when(
            is_window_titlebar && !extension_items.is_empty(),
            |element| {
                element.child(crate::views::titlebar::extension_actions(
                    state,
                    extension_items,
                    cx,
                ))
            },
        )
        .when(
            !is_window_titlebar || state.prefs.show_topbar_actions,
            |element| {
                element.child(strip_actions(
                    state,
                    workspace,
                    group_id,
                    target_root_tab_id,
                    cx,
                ))
            },
        )
        .into_any_element()
}

struct TabCellSpec<'a> {
    group_id: &'a str,
    is_active: bool,
    tab_width: f32,
    shows_title: bool,
    indicator: TabIndicator,
}

fn tab_cell(
    state: &AppState,
    workspace: &WorkspaceState,
    tab: &Tab,
    spec: TabCellSpec<'_>,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let TabCellSpec {
        group_id,
        is_active,
        tab_width,
        shows_title,
        indicator,
    } = spec;
    let metrics = &state.metrics;
    let theme = &state.theme;
    let is_focused = workspace.focused_root_tab_id() == Some(tab.id.as_str());
    let group = SharedString::from(format!("workspace-tab-{}", tab.id));
    let tab_id = tab.id.clone();
    let selected_tab_id = tab.id.clone();
    let middle_close_tab_id = tab.id.clone();
    let dragged_tab_id = tab.id.clone();
    let dragged_group_id = group_id.to_owned();
    let menu_tab_id = tab.id.clone();
    let custom_tint = icon_color(tab.color_id.as_deref()).map(Hsla::from);
    let background = if is_active {
        theme.surface
    } else {
        gpui::transparent_black()
    };
    let base_tint = custom_tint
        .map(|color| with_alpha(color, if is_active { 0.18 } else { 0.04 }))
        .unwrap_or_else(gpui::transparent_black);
    let hover = custom_tint
        .map(|color| with_alpha(color, if is_active { 0.18 } else { 0.08 }))
        .unwrap_or_else(gpui::transparent_black);
    let icon_color = custom_tint.unwrap_or(if is_active { theme.fg } else { theme.fg_muted });
    let trailing = if tab.pinned {
        div().into_any_element()
    } else {
        div()
            .id(ElementId::Name(SharedString::from(format!(
                "close-workspace-tab-{tab_id}"
            ))))
            .absolute()
            .top_0()
            .flex()
            .items_center()
            .justify_center()
            .h_full()
            .when(shows_title, |element| {
                element.right(metrics.spacing5()).w(metrics.icon_md())
            })
            .when(!shows_title, |element| element.left_0().right_0().w_full())
            .cursor_pointer()
            .opacity(if is_active && shows_title { 1.0 } else { 0.0 })
            .group_hover(group.clone(), |style| style.opacity(1.0))
            .on_click(cx.listener(move |window: &mut MainWindow, _, _, cx| {
                cx.stop_propagation();
                window.close_tab(&tab_id, cx);
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(metrics.icon_md())
                    .rounded(metrics.radius_sm())
                    .hover(|style| style.bg(theme.hover))
                    .child(IconGlyph::new(
                        Icon::X,
                        metrics.font_caption(),
                        theme.fg_muted,
                    )),
            )
            .into_any_element()
    };

    let mut cell = div()
        .id(ElementId::Name(SharedString::from(format!(
            "workspace-tab-{}",
            tab.id
        ))))
        .group(group.clone())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .w(px(tab_width.max(TAB_MIN_WIDTH)))
        .h_full()
        .overflow_hidden()
        .bg(background)
        .border_r(px(1.0))
        .border_color(theme.border)
        .cursor_pointer()
        .on_click(cx.listener(move |window: &mut MainWindow, _, view, cx| {
            window.focus_workspace(view);
            window.select_root_tab(&selected_tab_id, cx);
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(
                move |window: &mut MainWindow, event: &MouseDownEvent, _, cx| {
                    if event.click_count == 2 {
                        window.start_tab_rename(dragged_tab_id.clone(), Some(event.position), cx);
                    } else {
                        window.begin_tab_drag(
                            dragged_tab_id.clone(),
                            dragged_group_id.clone(),
                            event.position,
                        );
                    }
                },
            ),
        )
        .when(!tab.pinned, |element| {
            element.on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |window: &mut MainWindow, _, _, cx| {
                    window.close_tab(&middle_close_tab_id, cx);
                }),
            )
        })
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(
                move |window: &mut MainWindow, event: &MouseDownEvent, view, cx| {
                    window.open_tab_menu(&menu_tab_id, event.position, view, cx);
                },
            ),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .left_0()
                .bg(base_tint),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .left_0()
                .group_hover(group.clone(), |style| style.bg(hover)),
        )
        .child(
            div()
                .relative()
                .flex()
                .flex_row()
                .flex_grow()
                .items_center()
                .gap(metrics.spacing3())
                .min_w(px(0.0))
                .h_full()
                .when(shows_title, |element| {
                    element.pl(metrics.spacing6()).pr(metrics.icon_xxl())
                })
                .when(!shows_title, |element| element.justify_center())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(!shows_title && !tab.pinned, |element| {
                            element.group_hover(group.clone(), |style| style.opacity(0.0))
                        })
                        .child(tab_indicator_glyph(
                            tab,
                            indicator,
                            metrics.icon_md(),
                            metrics.icon_sm(),
                            icon_color,
                            theme,
                        )),
                )
                .when(shows_title, |element| {
                    element.child(
                        div()
                            .flex_grow()
                            .min_w(px(0.0))
                            .text_size(metrics.font_body())
                            .font_weight(if is_active {
                                FontWeight::MEDIUM
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(if is_active { theme.fg } else { theme.fg_muted })
                            .truncate()
                            .child(SharedString::from(tab.title().to_owned())),
                    )
                }),
        )
        .child(trailing);

    if is_focused {
        cell = cell.child(
            div()
                .absolute()
                .right_0()
                .bottom_0()
                .left_0()
                .h(px(2.0))
                .bg(custom_tint.unwrap_or(theme.accent)),
        );
    } else if !is_active && let Some(color) = custom_tint {
        cell = cell.child(
            div()
                .absolute()
                .right_0()
                .bottom_0()
                .left_0()
                .h(px(2.0))
                .bg(color),
        );
    }

    cell.into_any_element()
}

fn new_tab_button(
    state: &AppState,
    group_id: &str,
    target_root_tab_id: Option<&str>,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let id = SharedString::from(format!("new-terminal-{group_id}"));
    let target_root_tab_id = target_root_tab_id.map(str::to_owned);
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .pl(state.metrics.spacing2())
        .w(state.metrics.icon_xxl())
        .h_full()
        .child(
            IconButton::new(
                id,
                Icon::Plus,
                state.metrics.scaled(13.0),
                state.metrics.control_medium(),
                state.theme.fg_muted,
                state.theme.fg,
            )
            .tooltip(
                state
                    .shortcuts
                    .tooltip("New Tab", muxy_core::shortcuts::ShortcutAction::NewTab),
                state.theme.raised(),
                state.theme.fg,
                state.theme.border,
            )
            .on_click(cx.listener(move |window: &mut MainWindow, _, _, cx| {
                if let Some(tab_id) = target_root_tab_id.as_deref() {
                    window.select_root_tab(tab_id, cx);
                }
                window.new_terminal_tab(cx);
            })),
        )
        .into_any_element()
}

fn strip_actions(
    state: &AppState,
    workspace: &WorkspaceState,
    group_id: &str,
    target_root_tab_id: Option<&str>,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let metrics = &state.metrics;
    let theme = &state.theme;
    let glyph_size = metrics.scaled(13.0);
    let box_size = metrics.control_medium();
    let maximize_icon = if workspace.maximized_area_id.is_some() {
        Icon::Restore
    } else {
        Icon::Maximize
    };
    let maximize_target = target_root_tab_id.map(str::to_owned);
    let split_right_target = target_root_tab_id.map(str::to_owned);
    let split_down_target = target_root_tab_id.map(str::to_owned);
    let show_maximize = workspace.maximized_area_id.is_some()
        || target_root_tab_id
            .and_then(|tab_id| workspace.visible_layout(tab_id))
            .is_some_and(|layout| layout.area_ids().len() > 1);
    let has_layouts = !state.layouts().is_empty();
    let visible_actions = strip_action_ids(show_maximize, has_layouts);

    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .h_full()
        .pr(metrics.spacing2())
        .when(visible_actions.contains(&"maximize"), |element| {
            element.child(
                IconButton::new(
                    SharedString::from(format!("maximize-pane-{group_id}")),
                    maximize_icon,
                    glyph_size,
                    box_size,
                    theme.fg_muted,
                    theme.fg,
                )
                .tooltip(
                    state.shortcuts.tooltip(
                        if workspace.maximized_area_id.is_some() {
                            "Restore Pane"
                        } else {
                            "Maximize Pane"
                        },
                        muxy_core::shortcuts::ShortcutAction::ToggleMaximizePane,
                    ),
                    theme.raised(),
                    theme.fg,
                    theme.border,
                )
                .on_click(cx.listener(move |window: &mut MainWindow, _, _, cx| {
                    if let Some(tab_id) = maximize_target.as_deref() {
                        window.select_root_tab(tab_id, cx);
                    }
                    window.toggle_maximize(cx);
                })),
            )
        })
        .when(visible_actions.contains(&"split-right"), |element| {
            element.child(
                IconButton::new(
                    SharedString::from(format!("split-pane-right-{group_id}")),
                    Icon::Columns,
                    glyph_size,
                    box_size,
                    theme.fg_muted,
                    theme.fg,
                )
                .tooltip(
                    state.shortcuts.tooltip(
                        "Split Right",
                        muxy_core::shortcuts::ShortcutAction::SplitRight,
                    ),
                    theme.raised(),
                    theme.fg,
                    theme.border,
                )
                .on_click(cx.listener(move |window: &mut MainWindow, _, _, cx| {
                    if let Some(tab_id) = split_right_target.as_deref() {
                        window.select_root_tab(tab_id, cx);
                    }
                    window.split_focused(Edge::Right, cx);
                })),
            )
        })
        .when(visible_actions.contains(&"split-down"), |element| {
            element.child(
                IconButton::new(
                    SharedString::from(format!("split-pane-down-{group_id}")),
                    Icon::Rows,
                    glyph_size,
                    box_size,
                    theme.fg_muted,
                    theme.fg,
                )
                .tooltip(
                    state.shortcuts.tooltip(
                        "Split Down",
                        muxy_core::shortcuts::ShortcutAction::SplitDown,
                    ),
                    theme.raised(),
                    theme.fg,
                    theme.border,
                )
                .on_click(cx.listener(move |window: &mut MainWindow, _, _, cx| {
                    if let Some(tab_id) = split_down_target.as_deref() {
                        window.select_root_tab(tab_id, cx);
                    }
                    window.split_focused(Edge::Bottom, cx);
                })),
            )
        })
        .when(visible_actions.contains(&"apply-layout"), |element| {
            element.child(
                IconButton::new(
                    SharedString::from(format!("apply-layout-{group_id}")),
                    Icon::LayoutSplit,
                    glyph_size,
                    box_size,
                    theme.fg_muted,
                    theme.fg,
                )
                .tooltip("Apply Layout", theme.raised(), theme.fg, theme.border)
                .on_click(cx.listener(
                    move |window: &mut MainWindow, event: &gpui::ClickEvent, view, cx| {
                        window.open_terminal_layout_menu(event.position(), view, cx);
                    },
                )),
            )
        })
        .into_any_element()
}

fn strip_action_ids(show_maximize: bool, has_layouts: bool) -> Vec<&'static str> {
    let mut ids = Vec::new();
    if show_maximize {
        ids.push("maximize");
    }
    ids.extend(["split-right", "split-down"]);
    if has_layouts {
        ids.push("apply-layout");
    }
    ids
}

struct SplitChildren {
    first: AnyElement,
    second: AnyElement,
}

fn split_container(
    state: &AppState,
    split_id: &str,
    is_top_level: bool,
    axis: Axis,
    ratio: f32,
    children: SplitChildren,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let SplitChildren { first, second } = children;
    let split_id_for_bounds = split_id.to_owned();
    let view = cx.weak_entity();
    let divider = split_divider(state, split_id, is_top_level, axis, ratio, cx);
    let mut container = div()
        .flex()
        .flex_grow()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .size_full();

    container = match axis {
        Axis::Horizontal => {
            let mut first_slot = div()
                .flex()
                .flex_basis(px(0.0))
                .min_w(px(0.0))
                .h_full()
                .child(first);
            first_slot.style().flex_grow = Some(ratio);
            let mut second_slot = div()
                .flex()
                .flex_basis(px(0.0))
                .min_w(px(0.0))
                .h_full()
                .child(second);
            second_slot.style().flex_grow = Some(1.0 - ratio);
            container
                .flex_row()
                .child(first_slot)
                .child(divider)
                .child(second_slot)
        }
        Axis::Vertical => {
            let mut first_slot = div()
                .flex()
                .flex_basis(px(0.0))
                .min_h(px(0.0))
                .w_full()
                .child(first);
            first_slot.style().flex_grow = Some(ratio);
            let mut second_slot = div()
                .flex()
                .flex_basis(px(0.0))
                .min_h(px(0.0))
                .w_full()
                .child(second);
            second_slot.style().flex_grow = Some(1.0 - ratio);
            container
                .flex_col()
                .child(first_slot)
                .child(divider)
                .child(second_slot)
        }
    };

    container
        .on_children_prepainted(move |bounds, _, cx| {
            let Some(bounds) = union_bounds(&bounds) else {
                return;
            };
            let _ = view.update(cx, |window, _| {
                window.record_split_bounds(&split_id_for_bounds, bounds);
            });
        })
        .into_any_element()
}

fn split_divider(
    state: &AppState,
    split_id: &str,
    is_top_level: bool,
    axis: Axis,
    ratio: f32,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let split_id = split_id.to_owned();
    let group = SharedString::from(format!("split-divider-group-{split_id}"));
    let id = ElementId::Name(SharedString::from(format!(
        "{}-split-divider-{split_id}",
        if is_top_level { "outer" } else { "pane" }
    )));
    let hit = div().id(id).absolute().on_mouse_down(
        MouseButton::Left,
        cx.listener(
            move |window: &mut MainWindow, event: &MouseDownEvent, _, _| {
                window.begin_resize(split_id.clone(), is_top_level, axis, ratio, event.position);
            },
        ),
    );

    match axis {
        Axis::Horizontal => div()
            .group(group.clone())
            .relative()
            .flex_none()
            .w(px(1.0))
            .h_full()
            .bg(state.theme.border_solid())
            .child(
                div()
                    .absolute()
                    .size_full()
                    .opacity(0.0)
                    .bg(state.theme.accent)
                    .group_hover(group.clone(), |style| style.opacity(1.0)),
            )
            .child(
                hit.left(px(-4.5))
                    .top_0()
                    .w(state.metrics.resize_handle_hit_area())
                    .h_full()
                    .cursor_ew_resize(),
            )
            .into_any_element(),
        Axis::Vertical => div()
            .group(group.clone())
            .relative()
            .flex_none()
            .w_full()
            .h(px(1.0))
            .bg(state.theme.border_solid())
            .child(
                div()
                    .absolute()
                    .size_full()
                    .opacity(0.0)
                    .bg(state.theme.accent)
                    .group_hover(group, |style| style.opacity(1.0)),
            )
            .child(
                hit.left_0()
                    .top(px(-4.5))
                    .w_full()
                    .h(state.metrics.resize_handle_hit_area())
                    .cursor_ns_resize(),
            )
            .into_any_element(),
    }
}

fn tab_indicator_glyph(
    tab: &Tab,
    indicator: TabIndicator,
    size: Pixels,
    progress_size: Pixels,
    color: Hsla,
    theme: &Theme,
) -> AnyElement {
    let content = if indicator.progress.is_active() {
        progress_circle(&tab.id, indicator.progress, progress_size, theme).into_any_element()
    } else if indicator.bell_flashing {
        IconGlyph::new(Icon::Bell, size, theme.accent).into_any_element()
    } else {
        tab_glyph(tab, size, color)
    };
    div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(size)
        .child(content)
        .when(shows_tab_indicator_dot(indicator), |element| {
            element.child(
                div()
                    .absolute()
                    .top(px(-2.0))
                    .right(px(-2.0))
                    .size(px(6.0))
                    .rounded_full()
                    .bg(theme.accent),
            )
        })
        .into_any_element()
}

fn progress_circle(
    tab_id: &str,
    progress: SurfaceProgress,
    size: Pixels,
    theme: &Theme,
) -> AnyElement {
    let color = match progress.kind {
        Some(SurfaceProgressKind::Error) => theme.danger,
        Some(SurfaceProgressKind::Paused) => theme.warning,
        _ => theme.accent,
    };
    if progress.kind != Some(SurfaceProgressKind::Indeterminate) {
        return progress_ring(size, progress.fraction(), color).into_any_element();
    }
    svg()
        .path("icons/progress-indeterminate.svg")
        .size(size)
        .text_color(color)
        .with_animation(
            SharedString::from(format!("terminal-progress-{tab_id}")),
            Animation::new(std::time::Duration::from_secs(1)).repeat(),
            |svg, delta| svg.with_transformation(gpui::Transformation::rotate(percentage(delta))),
        )
        .into_any_element()
}

fn progress_ring(size: Pixels, fraction: f32, color: Hsla) -> impl IntoElement {
    let line_width = px((f32::from(size) / 8.0).max(1.0));
    canvas(
        move |bounds, _, _| {
            (
                ring_path(bounds, 1.0, line_width),
                ring_path(bounds, fraction.max(0.001), line_width),
            )
        },
        move |_, paths, window, _| {
            if let Some(path) = paths.0 {
                window.paint_path(path, with_alpha(color, 0.25));
            }
            if let Some(path) = paths.1 {
                window.paint_path(path, color);
            }
        },
    )
    .size(size)
}

fn ring_path(
    bounds: Bounds<Pixels>,
    fraction: f32,
    line_width: Pixels,
) -> Option<gpui::Path<Pixels>> {
    let fraction = fraction.clamp(0.0, 1.0);
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let center_x = f32::from(bounds.origin.x) + width / 2.0;
    let center_y = f32::from(bounds.origin.y) + height / 2.0;
    let radius = (width.min(height) - f32::from(line_width)) / 2.0;
    let steps = (48.0 * fraction).ceil().max(1.0) as usize;
    let mut builder = PathBuilder::stroke(line_width);
    for step in 0..=steps {
        let angle = -std::f32::consts::FRAC_PI_2
            + std::f32::consts::TAU * fraction * step as f32 / steps as f32;
        let point = point(
            px(center_x + radius * angle.cos()),
            px(center_y + radius * angle.sin()),
        );
        if step == 0 {
            builder.move_to(point);
        } else {
            builder.line_to(point);
        }
    }
    if fraction == 1.0 {
        builder.close();
    }
    builder.build().ok()
}

fn tab_glyph(tab: &Tab, size: Pixels, color: Hsla) -> AnyElement {
    if tab.pinned {
        return IconGlyph::new(Icon::Pin, size, color).into_any_element();
    }
    match tab.custom_icon.as_ref() {
        Some(symbol) => SymbolGlyph::new(symbol.clone(), size, color).into_any_element(),
        None => IconGlyph::new(kind_icon(tab.kind), size, color).into_any_element(),
    }
}

fn kind_icon(kind: TabKind) -> Icon {
    match kind {
        TabKind::Terminal => Icon::Terminal,
        TabKind::Browser => Icon::Globe,
        TabKind::ExtensionWebView => Icon::Puzzle,
    }
}

fn kind_title(kind: TabKind) -> &'static str {
    match kind {
        TabKind::Terminal => "Terminal",
        TabKind::Browser => "Browser",
        TabKind::ExtensionWebView => "Extension",
    }
}

fn with_alpha(color: Hsla, alpha: f32) -> Hsla {
    Hsla {
        a: color.a * alpha,
        ..color
    }
}

fn union_bounds(bounds: &[Bounds<Pixels>]) -> Option<Bounds<Pixels>> {
    bounds
        .iter()
        .copied()
        .reduce(|combined, bounds| combined.union(&bounds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attention_stays_on_inactive_roots_and_unfocused_split_panes() {
        assert!(shows_terminal_attention(false, Some("a"), "a", true));
        assert!(shows_terminal_attention(true, Some("a"), "b", true));
        assert!(!shows_terminal_attention(true, Some("a"), "a", true));
        assert!(!shows_terminal_attention(false, None, "a", false));
    }

    #[test]
    fn unread_dot_never_replaces_progress_or_bell_and_coexists_with_attention() {
        let unread = TabIndicator {
            unread: true,
            ..Default::default()
        };
        assert!(shows_tab_indicator_dot(unread));
        assert!(!shows_tab_indicator_dot(TabIndicator {
            progress: SurfaceProgress {
                kind: Some(SurfaceProgressKind::Set),
                value: Some(0.5),
            },
            ..unread
        }));
        assert!(!shows_tab_indicator_dot(TabIndicator {
            bell_flashing: true,
            ..unread
        }));
        assert!(shows_tab_indicator_dot(TabIndicator {
            progress: SurfaceProgress {
                kind: Some(SurfaceProgressKind::Set),
                value: Some(0.5),
            },
            shows_attention: true,
            ..unread
        }));
    }

    #[test]
    fn progress_ring_paths_clamp_fraction_extremes() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), gpui::size(px(12.0), px(12.0)));
        assert!(ring_path(bounds, -1.0, px(1.5)).is_some());
        assert!(ring_path(bounds, 0.5, px(1.5)).is_some());
        assert!(ring_path(bounds, 2.0, px(1.5)).is_some());
    }

    #[test]
    fn chrome_tab_strip_contract_excludes_the_browser_action() {
        assert_eq!(
            strip_action_ids(false, false),
            vec!["split-right", "split-down"]
        );
        assert_eq!(
            strip_action_ids(true, true),
            vec!["maximize", "split-right", "split-down", "apply-layout"]
        );
    }
}
