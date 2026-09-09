use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::{
    AppContext, Context, Entity, Focusable, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};
use muxy_client::{ClientError, RunGrid};
use muxy_protocol::{ErrorCode, HistoryCursor, SearchMatch, SearchPage};
use muxy_ui::components::IconButton;
use muxy_ui::icon::Icon;
use muxy_ui::text_input::{InputEvent, InputStyle, SEARCH_CONTEXT, TextInput};
use muxy_ui::theme::{Metrics, Theme};

use super::pane::{PaneEvent, PaneState, TerminalPane};

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
const REFRESH: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SearchRequest {
    pub(crate) token: u64,
    pub(crate) query: String,
    pub(crate) ignore_case: bool,
    pub(crate) before: HistoryCursor,
}

#[derive(Debug, Default)]
pub(crate) struct Results {
    pub(crate) query: String,
    pub(crate) ignore_case: bool,
    pub(crate) matches: Vec<SearchMatch>,
    pub(crate) total_rows: u64,
    pub(crate) current: Option<usize>,
    pub(crate) loading_history: bool,
    pub(crate) error: Option<String>,
    pending: Option<SearchRequest>,
    restarted: bool,
}

impl Results {
    fn clear(&mut self) {
        self.matches.clear();
        self.current = None;
        self.pending = None;
        self.loading_history = false;
        self.error = None;
    }

    fn request(&mut self, before: HistoryCursor) -> SearchRequest {
        let request = SearchRequest {
            token: NEXT_REQUEST.fetch_add(1, Ordering::Relaxed),
            query: self.query.clone(),
            ignore_case: self.ignore_case,
            before,
        };
        self.pending = Some(request.clone());
        request
    }

    pub(crate) fn restart(&mut self) -> Option<SearchRequest> {
        self.clear();
        self.restarted = false;
        if self.query.is_empty() {
            return None;
        }
        if self.query.len() > 256 {
            self.error = Some("Query is too long".into());
            return None;
        }
        Some(self.request(HistoryCursor(0)))
    }

    pub(crate) fn receive(
        &mut self,
        request: &SearchRequest,
        result: Result<SearchPage, ClientError>,
    ) -> Option<SearchRequest> {
        if self.pending.as_ref() != Some(request) {
            return None;
        }
        self.pending = None;
        match result {
            Ok(page) => {
                if request.before.0 != 0 && page.total_rows != self.total_rows {
                    return self.stale();
                }
                self.total_rows = page.total_rows;
                self.matches.extend(page.matches);
                if self.current.is_none() && !self.matches.is_empty() {
                    self.current = Some(0);
                }
                page.next.map(|next| self.request(next))
            }
            Err(ClientError::Server(error)) if error.code == ErrorCode::StaleHistoryCursor => {
                self.stale()
            }
            Err(error) => {
                self.clear();
                self.error = Some(error.to_string());
                None
            }
        }
    }

    fn stale(&mut self) -> Option<SearchRequest> {
        self.clear();
        if self.restarted {
            self.error = Some("Output changed; search again".into());
            return None;
        }
        self.restarted = true;
        Some(self.request(HistoryCursor(0)))
    }

    pub(crate) fn step(&mut self, previous: bool) {
        let count = self.matches.len();
        if count == 0 {
            return;
        }
        self.current = Some(match (self.current, previous) {
            (None, _) => 0,
            (Some(0), true) => count - 1,
            (Some(index), true) => index - 1,
            (Some(index), false) => (index + 1) % count,
        });
    }

    pub(crate) fn counter(&self) -> String {
        if let Some(error) = &self.error {
            return error.clone();
        }
        if self.loading_history {
            return "Loading…".into();
        }
        format!(
            "{} of {}{}",
            self.current.map_or(0, |index| index + 1),
            self.matches.len(),
            if self.pending.is_some() { "…" } else { "" }
        )
    }

    pub(crate) fn highlights(&self, grid: &RunGrid, index: usize) -> Vec<(SearchMatch, bool)> {
        if grid.history_total != self.total_rows {
            return Vec::new();
        }
        let Some(row) = self
            .total_rows
            .checked_sub(grid.history.len() as u64)
            .and_then(|first| first.checked_add(index as u64))
        else {
            return Vec::new();
        };
        let start = self.matches.partition_point(|found| found.row > row);
        let cells = super::selection::cells(grid.content_row(index).unwrap_or_default());
        self.matches[start..]
            .iter()
            .take_while(|found| found.row == row)
            .enumerate()
            .filter(|(_, found)| self.matches_text(&cells, **found))
            .map(|(offset, found)| (*found, self.current == Some(start + offset)))
            .collect()
    }

