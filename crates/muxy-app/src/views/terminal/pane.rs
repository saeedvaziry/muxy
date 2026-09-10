use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    Bounds, Context, EventEmitter, FocusHandle, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Render, Styled, Task, Window, div, prelude::FluentBuilder, px,
};
use muxy_client::{Attachment, RunGrid};
use muxy_protocol::{
    ChannelId, ExitReason, ForegroundProcess, HistoryPage, InputModes, MetadataEvent, MouseAction,
    MouseEvent, SavedScreen, ScreenFrame, ScrollDirection, ServerPath, Size,
};

use super::{
    clipboard,
    colors::Palette,
    element, find, input,
    scroll::{HistoryRequest, Scroll},
    selection::{Point, Selection},
};

pub(crate) enum PaneEvent {
    Focused,
    OpenLink(muxy_app_core::opener::Target),
    ContextMenu(gpui::Point<gpui::Pixels>),
    Viewport(Size),
    Input(ChannelId, Vec<u8>),
    Mouse(ChannelId, MouseEvent),
    Title(String),
    Bell,
    History(HistoryRequest),
    Search(find::SearchRequest),
}

gpui::actions!(terminal, [CloseFind]);

pub(crate) fn register_shortcuts(registry: &mut muxy_ui::shortcuts::Registry<'_>) {
    registry.register(
        muxy_core::shortcuts::ShortcutId::TerminalCloseFind,
        &CloseFind,
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PaneState {
    Connecting,
    Live,
    Disconnected,
    Exited {
        reason: Option<ExitReason>,
        unavailable: bool,
    },
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "Visibility, focus restoration and bell activity are independent pane states"
)]
pub(crate) struct TerminalPane {
    pub(crate) grid: Option<RunGrid>,
    pub(crate) copy_on_select: bool,
    pub(crate) open_context: Option<muxy_app_core::opener::OpenContext>,
    pub(crate) link_hover: super::links::Hover,
    pub(crate) scroll: Scroll,
    pub(crate) find: Option<find::Find>,
    pub(crate) find_refresh: Option<Task<()>>,
    pub(crate) find_focus_pending: bool,
    pub(crate) selection: Option<Selection>,
    selection_rows: Vec<Option<Vec<muxy_protocol::Run>>>,
    selecting: Option<(Selection, usize)>,
    pub(crate) input_modes: InputModes,
    held_buttons: Vec<MouseButton>,
    last_mouse: Option<MouseEvent>,
    wheel_remainder: f32,
    pub(crate) geometry: Option<(Bounds<gpui::Pixels>, gpui::Size<gpui::Pixels>)>,
    pub(crate) cell_height: f32,
    pub(crate) native_visible: bool,
    #[cfg(target_os = "macos")]
    pub(crate) native_scroll: Option<muxy_ui::native_scroll::NativeScrollView>,
    native_sequence: u64,
    saved_history: Option<HistoryRequest>,
    pub(crate) focus: FocusHandle,
    pub(crate) focused: bool,
    pub(crate) cursor_blink: super::cursor::CursorBlink,
    pub(crate) focus_border: Option<gpui::Hsla>,
    pub(crate) corner_radius: gpui::Pixels,
    channel: Option<ChannelId>,
    viewport: Option<Size>,
    pub(crate) palette: Palette,
    pub(crate) terminal: muxy_settings::TerminalSettings,
    pub(crate) state: PaneState,
    pub(crate) process: Option<ForegroundProcess>,
    title: String,
    directory: ServerPath,
    pub(crate) bell_flashing: bool,
    bell_expiry: Option<Task<()>>,
}

impl EventEmitter<PaneEvent> for TerminalPane {}

impl TerminalPane {
    pub(crate) fn new(
        palette: Palette,
        terminal: muxy_settings::TerminalSettings,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            grid: None,
            copy_on_select: false,
            open_context: None,
            link_hover: super::links::Hover::default(),
            scroll: Scroll::default(),
            find: None,
            find_refresh: None,
            find_focus_pending: false,
            selection: None,
            selection_rows: Vec::new(),
            selecting: None,
            input_modes: InputModes::default(),
            held_buttons: Vec::new(),
            last_mouse: None,
            wheel_remainder: 0.0,
            geometry: None,
            cell_height: 16.0,
            native_visible: true,
            #[cfg(target_os = "macos")]
            native_scroll: None,
            native_sequence: 0,
            saved_history: None,
            focus: cx.focus_handle(),
            focused: true,
            cursor_blink: super::cursor::CursorBlink::default(),
            focus_border: None,
            corner_radius: px(0.0),
            channel: None,
            viewport: None,
            palette,
            terminal,
            state: PaneState::Connecting,
            process: None,
            title: String::new(),
            directory: ServerPath(Vec::new()),
            bell_flashing: false,
            bell_expiry: None,
        }
    }

    pub(crate) fn directory(&self) -> Option<PathBuf> {
        let path = PathBuf::from(std::ffi::OsString::from_vec(self.directory.0.clone()));
        path.is_absolute().then_some(path)
    }

    pub(crate) fn channel(&self) -> Option<ChannelId> {
        self.channel
    }

    pub(crate) fn viewport(&self) -> Option<Size> {
        self.viewport
    }

    pub(crate) fn set_state(&mut self, state: PaneState, cx: &mut Context<Self>) {
        self.channel = None;
        self.reset_input();
        self.scroll.reset();
        self.saved_history = None;
        self.state = state;
        self.cursor_blink.reset();
        self.restart_find(cx);
        self.validate_selection();
        if state != PaneState::Live
            && let Some(grid) = &mut self.grid
        {
            grid.cursor.visible = false;
        }
        cx.notify();
    }

    pub(crate) fn restore(&mut self, screen: SavedScreen, cx: &mut Context<Self>) {
        self.clear_selection();
        self.cursor_blink.reset();
        self.state = PaneState::Exited {
            reason: screen.reason,
            unavailable: false,
        };
        self.channel = None;
        self.reset_input();
        self.scroll.reset();
        self.saved_history = None;
        self.grid = Some(RunGrid::from_saved(screen));
        self.prepare_saved_history(cx);
        self.restart_find(cx);
        cx.notify();
    }

    pub(crate) fn attach(
        &mut self,
        attachment: Attachment,
        cx: &mut Context<Self>,
    ) -> Option<Size> {
        let resized = self.viewport.filter(|size| *size != attachment.grid.size);
        self.reset_input();
        self.clear_selection();
        self.scroll.reset();
        self.saved_history = None;
        self.channel = Some(attachment.channel);
        self.state = PaneState::Live;
        self.cursor_blink = super::cursor::CursorBlink::default();
        self.title = attachment.title;
        self.directory = attachment.directory;
        self.process = attachment.process;
        self.emit_title(cx);
        self.grid = Some(attachment.grid);
        self.restart_find(cx);
        if let Some(size) = resized
            && let Some(grid) = &mut self.grid
        {
            grid.resize(size);
        }
        cx.notify();
        resized
    }

    pub(crate) fn apply(&mut self, frame: &ScreenFrame, cx: &mut Context<Self>) {
        if let Some(grid) = &mut self.grid {
            if frame.reset {
                self.scroll.reset();
                self.saved_history = None;
            }
            if grid.cursor != frame.cursor {
                self.cursor_blink.reset();
            }
            grid.apply(frame);
            if self.scroll.view.is_none() {
                if frame.reset {
                    self.restart_find(cx);
                } else {
                    self.refresh_find(cx);
                }
            }
            self.validate_selection();
            cx.notify();
        }
    }

    pub(crate) fn scroll_rows(&mut self, delta: f32, cx: &mut Context<Self>) {
        if matches!(self.state, PaneState::Connecting | PaneState::Disconnected) {
            return;
        }
        if let Some(grid) = &self.grid {
            let height = usize::from(self.viewport.unwrap_or(grid.size).rows);
            let was_scrolled = self.scroll.view.is_some();
            if let Some(request) = self.scroll.move_rows(delta, grid, height) {
                self.request_history(request, cx);
            }
            if was_scrolled && self.scroll.view.is_none() {
                self.restart_find(cx);
            }
            self.validate_selection();
            cx.notify();
        }
    }

    pub(crate) fn request_history(&mut self, request: HistoryRequest, cx: &mut Context<Self>) {
        if request.before.0 == 0
            && let Some(saved) = self.saved_history
        {
            self.scroll.adopt_recent(saved);
        } else {
            cx.emit(PaneEvent::History(request));
        }
    }

    fn prepare_saved_history(&mut self, cx: &mut Context<Self>) {
        if self.saved_history.is_none()
            && matches!(
                self.state,
                PaneState::Exited {
                    unavailable: false,
                    ..
                }
            )
            && self.grid.as_ref().is_some_and(|grid| !grid.history_fresh)
        {
            let request = HistoryRequest::recent();
            self.saved_history = Some(request);
            cx.emit(PaneEvent::History(request));
        }
    }

    #[cfg(target_os = "macos")]
    fn setup_native_scroll(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.native_scroll.is_none() {
            let weak = cx.entity().downgrade();
            let app = cx.to_async();
            self.native_scroll = muxy_ui::native_scroll::NativeScrollView::new(
                &window.window_title(),
                move |position| {
                    let weak = weak.clone();
                    app.spawn(async move |cx| {
                        let _ = weak.update(cx, |pane, cx| pane.native_position(position, cx));
                    })
                    .detach();
                },
            );
        }
        if let Some(scroll) = &self.native_scroll {
            scroll.set_visible(
                self.native_visible
                    && !self.reports_wheel()
                    && self.grid.is_some()
                    && !matches!(self.state, PaneState::Connecting | PaneState::Disconnected),
            );
        }
    }

    #[cfg(target_os = "macos")]
    fn native_position(
        &mut self,
        position: muxy_ui::native_scroll::ScrollPosition,
        cx: &mut Context<Self>,
    ) {
        if !self.native_visible
            || self.reports_wheel()
            || matches!(self.state, PaneState::Connecting | PaneState::Disconnected)
        {
            return;
        }
        if position.revision != self.scroll.revision || position.sequence <= self.native_sequence {
            return;
        }
        self.native_sequence = position.sequence;
        if let Some(grid) = &self.grid {
            let height = usize::from(self.viewport.unwrap_or(grid.size).rows);
            let was_scrolled = self.scroll.view.is_some();
            if let Some(request) = self.scroll.move_to(
                position.from_bottom / f64::from(self.cell_height),
                grid,
                height,
            ) {
                self.request_history(request, cx);
            }
            if was_scrolled && self.scroll.view.is_none() {
                self.restart_find(cx);
            }
            self.validate_selection();
            cx.notify();
        }
    }

    pub(crate) fn receive_history(
        &mut self,
        request: HistoryRequest,
        result: Result<HistoryPage, muxy_client::ClientError>,
        cx: &mut Context<Self>,
    ) {
        let accepted = self.saved_history == Some(request) || self.scroll.expects(request);
        let stale = accepted
            && matches!(&result, Err(muxy_client::ClientError::Server(error)) if error.code == muxy_protocol::ErrorCode::StaleHistoryCursor);
        let changed = accepted
            && result.as_ref().is_ok_and(|page| {
                self.find.as_ref().is_some_and(|find| {
                    !find.results.matches.is_empty() && find.results.total_rows != page.total_rows
                })
            });
        if stale || changed {
            self.restart_find(cx);
        }
        let failed = accepted && result.is_err() && !stale;
        if self.saved_history == Some(request) {
            self.saved_history = None;
            if let (Ok(page), Some(grid)) = (&result, &mut self.grid) {
                grid.replace_history(page.clone());
            }
            if self.scroll.view.is_none() {
                self.validate_selection();
                cx.notify();
                return;
            }
        }
        let height = usize::from(
            self.viewport
                .or_else(|| self.grid.as_ref().map(|grid| grid.size))
                .map_or(24, |size| size.rows),
        );
        if let Some(request) = self.scroll.receive(request, result, height) {
            cx.emit(PaneEvent::History(request));
        }
        if failed {
            if let Some(find) = &mut self.find {
                find.results.loading_history = false;
                find.results.error = Some("Could not load match".into());
            }
        } else if self
            .find
            .as_ref()
            .is_some_and(|find| find.results.loading_history)
        {
            self.reveal_match(cx);
        }
        self.finish_command_selection(cx);
        self.validate_selection();
        cx.notify();
    }

    pub(crate) fn jump_prompt(&mut self, previous: bool, cx: &mut Context<Self>) {
        let Some(grid) = self.displayed_grid() else {
            return;
        };
        let anchor = self.visible_start(grid);
        let operation = if previous {
            super::scroll::PromptOperation::Previous
        } else {
            super::scroll::PromptOperation::Next
        };
        self.prompt_operation(operation, anchor, cx);
    }

    pub(crate) fn select_command_output(
        &mut self,
        position: Option<gpui::Point<gpui::Pixels>>,
        cx: &mut Context<Self>,
    ) {
        let Some(grid) = self.displayed_grid() else {
            return;
        };
        let anchor = position
            .and_then(|position| self.point_at(position, false))
            .and_then(|point| grid.history.len().checked_add_signed(point.row))
            .unwrap_or(grid.history.len() + usize::from(grid.cursor.row));
        self.prompt_operation(
            super::scroll::PromptOperation::Select {
                include_prompt: position.is_some(),
            },
            anchor,
            cx,
        );
    }

    fn prompt_operation(
        &mut self,
        operation: super::scroll::PromptOperation,
        anchor: usize,
        cx: &mut Context<Self>,
    ) {
        if self.state != PaneState::Live {
            return;
        }
        let Some(grid) = &self.grid else {
            return;
        };
        let height = usize::from(self.viewport.unwrap_or(grid.size).rows);
        if let Some(request) = self.scroll.prompt(operation, anchor, grid, height) {
            self.request_history(request, cx);
        }
        self.finish_command_selection(cx);
        self.validate_selection();
        cx.notify();
    }

    fn finish_command_selection(&mut self, cx: &mut Context<Self>) {
        if let Some((start, end)) = self.scroll.take_command_output()
            && let Some(grid) = self.displayed_grid()
        {
            self.select(
                Selection {
                    anchor: Point {
                        row: start,
                        column: 0,
                    },
                    head: Point {
                        row: end,
                        column: grid.size.cols,
                    },
                },
                cx,
            );
        }
    }

    pub(crate) fn scroll_to_bottom(&mut self, cx: &mut Context<Self>) {
        self.scroll.bottom();
        self.cursor_blink.reset();
        self.restart_find(cx);
        self.validate_selection();
        cx.notify();
    }

    pub(crate) fn metadata(&mut self, event: MetadataEvent, cx: &mut Context<Self>) {
        match event {
            MetadataEvent::ScreenPrompts { seq, rows } => {
                if let Some(grid) = &mut self.grid {
                    grid.screen_prompts(seq, rows);
                }
                cx.notify();
                return;
            }
            MetadataEvent::Links { seq, rows } => {
                if let Some(grid) = &mut self.grid {
                    grid.links.replace(seq, rows);
                }
                cx.notify();
                return;
            }
            MetadataEvent::CursorBlinking(enabled) => {
                if self.state == PaneState::Live && self.cursor_blink.enabled != enabled {
                    self.cursor_blink.enabled = enabled;
                    self.cursor_blink.reset();
                    cx.notify();
                }
                return;
            }
            MetadataEvent::InputModes(modes) => {
                if self.state == PaneState::Live && self.input_modes != modes {
                    let was_reporting = self.reports_wheel();
                    if !modes.mouse_tracking {
                        self.held_buttons.clear();
                    }
                    if self.held_buttons.is_empty() {
                        self.last_mouse = None;
                    }
                    self.wheel_remainder = 0.0;
                    self.input_modes = modes;
                    if !was_reporting && self.reports_wheel() {
                        self.clear_selection();
                        self.scroll.bottom();
                    }
                    cx.notify();
                }
                return;
            }
            MetadataEvent::History { total_rows } => {
                if let Some(grid) = &mut self.grid {
                    grid.history_total = total_rows;
                    grid.history_fresh = false;
                }
                cx.notify();
                return;
            }
            MetadataEvent::Title(title) => self.title = title,
            MetadataEvent::Directory(directory) => {
                self.directory = directory;
                self.link_hover = super::links::Hover::default();
            }
            MetadataEvent::ForegroundProcess { name, is_shell } => {
                self.process = Some(ForegroundProcess { name, is_shell });
            }
            MetadataEvent::Bell => {
                self.bell_flashing = true;
                self.bell_expiry = Some(cx.spawn(async move |pane, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(1250))
                        .await;
                    let _ = pane.update(cx, |pane, cx| {
                        pane.bell_flashing = false;
                        cx.emit(PaneEvent::Bell);
                    });
                }));
                cx.emit(PaneEvent::Bell);
                return;
            }
        }
        self.emit_title(cx);
    }

    fn emit_title(&self, cx: &mut Context<Self>) {
        cx.emit(PaneEvent::Title(muxy_app_core::title::derive(
            &self.title,
            self.process.as_ref(),
            &self.directory,
        )));
    }

    pub(crate) fn set_viewport(&mut self, size: Size, cx: &mut Context<Self>) {
        if self.viewport == Some(size) {
            return;
        }
        self.viewport = Some(size);
        if self.channel.is_some() {
            self.scroll.reset();
            self.saved_history = None;
        } else {
            self.scroll.resized(usize::from(size.rows));
            self.saved_history = None;
            self.prepare_saved_history(cx);
        }
        if self.channel.is_some()
            && let Some(grid) = &mut self.grid
        {
            grid.resize(size);
        }
        self.validate_selection();
        cx.emit(PaneEvent::Viewport(size));
        self.restart_find(cx);
        cx.notify();
    }

    pub(crate) fn displayed_grid(&self) -> Option<&RunGrid> {
        self.scroll.view.as_ref().or(self.grid.as_ref())
    }

    #[cfg(target_os = "macos")]
    #[allow(clippy::cast_precision_loss)]
    pub(super) fn scrollable_rows(&self, viewport_rows: u16) -> f64 {
        self.displayed_grid().map_or(0.0, |grid| {
            if self.channel.is_some() && self.scroll.view.is_none() {
                grid.history_total as f64
            } else {
                (grid.history_total as f64 + grid.rows.len() as f64 - f64::from(viewport_rows))
                    .max(0.0)
            }
        })
    }

    pub(crate) fn visible_start(&self, grid: &RunGrid) -> usize {
        if self.scroll.view.is_some() || matches!(self.state, PaneState::Exited { .. }) {
            self.scroll
                .start(grid, usize::from(self.viewport.unwrap_or(grid.size).rows))
        } else {
            grid.history.len()
        }
    }

    fn clear_selection(&mut self) {
        self.selection = None;
        self.selection_rows.clear();
        self.selecting = None;
    }

    fn reset_input(&mut self) {
        self.link_hover = super::links::Hover::default();
        self.input_modes = InputModes::default();
        self.held_buttons.clear();
        self.last_mouse = None;
        self.wheel_remainder = 0.0;
    }

    fn reports_mouse(&self, shift: bool) -> bool {
        self.state == PaneState::Live && self.input_modes.mouse_tracking && !shift
    }

    fn reports_wheel(&self) -> bool {
        self.state == PaneState::Live
            && (self.input_modes.mouse_tracking || self.input_modes.alternate_scroll)
    }

    pub(crate) fn focus_changed(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.state != PaneState::Live {
            return;
        }
        if !active {
            let buttons = std::mem::take(&mut self.held_buttons);
            if let (Some(channel), Some(event)) = (self.channel, self.last_mouse) {
                for button in buttons {
                    cx.emit(PaneEvent::Mouse(
                        channel,
                        MouseEvent {
                            action: MouseAction::Release,
                            button: Some(mouse_button(button)),
                            scroll: None,
                            ..event
                        },
                    ));
                }
            }
            self.selecting = None;
            self.last_mouse = None;
            self.wheel_remainder = 0.0;
        }
        if self.input_modes.focus_events
            && let Some(channel) = self.channel
        {
            cx.emit(PaneEvent::Input(
                channel,
                if active { b"\x1b[I" } else { b"\x1b[O" }.to_vec(),
            ));
        }
    }

    pub(crate) fn set_focused(&mut self, focused: bool, cx: &mut Context<Self>) {
        if self.focused != focused {
            self.focused = focused;
            self.cursor_blink.reset();
            self.focus_changed(focused, cx);
            cx.notify();
        }
    }

    pub(super) fn contains_mouse(&self, position: gpui::Point<gpui::Pixels>) -> bool {
        self.geometry
            .zip(self.grid.as_ref())
            .is_some_and(|((bounds, cell), grid)| {
                let size = self.viewport.unwrap_or(grid.size);
                Bounds::new(
                    bounds.origin,
                    gpui::size(
                        cell.width * f32::from(size.cols),
                        cell.height * f32::from(size.rows),
                    ),
                )
                .contains(&position)
            })
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn send_mouse(
        &mut self,
        action: MouseAction,
        button: Option<MouseButton>,
        scroll: Option<ScrollDirection>,
        position: gpui::Point<gpui::Pixels>,
        modifiers: gpui::Modifiers,
        cx: &mut Context<Self>,
    ) {
        if self.state != PaneState::Live {
            return;
        }
        let (Some(channel), Some((bounds, cell)), Some(grid)) =
            (self.channel, self.geometry, &self.grid)
        else {
            return;
        };
        let size = self.viewport.unwrap_or(grid.size);
        let event = MouseEvent {
            action,
            button: button.map(mouse_button),
            column: ((position.x - bounds.origin.x) / cell.width)
                .floor()
                .clamp(0.0, f32::from(size.cols.saturating_sub(1))) as u16,
            row: ((position.y - bounds.origin.y) / cell.height)
                .floor()
                .clamp(0.0, f32::from(size.rows.saturating_sub(1))) as u16,
            scroll,
            modifiers: muxy_protocol::Modifiers {
                shift: modifiers.shift,
                alt: modifiers.alt,
                ctrl: modifiers.control,
            },
        };
        if action == MouseAction::Motion && self.last_mouse == Some(event) {
            return;
        }
        self.last_mouse = Some(event);
        cx.emit(PaneEvent::Mouse(channel, event));
    }

    fn mouse_wheel(&mut self, event: &gpui::ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = match event.delta {
            gpui::ScrollDelta::Pixels(delta) => f32::from(delta.y) / self.cell_height.max(1.0),
            gpui::ScrollDelta::Lines(delta) => delta.y,
        };
        if self.reports_wheel() && !event.modifiers.shift {
            if delta.signum() != self.wheel_remainder.signum() {
                self.wheel_remainder = 0.0;
            }
            self.wheel_remainder = (self.wheel_remainder + delta).clamp(-100.0, 100.0);
            if self.wheel_remainder.abs() >= 1.0 {
                self.scroll_to_bottom(cx);
            }
            while self.wheel_remainder.abs() >= 1.0 {
                let direction = if self.wheel_remainder > 0.0 {
                    ScrollDirection::Up
                } else {
                    ScrollDirection::Down
                };
                self.send_mouse(
                    MouseAction::Scroll,
                    None,
                    Some(direction),
                    event.position,
                    event.modifiers,
                    cx,
                );
                self.wheel_remainder -= self.wheel_remainder.signum();
            }
            cx.stop_propagation();
            return;
        }
        self.wheel_remainder = 0.0;
        self.prepare_saved_history(cx);
        #[cfg(target_os = "macos")]
        if !self.reports_wheel()
            && let Some(position) = self
                .native_scroll
                .as_ref()
                .and_then(muxy_ui::native_scroll::NativeScrollView::forward_wheel)
        {
            self.native_position(position, cx);
            cx.stop_propagation();
            return;
        }
        if delta != 0.0 {
            self.scroll_rows(delta, cx);
            cx.stop_propagation();
        }
    }

    fn validate_selection(&mut self) {
        if let Some(selection) = self.selection
            && !self
                .displayed_grid()
                .is_some_and(|grid| selection.rows(grid) == self.selection_rows)
        {
            self.clear_selection();
        }
    }

    fn select(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.scroll.cancel_prompt();
        self.selection = (!selection.normalized().is_empty()).then_some(selection);
        self.selection_rows = self
            .displayed_grid()
            .map_or_else(Vec::new, |grid| selection.rows(grid));
        cx.notify();
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub(super) fn point_at(
        &self,
        position: gpui::Point<gpui::Pixels>,
        nearest: bool,
    ) -> Option<Point> {
        let (bounds, cell) = self.geometry?;
        let grid = self.displayed_grid()?;
        let x = (position.x - bounds.origin.x) / cell.width;
        let column = if nearest { x.round() } else { x.floor() }
            .clamp(0.0, f32::from(grid.size.cols)) as u16;
        let remainder = self.scroll.pixel_remainder(f32::from(cell.height));
        let y = (position.y - bounds.origin.y - px(remainder)) / cell.height;
        let height = self.viewport.unwrap_or(grid.size).rows;
        let last = height.saturating_sub(1) + u16::from(remainder < 0.0);
        let visible = y.floor().clamp(0.0, f32::from(last)) as usize;
        let index = self
            .visible_start(grid)
            .saturating_add(visible)
            .min(grid.history.len() + grid.rows.len().saturating_sub(1));
        Some(Point {
            row: isize::try_from(index).ok()? - isize::try_from(grid.history.len()).ok()?,
            column,
        })
    }

    fn mouse_down(
        &mut self,
        event: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window);
        cx.emit(PaneEvent::Focused);
        if event.button == MouseButton::Left && event.modifiers.platform {
            self.hover_link(event.position, true, cx);
            if let Some(target) = self.link_hover.target.clone() {
                self.selecting = None;
                cx.emit(PaneEvent::OpenLink(target));
                cx.stop_propagation();
                return;
            }
        }
        if self.reports_mouse(event.modifiers.shift) {
            self.clear_selection();
            self.scroll.bottom();
            if !self.held_buttons.contains(&event.button) {
                self.held_buttons.push(event.button);
            }
            self.send_mouse(
                MouseAction::Press,
                Some(event.button),
                None,
                event.position,
                event.modifiers,
                cx,
            );
            cx.notify();
            return;
        }
        if event.button == MouseButton::Right {
            self.selecting = None;
            cx.emit(PaneEvent::ContextMenu(event.position));
            cx.stop_propagation();
            return;
        }
        if event.button != MouseButton::Left {
            return;
        }
        self.clear_selection();
        let Some(point) = self.point_at(event.position, event.click_count < 2) else {
            return;
        };
        let Some(grid) = self.displayed_grid() else {
            return;
        };
        let selection = match event.click_count {
            0 | 1 => Selection {
                anchor: point,
                head: point,
            },
            2 => Selection::word(point, grid),
            _ => Selection::row(point, grid),
        };
        self.selecting = Some((selection, event.click_count));
        self.select(selection, cx);
    }

    pub(crate) fn mouse_move(&mut self, event: &gpui::MouseMoveEvent, cx: &mut Context<Self>) {
        if !self.native_visible {
            return;
        }
        self.hover_link(
            event.position,
            event.modifiers.platform && event.pressed_button.is_none(),
            cx,
        );
        if self.reports_mouse(event.modifiers.shift) && self.selecting.is_none() {
            if !self.held_buttons.is_empty()
                || (event.pressed_button.is_none() && self.contains_mouse(event.position))
            {
                self.send_mouse(
                    MouseAction::Motion,
                    self.held_buttons.last().copied(),
                    None,
                    event.position,
                    event.modifiers,
                    cx,
                );
            }
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.selecting = None;
            return;
        }
        let Some((anchor, clicks)) = self.selecting else {
            return;
        };
        let Some(point) = self.point_at(event.position, clicks < 2) else {
            return;
        };
        let Some(grid) = self.displayed_grid() else {
            return;
        };
        let target = match clicks {
            0 | 1 => Selection {
                anchor: point,
                head: point,
            },
            2 => Selection::word(point, grid),
            _ => Selection::row(point, grid),
        };
        let selection = if target.anchor < anchor.anchor {
            Selection {
                anchor: anchor.head,
                head: target.anchor,
            }
        } else {
            Selection {
                anchor: anchor.anchor,
                head: target.head,
            }
        };
        self.select(selection, cx);
    }

    fn mouse_up(&mut self, event: &gpui::MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.held_buttons.contains(&event.button) {
            self.held_buttons.retain(|button| *button != event.button);
            self.send_mouse(
                MouseAction::Release,
                Some(event.button),
                None,
                event.position,
                event.modifiers,
                cx,
            );
            return;
        }
        if event.button != MouseButton::Left {
            return;
        }
        let completed_selection = self.selecting.is_some();
        self.mouse_move(
            &gpui::MouseMoveEvent {
                position: event.position,
                pressed_button: Some(MouseButton::Left),
                modifiers: event.modifiers,
            },
            cx,
        );
        self.selecting = None;
        if completed_selection && self.copy_on_select {
            self.copy_selection(cx);
        }
    }

    pub(crate) fn copy_selection(&self, cx: &mut Context<Self>) {
        if let (Some(selection), Some(grid)) = (self.selection, self.displayed_grid()) {
            let text = selection.text(grid);
            if !text.is_empty() {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            }
        }
    }

    pub(crate) fn select_all(&mut self, cx: &mut Context<Self>) {
        let Some(grid) = self.displayed_grid() else {
            return;
        };
        let Some(history) = isize::try_from(grid.history.len()).ok() else {
            return;
        };
        let Some(last) = grid
            .rows
            .len()
            .checked_sub(1)
            .and_then(|row| isize::try_from(row).ok())
        else {
            return;
        };
        self.select(
            Selection {
                anchor: Point {
                    row: -history,
                    column: 0,
                },
                head: Point {
                    row: last,
                    column: grid.size.cols,
                },
            },
            cx,
        );
    }

    fn copy(&mut self, _: &muxy_ui::text_input::Copy, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selection(cx);
        cx.stop_propagation();
    }

    pub(crate) fn paste_clipboard(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
            && let Some(grid) = &self.grid
        {
            self.send_paste(&clipboard::paste(&text, grid.modes), cx);
        }
    }

    pub(crate) fn drop_paths(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if let Some(grid) = &self.grid
            && let Some(bytes) = clipboard::paths(paths, grid.modes)
        {
            self.send_paste(&bytes, cx);
        }
    }

    fn send_paste(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        if self.state == PaneState::Live
            && !bytes.is_empty()
            && let Some(channel) = self.channel
        {
            self.scroll_to_bottom(cx);
            for chunk in bytes.chunks(muxy_protocol::MAX_INPUT) {
                cx.emit(PaneEvent::Input(channel, chunk.to_vec()));
            }
        }
    }

    fn paste(&mut self, _: &muxy_ui::text_input::Paste, _: &mut Window, cx: &mut Context<Self>) {
        self.paste_clipboard(cx);
        cx.stop_propagation();
    }
}

