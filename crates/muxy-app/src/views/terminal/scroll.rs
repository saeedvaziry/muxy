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
}

impl Scroll {
    pub(crate) fn expects(&self, request: HistoryRequest) -> bool {
        self.pending == Some(request)
    }

    pub(crate) fn refresh(&mut self, height: usize) -> Option<HistoryRequest> {
        let view = self.view.as_mut()?;
        view.history.clear();
        view.history_cursor = None;
        view.history_fresh = false;
        self.pending = None;
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
    }

    pub(crate) fn reset(&mut self) {
        self.bottom();
    }

    pub(crate) fn resized(&mut self, height: usize) {
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
                view.history.clear();
                view.history_cursor = None;
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
                view.history.clear();
                view.history_cursor = None;
                view.history_fresh = false;
                self.offset = 0.0;
                self.request(height)
            }
            Err(_) => {
                if request.before.0 == 0 {
                    self.bottom();
                }
                None
            }
        }
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
