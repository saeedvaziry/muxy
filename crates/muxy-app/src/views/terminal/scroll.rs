use std::sync::atomic::{AtomicU64, Ordering};

use muxy_client::{ClientError, RunGrid};
use muxy_protocol::{ErrorCode, HistoryCursor, HistoryPage};

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HistoryRequest {
    pub(crate) token: u64,
    pub(crate) before: HistoryCursor,
    pub(crate) max_rows: u16,
}

impl HistoryRequest {
    pub(crate) fn recent() -> Self {
        Self {
            token: NEXT_REQUEST.fetch_add(1, Ordering::Relaxed),
            before: HistoryCursor(0),
            max_rows: 200,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Scroll {
    pub(crate) view: Option<RunGrid>,
    pub(crate) offset: f64,
    wanted: f64,
    pub(crate) elastic: f64,
    pub(crate) revision: u64,
    pending: Option<HistoryRequest>,
    restarted: bool,
    prompt: Option<PromptTarget>,
    command_output: Option<(isize, isize)>,
}

impl Scroll {
    pub(crate) fn expects(&self, request: HistoryRequest) -> bool {
        self.pending == Some(request)
    }

    pub(crate) fn refresh(&mut self, height: usize) -> Option<HistoryRequest> {
        let view = self.view.as_mut()?;
        view.clear_history();
        self.pending = None;
        self.prompt = None;
        self.command_output = None;
        self.revision = self.revision.wrapping_add(1);
        self.set_offset(height);
        self.request(height)
    }

    pub(crate) fn bottom(&mut self) {
        self.view = None;
        self.offset = 0.0;
        self.wanted = 0.0;
        self.elastic = 0.0;
        self.revision = self.revision.wrapping_add(1);
        self.pending = None;
        self.restarted = false;
        self.prompt = None;
        self.command_output = None;
    }

    pub(crate) fn reset(&mut self) {
        self.bottom();
    }

    pub(crate) fn resized(&mut self, height: usize) {
        self.prompt = None;
        self.command_output = None;
        self.pending = None;
        self.revision = self.revision.wrapping_add(1);
        self.elastic = 0.0;
        self.set_offset(height);
    }

    pub(crate) fn move_rows(
        &mut self,
        delta: f32,
        live: &RunGrid,
        height: usize,
    ) -> Option<HistoryRequest> {
        self.move_to(self.wanted + f64::from(delta), live, height)
    }

    pub(crate) fn move_to(
        &mut self,
        offset: f64,
        live: &RunGrid,
        height: usize,
    ) -> Option<HistoryRequest> {
        self.prompt = None;
        self.command_output = None;
        if !offset.is_finite() {
            return None;
        }
        if self.pending.is_none() {
            self.restarted = false;
        }
        if offset <= 0.0 {
            self.view = None;
            self.offset = 0.0;
            self.wanted = 0.0;
            self.elastic = offset;
            self.pending = None;
            return None;
        }
        if self.view.is_none() {
            let mut view = live.clone();
            if !view.history_fresh {
                view.clear_history();
            }
            self.view = Some(view);
        }
        self.wanted = offset;
        self.set_offset(height);
        if self.wanted == 0.0 {
            self.view = None;
            self.pending = None;
            return None;
        }
        self.request(height)
    }

    #[allow(clippy::cast_precision_loss)]
    fn set_offset(&mut self, height: usize) {
        if let Some(view) = &self.view {
            let maximum = view
                .history
                .len()
                .saturating_add(view.rows.len())
                .saturating_sub(height) as f64;
            let total =
                (view.history_total as f64 + view.rows.len() as f64 - height as f64).max(0.0);
            self.elastic = if view.history_fresh {
                (self.wanted - total).max(0.0)
            } else {
                0.0
            };
            if view.history_fresh {
                self.wanted = self.wanted.min(total);
            }
            self.offset = self.wanted.min(maximum);
            if view.history_fresh && view.history_cursor.is_none() {
                self.wanted = self.offset;
            }
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn request(&mut self, height: usize) -> Option<HistoryRequest> {
        if self.pending.is_some() {
            return None;
        }
        let view = self.view.as_ref()?;
        let maximum = view
            .history
            .len()
            .saturating_add(view.rows.len())
            .saturating_sub(height);
        let before = if !view.history_fresh {
            HistoryCursor(0)
        } else if self.offset >= maximum as f64 {
            view.history_cursor?
        } else {
            return None;
        };
        let request = HistoryRequest {
            token: NEXT_REQUEST.fetch_add(1, Ordering::Relaxed),
            before,
            max_rows: if before.0 == 0 { 200 } else { 500 },
        };
        self.pending = Some(request);
        Some(request)
    }

    pub(crate) fn receive(
        &mut self,
        request: HistoryRequest,
        result: Result<HistoryPage, ClientError>,
        height: usize,
    ) -> Option<HistoryRequest> {
        if self.pending != Some(request) {
            return None;
        }
        self.pending = None;
        let view = self.view.as_mut()?;
        match result {
            Ok(page) => {
                if request.before.0 == 0 {
                    if self.prompt.is_some() && !continues_snapshot(view, &page) {
                        self.prompt = None;
                        self.command_output = None;
                    }
                    view.replace_history(page);
                    #[allow(clippy::cast_precision_loss)]
                    let maximum = (view.history_total as f64 + view.rows.len() as f64
                        - height as f64)
                        .max(0.0);
                    if self.wanted > maximum {
                        self.wanted = maximum;
                        self.elastic = 0.0;
                        self.revision = self.revision.wrapping_add(1);
                    }
                } else {
                    view.fetch_older(page);
                }
                if self.prompt.is_some() {
                    return self.continue_prompt(height);
                }
                self.set_offset(height);
                if self.wanted == 0.0 {
                    self.bottom();
                    return None;
                }
                self.request(height)
            }
            Err(ClientError::Server(error))
                if error.code == ErrorCode::StaleHistoryCursor
                    && request.before.0 != 0
                    && !self.restarted =>
            {
                self.restarted = true;
                self.prompt = None;
                self.command_output = None;
                view.clear_history();
                self.offset = 0.0;
                self.request(height)
            }
            Err(_) => {
                self.prompt = None;
                if request.before.0 == 0 {
                    self.bottom();
                }
                None
            }
        }
    }

    pub(crate) fn prompt(
        &mut self,
        operation: PromptOperation,
        anchor: usize,
        live: &RunGrid,
        height: usize,
    ) -> Option<HistoryRequest> {
        let grid = self.view.as_ref().unwrap_or(live);
        self.prompt = Some(PromptTarget {
            operation,
            from_bottom: grid.history.len() + grid.rows.len() - anchor,
        });
        self.command_output = None;
        if self.view.is_none() {
            let mut view = live.clone();
            if !view.history_fresh {
                view.clear_history();
            }
            self.view = Some(view);
        }
        self.continue_prompt(height)
    }

    pub(crate) fn cancel_prompt(&mut self) {
        self.prompt = None;
        self.command_output = None;
    }

    pub(crate) fn take_command_output(&mut self) -> Option<(isize, isize)> {
        self.command_output.take()
    }

    #[allow(clippy::cast_precision_loss)]
    fn continue_prompt(&mut self, height: usize) -> Option<HistoryRequest> {
        let target = self.prompt?;
        let view = self.view.as_ref()?;
        if !view.history_fresh {
            return self.prompt_page(HistoryCursor(0));
        }
        let count = view.history.len() + view.rows.len();
        let anchor = count.checked_sub(target.from_bottom);
        let row = anchor.and_then(|anchor| match target.operation {
            PromptOperation::Previous
            | PromptOperation::Select {
                include_prompt: false,
            } => view.prompts.range(..anchor).next_back().copied(),
            PromptOperation::Next => view.prompts.range(anchor + 1..).next().copied(),
            PromptOperation::Select {
                include_prompt: true,
            } => view.prompts.range(..=anchor).next_back().copied(),
        });
        if row.is_none()
            && target.operation != PromptOperation::Next
            && let Some(before) = view.history_cursor
        {
            return self.prompt_page(before);
        }
        self.prompt = None;
        match (target.operation, row) {
            (PromptOperation::Select { .. }, Some(row)) => {
                let end = view
                    .prompts
                    .range(row + 1..)
                    .next()
                    .copied()
                    .unwrap_or(view.history.len() + usize::from(view.cursor.row) + 1);
                if end > row + 1 {
                    let history = isize::try_from(view.history.len()).ok()?;
                    self.command_output = Some((
                        isize::try_from(row + 1).ok()? - history,
                        isize::try_from(end - 1).ok()? - history,
                    ));
                }
            }
            (PromptOperation::Previous | PromptOperation::Next, Some(row)) => {
                self.wanted = count.saturating_sub(height).saturating_sub(row) as f64;
                self.offset = self.wanted;
                self.elastic = 0.0;
                self.revision = self.revision.wrapping_add(1);
                if self.wanted == 0.0 {
                    self.bottom();
                }
            }
            (PromptOperation::Next, None) => self.bottom(),
            (_, None) if self.offset == 0.0 => self.bottom(),
            _ => {}
        }
        None
    }

    fn prompt_page(&mut self, before: HistoryCursor) -> Option<HistoryRequest> {
        if self.pending.is_some() {
            return None;
        }
        let request = HistoryRequest {
            before,
            max_rows: if before.0 == 0 { 200 } else { 500 },
            ..HistoryRequest::recent()
        };
        self.pending = Some(request);
        Some(request)
    }

    pub(crate) fn adopt_recent(&mut self, request: HistoryRequest) {
        self.pending = Some(request);
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub(crate) fn start(&self, grid: &RunGrid, height: usize) -> usize {
        grid.history
            .len()
            .saturating_add(grid.rows.len())
            .saturating_sub(height)
            .saturating_sub(self.offset.ceil() as usize)
    }

    pub(crate) fn pixel_remainder(&self, cell_height: f32) -> f32 {
        #[allow(clippy::cast_possible_truncation)]
        let rows = (self.offset - self.offset.ceil() + self.elastic) as f32;
        rows * cell_height
    }

    pub(crate) fn requested_pixels(&self, cell_height: f32) -> f64 {
        (self.wanted + self.elastic) * f64::from(cell_height)
    }
}

fn continues_snapshot(view: &RunGrid, page: &HistoryPage) -> bool {
    view.history_total == page.total_rows
        && page.screen.as_ref().is_some_and(|screen| {
            screen.size == view.size
                && screen.cursor == view.cursor
                && screen.rows.len() == view.rows.len()
                && screen
                    .rows
                    .iter()
                    .zip(&view.rows)
                    .all(|(row, runs)| row.runs == *runs)
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use muxy_protocol::{
        AttachSnapshot, ChannelId, Cursor, ErrorReply, Modes, Row, Run, SavedScreen, ScreenFrame,
        ServerPath, Size, Style,
    };

    use super::*;

    fn row(index: u16, text: &str) -> Row {
        Row {
            index,
            runs: vec![Run {
                text: text.into(),
                width: 3,
                style: Style::default(),
            }],
        }
    }

    fn page(start: u16, end: u16, next: Option<HistoryCursor>) -> HistoryPage {
        HistoryPage {
            prompts: Vec::new(),
            rows: (start..end)
                .enumerate()
                .map(|(index, value)| {
                    row(u16::try_from(index).unwrap_or(u16::MAX), &value.to_string())
                })
                .collect(),
            next,
            total_rows: 100,
            screen: None,
        }
    }

    fn grid() -> RunGrid {
        RunGrid::from_snapshot(&AttachSnapshot {
            prompts: Vec::new(),
            channel: ChannelId(1),
            size: Size { cols: 10, rows: 5 },
            rows: page(100, 105, None).rows,
            cursor: Cursor {
                row: 4,
                col: 3,
                visible: true,
            },
            modes: Modes::default(),
            title: String::new(),
            directory: ServerPath(b"/tmp".to_vec()),
            history: page(80, 100, None).rows,
            history_cursor: Some(HistoryCursor(42)),
            history_total: 100,
        })
    }

    fn visible(scroll: &Scroll, live: &RunGrid, height: usize) -> Vec<String> {
        let grid = scroll.view.as_ref().unwrap_or(live);
        let start = scroll.start(grid, height);
        (start..start + height)
            .map(|index| {
                grid.content_row(index)
                    .unwrap_or(&[])
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect()
            })
            .collect()
    }

    fn stale() -> ClientError {
        ClientError::Server(ErrorReply {
            code: ErrorCode::StaleHistoryCursor,
            message: "changed".into(),
        })
    }

    #[test]
    fn prompt_navigation_loads_older_pages_then_returns_to_the_next_prompt() {
        let mut live = grid();
        live.prompts = [20, 24].into();
        let mut scroll = Scroll::default();
        scroll.prompt(PromptOperation::Previous, 24, &live, 5);
        assert_eq!(scroll.offset, 0.0);
        // From the live cursor, skipping the current prompt reaches the prior one.
        live.prompts = [5, 24].into();
        scroll.prompt(PromptOperation::Previous, 24, &live, 5);
        assert_eq!(scroll.start(scroll.view.as_ref().unwrap(), 5), 5);
        let request = scroll
            .prompt(PromptOperation::Previous, 5, &live, 5)
            .unwrap();
        assert_eq!(request.before, HistoryCursor(42));
        let mut older = page(60, 80, Some(HistoryCursor(7)));
        older.prompts = vec![10];
        assert!(scroll.receive(request, Ok(older), 5).is_none());
        assert_eq!(scroll.start(scroll.view.as_ref().unwrap(), 5), 10);
        scroll.prompt(PromptOperation::Next, 10, &live, 5);
        assert_eq!(scroll.start(scroll.view.as_ref().unwrap(), 5), 25);
        scroll.prompt(PromptOperation::Next, 25, &live, 5);
        assert!(scroll.view.is_none());
    }

    #[test]
    fn selecting_output_loads_the_prompt_before_the_clicked_row() {
        let live = grid();
        let mut scroll = Scroll::default();
        let request = scroll
            .prompt(
                PromptOperation::Select {
                    include_prompt: true,
                },
                2,
                &live,
                5,
            )
            .unwrap();
        let mut older = page(60, 80, Some(HistoryCursor(7)));
        older.prompts = vec![10];
        scroll.receive(request, Ok(older), 5);
        assert_eq!(scroll.take_command_output(), Some((-29, 4)));
        assert!(scroll.take_command_output().is_none());
    }

    #[test]
    fn prompt_jumps_cancel_on_stale_history_or_return_to_bottom() {
        let live = grid();
        let mut scroll = Scroll::default();
        let request = scroll
            .prompt(PromptOperation::Previous, 24, &live, 5)
            .unwrap();
        scroll.bottom();
        scroll.receive(request, Ok(page(60, 80, None)), 5);
        assert!(scroll.view.is_none());
        let request = scroll
            .prompt(PromptOperation::Previous, 24, &live, 5)
            .unwrap();
        let recent = scroll.receive(request, Err(stale()), 5).unwrap();
        assert!(scroll.prompt.is_none());
        let mut refreshed = page(90, 110, Some(HistoryCursor(7)));
        refreshed.prompts = vec![19];
        assert!(scroll.receive(recent, Ok(refreshed), 5).is_none());
        assert!(scroll.view.is_none());
        assert!(scroll.take_command_output().is_none());
    }

    #[test]
    fn refreshing_a_changed_snapshot_does_not_retarget_a_prompt_action() {
        for operation in [
            PromptOperation::Previous,
            PromptOperation::Select {
                include_prompt: true,
            },
        ] {
            let mut live = grid();
            live.history_fresh = false;
            let mut scroll = Scroll::default();
            let request = scroll.prompt(operation, 22, &live, 5).unwrap();
            let mut recent = page(90, 110, Some(HistoryCursor(7)));
            recent.total_rows = 110;
            recent.prompts = vec![19, 23];
            recent.screen = Some(SavedScreen {
                reason: None,
                size: live.size,
                rows: page(110, 115, None).rows,
                cursor: live.cursor,
            });
            assert!(scroll.receive(request, Ok(recent), 5).is_none());
            assert!(scroll.take_command_output().is_none());
            assert!(scroll.view.is_none());
        }
    }

    #[test]
    fn future_prompt_metadata_does_not_change_a_frozen_selection() {
        let mut live = grid();
        live.prompts = [20, 24].into();
        live.screen_prompts(3, vec![]);
        let mut scroll = Scroll::default();
        let request = scroll.prompt(
            PromptOperation::Select {
                include_prompt: true,
            },
            22,
            &live,
            5,
        );
        assert!(request.is_none());
        assert_eq!(scroll.take_command_output(), Some((1, 3)));
        live.apply(&ScreenFrame {
            seq: 3,
            reset: false,
            rows: vec![],
            cursor: live.cursor,
            modes: live.modes,
        });
        assert!(live.prompts.is_empty());
        assert_eq!(scroll.view.as_ref().unwrap().prompts, [20, 24].into());
    }

    #[test]
    fn fractional_scrolling_and_new_output_preserve_the_frozen_view() {
        let mut live = grid();
        let mut scroll = Scroll::default();
        scroll.move_rows(0.25, &live, 5);
        assert!(scroll.view.is_some());
        assert_eq!(scroll.offset, 0.25);
        assert_eq!(scroll.pixel_remainder(16.0), -12.0);
        scroll.move_rows(0.75, &live, 5);
        assert_eq!(scroll.offset, 1.0);
        assert_eq!(
            visible(&scroll, &live, 5),
            ["99", "100", "101", "102", "103"]
        );
        live.apply(&ScreenFrame {
            seq: 1,
            reset: false,
            rows: vec![row(0, "new")],
            cursor: live.cursor,
            modes: live.modes,
        });
        assert_eq!(
            visible(&scroll, &live, 5),
            ["99", "100", "101", "102", "103"]
        );
        scroll.bottom();
        assert_eq!(visible(&scroll, &live, 5)[0], "new");
    }

    #[test]
    fn prepending_a_page_keeps_the_visible_rows_and_does_not_duplicate_requests() {
        let live = grid();
        let mut scroll = Scroll::default();
        let request = scroll.move_rows(20.0, &live, 5).unwrap();
        let before = visible(&scroll, &live, 5);
        assert!(scroll.move_rows(0.0, &live, 5).is_none());
        assert!(
            scroll
                .receive(request, Ok(page(60, 80, Some(HistoryCursor(7)))), 5)
                .is_none()
        );
        assert_eq!(visible(&scroll, &live, 5), before);
        assert_eq!(scroll.offset, 20.0);
        let view = scroll.view.clone();
        scroll.receive(request, Ok(page(60, 80, Some(HistoryCursor(7)))), 5);
        assert_eq!(scroll.view, view);
    }

    #[test]
    fn entering_saved_scrollback_reuses_the_pending_recent_read() {
        let mut live = grid();
        live.history_fresh = false;
        let mut scroll = Scroll::default();
        let preload = HistoryRequest::recent();
        scroll.move_rows(3.5, &live, 5);
        scroll.adopt_recent(preload);
        assert!(scroll.move_rows(2.0, &live, 5).is_none());
        scroll.receive(preload, Ok(page(80, 100, Some(HistoryCursor(7)))), 5);
        assert_eq!(scroll.offset, 5.5);
        assert!(scroll.pending.is_none());
    }

    #[test]
    fn a_large_gesture_pages_sequentially_and_bottom_ignores_the_outstanding_reply() {
        let live = grid();
        let mut scroll = Scroll::default();
        let first = scroll.move_rows(80.0, &live, 5).unwrap();
        assert!(scroll.move_rows(1.0, &live, 5).is_none());
        let second = scroll
            .receive(first, Ok(page(60, 80, Some(HistoryCursor(7)))), 5)
            .unwrap();
        assert_ne!(first.token, second.token);
        assert_eq!(second.before, HistoryCursor(7));
        scroll.bottom();
        scroll.receive(second, Ok(page(40, 60, Some(HistoryCursor(3)))), 5);
        assert!(scroll.view.is_none());
        assert_eq!(scroll.offset, 0.0);
        let mut replacement = Scroll::default();
        let next = replacement.move_rows(20.0, &live, 5).unwrap();
        assert_ne!(next.token, second.token);
        replacement.receive(second, Ok(page(40, 60, None)), 5);
        assert_eq!(replacement.pending, Some(next));
    }

    #[test]
    fn stale_history_refreshes_once_and_does_not_loop_on_another_stale_reply() {
        let live = grid();
        let mut scroll = Scroll::default();
        let older = scroll.move_rows(100.0, &live, 5).unwrap();
        let refresh = scroll.receive(older, Err(stale()), 5).unwrap();
        assert_eq!(refresh.before, HistoryCursor(0));
        let older = scroll
            .receive(refresh, Ok(page(80, 100, Some(HistoryCursor(10)))), 5)
            .unwrap();
        assert!(scroll.receive(older, Err(stale()), 5).is_none());
        assert!(scroll.pending.is_none());
    }

    #[test]
    fn cleared_history_clamps_the_view_without_leaving_a_stretched_blank_area() {
        let live = grid();
        let mut scroll = Scroll::default();
        let older = scroll.move_rows(100.0, &live, 5).unwrap();
        let refresh = scroll.receive(older, Err(stale()), 5).unwrap();
        let mut page = page(0, 3, None);
        page.total_rows = 3;
        let revision = scroll.revision;
        scroll.receive(refresh, Ok(page), 5);
        assert_eq!(scroll.offset, 3.0);
        assert_eq!(scroll.elastic, 0.0);
        assert_eq!(scroll.requested_pixels(16.0), 48.0);
        assert_ne!(scroll.revision, revision);
    }

    #[test]
    fn elastic_motion_is_kept_separate_from_the_paged_position() {
        let live = grid();
        let mut scroll = Scroll::default();
        scroll.move_to(100.5, &live, 5);
        assert_eq!(scroll.wanted, 100.0);
        assert_eq!(scroll.elastic, 0.5);
        assert_eq!(scroll.requested_pixels(16.0), 1608.0);
    }

    #[test]
    fn fresh_reads_replace_the_screen_and_failures_allow_retry() {
        let mut live = grid();
        live.history_fresh = false;
        let mut scroll = Scroll::default();
        let request = scroll.move_rows(1.0, &live, 5).unwrap();
        assert_eq!(request.before, HistoryCursor(0));
        assert_eq!(scroll.offset, 0.0);
        scroll.receive(request, Err(ClientError::Timeout), 5);
        assert!(scroll.view.is_none());
        let request = scroll.move_rows(1.0, &live, 5).unwrap();
        let mut page = page(180, 200, Some(HistoryCursor(9)));
        page.screen = Some(SavedScreen {
            size: live.size,
            rows: self::page(200, 205, None).rows,
            cursor: live.cursor,
            reason: None,
        });
        scroll.receive(request, Ok(page), 5);
        assert_eq!(
            visible(&scroll, &live, 5),
            ["199", "200", "201", "202", "203"]
        );
    }

    #[test]
    fn empty_and_unsupported_history_do_not_freeze_the_live_screen() {
        let mut live = grid();
        live.history.clear();
        live.history_cursor = None;
        live.history_total = 0;
        let mut scroll = Scroll::default();
        assert!(scroll.move_rows(5.0, &live, 5).is_none());
        assert!(scroll.view.is_none());
        live.history_fresh = false;
        let request = scroll.move_rows(5.0, &live, 5).unwrap();
        scroll.receive(
            request,
            Err(ClientError::Server(ErrorReply {
                code: ErrorCode::HistoryUnavailable,
                message: "unsupported".into(),
            })),
            5,
        );
        assert!(scroll.view.is_none());
        assert!(scroll.move_rows(5.0, &live, 5).is_some());
    }

    #[test]
    fn resizing_saved_content_ignores_an_outstanding_reply() {
        let live = grid();
        let mut scroll = Scroll::default();
        let request = scroll.move_rows(20.0, &live, 5).unwrap();
        let before = scroll.view.clone();
        scroll.resized(3);
        scroll.receive(request, Ok(page(60, 80, None)), 3);
        assert_eq!(scroll.view, before);
        assert!(scroll.pending.is_none());
    }

    #[test]
    fn saved_content_keeps_its_width_and_resize_changes_only_the_visible_rows() {
        let live = grid();
        let saved = RunGrid::from_saved(SavedScreen {
            size: live.size,
            rows: page(100, 105, None).rows,
            cursor: live.cursor,
            reason: None,
        });
        let mut scroll = Scroll::default();
        let request = scroll.move_rows(2.0, &saved, 3).unwrap();
        scroll.receive(request, Ok(page(80, 100, None)), 3);
        let frozen = scroll.view.as_ref().unwrap().clone();
        assert_eq!(visible(&scroll, &saved, 3), ["100", "101", "102"]);
        assert_eq!(visible(&scroll, &saved, 2), ["101", "102"]);
        assert_eq!(scroll.view.as_ref().unwrap(), &frozen);
        assert_eq!(frozen.size.cols, 10);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PromptOperation {
    Previous,
    Next,
    Select { include_prompt: bool },
}

#[derive(Clone, Copy, Debug)]
struct PromptTarget {
    operation: PromptOperation,
    from_bottom: usize,
}
