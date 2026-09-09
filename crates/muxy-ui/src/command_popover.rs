use muxy_core::shortcuts::ShortcutId;

use crate::components::IconGlyph;
#[cfg(target_os = "macos")]
use crate::components::SymbolGlyph;
use crate::icon::Icon;
use crate::scrollbar::{MINIMUM_THUMB_LENGTH, ThumbGeometry};
use crate::text_input::{self, InputEvent, InputStyle, TextInput};
use crate::theme::{Metrics, Theme};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    Hsla, InteractiveElement, IntoElement, ListAlignment, ListOffset, ListState, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, actions, div, px,
};
use std::collections::HashMap;

const KEY_CONTEXT: &str = "CommandPopover";

actions!(
    command_popover,
    [
        SelectNext,
        SelectPrevious,
        Confirm,
        SecondaryConfirm,
        Dismiss,
        NextTab,
        PreviousTab,
        TabPressed,
        NavigateBack,
    ]
);

pub fn register_shortcuts(registry: &mut crate::shortcuts::Registry<'_>) {
    registry.register(ShortcutId::PopoverSelectNext, &SelectNext);
    registry.register(ShortcutId::PopoverSelectPrevious, &SelectPrevious);
    registry.register(ShortcutId::PopoverConfirm, &Confirm);
    registry.register(ShortcutId::PopoverSecondaryConfirm, &SecondaryConfirm);
    registry.register(ShortcutId::PopoverTabPressed, &TabPressed);
    registry.register(ShortcutId::PopoverNavigateBack, &NavigateBack);
    registry.register(ShortcutId::PopoverDismiss, &Dismiss);
    registry.register(ShortcutId::PopoverNextTab, &NextTab);
    registry.register(ShortcutId::PopoverPreviousTab, &PreviousTab);
}

