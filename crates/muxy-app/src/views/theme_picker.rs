use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    Subscription,
};
use muxy_ui::command_popover::{
    CommandPopover, CommandPopoverConfig, CommandPopoverDensity, CommandPopoverEvent,
    CommandPopoverItem, CommandPopoverPresentation, CommandPopoverRow, CommandPopoverStatus,
    CommandPopoverTab,
};
use muxy_ui::theme::{Metrics, Theme};

use crate::theme::Entry;

pub(crate) enum ThemeEvent {
    Selected(String),
    Dismiss,
}

pub(crate) struct ThemePicker {
    entries: Vec<Entry>,
    active: String,
    picker: Entity<CommandPopover>,
    metrics: Metrics,
    _subscription: Subscription,
}

impl EventEmitter<ThemeEvent> for ThemePicker {}

impl Focusable for ThemePicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.picker.focus_handle(cx)
    }
}

impl ThemePicker {
    pub(crate) fn new(
        entries: Vec<Entry>,
        active: String,
        theme: Theme,
        metrics: Metrics,
        cx: &mut Context<Self>,
    ) -> Self {
        let picker = cx.new(|cx| {
            CommandPopover::new(
                CommandPopoverConfig {
                    id: "theme-browser".into(),
                    presentation: CommandPopoverPresentation::Popover,
                    density: CommandPopoverDensity::Compact,
                    tabs: vec![CommandPopoverTab::new("themes", "Themes")],
                    placeholder: "Search themes…".into(),
                    footer_actions: Vec::new(),
                    footer_hints: Vec::new(),
                    width: Some(340.0),
                    height: None,
                    max_height: Some(360.0),
                    completion_on_tab: false,
                    confirm_on_click: true,
                },
                theme,
                metrics,
                cx,
            )
        });
        let subscription = cx.subscribe(&picker, |browser: &mut Self, _, event, cx| match event {
            CommandPopoverEvent::QueryChanged { query, .. } => browser.sync_picker(query, cx),
            CommandPopoverEvent::Confirmed(selection)
            | CommandPopoverEvent::SecondaryConfirmed(selection) => {
                if let Some(entry) = selection
                    .id
                    .strip_prefix("theme-")
                    .and_then(|index| index.parse::<usize>().ok())
                    .and_then(|index| browser.entries.get(index))
                {
                    cx.emit(ThemeEvent::Selected(entry.name.clone()));
                }
            }
            CommandPopoverEvent::Dismissed => cx.emit(ThemeEvent::Dismiss),
            _ => {}
        });
        let browser = Self {
            entries,
            active,
            picker,
            metrics,
            _subscription: subscription,
        };
        browser.sync_picker("", cx);
        browser
    }

    pub(crate) fn set_appearance(&mut self, active: String, theme: Theme, cx: &mut Context<Self>) {
        self.active = active;
        self.picker.update(cx, |picker, cx| {
            picker.set_appearance(theme, self.metrics, cx);
        });
        let query = self.picker.read(cx).query().to_owned();
        self.sync_picker(&query, cx);
    }

    fn sync_picker(&self, query: &str, cx: &mut Context<Self>) {
        let query = query.trim().to_lowercase();
        let items: Vec<_> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.name.to_lowercase().contains(&query))
            .map(|(index, entry)| {
                let mut row = CommandPopoverRow::new(format!("theme-{index}"), entry.name.clone());
                row.current = entry.name == self.active;
                row.swatches = (0..16)
                    .filter_map(|slot| entry.scheme.palette_color(slot).map(Into::into))
                    .collect();
                CommandPopoverItem::Row(row)
            })
            .collect();
        let status = if items.is_empty() {
            CommandPopoverStatus::Empty("No themes found".into())
        } else {
            CommandPopoverStatus::Ready
        };
        self.picker.update(cx, |picker, cx| {
            picker.set_items(items, cx);
            picker.set_status(status, cx);
        });
    }
}

impl Render for ThemePicker {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        self.picker.clone()
    }
}