    fn matches_text(&self, cells: &[(std::ops::Range<u16>, &str)], found: SearchMatch) -> bool {
        let start = cells.partition_point(|(columns, _)| columns.end <= found.start);
        let text: String = cells[start..]
            .iter()
            .take_while(|(columns, _)| columns.start < found.end)
            .map(|(_, text)| *text)
            .collect();
        if self.ignore_case {
            let fold = |text: &str| {
                text.chars()
                    .flat_map(char::to_lowercase)
                    .collect::<String>()
            };
            fold(&text).contains(&fold(&self.query))
        } else {
            text.contains(&self.query)
        }
    }
}

pub(crate) struct Find {
    pub(crate) input: Entity<TextInput>,
    pub(crate) results: Results,
    theme: Theme,
    metrics: Metrics,
    reveal_first: bool,
    _subscription: Subscription,
}

impl TerminalPane {
    pub(crate) fn open_find(
        &mut self,
        theme: &Theme,
        metrics: Metrics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.grid.is_none()
            || matches!(self.state, PaneState::Connecting | PaneState::Disconnected)
        {
            return;
        }
        if let Some(find) = &self.find {
            find.input.update(cx, TextInput::select_all_text);
            find.input.focus_handle(cx).focus(window);
            return;
        }
        let input = cx.new(|cx| {
            TextInput::new(InputStyle::compact(theme, &metrics), cx)
                .with_placeholder("Find in terminal…")
                .with_key_context(SEARCH_CONTEXT)
        });
        let subscription = cx.subscribe(&input, |pane, input, event, cx| match event {
            InputEvent::Changed => {
                if let Some(find) = &mut pane.find {
                    input.read(cx).text().clone_into(&mut find.results.query);
                }
                pane.find_query_changed(cx);
            }
            InputEvent::Submitted => pane.step_find(false, cx),
            InputEvent::Cancelled => pane.close_find(cx),
        });
        input.focus_handle(cx).focus(window);
        self.find = Some(Find {
            input,
            results: Results {
                ignore_case: true,
                ..Results::default()
            },
            theme: theme.clone(),
            metrics,
            reveal_first: false,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(crate) fn close_find(&mut self, cx: &mut Context<Self>) {
        self.find = None;
        self.find_refresh = None;
        self.find_focus_pending = true;
        cx.notify();
    }

    pub(crate) fn update_find_theme(&mut self, theme: &Theme, cx: &mut Context<Self>) {
        if let Some(find) = &mut self.find {
            find.theme = theme.clone();
            find.input.update(cx, |input, cx| {
                input.set_style(InputStyle::compact(theme, &find.metrics), cx);
            });
        }
    }

    pub(crate) fn restart_find(&mut self, cx: &mut Context<Self>) {
        if let Some(find) = &mut self.find {
            find.results.clear();
        } else {
            return;
        }
        self.find_refresh = None;
        self.schedule_find(false, cx);
    }

    pub(crate) fn refresh_find(&mut self, cx: &mut Context<Self>) {
        self.schedule_find(true, cx);
    }

    fn schedule_find(&mut self, live_only: bool, cx: &mut Context<Self>) {
        if self.find.is_none() {
            return;
        }
        // Keep the scheduled deadline during a stream of frames.
        if self.find_refresh.is_none() {
            self.find_refresh = Some(cx.spawn(async move |pane, cx| {
                cx.background_executor().timer(REFRESH).await;
                let _ = pane.update(cx, |pane, cx| {
                    pane.find_refresh = None;
                    if matches!(pane.state, PaneState::Connecting | PaneState::Disconnected)
                        || (live_only && pane.scroll.view.is_some())
                    {
                        return;
                    }
                    // Finish the current walk even when a reply takes longer than a frame.
                    if pane
                        .find
                        .as_ref()
                        .is_some_and(|find| find.results.pending.is_some())
                    {
                        pane.schedule_find(live_only, cx);
                        return;
                    }
                    if let Some(request) =
                        pane.find.as_mut().and_then(|find| find.results.restart())
                    {
                        cx.emit(PaneEvent::Search(request));
                    }
                    cx.notify();
                });
            }));
        }
        cx.notify();
    }

    fn find_query_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(find) = &mut self.find {
            find.reveal_first = true;
        }
        let height = usize::from(self.viewport().map_or(24, |size| size.rows));
        if let Some(request) = self.scroll.refresh(height) {
            self.request_history(request, cx);
        }
        self.find_refresh = None;
        if let Some(request) = self.find.as_mut().and_then(|find| find.results.restart()) {
            cx.emit(PaneEvent::Search(request));
        }
        cx.notify();
    }

    pub(crate) fn receive_search(
        &mut self,
        request: &SearchRequest,
        result: Result<SearchPage, ClientError>,
        cx: &mut Context<Self>,
    ) {
        let Some(find) = &mut self.find else {
            return;
        };
        let was_empty = find.results.current.is_none();
        if let Some(next) = find.results.receive(request, result) {
            cx.emit(PaneEvent::Search(next));
        }
        if was_empty && find.results.current.is_some() && find.reveal_first {
            find.reveal_first = false;
            self.reveal_match(cx);
        }
        cx.notify();
    }

    pub(crate) fn step_find(&mut self, previous: bool, cx: &mut Context<Self>) {
        if let Some(find) = &mut self.find {
            find.results.step(previous);
        }
        self.reveal_match(cx);
        cx.notify();
    }

    #[allow(clippy::cast_precision_loss)]
    pub(crate) fn reveal_match(&mut self, cx: &mut Context<Self>) {
        let Some(find) = &self.find else {
            return;
        };
        let Some(found) = find
            .results
            .current
            .and_then(|index| find.results.matches.get(index))
            .copied()
        else {
            return;
        };
        let total = find.results.total_rows;
        let Some(grid) = self.displayed_grid() else {
            return;
        };
        let height = usize::from(self.viewport().unwrap_or(grid.size).rows);
        let start = self.visible_start(grid);
        if grid
            .search_content_row(found.row, total)
            .is_some_and(|index| (start..start + height).contains(&index))
        {
            if let Some(find) = &mut self.find {
                find.results.loading_history = false;
            }
            return;
        }
        let screen_rows = grid.rows.len() as u64;
        let offset = total
            .saturating_add(screen_rows)
            .saturating_sub(found.row)
            .saturating_sub(height as u64 / 2)
            .max(1);
        let Some(live) = &self.grid else {
            return;
        };
        if let Some(request) = self.scroll.move_to(offset as f64, live, height) {
            self.request_history(request, cx);
        }
        let loaded = self
            .displayed_grid()
            .and_then(|grid| grid.search_content_row(found.row, total))
            .is_some();
        if let Some(find) = &mut self.find {
            find.results.loading_history = !loaded;
        }
        self.scroll.revision = self.scroll.revision.wrapping_add(1);
    }
}

pub(crate) fn bar(pane: &TerminalPane, cx: &mut Context<TerminalPane>) -> Option<gpui::AnyElement> {
    let find = pane.find.as_ref()?;
    let theme = &find.theme;
    let metrics = find.metrics;
    let button = |id: &'static str, icon| {
        IconButton::new(
            id,
            icon,
            metrics.icon_sm(),
            metrics.control_medium(),
            theme.fg_muted,
            theme.fg,
        )
    };
    Some(
        div()
            .id("terminal-find")
            .debug_selector(|| "terminal-find".into())
            .flex()
            .flex_none()
            .items_center()
            .gap(metrics.spacing2())
            .p(metrics.spacing2())
            .bg(theme.surface)
            .border_b_1()
            .border_color(theme.border)
            .map(|mut bar| {
                for button in gpui::MouseButton::all() {
                    bar = bar
                        .on_mouse_down(button, |_, _, cx| cx.stop_propagation())
                        .on_mouse_up(button, |_, _, cx| cx.stop_propagation());
                }
                bar
            })
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .px(metrics.spacing2())
                    .child(find.input.clone()),
            )
            .child(
                div()
                    .max_w(metrics.scaled(140.0))
                    .truncate()
                    .text_size(metrics.font_caption())
                    .text_color(theme.fg_muted)
                    .child(find.results.counter()),
            )
            .child(
                button("find-previous", Icon::ChevronLeft)
                    .tooltip("Previous match", theme.surface, theme.fg, theme.border)
                    .on_click(cx.listener(|pane, _, _, cx| pane.step_find(true, cx))),
            )
            .child(
                button("find-next", Icon::ChevronRight)
                    .tooltip("Next match", theme.surface, theme.fg, theme.border)
                    .on_click(cx.listener(|pane, _, _, cx| pane.step_find(false, cx))),
            )
            .child(
                div()
                    .id("find-ignore-case")
                    .debug_selector(|| "find-ignore-case".into())
                    .cursor_pointer()
                    .px(metrics.spacing2())
                    .text_size(metrics.font_body())
                    .text_color(if find.results.ignore_case {
                        theme.accent
                    } else {
                        theme.fg_muted
                    })
                    .on_click(cx.listener(|pane, _, _, cx| {
                        if let Some(find) = &mut pane.find {
                            find.results.ignore_case = !find.results.ignore_case;
                        }
                        pane.find_query_changed(cx);
                    }))
                    .child("Aa"),
            )
            .child(
                button("find-close", Icon::X)
                    .tooltip("Close search", theme.surface, theme.fg, theme.border)
                    .on_click(cx.listener(|pane, _, _, cx| pane.close_find(cx))),
            )
            .into_any_element(),
    )
}