fn mouse_button(button: MouseButton) -> muxy_protocol::MouseButton {
    match button {
        MouseButton::Left => muxy_protocol::MouseButton::Left,
        MouseButton::Middle => muxy_protocol::MouseButton::Middle,
        MouseButton::Right => muxy_protocol::MouseButton::Right,
        MouseButton::Navigate(gpui::NavigationDirection::Back) => muxy_protocol::MouseButton::Back,
        MouseButton::Navigate(gpui::NavigationDirection::Forward) => {
            muxy_protocol::MouseButton::Forward
        }
    }
}

impl TerminalPane {
    fn terminal_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus.is_focused(window) {
            return;
        }
        if let (Some(channel), Some(grid)) = (self.channel, &self.grid)
            && let Some(bytes) = input::encode(&event.keystroke, grid.modes)
        {
            self.scroll_to_bottom(cx);
            cx.emit(PaneEvent::Input(channel, bytes));
            cx.stop_propagation();
        }
    }
}

impl Render for TerminalPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.find_focus_pending {
            self.find_focus_pending = false;
            self.focus.focus(window);
        }
        #[cfg(target_os = "macos")]
        self.setup_native_scroll(window, cx);
        self.hover_link(window.mouse_position(), window.modifiers().platform, cx);
        let palette = self.palette;
        div()
            .id("terminal-pane")
            .debug_selector(|| "terminal-pane".to_owned())
            .track_focus(&self.focus)
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .rounded(self.corner_radius)
            .bg(gpui::rgb(palette.background))
            .flex()
            .flex_col()
            .map(|mut pane| {
                for button in MouseButton::all() {
                    pane = pane
                        .on_mouse_down(button, cx.listener(Self::mouse_down))
                        .on_mouse_up(button, cx.listener(Self::mouse_up))
                        .on_mouse_up_out(button, cx.listener(Self::mouse_up));
                }
                pane
            })
            .key_context(if self.find.is_some() {
                "TerminalFindOpen"
            } else {
                "TerminalPane"
            })
            .on_action(cx.listener(|pane, _: &CloseFind, _, cx| {
                pane.close_find(cx);
                cx.stop_propagation();
            }))
            .when(self.link_hover.target.is_some(), |pane| {
                pane.cursor_pointer()
            })
            .on_modifiers_changed(cx.listener(
                |pane, event: &gpui::ModifiersChangedEvent, window, cx| {
                    pane.hover_link(window.mouse_position(), event.modifiers.platform, cx);
                },
            ))
            .when(self.state == PaneState::Live, |pane| {
                pane.drag_over::<gpui::ExternalPaths>(move |style, _, _, _| {
                    style.border_1().border_color(gpui::rgb(palette.cursor))
                })
                .on_drop(cx.listener(
                    |pane, paths: &gpui::ExternalPaths, window, cx| {
                        pane.focus.focus(window);
                        cx.emit(PaneEvent::Focused);
                        pane.drop_paths(paths.paths(), cx);
                        cx.stop_propagation();
                    },
                ))
            })
            .on_action(
                cx.listener(|pane, _: &muxy_ui::text_input::SelectAll, _, cx| {
                    pane.select_all(cx);
                    cx.stop_propagation();
                }),
            )
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_scroll_wheel(cx.listener(|pane, event: &gpui::ScrollWheelEvent, _, cx| {
                pane.mouse_wheel(event, cx);
            }))
            .on_action(
                cx.listener(|pane, _: &crate::views::workspace::ScrollToBottom, _, cx| {
                    pane.scroll_to_bottom(cx);
                    cx.stop_propagation();
                }),
            )
            .on_key_down(cx.listener(Self::terminal_key_down))
            .children(find::bar(self, cx))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(element::terminal(cx.entity(), palette)),
            )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use gpui::{AppContext, TestAppContext, point, size};
    use muxy_protocol::{Cursor, Modes, Row, Run, Style};

    fn row(index: u16, text: &str) -> Row {
        Row {
            index,
            runs: vec![Run {
                text: text.into(),
                width: u16::try_from(text.len()).unwrap(),
                style: Style::default(),
            }],
        }
    }

    fn grid() -> RunGrid {
        RunGrid {
            prompts: std::collections::BTreeSet::default(),
            prompt_state: muxy_client::ScreenPrompts::default(),
            links: muxy_client::ScreenLinks::default(),
            size: Size { cols: 20, rows: 3 },
            rows: ["alpha beta", "second row", "prompt"]
                .into_iter()
                .enumerate()
                .map(|(index, text)| row(u16::try_from(index).unwrap(), text).runs)
                .collect(),
            cursor: Cursor {
                row: 2,
                col: 6,
                visible: true,
            },
            modes: Modes::default(),
            history: vec![row(0, "history row")].into(),
            history_cursor: None,
            history_total: 1,
            history_fresh: true,
        }
    }

    fn prepare_mouse(pane: &mut TerminalPane) {
        pane.grid = Some(grid());
        pane.state = PaneState::Live;
        pane.channel = Some(ChannelId(1));
        pane.viewport = Some(Size { cols: 20, rows: 3 });
        pane.geometry = Some((
            Bounds::new(point(px(0.0), px(0.0)), size(px(200.0), px(60.0))),
            size(px(10.0), px(20.0)),
        ));
        pane.cell_height = 20.0;
    }

    #[gpui::test]
    fn prompt_jumps_start_at_the_viewport_not_an_already_visible_prompt(cx: &mut TestAppContext) {
        let pane = cx.new(|cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        pane.update(cx, |pane, cx| {
            prepare_mouse(pane);
            pane.grid.as_mut().unwrap().prompts = [0, 1, 3].into();
            pane.jump_prompt(true, cx);
            assert_eq!(pane.visible_start(pane.displayed_grid().unwrap()), 0);
            pane.jump_prompt(false, cx);
            assert!(pane.scroll.view.is_none());
        });
    }

    #[gpui::test]
    fn manual_selection_cancels_command_selection_waiting_for_history(cx: &mut TestAppContext) {
        let pane = cx.new(|cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        pane.update(cx, |pane, cx| {
            prepare_mouse(pane);
            let grid = pane.grid.as_mut().unwrap();
            grid.history_fresh = false;
            let history = HistoryPage {
                rows: grid.history.iter().cloned().collect(),
                next: None,
                total_rows: grid.history_total,
                prompts: vec![0, 1, 3],
                screen: Some(SavedScreen {
                    size: grid.size,
                    rows: grid
                        .rows
                        .iter()
                        .enumerate()
                        .map(|(index, runs)| Row {
                            index: u16::try_from(index).unwrap(),
                            runs: runs.clone(),
                        })
                        .collect(),
                    cursor: grid.cursor,
                    reason: None,
                }),
            };
            let request = pane
                .scroll
                .prompt(
                    super::super::scroll::PromptOperation::Select {
                        include_prompt: false,
                    },
                    3,
                    grid,
                    3,
                )
                .unwrap();
            let manual = Selection {
                anchor: Point { row: 0, column: 0 },
                head: Point { row: 0, column: 5 },
            };
            pane.select(manual, cx);
            pane.receive_history(request, Ok(history), cx);
            assert_eq!(pane.selection, Some(manual));
        });
    }

    fn searchable_pane(window: &mut Window, cx: &mut Context<TerminalPane>) -> TerminalPane {
        let mut pane = TerminalPane::new(
            Palette::new(true),
            muxy_settings::TerminalSettings::default(),
            cx,
        );
        let mut grid = grid();
        grid.history = (0..200).map(|index| row(index, "alpha")).collect();
        grid.history_total = 200;
        pane.grid = Some(grid);
        pane.state = PaneState::Live;
        pane.open_find(
            &muxy_ui::theme::Theme::from_scheme(&muxy_ui::theme::ColorScheme::default()),
            muxy_ui::theme::Metrics::new(1.0),
            window,
            cx,
        );
        pane
    }

    fn search_page(total_rows: u64) -> muxy_protocol::SearchPage {
        muxy_protocol::SearchPage {
            matches: vec![muxy_protocol::SearchMatch {
                row: total_rows,
                start: 0,
                end: 5,
            }],
            next: None,
            total_rows,
            scanned_rows: 203,
        }
    }

    #[gpui::test]
    fn same_count_history_metadata_refreshes_cached_rows_without_a_frame(cx: &mut TestAppContext) {
        let pane = cx.new(|cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        let requests = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|cx| {
            let requests = requests.clone();
            cx.subscribe(&pane, move |_, event, _| {
                if let PaneEvent::History(request) = event {
                    requests.borrow_mut().push(*request);
                }
            })
            .detach();
        });
        pane.update(cx, |pane, cx| {
            prepare_mouse(pane);
            pane.metadata(MetadataEvent::History { total_rows: 1 }, cx);
            assert!(!pane.grid.as_ref().unwrap().history_fresh);
            pane.scroll_rows(1.0, cx);
            assert!(pane.scroll.view.as_ref().unwrap().history.is_empty());
        });
        cx.run_until_parked();
        assert_eq!(requests.borrow().len(), 1);
        let request = requests.borrow()[0];
        assert_eq!(request.before, muxy_protocol::HistoryCursor(0));
        pane.update(cx, |pane, cx| {
            pane.receive_history(
                request,
                Ok(HistoryPage {
                    prompts: Vec::new(),
                    rows: vec![row(0, "new history")],
                    next: None,
                    total_rows: 1,
                    screen: None,
                }),
                cx,
            );
            let view = pane.scroll.view.as_ref().unwrap();
            assert!(view.history_fresh);
            assert_eq!(view.history.front(), Some(&row(0, "new history")));
        });
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn live_resize_scrollbar_counts_history_not_stale_screen_rows(cx: &mut TestAppContext) {
        let pane = cx.new(|cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        pane.update(cx, |pane, cx| {
            prepare_mouse(pane);
            let grid = pane.grid.as_mut().unwrap();
            grid.rows = vec![Vec::new(); 3];
            grid.rows[0] = row(0, "prompt").runs;
            grid.history.clear();
            grid.history_total = 0;
            grid.cursor.row = 0;
            let mut seq = 0;
            for history in [0_u32, 12] {
                pane.grid.as_mut().unwrap().history_total = u64::from(history);
                for rows in [8, 3, 10, 2] {
                    assert!(
                        (pane.scrollable_rows(rows) - f64::from(history)).abs() < 0.01,
                        "before viewport update to {rows} rows"
                    );
                    pane.set_viewport(Size { cols: 20, rows }, cx);
                    for frame_rows in [15, rows] {
                        seq += 1;
                        pane.apply(
                            &ScreenFrame {
                                seq,
                                reset: true,
                                rows: (0..frame_rows)
                                    .map(|index| Row {
                                        index,
                                        runs: vec![],
                                    })
                                    .collect(),
                                cursor: Cursor {
                                    row: 0,
                                    col: 0,
                                    visible: true,
                                },
                                modes: Modes::default(),
                            },
                            cx,
                        );
                        assert!(
                            (pane.scrollable_rows(rows) - f64::from(history)).abs() < 0.01,
                            "after {frame_rows}-row frame for {rows}-row viewport"
                        );
                    }
                    assert_eq!(pane.grid.as_ref().unwrap().rows.len(), usize::from(rows));
                    assert!(pane.scroll.view.is_none());
                }
            }
        });
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn saved_and_frozen_scrollbars_keep_screen_overflow(cx: &mut TestAppContext) {
        let pane = cx.new(|cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        pane.update(cx, |pane, cx| {
            prepare_mouse(pane);
            pane.scroll_rows(1.0, cx);
            assert!(pane.scroll.view.is_some());
            assert!((pane.scrollable_rows(2) - 2.0).abs() < 0.01);
            assert!((pane.scrollable_rows(5)).abs() < 0.01);
            pane.restore(
                SavedScreen {
                    size: Size { cols: 20, rows: 3 },
                    rows: (0..3).map(|index| row(index, "saved output")).collect(),
                    cursor: Cursor {
                        row: 2,
                        col: 0,
                        visible: false,
                    },
                    reason: None,
                },
                cx,
            );
            for (rows, overflow) in [(2, 1.0), (5, 0.0), (1, 2.0)] {
                pane.set_viewport(Size { cols: 20, rows }, cx);
                assert!((pane.scrollable_rows(rows) - overflow).abs() < 0.01);
                assert_eq!(pane.grid.as_ref().unwrap().rows.len(), 3);
            }
        });
    }

    #[gpui::test]
    fn search_refreshes_after_wheel_and_native_scrolling_return_to_live_output(
        cx: &mut TestAppContext,
    ) {
        let (pane, cx) = cx.add_window_view(searchable_pane);
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(300));
        cx.run_until_parked();
        let searches = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|_, cx| {
            let searches = searches.clone();
            cx.subscribe(&pane, move |_, event, _| {
                if let PaneEvent::Search(request) = event {
                    searches.borrow_mut().push(request.clone());
                }
            })
            .detach();
        });
        for native in [false, cfg!(target_os = "macos")] {
            searches.borrow_mut().clear();
            pane.update(cx, |pane, cx| {
                let find = pane.find.as_mut().unwrap();
                find.results.query = "alpha".into();
                let request = find.results.restart().unwrap();
                find.results.receive(&request, Ok(search_page(200)));
                pane.grid.as_mut().unwrap().history_total = 200;
                pane.scroll_rows(20.0, cx);
                assert!(pane.scroll.view.is_some());
                pane.metadata(MetadataEvent::History { total_rows: 201 }, cx);
                pane.apply(
                    &ScreenFrame {
                        seq: 1,
                        reset: false,
                        rows: vec![row(0, "alpha")],
                        cursor: pane.grid.as_ref().unwrap().cursor,
                        modes: Modes::default(),
                    },
                    cx,
                );
                assert_eq!(pane.find.as_ref().unwrap().results.total_rows, 200);
                if native {
                    #[cfg(target_os = "macos")]
                    pane.native_position(
                        muxy_ui::native_scroll::ScrollPosition {
                            from_bottom: 0.0,
                            revision: pane.scroll.revision,
                            sequence: pane.native_sequence + 1,
                        },
                        cx,
                    );
                } else {
                    pane.scroll_rows(-1000.0, cx);
                }
                assert!(pane.scroll.view.is_none());
            });
            cx.run_until_parked();
            cx.executor().advance_clock(Duration::from_millis(300));
            cx.run_until_parked();
            let searches = searches.borrow();
            assert_eq!(searches.len(), 1, "native: {native}");
            assert_eq!(searches[0].query, "alpha");
            assert_eq!(searches[0].before, muxy_protocol::HistoryCursor(0));
        }
    }

    #[gpui::test]
    fn search_accepts_delayed_replies_during_continuous_frames(cx: &mut TestAppContext) {
        let (pane, cx) = cx.add_window_view(searchable_pane);
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(300));
        cx.run_until_parked();
        let request = pane.update(cx, |pane, cx| {
            let find = pane.find.as_mut().unwrap();
            find.results.query = "alpha".into();
            let request = find.results.restart().unwrap();
            pane.apply(
                &ScreenFrame {
                    seq: 1,
                    reset: false,
                    rows: vec![row(0, "alpha")],
                    cursor: pane.grid.as_ref().unwrap().cursor,
                    modes: Modes::default(),
                },
                cx,
            );
            request
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(300));
        cx.run_until_parked();
        pane.update(cx, |pane, cx| {
            pane.receive_search(&request, Ok(search_page(200)), cx);
            let results = &pane.find.as_ref().unwrap().results;
            assert_eq!(results.counter(), "1 of 1");
            assert_eq!(
                results.highlights(pane.grid.as_ref().unwrap(), 200).len(),
                1
            );
        });
        cx.executor().advance_clock(Duration::from_millis(300));
        cx.run_until_parked();
        pane.read_with(cx, |pane, _| {
            assert_eq!(pane.find.as_ref().unwrap().results.counter(), "0 of 0…");
        });
    }

    #[gpui::test]
    fn reporting_tracks_buttons_clamps_positions_and_deduplicates_motion(cx: &mut TestAppContext) {
        let (pane, cx) = cx.add_window_view(|_, cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        let reported = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|window, cx| {
            let events = reported.clone();
            cx.subscribe(&pane, move |_, event, _| {
                if let PaneEvent::Mouse(channel, event) = event {
                    assert_eq!(*channel, ChannelId(1));
                    events.borrow_mut().push(*event);
                }
            })
            .detach();
            pane.update(cx, |pane, cx| {
                prepare_mouse(pane);
                pane.input_modes.mouse_tracking = true;
                pane.mouse_move(
                    &gpui::MouseMoveEvent {
                        position: point(px(250.0), px(5.0)),
                        ..gpui::MouseMoveEvent::default()
                    },
                    cx,
                );
                for button in MouseButton::all() {
                    pane.mouse_down(
                        &gpui::MouseDownEvent {
                            button,
                            position: point(px(13.0), px(25.0)),
                            ..gpui::MouseDownEvent::default()
                        },
                        window,
                        cx,
                    );
                    let motion = gpui::MouseMoveEvent {
                        position: point(px(45.0), px(25.0)),
                        pressed_button: Some(button),
                        ..gpui::MouseMoveEvent::default()
                    };
                    pane.mouse_move(&motion, cx);
                    pane.mouse_move(&motion, cx);
                    pane.mouse_up(
                        &gpui::MouseUpEvent {
                            button,
                            position: point(px(500.0), px(500.0)),
                            modifiers: gpui::Modifiers {
                                shift: true,
                                ..gpui::Modifiers::default()
                            },
                            ..gpui::MouseUpEvent::default()
                        },
                        window,
                        cx,
                    );
                }
                assert!(pane.selection.is_none());
                assert!(pane.held_buttons.is_empty());
            });
        });
        cx.run_until_parked();
        let events = reported.borrow();
        assert_eq!(events.len(), 15);
        for (events, button) in events.chunks_exact(3).zip(MouseButton::all()) {
            assert_eq!(
                events.iter().map(|event| event.action).collect::<Vec<_>>(),
                [
                    MouseAction::Press,
                    MouseAction::Motion,
                    MouseAction::Release
                ]
            );
            assert!(
                events
                    .iter()
                    .all(|event| event.button == Some(mouse_button(button)))
            );
            assert_eq!((events[0].column, events[0].row), (1, 1));
            assert_eq!((events[1].column, events[1].row), (4, 1));
            assert_eq!((events[2].column, events[2].row), (19, 2));
            assert!(events[2].modifiers.shift);
        }
    }

    #[gpui::test]
    fn shift_selection_stays_local_until_release_and_can_be_copied(cx: &mut TestAppContext) {
        let (pane, cx) = cx.add_window_view(|_, cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        let reported = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|window, cx| {
            let events = reported.clone();
            cx.subscribe(&pane, move |_, event, _| {
                if let PaneEvent::Mouse(_, event) = event {
                    events.borrow_mut().push(*event);
                }
            })
            .detach();
            pane.update(cx, |pane, cx| {
                prepare_mouse(pane);
                pane.input_modes.mouse_tracking = true;
                pane.mouse_down(
                    &gpui::MouseDownEvent {
                        position: point(px(0.0), px(5.0)),
                        click_count: 1,
                        modifiers: gpui::Modifiers {
                            shift: true,
                            ..gpui::Modifiers::default()
                        },
                        ..gpui::MouseDownEvent::default()
                    },
                    window,
                    cx,
                );
                pane.mouse_move(
                    &gpui::MouseMoveEvent {
                        position: point(px(50.0), px(5.0)),
                        pressed_button: Some(MouseButton::Left),
                        ..gpui::MouseMoveEvent::default()
                    },
                    cx,
                );
                pane.mouse_up(
                    &gpui::MouseUpEvent {
                        position: point(px(50.0), px(5.0)),
                        ..gpui::MouseUpEvent::default()
                    },
                    window,
                    cx,
                );
                pane.copy(&muxy_ui::text_input::Copy, window, cx);
                assert_eq!(
                    cx.read_from_clipboard().unwrap().text().as_deref(),
                    Some("alpha")
                );
                assert!(pane.held_buttons.is_empty());
            });
        });
        cx.run_until_parked();
        assert!(reported.borrow().is_empty());
    }

    #[gpui::test]
    fn wheel_routes_by_modes_preserves_fractions_and_shift_uses_history(cx: &mut TestAppContext) {
        let pane = cx.new(|cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        let reported = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let events = reported.clone();
        cx.update(|cx| {
            cx.subscribe(&pane, move |_, event, _| {
                if let PaneEvent::Mouse(_, event) = event {
                    events.borrow_mut().push(*event);
                }
            })
            .detach();
        });
        pane.update(cx, |pane, cx| {
            prepare_mouse(pane);
            let mut wheel = gpui::ScrollWheelEvent {
                position: point(px(15.0), px(5.0)),
                delta: gpui::ScrollDelta::Lines(point(0.0, 1.0)),
                ..gpui::ScrollWheelEvent::default()
            };
            pane.mouse_wheel(&wheel, cx);
            assert!(pane.scroll.view.is_some());
            for modes in [
                InputModes {
                    mouse_tracking: true,
                    ..InputModes::default()
                },
                InputModes {
                    alternate_scroll: true,
                    ..InputModes::default()
                },
            ] {
                pane.metadata(MetadataEvent::InputModes(modes), cx);
                pane.scroll_to_bottom(cx);
                wheel.delta = gpui::ScrollDelta::Pixels(point(px(0.0), px(8.0)));
                for _ in 0..3 {
                    pane.mouse_wheel(&wheel, cx);
                }
                assert!(pane.scroll.view.is_none());
                wheel.delta = gpui::ScrollDelta::Lines(point(0.0, -2.0));
                pane.mouse_wheel(&wheel, cx);
                wheel.modifiers.shift = true;
                wheel.delta = gpui::ScrollDelta::Lines(point(0.0, 1.0));
                pane.mouse_wheel(&wheel, cx);
                assert!(pane.scroll.view.is_some());
                wheel.modifiers.shift = false;
                pane.mouse_wheel(&wheel, cx);
                assert!(pane.scroll.view.is_none());
            }
        });
        cx.run_until_parked();
        let events = reported.borrow();
        assert_eq!(events.len(), 8);
        for chunk in events.chunks_exact(4) {
            assert_eq!(
                chunk.iter().map(|event| event.scroll).collect::<Vec<_>>(),
                [
                    Some(ScrollDirection::Up),
                    Some(ScrollDirection::Down),
                    Some(ScrollDirection::Down),
                    Some(ScrollDirection::Up),
                ]
            );
            assert!(chunk.iter().all(|event| event.action == MouseAction::Scroll
                && event.button.is_none()
                && event.column == 1
                && event.row == 0));
        }
    }

    #[gpui::test]
    fn overlays_suppress_motion_but_release_a_previously_reported_button(cx: &mut TestAppContext) {
        let (pane, cx) = cx.add_window_view(|_, cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        let reported = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|window, cx| {
            let events = reported.clone();
            cx.subscribe(&pane, move |_, event, _| {
                if let PaneEvent::Mouse(_, event) = event {
                    events.borrow_mut().push(*event);
                }
            })
            .detach();
            pane.update(cx, |pane, cx| {
                prepare_mouse(pane);
                pane.input_modes.mouse_tracking = true;
                let position = point(px(15.0), px(5.0));
                pane.mouse_down(
                    &gpui::MouseDownEvent {
                        position,
                        ..gpui::MouseDownEvent::default()
                    },
                    window,
                    cx,
                );
                pane.native_visible = false;
                pane.mouse_move(
                    &gpui::MouseMoveEvent {
                        position,
                        pressed_button: Some(MouseButton::Left),
                        ..gpui::MouseMoveEvent::default()
                    },
                    cx,
                );
                pane.mouse_up(
                    &gpui::MouseUpEvent {
                        position,
                        ..gpui::MouseUpEvent::default()
                    },
                    window,
                    cx,
                );
                pane.mouse_move(
                    &gpui::MouseMoveEvent {
                        position,
                        ..gpui::MouseMoveEvent::default()
                    },
                    cx,
                );
                assert!(pane.held_buttons.is_empty());
                pane.native_visible = true;
                pane.mouse_move(
                    &gpui::MouseMoveEvent {
                        position,
                        ..gpui::MouseMoveEvent::default()
                    },
                    cx,
                );
            });
        });
        cx.run_until_parked();
        assert_eq!(
            reported
                .borrow()
                .iter()
                .map(|event| event.action)
                .collect::<Vec<_>>(),
            [
                MouseAction::Press,
                MouseAction::Release,
                MouseAction::Motion
            ]
        );
    }

    #[gpui::test]
    fn focus_reports_require_the_mode_and_non_live_panes_drop_all_terminal_mouse_input(
        cx: &mut TestAppContext,
    ) {
        let (pane, cx) = cx.add_window_view(|_, cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        let reported = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let input = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|window, cx| {
            let events = reported.clone();
            let bytes = input.clone();
            cx.subscribe(&pane, move |_, event, _| match event {
                PaneEvent::Mouse(_, event) => events.borrow_mut().push(*event),
                PaneEvent::Input(_, input) => bytes.borrow_mut().push(input.clone()),
                _ => {}
            })
            .detach();
            pane.update(cx, |pane, cx| {
                prepare_mouse(pane);
                pane.focus_changed(true, cx);
                pane.focus_changed(false, cx);
                pane.input_modes.focus_events = true;
                pane.focus_changed(true, cx);
                pane.focus_changed(false, cx);
                for state in [
                    PaneState::Connecting,
                    PaneState::Disconnected,
                    PaneState::Exited {
                        reason: None,
                        unavailable: false,
                    },
                ] {
                    pane.set_state(state, cx);
                    pane.input_modes = InputModes {
                        mouse_tracking: true,
                        alternate_scroll: true,
                        focus_events: true,
                    };
                    pane.focus_changed(true, cx);
                    pane.focus_changed(false, cx);
                    pane.mouse_down(&gpui::MouseDownEvent::default(), window, cx);
                    pane.mouse_move(&gpui::MouseMoveEvent::default(), cx);
                    pane.mouse_up(&gpui::MouseUpEvent::default(), window, cx);
                    pane.mouse_wheel(
                        &gpui::ScrollWheelEvent {
                            delta: gpui::ScrollDelta::Lines(point(0.0, 1.0)),
                            ..gpui::ScrollWheelEvent::default()
                        },
                        cx,
                    );
                }
            });
        });
        cx.run_until_parked();
        assert!(reported.borrow().is_empty());
        assert_eq!(&*input.borrow(), &[b"\x1b[I".to_vec(), b"\x1b[O".to_vec()]);
    }

    #[gpui::test]
    fn frames_preserve_unchanged_selection_and_clear_changed_rows(cx: &mut TestAppContext) {
        let pane = cx.new(|cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        pane.update(cx, |pane, cx| {
            pane.grid = Some(grid());
            pane.state = PaneState::Live;
            let selection = Selection {
                anchor: Point { row: 0, column: 0 },
                head: Point { row: 0, column: 5 },
            };
            pane.select(selection, cx);
            let mut frame = ScreenFrame {
                seq: 1,
                reset: false,
                rows: vec![row(2, "new prompt")],
                cursor: pane.grid.as_ref().unwrap().cursor,
                modes: Modes::default(),
            };
            pane.apply(&frame, cx);
            assert_eq!(pane.selection, Some(selection));
            frame.rows = vec![row(0, "alpha beta")];
            pane.apply(&frame, cx);
            assert_eq!(pane.selection, Some(selection));
            frame.reset = true;
            frame.rows = vec![row(0, "alpha beta")];
            pane.apply(&frame, cx);
            assert_eq!(pane.selection, Some(selection));
            frame.rows = vec![];
            pane.apply(&frame, cx);
            assert!(pane.selection.is_none());
            pane.grid = Some(grid());
            pane.scroll_rows(1.0, cx);
            pane.select(selection, cx);
            frame.reset = false;
            frame.rows = vec![row(0, "changed live")];
            pane.apply(&frame, cx);
            assert_eq!(pane.selection, Some(selection));
            pane.scroll_to_bottom(cx);
            assert!(pane.selection.is_none());
        });
    }

    #[gpui::test]
    fn fractional_scroll_hit_testing_includes_the_bottom_partial_row(cx: &mut TestAppContext) {
        let pane = cx.new(|cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        pane.update(cx, |pane, cx| {
            pane.grid = Some(grid());
            pane.state = PaneState::Live;
            pane.viewport = Some(Size { cols: 20, rows: 3 });
            pane.geometry = Some((
                Bounds::new(point(px(0.0), px(0.0)), size(px(200.0), px(60.0))),
                size(px(10.0), px(20.0)),
            ));
            pane.scroll_rows(0.5, cx);
            for (y, row) in [(5.0, -1), (15.0, 0), (35.0, 1), (55.0, 2)] {
                assert_eq!(
                    pane.point_at(point(px(10.0), px(y)), true),
                    Some(Point { row, column: 1 })
                );
            }
            let bottom = pane.point_at(point(px(10.0), px(55.0)), true).unwrap();
            let grid = pane.displayed_grid().unwrap();
            assert_eq!(Selection::row(bottom, grid).text(grid), "prompt");
        });
    }

    #[gpui::test]
    fn large_pastes_keep_one_bracketed_stream_in_protocol_sized_messages(cx: &mut TestAppContext) {
        let (pane, cx) = cx.add_window_view(|_, cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        let input = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|_, cx| {
            let input = input.clone();
            cx.subscribe(&pane, move |_, event, _| {
                if let PaneEvent::Input(_, bytes) = event {
                    input.borrow_mut().push(bytes.clone());
                }
            })
            .detach();
        });
        for length in [
            muxy_protocol::MAX_INPUT - 12,
            muxy_protocol::MAX_INPUT,
            muxy_protocol::MAX_INPUT + 1,
        ] {
            for bracketed_paste in [false, true] {
                let text = format!("{}界", "a".repeat(length - 3));
                let expected = if bracketed_paste {
                    format!("\x1b[200~{text}\x1b[201~")
                } else {
                    text.clone()
                };
                input.borrow_mut().clear();
                cx.update(|window, cx| {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                    pane.update(cx, |pane, cx| {
                        pane.grid = Some(grid());
                        pane.grid.as_mut().unwrap().modes.bracketed_paste = bracketed_paste;
                        pane.channel = Some(ChannelId(1));
                        pane.state = PaneState::Live;
                        pane.paste(&muxy_ui::text_input::Paste, window, cx);
                    });
                });
                cx.run_until_parked();
                let messages = input.borrow();
                assert!(
                    messages
                        .iter()
                        .all(|message| muxy_protocol::validate_input(message).is_ok())
                );
                assert_eq!(messages.concat(), expected.as_bytes());
            }
        }
    }

    #[gpui::test]
    #[allow(clippy::too_many_lines)]
    fn mouse_copy_paste_and_non_live_panes(cx: &mut TestAppContext) {
        let (pane, cx) = cx.add_window_view(|_, cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        let input = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|window, cx| {
            let input = input.clone();
            cx.subscribe(&pane, move |_, event, _| {
                if let PaneEvent::Input(_, bytes) = event {
                    input.borrow_mut().push(bytes.clone());
                }
            })
            .detach();
            pane.read(cx).focus.focus(window);
        });
        cx.run_until_parked();
        pane.update(cx, |pane, _| {
            pane.grid = Some(grid());
            pane.state = PaneState::Live;
            pane.channel = Some(ChannelId(1));
            pane.viewport = Some(Size { cols: 20, rows: 3 });
            pane.geometry = Some((
                Bounds::new(point(px(0.0), px(0.0)), size(px(200.0), px(60.0))),
                size(px(10.0), px(20.0)),
            ));
        });
        cx.update(|window, cx| {
            pane.update(cx, |pane, cx| {
                let down = gpui::MouseDownEvent {
                    button: MouseButton::Left,
                    position: point(px(0.0), px(5.0)),
                    click_count: 1,
                    ..gpui::MouseDownEvent::default()
                };
                pane.mouse_down(&down, window, cx);
                assert!(pane.selection.is_none());
                pane.mouse_move(
                    &gpui::MouseMoveEvent {
                        position: point(px(50.0), px(5.0)),
                        pressed_button: Some(MouseButton::Left),
                        ..gpui::MouseMoveEvent::default()
                    },
                    cx,
                );
                pane.mouse_up(
                    &gpui::MouseUpEvent {
                        button: MouseButton::Left,
                        position: point(px(50.0), px(5.0)),
                        ..gpui::MouseUpEvent::default()
                    },
                    window,
                    cx,
                );
                pane.copy(&muxy_ui::text_input::Copy, window, cx);
                assert_eq!(
                    cx.read_from_clipboard().unwrap().text().as_deref(),
                    Some("alpha")
                );
                pane.mouse_down(&down, window, cx);
                pane.mouse_up(
                    &gpui::MouseUpEvent {
                        button: MouseButton::Left,
                        position: down.position,
                        ..gpui::MouseUpEvent::default()
                    },
                    window,
                    cx,
                );
                assert!(pane.selection.is_none());
                pane.copy(&muxy_ui::text_input::Copy, window, cx);
                assert_eq!(
                    cx.read_from_clipboard().unwrap().text().as_deref(),
                    Some("alpha")
                );
                cx.write_to_clipboard(gpui::ClipboardItem::new_string("one\ntwo".into()));
                pane.grid.as_mut().unwrap().modes.bracketed_paste = true;
                pane.paste(&muxy_ui::text_input::Paste, window, cx);
                for state in [
                    PaneState::Exited {
                        reason: None,
                        unavailable: false,
                    },
                    PaneState::Disconnected,
                ] {
                    pane.set_state(state, cx);
                    pane.select(
                        Selection::row(Point { row: 1, column: 0 }, pane.grid.as_ref().unwrap()),
                        cx,
                    );
                    pane.copy(&muxy_ui::text_input::Copy, window, cx);
                    assert_eq!(
                        cx.read_from_clipboard().unwrap().text().as_deref(),
                        Some("second row")
                    );
                    pane.paste(&muxy_ui::text_input::Paste, window, cx);
                }
            });
        });
        cx.run_until_parked();
        assert_eq!(&*input.borrow(), &[b"\x1b[200~one\rtwo\x1b[201~".to_vec()]);
    }

    #[gpui::test]
    fn terminal_focused_find_dismissal_is_remappable(cx: &mut TestAppContext) {
        struct Remapped;
        impl muxy_core::shortcuts::ShortcutSettings for Remapped {
            fn keys(&self, id: &str, context: Option<&str>) -> Vec<String> {
                if id == "terminal.close_find" {
                    vec!["ctrl-k".into()]
                } else {
                    muxy_core::shortcuts::ShortcutSettings::keys(
                        &muxy_core::shortcuts::Defaults,
                        id,
                        context,
                    )
                }
            }
        }
        cx.update(|cx| {
            let mut registry = muxy_ui::shortcuts::Registry::new(&Remapped);
            register_shortcuts(&mut registry);
            muxy_ui::text_input::register_shortcuts(&mut registry);
            cx.bind_keys(registry.into_bindings());
        });
        let (pane, cx) = cx.add_window_view(|window, cx| {
            let mut pane = TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            );
            pane.grid = Some(grid());
            pane.channel = Some(ChannelId(1));
            pane.state = PaneState::Live;
            pane.open_find(
                &muxy_ui::theme::Theme::from_scheme(&muxy_ui::theme::ColorScheme::default()),
                muxy_ui::theme::Metrics::new(1.0),
                window,
                cx,
            );
            pane.focus.focus(window);
            pane
        });
        cx.simulate_keystrokes("escape");
        assert!(pane.read_with(cx, |pane, _| pane.find.is_some()));
        cx.simulate_keystrokes("ctrl-k");
        assert!(pane.read_with(cx, |pane, _| pane.find.is_none()));
    }
    #[gpui::test]
    fn phase20_copy_on_select_runs_only_at_selection_completion(cx: &mut TestAppContext) {
        let (pane, cx) = cx.add_window_view(|_, cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        for enabled in [false, true] {
            cx.update(|window, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string("original".into()));
                pane.update(cx, |pane, cx| {
                    prepare_mouse(pane);
                    pane.copy_on_select = enabled;
                    pane.mouse_down(
                        &gpui::MouseDownEvent {
                            position: point(px(0.0), px(5.0)),
                            click_count: 1,
                            ..gpui::MouseDownEvent::default()
                        },
                        window,
                        cx,
                    );
                    pane.mouse_move(
                        &gpui::MouseMoveEvent {
                            position: point(px(50.0), px(5.0)),
                            pressed_button: Some(MouseButton::Left),
                            ..gpui::MouseMoveEvent::default()
                        },
                        cx,
                    );
                    assert_eq!(
                        cx.read_from_clipboard()
                            .and_then(|item| item.text())
                            .as_deref(),
                        Some("original")
                    );
                    pane.mouse_up(
                        &gpui::MouseUpEvent {
                            position: point(px(50.0), px(5.0)),
                            ..gpui::MouseUpEvent::default()
                        },
                        window,
                        cx,
                    );
                    assert_eq!(
                        cx.read_from_clipboard()
                            .and_then(|item| item.text())
                            .as_deref(),
                        Some(if enabled { "alpha" } else { "original" })
                    );
                    pane.select_all(cx);
                    pane.copy_selection(cx);
                    assert_eq!(
                        cx.read_from_clipboard()
                            .and_then(|item| item.text())
                            .as_deref(),
                        Some("history row\nalpha beta\nsecond row\nprompt")
                    );
                });
            });
        }
    }

    #[gpui::test]
    fn phase20_command_click_opens_and_plain_click_selects_even_with_mouse_reporting(
        cx: &mut TestAppContext,
    ) {
        let (pane, cx) = cx.add_window_view(|_, cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|window, cx| {
            let links = opened.clone();
            cx.subscribe(&pane, move |_, event, _| {
                if let PaneEvent::OpenLink(text) = event {
                    links.borrow_mut().push(text.clone());
                }
            })
            .detach();
            pane.update(cx, |pane, cx| {
                prepare_mouse(pane);
                pane.metadata(
                    MetadataEvent::Links {
                        seq: 0,
                        rows: vec![muxy_protocol::LinkRow {
                            row: 0,
                            spans: vec![muxy_protocol::LinkSpan {
                                start: 0,
                                end: 5,
                                uri: "https://example.com".into(),
                            }],
                        }],
                    },
                    cx,
                );
                let point = point(px(10.0), px(5.0));
                pane.hover_link(point, true, cx);
                assert!(pane.link_hover.target.is_some());
                pane.hover_link(point, false, cx);
                assert!(pane.link_hover.target.is_none());
                pane.mouse_down(
                    &gpui::MouseDownEvent {
                        position: point,
                        click_count: 2,
                        ..gpui::MouseDownEvent::default()
                    },
                    window,
                    cx,
                );
                assert!(pane.selection.is_some());
                pane.input_modes.mouse_tracking = true;
                pane.mouse_down(
                    &gpui::MouseDownEvent {
                        position: point,
                        modifiers: gpui::Modifiers {
                            platform: true,
                            ..gpui::Modifiers::default()
                        },
                        ..gpui::MouseDownEvent::default()
                    },
                    window,
                    cx,
                );
                assert!(pane.held_buttons.is_empty());
            });
        });
        cx.run_until_parked();
        assert_eq!(
            *opened.borrow(),
            [muxy_app_core::opener::Target::Url(
                "https://example.com".into()
            )]
        );
    }

    #[gpui::test]
    fn phase20_file_drop_only_sends_to_live_sessions(cx: &mut TestAppContext) {
        let pane = cx.new(|cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        let sent = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|cx| {
            let inputs = sent.clone();
            cx.subscribe(&pane, move |_, event, _| {
                if let PaneEvent::Input(_, bytes) = event {
                    inputs.borrow_mut().extend_from_slice(bytes);
                }
            })
            .detach();
        });
        pane.update(cx, |pane, cx| {
            prepare_mouse(pane);
            pane.grid.as_mut().unwrap().modes.bracketed_paste = true;
            pane.drop_paths(&["/tmp/a b".into()], cx);
            for state in [
                PaneState::Disconnected,
                PaneState::Connecting,
                PaneState::Exited {
                    reason: None,
                    unavailable: false,
                },
            ] {
                pane.set_state(state, cx);
                pane.drop_paths(&["/tmp/ignored".into()], cx);
            }
        });
        cx.run_until_parked();
        assert_eq!(*sent.borrow(), b"\x1b[200~'/tmp/a b'\x1b[201~");
    }

    #[gpui::test]
    fn command_click_on_rejected_candidates_preserves_selection_and_mouse_reporting(
        cx: &mut TestAppContext,
    ) {
        let (pane, cx) = cx.add_window_view(|_, cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|window, cx| {
            let links = opened.clone();
            cx.subscribe(&pane, move |_, event, _| {
                if let PaneEvent::OpenLink(target) = event {
                    links.borrow_mut().push(target.clone());
                }
            })
            .detach();
            for uri in [
                None,
                Some("javascript:alert(1)"),
                Some("file://remote/etc/hosts"),
            ] {
                for reporting in [false, true] {
                    pane.update(cx, |pane, cx| {
                        prepare_mouse(pane);
                        let rows = uri
                            .map(|uri| {
                                vec![muxy_protocol::LinkRow {
                                    row: 0,
                                    spans: vec![muxy_protocol::LinkSpan {
                                        start: 0,
                                        end: 5,
                                        uri: uri.into(),
                                    }],
                                }]
                            })
                            .unwrap_or_default();
                        pane.metadata(MetadataEvent::Links { seq: 0, rows }, cx);
                        pane.link_hover = super::super::links::Hover::default();
                        pane.input_modes.mouse_tracking = reporting;
                        pane.mouse_down(
                            &gpui::MouseDownEvent {
                                position: point(px(10.0), px(5.0)),
                                click_count: 2,
                                modifiers: gpui::Modifiers {
                                    platform: true,
                                    ..gpui::Modifiers::default()
                                },
                                ..gpui::MouseDownEvent::default()
                            },
                            window,
                            cx,
                        );
                        assert!(pane.link_hover.target.is_none());
                        assert_eq!(pane.selection.is_some(), !reporting);
                        assert_eq!(!pane.held_buttons.is_empty(), reporting);
                        pane.mouse_up(
                            &gpui::MouseUpEvent {
                                position: point(px(10.0), px(5.0)),
                                modifiers: gpui::Modifiers {
                                    platform: true,
                                    ..gpui::Modifiers::default()
                                },
                                ..gpui::MouseUpEvent::default()
                            },
                            window,
                            cx,
                        );
                    });
                }
            }
        });
        cx.run_until_parked();
        assert!(opened.borrow().is_empty());
    }

    #[gpui::test]
    fn command_click_uses_a_resolved_file_and_context_changes_discard_it(cx: &mut TestAppContext) {
        use muxy_app_core::opener::{FileLocation, Target};
        let (pane, cx) = cx.add_window_view(|_, cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        let target = Target::File(FileLocation {
            path: "/project/alpha".into(),
            line: None,
            column: None,
        });
        let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|window, cx| {
            let links = opened.clone();
            cx.subscribe(&pane, move |_, event, _| {
                if let PaneEvent::OpenLink(target) = event {
                    links.borrow_mut().push(target.clone());
                }
            })
            .detach();
            pane.update(cx, |pane, cx| {
                prepare_mouse(pane);
                let position = point(px(10.0), px(5.0));
                pane.hover_link(position, true, cx);
                pane.link_hover.target = Some(target.clone());
                pane.mouse_down(
                    &gpui::MouseDownEvent {
                        position,
                        modifiers: gpui::Modifiers {
                            platform: true,
                            ..gpui::Modifiers::default()
                        },
                        ..gpui::MouseDownEvent::default()
                    },
                    window,
                    cx,
                );
                assert!(pane.selecting.is_none());
                pane.metadata(MetadataEvent::Directory(ServerPath(b"/other".to_vec())), cx);
                assert!(pane.link_hover.target.is_none());
                pane.hover_link(position, true, cx);
                pane.link_hover.target = Some(target.clone());
                pane.set_state(PaneState::Disconnected, cx);
                assert!(pane.link_hover.target.is_none());
            });
        });
        cx.run_until_parked();
        assert_eq!(*opened.borrow(), [target]);
    }
}