pub fn key_bindings() -> Vec<gpui::KeyBinding> {
    let mut registry = crate::shortcuts::Registry::new(&muxy_core::shortcuts::Defaults);
    register_shortcuts(&mut registry);
    registry.into_bindings()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandPopoverPresentation {
    Modal,
    Popover,
    Embedded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandPopoverDensity {
    Comfortable,
    Compact,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CommandPopoverLayout {
    inline_tabs: bool,
    header_height: f32,
    row_height: f32,
    section_height: f32,
    tab_height: f32,
    panel_radius: f32,
    row_radius: f32,
    horizontal_inset: f32,
    outer_item_inset: f32,
    item_inset: f32,
    item_gap: f32,
    list_vertical_inset: f32,
    status_height: f32,
}

impl CommandPopoverLayout {
    fn resolve(presentation: CommandPopoverPresentation, density: CommandPopoverDensity) -> Self {
        match density {
            CommandPopoverDensity::Compact => Self {
                inline_tabs: presentation == CommandPopoverPresentation::Popover,
                header_height: 32.0,
                row_height: 32.0,
                section_height: 22.0,
                tab_height: 24.0,
                panel_radius: 6.0,
                row_radius: 4.0,
                horizontal_inset: 8.0,
                outer_item_inset: 4.0,
                item_inset: 4.0,
                item_gap: 6.0,
                list_vertical_inset: 4.0,
                status_height: 48.0,
            },
            CommandPopoverDensity::Comfortable
                if presentation == CommandPopoverPresentation::Popover =>
            {
                Self {
                    inline_tabs: true,
                    header_height: 42.0,
                    row_height: 46.0,
                    section_height: 28.0,
                    tab_height: 28.0,
                    panel_radius: 8.0,
                    row_radius: 4.0,
                    horizontal_inset: 10.0,
                    outer_item_inset: 4.0,
                    item_inset: 6.0,
                    item_gap: 8.0,
                    list_vertical_inset: 0.0,
                    status_height: 80.0,
                }
            }
            CommandPopoverDensity::Comfortable => Self {
                inline_tabs: false,
                header_height: 52.0,
                row_height: 54.0,
                section_height: 54.0,
                tab_height: 32.0,
                panel_radius: 10.0,
                row_radius: 6.0,
                horizontal_inset: 14.0,
                outer_item_inset: 6.0,
                item_inset: 8.0,
                item_gap: 8.0,
                list_vertical_inset: 0.0,
                status_height: 96.0,
            },
        }
    }

    fn item_height(self, item: &CommandPopoverItem) -> f32 {
        match item {
            CommandPopoverItem::Section(_) => self.section_height,
            CommandPopoverItem::Row(_) => self.row_height,
        }
    }
}

fn input_style(density: CommandPopoverDensity, theme: &Theme, metrics: Metrics) -> InputStyle {
    match density {
        CommandPopoverDensity::Comfortable => InputStyle::field(theme, &metrics),
        CommandPopoverDensity::Compact => InputStyle::compact(theme, &metrics),
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "GPUI measures row heights with f32 coordinates."
)]
fn content_height(
    layout: CommandPopoverLayout,
    tab_count: usize,
    items: &[CommandPopoverItem],
    status: &CommandPopoverStatus,
    detail_line_count: Option<usize>,
    has_footer: bool,
) -> f32 {
    let tabs = if tab_count > 1 && !layout.inline_tabs {
        layout.tab_height + 12.0
    } else {
        0.0
    };
    let body = if let Some(line_count) = detail_line_count {
        line_count.max(1) as f32 * 20.0
    } else if matches!(status, CommandPopoverStatus::Ready) && !items.is_empty() {
        items
            .iter()
            .map(|item| layout.item_height(item))
            .sum::<f32>()
            + layout.list_vertical_inset * 2.0
    } else {
        layout.status_height
    };
    let footer = if has_footer { 37.0 } else { 0.0 };
    let border = 2.0;
    tabs + layout.header_height + body + footer + border
}

fn scrollbar_item_heights(
    layout: CommandPopoverLayout,
    scale: f32,
    items: &[CommandPopoverItem],
    detail_line_count: Option<usize>,
) -> Vec<f32> {
    if let Some(line_count) = detail_line_count {
        return vec![20.0 * scale; line_count];
    }
    items
        .iter()
        .map(|item| layout.item_height(item) * scale)
        .collect()
}

fn scrollbar_offset(heights: &[f32], offset: ListOffset) -> f32 {
    heights.iter().take(offset.item_ix).sum::<f32>() + f32::from(offset.offset_in_item)
}

fn scrollbar_list_offset(heights: &[f32], target: f32) -> ListOffset {
    let mut remaining = target.max(0.0);
    for (item_ix, height) in heights.iter().copied().enumerate() {
        if remaining < height {
            return ListOffset {
                item_ix,
                offset_in_item: px(remaining),
            };
        }
        remaining -= height;
    }
    ListOffset {
        item_ix: heights.len(),
        offset_in_item: px(0.0),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CommandPopoverGeometry {
    pub width: f32,
    pub height: f32,
    pub top: f32,
    pub backdrop: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CommandPopoverMetrics {
    pub modal_width: f32,
    pub modal_height: f32,
    pub modal_top: f32,
    pub popover_width: f32,
    pub popover_height: f32,
    pub viewport_margin: f32,
}

impl Default for CommandPopoverMetrics {
    fn default() -> Self {
        Self {
            modal_width: 640.0,
            modal_height: 520.0,
            modal_top: 48.0,
            popover_width: 640.0,
            popover_height: 520.0,
            viewport_margin: 8.0,
        }
    }
}

impl CommandPopoverMetrics {
    pub fn resolve(
        self,
        presentation: CommandPopoverPresentation,
        viewport_width: f32,
        viewport_height: f32,
    ) -> CommandPopoverGeometry {
        match presentation {
            CommandPopoverPresentation::Modal => {
                let width = self
                    .modal_width
                    .min((viewport_width - self.viewport_margin * 2.0).max(0.0));
                let top = if viewport_width < self.modal_width {
                    self.viewport_margin * 2.0
                } else {
                    self.modal_top
                }
                .min((viewport_height - self.viewport_margin).max(0.0));
                CommandPopoverGeometry {
                    width,
                    height: self
                        .modal_height
                        .min((viewport_height - top - self.viewport_margin * 2.0).max(0.0)),
                    top,
                    backdrop: true,
                }
            }
            CommandPopoverPresentation::Popover => CommandPopoverGeometry {
                width: self
                    .popover_width
                    .min((viewport_width - self.viewport_margin * 2.0).max(0.0)),
                height: self
                    .popover_height
                    .min((viewport_height - self.viewport_margin * 2.0).max(0.0)),
                top: 0.0,
                backdrop: false,
            },
            CommandPopoverPresentation::Embedded => CommandPopoverGeometry {
                width: viewport_width,
                height: viewport_height,
                top: 0.0,
                backdrop: false,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CommandPopoverLeading {
    Icon(Icon),
    Symbol(SharedString),
    Text(SharedString),
    Asset(SharedString),
    Swatch(Hsla),
}

#[derive(Clone, Debug, PartialEq)]
#[must_use]
pub struct CommandPopoverAction {
    pub id: SharedString,
    pub label: SharedString,
    pub icon: Option<CommandPopoverLeading>,
    pub destructive: bool,
    pub disabled: bool,
}

impl CommandPopoverAction {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            destructive: false,
            disabled: false,
        }
    }

    pub fn icon(mut self, icon: CommandPopoverLeading) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn destructive(mut self, destructive: bool) -> Self {
        self.destructive = destructive;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CommandPopoverRow {
    pub id: SharedString,
    pub title: SharedString,
    pub subtitle: Option<SharedString>,
    pub leading: Option<CommandPopoverLeading>,
    pub trailing: Option<SharedString>,
    pub actions: Vec<CommandPopoverAction>,
    pub swatches: Vec<Hsla>,
    pub current: bool,
    pub selected: bool,
    pub disabled: bool,
}

impl CommandPopoverRow {
    pub fn new(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            subtitle: None,
            leading: None,
            trailing: None,
            actions: Vec::new(),
            swatches: Vec::new(),
            current: false,
            selected: false,
            disabled: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[must_use]
pub enum CommandPopoverItem {
    Section(SharedString),
    Row(CommandPopoverRow),
}

impl CommandPopoverItem {
    pub fn section(label: impl Into<SharedString>) -> Self {
        Self::Section(label.into())
    }

    pub fn row(id: impl Into<SharedString>) -> Self {
        let id = id.into();
        Self::Row(CommandPopoverRow::new(id.clone(), id))
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        if let Self::Row(row) = &mut self {
            row.disabled = disabled;
        }
        self
    }

    pub fn row_data(self) -> Option<CommandPopoverRow> {
        match self {
            Self::Row(row) => Some(row),
            Self::Section(_) => None,
        }
    }

    fn selectable_id(&self) -> Option<&SharedString> {
        match self {
            Self::Row(row) if !row.disabled => Some(&row.id),
            Self::Section(_) | Self::Row(_) => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandPopoverSelection {
    pub id: SharedString,
}

impl CommandPopoverSelection {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self { id: id.into() }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandPopoverEscape {
    CloseInlineAction,
    Dismiss,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownCommandPopoverTab;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownCommandPopoverRow;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum CommandPopoverStatus {
    #[default]
    Ready,
    Loading(SharedString),
    Empty(SharedString),
    Error(SharedString),
}

#[derive(Clone, Debug, Default)]
struct CommandPopoverTabState {
    query: String,
    items: Vec<CommandPopoverItem>,
    selected_id: Option<SharedString>,
    inline_action: Option<(SharedString, SharedString)>,
    status: CommandPopoverStatus,
}

#[derive(Clone, Debug)]
pub struct CommandPopoverState {
    tabs: Vec<SharedString>,
    active_tab: SharedString,
    active: CommandPopoverTabState,
    tab_states: HashMap<SharedString, CommandPopoverTabState>,
}

impl CommandPopoverState {
    pub fn new(tabs: impl IntoIterator<Item = impl Into<SharedString>>) -> Self {
        let tabs = tabs.into_iter().map(Into::into).collect::<Vec<_>>();
        assert!(!tabs.is_empty());
        let active_tab = tabs[0].clone();
        let tab_states = tabs
            .iter()
            .filter(|tab| **tab != active_tab)
            .cloned()
            .map(|tab| (tab, CommandPopoverTabState::default()))
            .collect();
        Self {
            tabs,
            active_tab,
            active: CommandPopoverTabState::default(),
            tab_states,
        }
    }

    pub fn active_tab(&self) -> &str {
        self.active_tab.as_ref()
    }

    pub fn tabs(&self) -> &[SharedString] {
        &self.tabs
    }

    pub fn query(&self) -> &str {
        &self.active_state().query
    }

    pub fn item_count(&self) -> usize {
        self.active_state().items.len()
    }

    pub fn items(&self) -> &[CommandPopoverItem] {
        &self.active_state().items
    }

    pub fn status(&self) -> &CommandPopoverStatus {
        &self.active_state().status
    }

    pub fn set_status(&mut self, status: CommandPopoverStatus) {
        let state = self.active_state_mut();
        state.status = status;
        if state.status != CommandPopoverStatus::Ready {
            state.inline_action = None;
        }
    }

    pub fn set_query(&mut self, query: impl Into<String>) {
        self.active_state_mut().query = query.into();
    }

    pub fn set_items(&mut self, items: Vec<CommandPopoverItem>) {
        let previous = self.active_state().selected_id.clone();
        let selected_id = previous
            .filter(|selected| {
                items
                    .iter()
                    .any(|item| item.selectable_id() == Some(selected))
            })
            .or_else(|| {
                items
                    .iter()
                    .find_map(CommandPopoverItem::selectable_id)
                    .cloned()
            });
        let state = self.active_state_mut();
        state.items = items;
        state.selected_id = selected_id;
        state.inline_action = None;
        state.status = CommandPopoverStatus::Ready;
    }

    pub fn activate_tab(&mut self, tab: &str) -> Result<(), UnknownCommandPopoverTab> {
        if self.active_tab.as_ref() == tab {
            return Ok(());
        }
        let Some(tab) = self.tabs.iter().find(|candidate| candidate.as_ref() == tab) else {
            return Err(UnknownCommandPopoverTab);
        };
        let next = self
            .tab_states
            .remove(tab)
            .ok_or(UnknownCommandPopoverTab)?;
        self.tab_states.insert(
            self.active_tab.clone(),
            std::mem::replace(&mut self.active, next),
        );
        self.active_tab = tab.clone();
        Ok(())
    }

    pub fn selected_row_id(&self) -> Option<&str> {
        if *self.status() != CommandPopoverStatus::Ready {
            return None;
        }
        self.active_state().selected_id.as_ref().map(AsRef::as_ref)
    }

    pub fn select_next(&mut self) {
        self.move_selection(1);
    }

    pub fn select_previous(&mut self) {
        self.move_selection(-1);
    }

    pub fn select_last(&mut self) {
        if *self.status() != CommandPopoverStatus::Ready {
            return;
        }
        let selected = self
            .active_state()
            .items
            .iter()
            .rev()
            .find_map(CommandPopoverItem::selectable_id)
            .cloned();
        self.active_state_mut().selected_id = selected;
    }

    pub fn select_row(&mut self, id: &str) -> Result<(), UnknownCommandPopoverRow> {
        if *self.status() != CommandPopoverStatus::Ready
            || !self.active_state().items.iter().any(|item| {
                item.selectable_id()
                    .is_some_and(|candidate| candidate.as_ref() == id)
            })
        {
            return Err(UnknownCommandPopoverRow);
        }
        self.active_state_mut().selected_id = Some(SharedString::from(id.to_owned()));
        Ok(())
    }

    pub fn open_inline_action(
        &mut self,
        id: &str,
        action: &str,
    ) -> Result<(), UnknownCommandPopoverRow> {
        self.select_row(id)?;
        self.active_state_mut().inline_action =
            Some((id.to_owned().into(), action.to_owned().into()));
        Ok(())
    }

    pub fn escape(&mut self) -> CommandPopoverEscape {
        if self.active_state_mut().inline_action.take().is_some() {
            CommandPopoverEscape::CloseInlineAction
        } else {
            CommandPopoverEscape::Dismiss
        }
    }

    pub fn confirm(&self) -> Option<CommandPopoverSelection> {
        self.selected_row_id()
            .map(|id| CommandPopoverSelection::new(id.to_owned()))
    }

    fn inline_action(&self) -> Option<(&str, &str)> {
        self.active_state()
            .inline_action
            .as_ref()
            .map(|(row, action)| (row.as_ref(), action.as_ref()))
    }

    fn active_state(&self) -> &CommandPopoverTabState {
        &self.active
    }

    fn active_state_mut(&mut self) -> &mut CommandPopoverTabState {
        &mut self.active
    }

    fn move_selection(&mut self, delta: isize) {
        if *self.status() != CommandPopoverStatus::Ready {
            return;
        }
        let selectable = self
            .active_state()
            .items
            .iter()
            .filter_map(CommandPopoverItem::selectable_id)
            .cloned()
            .collect::<Vec<_>>();
        if selectable.is_empty() {
            self.active_state_mut().selected_id = None;
            return;
        }
        let current = self
            .selected_row_id()
            .and_then(|selected| selectable.iter().position(|id| id.as_ref() == selected))
            .unwrap_or(if delta > 0 { selectable.len() - 1 } else { 0 });
        let next = if delta > 0 {
            (current + 1) % selectable.len()
        } else {
            (current + selectable.len() - 1) % selectable.len()
        };
        self.active_state_mut().selected_id = Some(selectable[next].clone());
    }
}

#[derive(Clone, Debug)]
pub struct CommandPopoverTab {
    pub id: SharedString,
    pub label: SharedString,
}

#[derive(Clone, Debug)]
pub struct CommandPopoverHint {
    pub key: SharedString,
    pub label: SharedString,
}

impl CommandPopoverHint {
    pub fn new(key: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
        }
    }
}

impl CommandPopoverTab {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct CommandPopoverConfig {
    pub id: SharedString,
    pub presentation: CommandPopoverPresentation,
    pub density: CommandPopoverDensity,
    pub tabs: Vec<CommandPopoverTab>,
    pub placeholder: SharedString,
    pub footer_actions: Vec<CommandPopoverAction>,
    pub footer_hints: Vec<CommandPopoverHint>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub max_height: Option<f32>,
    pub completion_on_tab: bool,
    pub confirm_on_click: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CommandPopoverEvent {
    QueryChanged {
        tab: SharedString,
        query: SharedString,
    },
    TabChanged(SharedString),
    SelectionChanged(CommandPopoverSelection),
    RowClicked {
        row: SharedString,
        shift: bool,
        platform: bool,
    },
    Confirmed(CommandPopoverSelection),
    SecondaryConfirmed(CommandPopoverSelection),
    Submitted {
        secondary: bool,
    },
    CompletionRequested,
    NavigateBackRequested,
    RowAction {
        row: SharedString,
        action: SharedString,
    },
    FooterAction(SharedString),
    InlineActionDismissed,
    Dismissed,
}

pub struct CommandPopover {
    config: CommandPopoverConfig,
    state: CommandPopoverState,
    input: Entity<TextInput>,
    focus_handle: FocusHandle,
    scroll: ListState,
    theme: Theme,
    metrics: Metrics,
    focused: bool,
    header_detail: Option<SharedString>,
    detail: Option<(SharedString, Vec<SharedString>)>,
    confirmation_message: Option<SharedString>,
    scrollbar_drag: Option<Pixels>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<CommandPopoverEvent> for CommandPopover {}

impl Focusable for CommandPopover {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl CommandPopover {
    pub fn new(
        config: CommandPopoverConfig,
        theme: Theme,
        metrics: Metrics,
        cx: &mut Context<Self>,
    ) -> Self {
        assert!(!config.tabs.is_empty());
        assert!(config.height.is_none() || config.max_height.is_none());
        let style = input_style(config.density, &theme, metrics);
        let input = cx.new(|cx| {
            TextInput::new(style, cx)
                .with_key_context(text_input::BARE_CONTEXT)
                .with_placeholder(config.placeholder.clone())
        });
        let subscription = cx.subscribe(
            &input,
            |popover: &mut Self, input: Entity<TextInput>, event, cx| {
                if !matches!(event, InputEvent::Changed) {
                    return;
                }
                let query = input.read(cx).text().to_owned();
                popover.state.set_query(query.clone());
                cx.emit(CommandPopoverEvent::QueryChanged {
                    tab: popover.state.active_tab.clone(),
                    query: query.into(),
                });
                cx.notify();
            },
        );
        let state = CommandPopoverState::new(config.tabs.iter().map(|tab| tab.id.clone()));
        let row_height =
            CommandPopoverLayout::resolve(config.presentation, config.density).row_height;
        Self {
            config,
            state,
            input,
            focus_handle: cx.focus_handle(),
            scroll: ListState::new(0, ListAlignment::Top, metrics.scaled(row_height)),
            theme,
            metrics,
            focused: false,
            header_detail: None,
            detail: None,
            confirmation_message: None,
            scrollbar_drag: None,
            _subscriptions: vec![subscription],
        }
    }

    pub fn input(&self) -> Entity<TextInput> {
        self.input.clone()
    }

    pub fn active_tab(&self) -> &str {
        self.state.active_tab()
    }

    pub fn query(&self) -> &str {
        self.state.query()
    }

    pub fn set_placeholder(&self, placeholder: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.input
            .update(cx, |input, _| input.set_placeholder(placeholder.into()));
    }

    pub fn set_query(&mut self, query: impl Into<String>, cx: &mut Context<Self>) {
        let query = query.into();
        self.state.set_query(query.clone());
        self.input.update(cx, |input, cx| input.set_text(query, cx));
    }

    pub fn set_items(&mut self, items: Vec<CommandPopoverItem>, cx: &mut Context<Self>) {
        self.state.set_items(items);
        self.confirmation_message = None;
        if self.detail.is_none() {
            self.scroll.reset(self.state.item_count());
            self.scroll_to_selection();
        }
        cx.notify();
    }

    pub fn set_status(&mut self, status: CommandPopoverStatus, cx: &mut Context<Self>) {
        self.state.set_status(status);
        cx.notify();
    }

    pub fn set_footer_actions(
        &mut self,
        actions: Vec<CommandPopoverAction>,
        cx: &mut Context<Self>,
    ) {
        self.config.footer_actions = actions;
        cx.notify();
    }

    pub fn set_header_detail(
        &mut self,
        detail: Option<impl Into<SharedString>>,
        cx: &mut Context<Self>,
    ) {
        self.header_detail = detail.map(Into::into);
        cx.notify();
    }

    pub fn select_row(
        &mut self,
        id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), UnknownCommandPopoverRow> {
        self.state.select_row(id)?;
        self.scroll_to_selection();
        cx.notify();
        Ok(())
    }

    pub fn set_appearance(&mut self, theme: Theme, metrics: Metrics, cx: &mut Context<Self>) {
        let style = input_style(self.config.density, &theme, metrics);
        self.theme = theme;
        self.metrics = metrics;
        self.input
            .update(cx, |input, cx| input.set_style(style, cx));
        cx.notify();
    }

    pub fn show_detail(
        &mut self,
        title: impl Into<SharedString>,
        body: &str,
        cx: &mut Context<Self>,
    ) {
        let lines = body
            .lines()
            .map(|line| SharedString::from(line.to_owned()))
            .collect::<Vec<_>>();
        self.detail = Some((title.into(), lines));
        self.focused = false;
        self.scroll
            .reset(self.detail.as_ref().map_or(0, |(_, lines)| lines.len()));
        cx.notify();
    }

    pub fn open_confirmation(
        &mut self,
        row: &str,
        action: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), UnknownCommandPopoverRow> {
        self.open_confirmation_with_message(row, action, None, cx)
    }

    pub fn open_confirmation_with_message(
        &mut self,
        row: &str,
        action: &str,
        message: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> Result<(), UnknownCommandPopoverRow> {
        self.state.open_inline_action(row, action)?;
        self.confirmation_message = message;
        cx.notify();
        Ok(())
    }

    pub fn activate_tab(
        &mut self,
        tab: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), UnknownCommandPopoverTab> {
        self.state.activate_tab(tab)?;
        self.detail = None;
        self.focused = false;
        let query = self.state.query().to_owned();
        self.input.update(cx, |input, cx| input.set_text(query, cx));
        cx.emit(CommandPopoverEvent::TabChanged(
            self.state.active_tab.clone(),
        ));
        self.scroll.reset(self.state.item_count());
        self.scroll_to_selection();
        cx.notify();
        Ok(())
    }

    fn move_selection(&mut self, direction: isize, cx: &mut Context<Self>) {
        if self.detail.is_some() {
            return;
        }
        if direction > 0 {
            self.state.select_next();
        } else {
            self.state.select_previous();
        }
        self.scroll_to_selection();
        if let Some(selection) = self.state.confirm() {
            cx.emit(CommandPopoverEvent::SelectionChanged(selection));
        }
        cx.notify();
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, cx);
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        if !self.accepts_submission() {
            return;
        }
        if let Some(selection) = self.state.confirm() {
            cx.emit(CommandPopoverEvent::Confirmed(selection));
        } else {
            cx.emit(CommandPopoverEvent::Submitted { secondary: false });
        }
    }

    fn secondary_confirm(&mut self, _: &SecondaryConfirm, _: &mut Window, cx: &mut Context<Self>) {
        if !self.accepts_submission() {
            return;
        }
        if let Some(selection) = self.state.confirm() {
            cx.emit(CommandPopoverEvent::SecondaryConfirmed(selection));
        } else {
            cx.emit(CommandPopoverEvent::Submitted { secondary: true });
        }
    }

    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        if self.detail.take().is_some() {
            self.scroll.reset(self.state.item_count());
            window.focus(&self.input.focus_handle(cx));
            self.focused = true;
            cx.notify();
            return;
        }
        match self.state.escape() {
            CommandPopoverEscape::CloseInlineAction => {
                self.confirmation_message = None;
                cx.emit(CommandPopoverEvent::InlineActionDismissed);
                cx.notify();
            }
            CommandPopoverEscape::Dismiss => cx.emit(CommandPopoverEvent::Dismissed),
        }
    }

    fn next_tab(&mut self, _: &NextTab, _: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab(1, cx);
    }

    fn previous_tab(&mut self, _: &PreviousTab, _: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab(-1, cx);
    }

    fn tab_pressed(&mut self, _: &TabPressed, _: &mut Window, cx: &mut Context<Self>) {
        if !self.accepts_submission() {
            return;
        }
        if self.config.completion_on_tab {
            cx.emit(CommandPopoverEvent::CompletionRequested);
        } else {
            self.move_selection(1, cx);
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "GPUI action listeners use the entity receiver."
    )]
    fn navigate_back(&mut self, _: &NavigateBack, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(CommandPopoverEvent::NavigateBackRequested);
    }

    fn accepts_submission(&self) -> bool {
        self.detail.is_none()
            && matches!(
                self.state.status(),
                CommandPopoverStatus::Ready | CommandPopoverStatus::Empty(_)
            )
    }

    fn cycle_tab(&mut self, delta: isize, cx: &mut Context<Self>) {
        let current = self
            .config
            .tabs
            .iter()
            .position(|tab| tab.id.as_ref() == self.state.active_tab())
            .unwrap_or(0);
        if self.config.tabs.is_empty() {
            return;
        }
        let next = if delta > 0 {
            (current + 1) % self.config.tabs.len()
        } else {
            (current + self.config.tabs.len() - 1) % self.config.tabs.len()
        };
        let tab = self.config.tabs[next].id.clone();
        let _ = self.activate_tab(&tab, cx);
    }

    fn scroll_to_selection(&self) {
        if let Some(selected) = self.state.selected_row_id()
            && let Some(index) = self.state.items().iter().position(|item| {
                item.selectable_id()
                    .is_some_and(|candidate| candidate.as_ref() == selected)
            })
        {
            self.scroll.scroll_to_reveal_item(index);
        }
    }

    fn fitted_height(&self, layout: CommandPopoverLayout) -> f32 {
        let logical_height = content_height(
            layout,
            self.config.tabs.len(),
            self.state.items(),
            self.state.status(),
            self.detail.as_ref().map(|(_, lines)| lines.len()),
            !self.config.footer_actions.is_empty() || !self.config.footer_hints.is_empty(),
        );
        f32::from(self.metrics.scaled(logical_height))
    }

    fn render_leading(
        leading: &CommandPopoverLeading,
        color: Hsla,
        metrics: Metrics,
    ) -> AnyElement {
        match leading {
            CommandPopoverLeading::Icon(icon) => {
                IconGlyph::new(*icon, metrics.icon_md(), color).into_any_element()
            }
            CommandPopoverLeading::Symbol(symbol) => {
                #[cfg(target_os = "macos")]
                {
                    SymbolGlyph::new(symbol.clone(), metrics.icon_md(), color).into_any_element()
                }
                #[cfg(not(target_os = "macos"))]
                {
                    IconGlyph::new(
                        Icon::from_symbol(symbol).unwrap_or(Icon::Puzzle),
                        metrics.icon_md(),
                        color,
                    )
                    .into_any_element()
                }
            }
            CommandPopoverLeading::Text(text) => div()
                .w(metrics.icon_md())
                .flex_none()
                .text_size(metrics.font_caption())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .child(text.clone())
                .into_any_element(),
            CommandPopoverLeading::Asset(path) => gpui::svg()
                .path(path.clone())
                .size(metrics.icon_md())
                .flex_none()
                .text_color(color)
                .into_any_element(),
            CommandPopoverLeading::Swatch(color) => div()
                .size(metrics.icon_md())
                .rounded(metrics.radius_sm())
                .bg(*color)
                .into_any_element(),
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "The declarative row tree keeps its layout and event handlers together."
    )]
    fn render_item(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(item) = self.state.items().get(index).cloned() else {
            return div().into_any_element();
        };
        let layout = CommandPopoverLayout::resolve(self.config.presentation, self.config.density);
        match item {
            CommandPopoverItem::Section(label) => div()
                .w_full()
                .h(self.metrics.scaled(layout.section_height))
                .flex()
                .items_end()
                .px(self.metrics.scaled(layout.horizontal_inset))
                .pb(self.metrics.spacing2())
                .when(
                    self.config.presentation != CommandPopoverPresentation::Popover,
                    |element| element.border_b_1().border_color(self.theme.border),
                )
                .text_size(self.metrics.font_footnote())
                .font_weight(FontWeight::MEDIUM)
                .text_color(self.theme.fg_muted)
                .child(label)
                .into_any_element(),
            CommandPopoverItem::Row(row) => {
                if self
                    .state
                    .inline_action()
                    .is_some_and(|(candidate, _)| candidate == row.id.as_ref())
                {
                    return self.render_confirmation(&row, cx);
                }
                let highlighted = self.state.selected_row_id() == Some(row.id.as_ref());
                let row_height = layout.row_height;
                let id = row.id.clone();
                let hover_id = row.id.clone();
                let group = SharedString::from(format!("command-row-{}", row.id));
                let swatches = row.swatches.clone();
                let selected_background =
                    if self.config.presentation == CommandPopoverPresentation::Embedded {
                        self.theme.hover
                    } else {
                        self.theme.surface
                    };
                let mut content = div()
                    .id(SharedString::from(format!(
                        "command-popover-row-{}",
                        row.id
                    )))
                    .group(group.clone())
                    .w_full()
                    .h(self.metrics.scaled(row_height))
                    .px(self.metrics.scaled(
                        if self.config.presentation == CommandPopoverPresentation::Modal {
                            layout.horizontal_inset
                        } else {
                            layout.item_inset
                        },
                    ))
                    .when(
                        self.config.presentation != CommandPopoverPresentation::Modal,
                        |element| element.rounded(self.metrics.scaled(layout.row_radius)),
                    )
                    .flex()
                    .items_center()
                    .gap(self.metrics.scaled(layout.item_gap))
                    .text_color(if row.disabled {
                        self.theme.fg_dim
                    } else {
                        self.theme.fg
                    })
                    .when(highlighted || row.selected, |element| {
                        element.bg(selected_background)
                    })
                    .when(!row.disabled, |element| {
                        element
                            .cursor_pointer()
                            .hover(|style| style.bg(self.theme.hover))
                            .on_hover(cx.listener(move |popover, hovered: &bool, _, cx| {
                                if *hovered {
                                    if popover.detail.is_some()
                                        || popover.state.select_row(&hover_id).is_err()
                                    {
                                        return;
                                    }
                                    if let Some(selection) = popover.state.confirm() {
                                        cx.emit(CommandPopoverEvent::SelectionChanged(selection));
                                    }
                                    cx.notify();
                                }
                            }))
                            .on_click(cx.listener(
                                move |popover, event: &gpui::ClickEvent, _, cx| {
                                    if popover.detail.is_some()
                                        || popover.state.select_row(&id).is_err()
                                    {
                                        return;
                                    }
                                    cx.emit(CommandPopoverEvent::RowClicked {
                                        row: id.clone(),
                                        shift: event.modifiers().shift,
                                        platform: event.modifiers().platform,
                                    });
                                    if popover.config.confirm_on_click
                                        && let Some(selection) = popover.state.confirm()
                                    {
                                        cx.emit(CommandPopoverEvent::Confirmed(selection));
                                    }
                                },
                            ))
                    });
                if row.current || row.selected {
                    content = content.child(
                        IconGlyph::new(Icon::Check, self.metrics.icon_sm(), self.theme.accent)
                            .into_any_element(),
                    );
                } else if let Some(leading) = &row.leading {
                    content = content.child(Self::render_leading(
                        leading,
                        self.theme.fg_muted,
                        self.metrics,
                    ));
                } else {
                    content = content.child(div().w(self.metrics.icon_sm()));
                }
                content = content.child(
                    div()
                        .min_w(px(0.0))
                        .flex_grow()
                        .flex()
                        .flex_col()
                        .gap(self.metrics.spacing1())
                        .child(
                            div()
                                .min_w(px(0.0))
                                .truncate()
                                .text_size(self.metrics.font_body())
                                .font_weight(if row.current || row.selected {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .child(row.title),
                        )
                        .when_some(row.subtitle, |element, subtitle| {
                            element.child(
                                div()
                                    .min_w(px(0.0))
                                    .truncate()
                                    .text_size(self.metrics.font_footnote())
                                    .text_color(self.theme.fg_muted)
                                    .child(subtitle),
                            )
                        }),
                );
                if !swatches.is_empty() {
                    let mut preview = div()
                        .w(self.metrics.scaled(112.0))
                        .h(self.metrics.scaled(10.0))
                        .flex_none()
                        .flex()
                        .rounded(self.metrics.radius_sm())
                        .overflow_hidden();
                    for color in swatches {
                        preview = preview.child(div().h_full().flex_grow().bg(color));
                    }
                    content = content.child(preview);
                }
                if let Some(trailing) = row.trailing {
                    content = content.child(
                        div()
                            .flex_none()
                            .text_size(self.metrics.font_footnote())
                            .text_color(self.theme.fg_muted)
                            .child(trailing),
                    );
                }
                if !row.actions.is_empty() {
                    let mut actions = div()
                        .flex()
                        .items_center()
                        .gap(self.metrics.spacing1())
                        .opacity(0.0)
                        .group_hover(group, |style| style.opacity(1.0));
                    for action in row.actions {
                        let row_id = row.id.clone();
                        let action_id = action.id.clone();
                        let disabled = action.disabled;
                        let color = if action.destructive {
                            self.theme.danger
                        } else {
                            self.theme.fg_muted
                        };
                        let icon_only = action.icon.is_some();
                        actions = actions.child(
                            div()
                                .id(SharedString::from(format!(
                                    "command-popover-action-{}-{}",
                                    row_id, action.id
                                )))
                                .h(
                                    if self.config.presentation
                                        == CommandPopoverPresentation::Popover
                                    {
                                        self.metrics.control_small()
                                    } else {
                                        self.metrics.control_medium()
                                    },
                                )
                                .when(icon_only, |element| {
                                    element.w(
                                        if self.config.presentation
                                            == CommandPopoverPresentation::Popover
                                        {
                                            self.metrics.control_small()
                                        } else {
                                            self.metrics.control_medium()
                                        },
                                    )
                                })
                                .when(!icon_only, |element| element.px(self.metrics.spacing3()))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(self.metrics.radius_sm())
                                .when(disabled, |element| element.opacity(0.4))
                                .when(!disabled, |element| {
                                    element
                                        .cursor_pointer()
                                        .hover(|style| style.bg(self.theme.hover))
                                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation();
                                        })
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.emit(CommandPopoverEvent::RowAction {
                                                row: row_id.clone(),
                                                action: action_id.clone(),
                                            });
                                        }))
                                })
                                .when_some(action.icon, |element, icon| {
                                    element.child(Self::render_leading(&icon, color, self.metrics))
                                })
                                .when(!icon_only, |element| {
                                    element
                                        .text_size(self.metrics.font_footnote())
                                        .text_color(color)
                                        .child(action.label.clone())
                                }),
                        );
                    }
                    content = content.child(actions);
                }
                let _ = window;
                div()
                    .w_full()
                    .h(self.metrics.scaled(row_height))
                    .when(
                        self.config.presentation != CommandPopoverPresentation::Modal,
                        |element| element.px(self.metrics.scaled(layout.outer_item_inset)),
                    )
                    .child(content)
                    .into_any_element()
            }
        }
    }

    fn render_confirmation(
        &mut self,
        row: &CommandPopoverRow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let layout = CommandPopoverLayout::resolve(self.config.presentation, self.config.density);
        let action_id = self
            .state
            .inline_action()
            .map(|(_, action)| SharedString::from(action.to_owned()))
            .unwrap_or_default();
        let label = row
            .actions
            .iter()
            .find(|action| action.id == action_id)
            .map_or_else(|| "Confirm".into(), |action| action.label.clone());
        let message = self
            .confirmation_message
            .clone()
            .unwrap_or_else(|| format!("{label}?").into());
        let row_id = row.id.clone();
        let confirmed_action = SharedString::from(format!("confirm:{action_id}"));
        let content = div()
            .w_full()
            .h_full()
            .px(self.metrics.scaled(layout.item_inset))
            .rounded(self.metrics.scaled(layout.row_radius))
            .flex()
            .items_center()
            .gap(self.metrics.spacing3())
            .bg(self.theme.danger.opacity(0.12))
            .child(
                div()
                    .min_w(px(0.0))
                    .flex_grow()
                    .text_size(self.metrics.font_footnote())
                    .text_color(self.theme.fg)
                    .truncate()
                    .child(message),
            )
            .child(
                div()
                    .id("command-popover-confirm-cancel")
                    .px(self.metrics.spacing3())
                    .h(self.metrics.control_small())
                    .flex()
                    .items_center()
                    .rounded(self.metrics.radius_sm())
                    .cursor_pointer()
                    .hover(|style| style.bg(self.theme.hover))
                    .child("Cancel")
                    .on_click(cx.listener(|popover, _, _, cx| {
                        let _ = popover.state.escape();
                        popover.confirmation_message = None;
                        cx.emit(CommandPopoverEvent::InlineActionDismissed);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("command-popover-confirm-action")
                    .px(self.metrics.spacing3())
                    .h(self.metrics.control_small())
                    .flex()
                    .items_center()
                    .rounded(self.metrics.radius_sm())
                    .cursor_pointer()
                    .text_color(self.theme.danger)
                    .hover(|style| style.bg(self.theme.hover))
                    .child(label)
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(CommandPopoverEvent::RowAction {
                            row: row_id.clone(),
                            action: confirmed_action.clone(),
                        });
                    })),
            );
        div()
            .w_full()
            .h(self.metrics.scaled(layout.row_height))
            .px(self.metrics.scaled(layout.outer_item_inset))
            .child(content)
            .into_any_element()
    }

    fn render_detail_item(&self, index: usize) -> AnyElement {
        let line = self
            .detail
            .as_ref()
            .and_then(|(_, lines)| lines.get(index))
            .cloned()
            .unwrap_or_default();
        div()
            .w_full()
            .min_h(self.metrics.scaled(20.0))
            .px(self.metrics.spacing5())
            .text_size(self.metrics.font_caption())
            .font_family("monospace")
            .text_color(self.theme.fg)
            .child(line)
            .into_any_element()
    }

    fn render_tab_strip(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.config.tabs.len() <= 1 {
            return div().into_any_element();
        }
        let layout = CommandPopoverLayout::resolve(self.config.presentation, self.config.density);
        let mut strip = div()
            .flex()
            .items_center()
            .border_1()
            .border_color(self.theme.border)
            .rounded(self.metrics.scaled(layout.row_radius))
            .overflow_hidden();
        for tab in self.config.tabs.clone() {
            let selected = tab.id.as_ref() == self.state.active_tab();
            let id = tab.id.clone();
            strip = strip.child(
                div()
                    .id(SharedString::from(format!(
                        "command-popover-tab-{}",
                        tab.id
                    )))
                    .px(self.metrics.spacing5())
                    .h(self.metrics.scaled(layout.tab_height))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .text_size(self.metrics.font_body())
                    .font_weight(if selected {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::MEDIUM
                    })
                    .text_color(if selected {
                        self.theme.accent
                    } else {
                        self.theme.fg_muted
                    })
                    .when(selected, |element| element.bg(self.theme.accent_soft))
                    .when(!selected, |element| {
                        element.hover(|style| style.bg(self.theme.hover))
                    })
                    .on_click(cx.listener(move |popover, _, _, cx| {
                        let _ = popover.activate_tab(&id, cx);
                    }))
                    .child(tab.label),
            );
        }
        strip.into_any_element()
    }

    fn render_tabs(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let strip = self.render_tab_strip(cx);
        div()
            .flex()
            .items_center()
            .px(self.metrics.spacing5())
            .pt(self.metrics.spacing4())
            .pb(self.metrics.spacing2())
            .child(strip)
            .into_any_element()
    }

    fn render_footer(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.config.footer_actions.is_empty() && self.config.footer_hints.is_empty() {
            return None;
        }
        let mut footer = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(self.metrics.spacing2())
            .p(self.metrics.spacing3())
            .border_t_1()
            .border_color(self.theme.border);
        let mut hints = div().flex().items_center().gap(self.metrics.spacing4());
        for hint in self.config.footer_hints.clone() {
            hints = hints.child(
                div()
                    .flex()
                    .items_center()
                    .gap(self.metrics.spacing2())
                    .text_size(self.metrics.font_footnote())
                    .text_color(self.theme.fg_muted)
                    .child(
                        div()
                            .px(self.metrics.spacing2())
                            .py(self.metrics.spacing1())
                            .rounded(self.metrics.radius_sm())
                            .border_1()
                            .border_color(self.theme.border)
                            .bg(self.theme.surface)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(hint.key),
                    )
                    .child(hint.label),
            );
        }
        footer = footer.child(hints);
        let mut actions = div()
            .flex()
            .items_center()
            .justify_end()
            .gap(self.metrics.spacing2());
        for action in self.config.footer_actions.clone() {
            let action_id = action.id.clone();
            let button = div()
                .id(SharedString::from(format!(
                    "command-popover-footer-{}",
                    action.id
                )))
                .px(self.metrics.spacing4())
                .h(self.metrics.control_medium())
                .flex()
                .items_center()
                .gap(self.metrics.spacing2())
                .rounded(self.metrics.radius_sm())
                .text_size(self.metrics.font_footnote())
                .text_color(if action.destructive {
                    self.theme.danger
                } else {
                    self.theme.fg
                })
                .when(action.disabled, |element| element.opacity(0.4))
                .when(!action.disabled, |element| {
                    element
                        .cursor_pointer()
                        .hover(|style| style.bg(self.theme.hover))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(CommandPopoverEvent::FooterAction(action_id.clone()));
                        }))
                })
                .when_some(action.icon, |element, icon| {
                    element.child(Self::render_leading(
                        &icon,
                        self.theme.fg_muted,
                        self.metrics,
                    ))
                })
                .child(action.label);
            actions = actions.child(button);
        }
        footer = footer.child(actions);
        Some(footer.into_any_element())
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "Scrollbar calculations return to GPUI f32 pixel coordinates."
    )]
    fn scrollbar_geometry(&self) -> Option<CommandPopoverScrollbarGeometry> {
        let viewport = self.scroll.viewport_bounds();
        let visible = f64::from(viewport.size.height);
        let layout = CommandPopoverLayout::resolve(self.config.presentation, self.config.density);
        let scale = f32::from(self.metrics.scaled(1.0));
        let heights = scrollbar_item_heights(
            layout,
            scale,
            self.state.items(),
            self.detail.as_ref().map(|(_, lines)| lines.len()),
        );
        let vertical_inset = if self.detail.is_none() {
            f32::from(self.metrics.scaled(layout.list_vertical_inset)) * 2.0
        } else {
            0.0
        };
        let content = heights.iter().sum::<f32>() + vertical_inset;
        let maximum = f64::from((content - f32::from(viewport.size.height)).max(0.0));
        let inset = f64::from(self.metrics.spacing2());
        let track = (visible - inset * 2.0).max(0.0);
        let offset = f64::from(
            scrollbar_offset(&heights, self.scroll.logical_scroll_top()).clamp(0.0, maximum as f32),
        );
        let thumb = ThumbGeometry::from_lengths(
            visible + maximum,
            visible,
            offset,
            track,
            MINIMUM_THUMB_LENGTH,
        )?;
        Some(CommandPopoverScrollbarGeometry {
            track_origin: viewport.origin.y + self.metrics.spacing2(),
            track_length: px(track as f32),
            thumb_origin: px(thumb.origin as f32),
            thumb_length: px(thumb.length as f32),
            maximum_offset: px(maximum as f32),
        })
    }

    fn begin_scrollbar_drag(
        &mut self,
        event: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(geometry) = self.scrollbar_geometry() else {
            return;
        };
        let thumb_top = geometry.track_origin + geometry.thumb_origin;
        let thumb_bottom = thumb_top + geometry.thumb_length;
        let grab = if event.position.y >= thumb_top && event.position.y <= thumb_bottom {
            event.position.y - thumb_top
        } else {
            geometry.thumb_length / 2.0
        };
        self.scroll.scrollbar_drag_started();
        self.scrollbar_drag = Some(grab);
        self.drag_scrollbar_to(event.position.y, geometry);
        cx.stop_propagation();
        cx.notify();
    }

    fn drag_scrollbar(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !event.dragging() || self.scrollbar_drag.is_none() {
            return;
        }
        let Some(geometry) = self.scrollbar_geometry() else {
            return;
        };
        self.drag_scrollbar_to(event.position.y, geometry);
        cx.stop_propagation();
        cx.notify();
    }

    fn drag_scrollbar_to(&self, pointer_y: Pixels, geometry: CommandPopoverScrollbarGeometry) {
        let travel = geometry.track_length - geometry.thumb_length;
        if travel <= px(0.0) {
            return;
        }
        let grab = self.scrollbar_drag.unwrap_or(geometry.thumb_length / 2.0);
        let origin = (pointer_y - geometry.track_origin - grab).clamp(px(0.0), travel);
        let offset = geometry.maximum_offset * (origin / travel);
        let layout = CommandPopoverLayout::resolve(self.config.presentation, self.config.density);
        let heights = scrollbar_item_heights(
            layout,
            f32::from(self.metrics.scaled(1.0)),
            self.state.items(),
            self.detail.as_ref().map(|(_, lines)| lines.len()),
        );
        self.scroll
            .scroll_to(scrollbar_list_offset(&heights, f32::from(offset)));
    }

    fn end_scrollbar_drag(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.scrollbar_drag.take().is_some() {
            self.scroll.scrollbar_drag_ended();
            cx.notify();
        }
    }

    fn render_scrollbar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let geometry = self.scrollbar_geometry()?;
        let dragging = self.scrollbar_drag.is_some();
        Some(
            div()
                .id("command-popover-scrollbar")
                .absolute()
                .right(self.metrics.spacing1())
                .top(self.metrics.spacing2())
                .w(self.metrics.scaled(8.0))
                .h(geometry.track_length)
                .cursor_pointer()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(Self::begin_scrollbar_drag),
                )
                .child(
                    div()
                        .absolute()
                        .right(self.metrics.spacing1())
                        .top(geometry.thumb_origin)
                        .w(self.metrics.scaled(if dragging { 5.0 } else { 4.0 }))
                        .h(geometry.thumb_length)
                        .rounded(self.metrics.radius_sm())
                        .bg(self
                            .theme
                            .fg_muted
                            .opacity(if dragging { 0.72 } else { 0.46 })),
                )
                .into_any_element(),
        )
    }
}

#[derive(Clone, Copy)]
struct CommandPopoverScrollbarGeometry {
    track_origin: Pixels,
    track_length: Pixels,
    thumb_origin: Pixels,
    thumb_length: Pixels,
    maximum_offset: Pixels,
}

impl Render for CommandPopover {
    #[allow(
        clippy::too_many_lines,
        reason = "The declarative popup tree keeps its focus and layout behavior together."
    )]
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused
            && (self.detail.is_some()
                || self.focus_handle.is_focused(window)
                || self.config.presentation != CommandPopoverPresentation::Embedded)
        {
            self.focused = true;
            if self.detail.is_some() {
                window.focus(&self.focus_handle);
            } else {
                window.focus(&self.input.focus_handle(cx));
            }
        }
        let viewport = window.viewport_size();
        let layout = CommandPopoverLayout::resolve(self.config.presentation, self.config.density);
        let mut geometry = CommandPopoverMetrics::default().resolve(
            self.config.presentation,
            f32::from(viewport.width),
            f32::from(viewport.height),
        );
        if let Some(width) = self.config.width {
            geometry.width = f32::from(self.metrics.scaled(width)).min(
                (f32::from(viewport.width)
                    - CommandPopoverMetrics::default().viewport_margin * 2.0)
                    .max(0.0),
            );
        }
        if let Some(height) = self.config.height {
            geometry.height = f32::from(self.metrics.scaled(height)).min(
                (f32::from(viewport.height)
                    - geometry.top
                    - CommandPopoverMetrics::default().viewport_margin * 2.0)
                    .max(0.0),
            );
        }
        if let Some(max_height) = self.config.max_height {
            geometry.height = self.fitted_height(layout).min(
                f32::from(self.metrics.scaled(max_height)).min(
                    (f32::from(viewport.height)
                        - geometry.top
                        - CommandPopoverMetrics::default().viewport_margin * 2.0)
                        .max(0.0),
                ),
            );
        }
        let mut panel = div()
            .id(self.config.id.clone())
            .debug_selector(|| self.config.id.to_string())
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::secondary_confirm))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .on_action(cx.listener(Self::tab_pressed))
            .on_action(cx.listener(Self::navigate_back))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_move(cx.listener(Self::drag_scrollbar))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(Self::end_scrollbar_drag),
            )
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(Self::end_scrollbar_drag),
            )
            .occlude()
            .flex()
            .flex_col()
            .when(
                self.config.presentation == CommandPopoverPresentation::Embedded,
                Styled::w_full,
            )
            .when(
                self.config.presentation != CommandPopoverPresentation::Embedded,
                |element| element.w(px(geometry.width)),
            )
            .h(px(geometry.height))
            .bg(
                if self.config.presentation == CommandPopoverPresentation::Embedded {
                    self.theme.surface
                } else {
                    self.theme.bg
                },
            )
            .overflow_hidden();
        if self.config.presentation == CommandPopoverPresentation::Embedded {
            panel = panel
                .rounded(self.metrics.scaled(layout.panel_radius))
                .border_1()
                .border_color(self.theme.border);
        } else {
            panel = panel
                .rounded(self.metrics.scaled(layout.panel_radius))
                .border_1()
                .border_color(self.theme.border)
                .shadow_lg();
        }
        if self.config.tabs.len() > 1 && !layout.inline_tabs {
            panel = panel.child(self.render_tabs(cx));
        }
        panel = if let Some((title, _)) = &self.detail {
            let title = title.clone();
            panel.child(
                div()
                    .id("command-popover-detail-header")
                    .h(self.metrics.scaled(layout.header_height))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(self.metrics.spacing3())
                    .px(self.metrics.spacing5())
                    .border_b_1()
                    .border_color(self.theme.border)
                    .cursor_pointer()
                    .hover(|style| style.bg(self.theme.hover))
                    .on_click(cx.listener(|popover, _, window, cx| {
                        popover.dismiss(&Dismiss, window, cx);
                    }))
                    .child(IconGlyph::new(
                        Icon::ChevronLeft,
                        self.metrics.icon_md(),
                        self.theme.fg_muted,
                    ))
                    .child(title),
            )
        } else {
            let has_inline_tabs = self.config.tabs.len() > 1 && layout.inline_tabs;
            let inline_tabs = has_inline_tabs.then(|| self.render_tab_strip(cx));
            let header_detail = self.header_detail.clone();
            panel.child(
                div()
                    .h(self.metrics.scaled(layout.header_height))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(self.metrics.spacing3())
                    .px(self.metrics.scaled(layout.horizontal_inset))
                    .border_b_1()
                    .border_color(self.theme.border)
                    .when(
                        self.config.presentation != CommandPopoverPresentation::Popover,
                        |element| {
                            element.child(IconGlyph::new(
                                Icon::Search,
                                self.metrics.icon_md(),
                                self.theme.fg_muted,
                            ))
                        },
                    )
                    .child(
                        div()
                            .min_w(px(0.0))
                            .flex_grow()
                            .when(has_inline_tabs, |element| {
                                element.pr(self.metrics.spacing5())
                            })
                            .child(self.input.clone()),
                    )
                    .when_some(header_detail, |element, detail| {
                        element.child(
                            div()
                                .flex_none()
                                .text_size(self.metrics.font_caption())
                                .text_color(self.theme.fg_muted)
                                .child(detail),
                        )
                    })
                    .when_some(inline_tabs, ParentElement::child),
            )
        };
        let list = gpui::list(
            self.scroll.clone(),
            cx.processor(|popover, index: usize, window, cx| {
                if popover.detail.is_some() {
                    popover.render_detail_item(index)
                } else {
                    popover.render_item(index, window, cx)
                }
            }),
        )
        .w_full()
        .when(
            self.config.presentation != CommandPopoverPresentation::Modal || self.detail.is_some(),
            |element| element.pr(self.metrics.spacing5()),
        )
        .flex_grow()
        .min_h(px(0.0))
        .when(self.detail.is_none(), |element| {
            element.py(self.metrics.scaled(layout.list_vertical_inset))
        });
        let list = div()
            .relative()
            .min_h(px(0.0))
            .flex_grow()
            .flex()
            .flex_col()
            .child(list)
            .when_some(self.render_scrollbar(cx), |element, scrollbar| {
                element.child(scrollbar)
            });
        panel = match (self.detail.is_some(), self.state.status()) {
            (true, _) => panel.child(list),
            (false, CommandPopoverStatus::Ready) if self.state.item_count() > 0 => {
                panel.child(list)
            }
            (false, CommandPopoverStatus::Ready) => panel.child(status_message(
                "No matches",
                self.theme.fg_muted,
                self.metrics,
            )),
            (
                false,
                CommandPopoverStatus::Loading(message) | CommandPopoverStatus::Empty(message),
            ) => panel.child(status_message(message, self.theme.fg_muted, self.metrics)),
            (false, CommandPopoverStatus::Error(message)) => {
                panel.child(status_message(message, self.theme.danger, self.metrics))
            }
        };
        if let Some(footer) = self.render_footer(cx) {
            panel = panel.child(footer);
        }
        match self.config.presentation {
            CommandPopoverPresentation::Modal => div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .when(geometry.backdrop, |element| {
                    element.bg(gpui::hsla(0.0, 0.0, 0.0, 0.3))
                })
                .pt(px(geometry.top))
                .child(panel)
                .into_any_element(),
            CommandPopoverPresentation::Popover | CommandPopoverPresentation::Embedded => {
                panel.into_any_element()
            }
        }
    }
}

fn status_message(message: impl Into<SharedString>, color: Hsla, metrics: Metrics) -> AnyElement {
    div()
        .flex_grow()
        .flex()
        .items_center()
        .justify_center()
        .text_size(metrics.font_body())
        .text_color(color)
        .child(message.into())
        .into_any_element()
}

impl std::fmt::Debug for CommandPopover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandPopover")
            .field("config", &self.config)
            .field("state", &self.state)
            .field("detail", &self.detail)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "These geometry cases use exactly representable values."
)]
mod tests {
    use super::*;

    fn rows() -> Vec<CommandPopoverItem> {
        vec![
            CommandPopoverItem::section("Local Branches"),
            CommandPopoverItem::row("main"),
            CommandPopoverItem::row("feature"),
            CommandPopoverItem::section("Remote Branches"),
            CommandPopoverItem::row("origin/main"),
        ]
    }

    #[test]
    fn modal_popover_and_embedded_presentations_have_stable_responsive_geometry() {
        let metrics = CommandPopoverMetrics::default();
        let modal = metrics.resolve(CommandPopoverPresentation::Modal, 1440.0, 900.0);
        assert_eq!(modal.width, 640.0);
        assert_eq!(modal.height, 520.0);
        assert_eq!(modal.top, 48.0);
        assert!(modal.backdrop);

        let compact = metrics.resolve(CommandPopoverPresentation::Modal, 390.0, 844.0);
        assert_eq!(compact.width, 374.0);
        assert_eq!(compact.height, 520.0);
        assert_eq!(compact.top, 16.0);

        let popover = metrics.resolve(CommandPopoverPresentation::Popover, 1440.0, 900.0);
        assert_eq!(popover.width, 640.0);
        assert_eq!(popover.height, 520.0);
        assert!(!popover.backdrop);

        let embedded = metrics.resolve(CommandPopoverPresentation::Embedded, 900.0, 700.0);
        assert_eq!(embedded.width, 900.0);
        assert_eq!(embedded.height, 700.0);
        assert_eq!(embedded.top, 0.0);
        assert!(!embedded.backdrop);
    }

    #[test]
    fn popovers_use_compact_chrome_without_compressing_modal_content() {
        let popover = CommandPopoverLayout::resolve(
            CommandPopoverPresentation::Popover,
            CommandPopoverDensity::Comfortable,
        );
        let modal = CommandPopoverLayout::resolve(
            CommandPopoverPresentation::Modal,
            CommandPopoverDensity::Comfortable,
        );

        assert_eq!(popover.header_height, 42.0);
        assert!(popover.inline_tabs);
        assert_eq!(popover.row_height, 46.0);
        assert_eq!(popover.section_height, 28.0);
        assert_eq!(popover.tab_height, 28.0);
        assert_eq!(popover.panel_radius, 8.0);
        assert!(popover.row_height < modal.row_height);
        assert!(popover.section_height < modal.section_height);
        assert!(popover.panel_radius < modal.panel_radius);
        assert!(!modal.inline_tabs);
        assert_eq!(
            popover.horizontal_inset,
            popover.outer_item_inset + popover.item_inset
        );
        assert_eq!(
            modal.horizontal_inset,
            modal.outer_item_inset + modal.item_inset
        );
    }

    #[test]
    fn compact_dropdown_density_reduces_search_and_row_chrome() {
        let comfortable = CommandPopoverLayout::resolve(
            CommandPopoverPresentation::Popover,
            CommandPopoverDensity::Comfortable,
        );
        let compact = CommandPopoverLayout::resolve(
            CommandPopoverPresentation::Popover,
            CommandPopoverDensity::Compact,
        );

        assert_eq!(compact.header_height, 32.0);
        assert_eq!(compact.row_height, 32.0);
        assert_eq!(compact.section_height, 22.0);
        assert_eq!(compact.list_vertical_inset, 4.0);
        assert_eq!(compact.horizontal_inset, 8.0);
        assert_eq!(
            compact.horizontal_inset,
            compact.outer_item_inset + compact.item_inset
        );
        assert!(compact.header_height < comfortable.header_height);
        assert!(compact.row_height < comfortable.row_height);

        let mut palette_row = CommandPopoverRow::new("theme", "Theme");
        palette_row.swatches = vec![gpui::hsla(0.0, 0.0, 0.0, 1.0); 16];
        assert_eq!(
            compact.item_height(&CommandPopoverItem::Row(palette_row)),
            compact.row_height
        );
    }

    #[test]
    fn dropdown_content_height_fits_short_lists_without_empty_reserved_space() {
        let layout = CommandPopoverLayout::resolve(
            CommandPopoverPresentation::Popover,
            CommandPopoverDensity::Compact,
        );
        let items = vec![
            CommandPopoverItem::row("first"),
            CommandPopoverItem::row("second"),
        ];

        assert_eq!(
            content_height(layout, 1, &items, &CommandPopoverStatus::Ready, None, false,),
            106.0
        );
        assert_eq!(
            content_height(
                layout,
                1,
                &[],
                &CommandPopoverStatus::Empty("No matches".into()),
                None,
                false,
            ),
            82.0
        );
    }

    #[test]
    fn embedded_dropdown_height_counts_sections_rows_insets_and_panel_border() {
        let layout = CommandPopoverLayout::resolve(
            CommandPopoverPresentation::Embedded,
            CommandPopoverDensity::Compact,
        );
        let items = vec![
            CommandPopoverItem::section("Local Branches"),
            CommandPopoverItem::row("main"),
            CommandPopoverItem::row("feature"),
        ];

        assert_eq!(
            content_height(layout, 1, &items, &CommandPopoverStatus::Ready, None, false,),
            128.0
        );
    }

    #[test]
    fn tabs_preserve_query_selection_and_scroll_without_rebuilding_inactive_state() {
        let mut state = CommandPopoverState::new(["branches", "stashes"]);
        state.set_items(rows());
        state.set_query("feat");
        state.select_next();
        assert_eq!(state.selected_row_id(), Some("feature"));

        assert!(state.activate_tab("stashes").is_ok());
        state.set_items(vec![
            CommandPopoverItem::section("Stashes"),
            CommandPopoverItem::row("stash@{0}"),
            CommandPopoverItem::row("stash@{1}"),
        ]);
        state.set_query("wip");
        state.select_last();

        assert!(state.activate_tab("branches").is_ok());
        assert_eq!(state.query(), "feat");
        assert_eq!(state.selected_row_id(), Some("feature"));
        assert_eq!(state.item_count(), 5);

        assert!(state.activate_tab("stashes").is_ok());
        assert_eq!(state.query(), "wip");
        assert_eq!(state.selected_row_id(), Some("stash@{1}"));
    }

    #[test]
    fn keyboard_navigation_skips_sections_and_disabled_rows_and_wraps() {
        let mut state = CommandPopoverState::new(["branches"]);
        state.set_items(vec![
            CommandPopoverItem::section("Local Branches"),
            CommandPopoverItem::row("main"),
            CommandPopoverItem::row("busy").disabled(true),
            CommandPopoverItem::section("Remote Branches"),
            CommandPopoverItem::row("origin/main"),
        ]);

        assert_eq!(state.selected_row_id(), Some("main"));
        state.select_next();
        assert_eq!(state.selected_row_id(), Some("origin/main"));
        state.select_next();
        assert_eq!(state.selected_row_id(), Some("main"));
        state.select_previous();
        assert_eq!(state.selected_row_id(), Some("origin/main"));
    }

    #[test]
    fn status_messages_hide_stale_actions_until_items_are_ready() {
        let mut state = CommandPopoverState::new(["main"]);
        state.set_items(vec![
            CommandPopoverItem::row("old"),
            CommandPopoverItem::row("next"),
        ]);
        for status in [
            CommandPopoverStatus::Loading("Searching".into()),
            CommandPopoverStatus::Empty("No results".into()),
            CommandPopoverStatus::Error("Failed".into()),
        ] {
            state.set_status(status);
            state.select_next();
            state.select_last();
            assert!(state.selected_row_id().is_none());
            assert!(state.confirm().is_none());
            assert!(state.select_row("old").is_err());
            assert!(state.open_inline_action("old", "delete").is_err());
        }
        state.set_status(CommandPopoverStatus::Ready);
        assert_eq!(state.selected_row_id(), Some("old"));
    }

    #[test]
    fn replacing_items_retains_identity_or_selects_the_first_actionable_row() {
        let mut state = CommandPopoverState::new(["branches"]);
        state.set_items(rows());
        state.select_last();
        assert_eq!(state.selected_row_id(), Some("origin/main"));

        state.set_items(vec![
            CommandPopoverItem::section("Local Branches"),
            CommandPopoverItem::row("main"),
            CommandPopoverItem::row("origin/main"),
        ]);
        assert_eq!(state.selected_row_id(), Some("origin/main"));

        state.set_items(vec![
            CommandPopoverItem::section("Local Branches"),
            CommandPopoverItem::row("main"),
        ]);
        assert_eq!(state.selected_row_id(), Some("main"));
    }

    #[test]
    fn escape_closes_nested_actions_before_the_surface_and_confirm_is_identity_based() {
        let mut state = CommandPopoverState::new(["branches"]);
        state.set_items(rows());
        assert!(state.open_inline_action("feature", "delete").is_ok());
        assert_eq!(state.escape(), CommandPopoverEscape::CloseInlineAction);
        assert_eq!(state.escape(), CommandPopoverEscape::Dismiss);

        assert!(state.select_row("feature").is_ok());
        assert_eq!(
            state.confirm(),
            Some(CommandPopoverSelection::new("feature"))
        );
    }

    #[test]
    fn source_uses_one_variable_height_virtual_list_and_has_no_eager_scroll_path() {
        let source = include_str!("command_popover.rs");
        let virtual_list_call = ["gpui::", "list("].concat();
        let uniform_list_call = ["uniform_", "list("].concat();
        let eager_scroll = ["overflow_y_", "scroll"].concat();
        assert_eq!(source.matches(&virtual_list_call).count(), 1);
        assert!(!source.contains(&uniform_list_call));
        assert!(!source.contains(&eager_scroll));
    }

    #[test]
    fn virtual_list_exposes_one_draggable_scrollbar() {
        let source = include_str!("command_popover.rs");
        assert_eq!(source.matches("command-popover-scrollbar\"").count(), 1);
        assert!(source.contains("scrollbar_drag_started"));
        assert!(source.contains("scroll_to(scrollbar_list_offset"));
        assert!(source.contains("scrollbar_drag_ended"));
    }

    #[test]
    fn scrollbar_maps_the_full_unmeasured_logical_list() {
        let layout = CommandPopoverLayout::resolve(
            CommandPopoverPresentation::Popover,
            CommandPopoverDensity::Compact,
        );
        let mut items = vec![CommandPopoverItem::section("Providers")];
        items.extend((0..20).map(|index| CommandPopoverItem::row(index.to_string())));
        let heights = scrollbar_item_heights(layout, 1.0, &items, None);
        let content = heights.iter().sum::<f32>();
        let visible = 160.0;
        let target = content - visible;
        let offset = scrollbar_list_offset(&heights, target);

        assert!(offset.item_ix >= 15);
        assert_eq!(scrollbar_offset(&heights, offset), target);
        assert_eq!(scrollbar_offset(&heights, offset) + visible, content);
    }
}

#[cfg(test)]
mod gpui_regression_tests {
    use super::*;
    use crate::theme::ColorScheme;
    use gpui::{TestAppContext, VisualTestContext};

    struct Host {
        popover: Entity<CommandPopover>,
        events: Vec<CommandPopoverEvent>,
        _subscription: Subscription,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("popover-test-host")
                .size_full()
                .child(self.popover.clone())
        }
    }

    fn open(
        cx: &mut TestAppContext,
        presentation: CommandPopoverPresentation,
        completion_on_tab: bool,
    ) -> (Entity<Host>, &mut VisualTestContext) {
        cx.update(|cx| {
            cx.bind_keys(text_input::key_bindings());
            cx.bind_keys(key_bindings());
        });
        let (host, cx) = cx.add_window_view(|_, cx| {
            let popover = cx.new(|cx| {
                CommandPopover::new(
                    CommandPopoverConfig {
                        id: "tested-popover".into(),
                        presentation,
                        density: CommandPopoverDensity::Compact,
                        tabs: vec![
                            CommandPopoverTab::new("first", "First"),
                            CommandPopoverTab::new("second", "Second"),
                        ],
                        placeholder: "Search".into(),
                        footer_actions: Vec::new(),
                        footer_hints: Vec::new(),
                        width: None,
                        height: Some(300.0),
                        max_height: None,
                        completion_on_tab,
                        confirm_on_click: false,
                    },
                    Theme::from_scheme(&ColorScheme::default()),
                    Metrics::new(1.0),
                    cx,
                )
            });
            let subscription = cx.subscribe(&popover, |host: &mut Host, _, event, _| {
                host.events.push(event.clone());
            });
            Host {
                popover,
                events: Vec::new(),
                _subscription: subscription,
            }
        });
        let popover = host.read_with(cx, |host, _| host.popover.clone());
        cx.update(|window, cx| window.focus(&popover.read(cx).input.focus_handle(cx)));
        cx.run_until_parked();
        (host, cx)
    }

    fn assert_input_focused(popover: &Entity<CommandPopover>, cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            assert!(
                popover.read(cx).input.focus_handle(cx).is_focused(window),
                "input lost focus in {:?}",
                popover.read(cx).config.presentation,
            );
        });
    }

    #[gpui::test]
    fn nested_detail_escape_restores_input_focus(cx: &mut TestAppContext) {
        for presentation in [
            CommandPopoverPresentation::Modal,
            CommandPopoverPresentation::Popover,
            CommandPopoverPresentation::Embedded,
        ] {
            let (host, cx) = open(cx, presentation, false);
            let popover = host.read_with(cx, |host, _| host.popover.clone());
            popover.update(cx, |popover, cx| {
                popover.set_items(vec![CommandPopoverItem::row("first")], cx);
                popover.show_detail("Details", "one\ntwo\nthree", cx);
            });
            cx.run_until_parked();
            cx.update(|window, cx| assert!(popover.read(cx).focus_handle.is_focused(window)));
            cx.simulate_keystrokes("escape");
            assert!(popover.read_with(cx, |popover, _| popover.detail.is_none()));
            assert_eq!(
                popover.read_with(cx, |popover, _| popover.scroll.item_count()),
                1
            );
            assert_input_focused(&popover, cx);
            assert!(
                !host
                    .read_with(cx, |host, _| host.events.clone())
                    .contains(&CommandPopoverEvent::Dismissed)
            );
            cx.simulate_keystrokes("escape");
            assert_eq!(
                host.read_with(cx, |host, _| host.events.clone()),
                vec![CommandPopoverEvent::Dismissed]
            );
        }
    }

    #[gpui::test]
    fn detail_refresh_and_tab_change_keep_the_visible_content_consistent(cx: &mut TestAppContext) {
        for presentation in [
            CommandPopoverPresentation::Modal,
            CommandPopoverPresentation::Popover,
            CommandPopoverPresentation::Embedded,
        ] {
            let (host, cx) = open(cx, presentation, false);
            let popover = host.read_with(cx, |host, _| host.popover.clone());
            popover.update(cx, |popover, cx| {
                popover.show_detail("Details", "one\ntwo\nthree", cx);
            });
            cx.run_until_parked();
            popover.update(cx, |popover, cx| {
                popover.set_items(vec![CommandPopoverItem::row("replacement")], cx);
            });
            cx.run_until_parked();
            assert_eq!(
                popover.read_with(cx, |popover, _| popover.scroll.item_count()),
                3
            );
            cx.simulate_keystrokes("down enter alt-enter tab");
            assert!(host.read_with(cx, |host, _| host.events.clone()).is_empty());
            cx.simulate_keystrokes("ctrl-tab");
            assert_eq!(
                popover.read_with(cx, |popover, _| popover.active_tab().to_owned()),
                "second"
            );
            assert!(popover.read_with(cx, |popover, _| popover.detail.is_none()));
            assert_eq!(
                popover.read_with(cx, |popover, _| popover.scroll.item_count()),
                0
            );
            assert_input_focused(&popover, cx);
            assert!(
                host.read_with(cx, |host, _| host.events.clone())
                    .contains(&CommandPopoverEvent::TabChanged("second".into()))
            );
            cx.simulate_keystrokes("ctrl-shift-tab enter");
            assert!(host.read_with(cx, |host, _| host.events.clone()).contains(
                &CommandPopoverEvent::Confirmed(CommandPopoverSelection::new("replacement"))
            ));
        }
    }

    #[gpui::test]
    fn hidden_status_rows_do_not_receive_keyboard_actions(cx: &mut TestAppContext) {
        let (host, cx) = open(cx, CommandPopoverPresentation::Popover, true);
        let popover = host.read_with(cx, |host, _| host.popover.clone());
        popover.update(cx, |popover, cx| {
            popover.set_items(
                vec![
                    CommandPopoverItem::row("old"),
                    CommandPopoverItem::row("next"),
                ],
                cx,
            );
        });
        for status in [
            CommandPopoverStatus::Loading("Searching".into()),
            CommandPopoverStatus::Error("Failed".into()),
        ] {
            popover.update(cx, |popover, cx| popover.set_status(status, cx));
            cx.run_until_parked();
            cx.simulate_keystrokes("down up enter alt-enter cmd-enter tab shift-tab");
            assert!(host.read_with(cx, |host, _| host.events.clone()).is_empty());
        }
        popover.update(cx, |popover, cx| {
            popover.set_status(CommandPopoverStatus::Empty("No results".into()), cx);
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("down up enter alt-enter cmd-enter tab shift-tab");
        assert_eq!(
            host.read_with(cx, |host, _| host.events.clone()),
            vec![
                CommandPopoverEvent::Submitted { secondary: false },
                CommandPopoverEvent::Submitted { secondary: true },
                CommandPopoverEvent::Submitted { secondary: true },
                CommandPopoverEvent::CompletionRequested,
            ]
        );
        host.update(cx, |host, _| host.events.clear());
        popover.update(cx, |popover, cx| {
            popover.set_status(CommandPopoverStatus::Ready, cx);
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        assert_eq!(
            host.read_with(cx, |host, _| host.events.clone()),
            vec![CommandPopoverEvent::Confirmed(
                CommandPopoverSelection::new("old")
            )]
        );
    }
}
