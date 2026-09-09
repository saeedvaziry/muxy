use std::collections::VecDeque;

use muxy_protocol::{
    AttachSnapshot, Cursor, HistoryCursor, HistoryPage, Modes, Row, Run, SavedScreen, ScreenFrame,
    Size,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunGrid {
    pub size: Size,
    pub rows: Vec<Vec<Run>>,
    pub cursor: Cursor,
    pub modes: Modes,
    pub history: VecDeque<Row>,
    pub history_cursor: Option<HistoryCursor>,
    pub history_total: u64,
    pub history_fresh: bool,
}

impl RunGrid {
    pub fn from_snapshot(snapshot: &AttachSnapshot) -> Self {
        let mut grid = Self {
            size: snapshot.size,
            rows: blank_rows(snapshot.size.rows),
            cursor: snapshot.cursor,
            modes: snapshot.modes,
            history: snapshot.history.clone().into(),
            history_cursor: snapshot.history_cursor,
            history_total: snapshot.history_total,
            history_fresh: true,
        };
        grid.replace_rows(&snapshot.rows);
        grid
    }

    pub fn apply(&mut self, frame: &ScreenFrame) {
        self.history_fresh = false;
        if frame.reset {
            let rows = usize::from(self.size.rows).max(
                frame
                    .rows
                    .iter()
                    .map(|row| usize::from(row.index) + 1)
                    .max()
                    .unwrap_or(0),
            );
            self.rows = vec![Vec::new(); rows];
        }
        self.replace_rows(&frame.rows);
        self.cursor = frame.cursor;
        self.modes = frame.modes;
    }

    pub fn resize(&mut self, size: Size) {
        self.size = size;
        self.rows = blank_rows(size.rows);
        self.history.clear();
        self.history_cursor = None;
        self.history_fresh = false;
    }

    pub fn from_saved(screen: SavedScreen) -> Self {
        let mut cursor = screen.cursor;
        cursor.visible = false;
        Self {
            size: screen.size,
            rows: screen.rows.into_iter().map(|row| row.runs).collect(),
            cursor,
            modes: Modes::default(),
            history: VecDeque::new(),
            history_cursor: None,
            history_total: 0,
            history_fresh: false,
        }
    }

    pub fn replace_history(&mut self, page: HistoryPage) {
        if let Some(screen) = page.screen {
            self.size = screen.size;
            self.rows = screen.rows.into_iter().map(|row| row.runs).collect();
            self.cursor = screen.cursor;
        }
        self.history = page.rows.into();
        self.history_cursor = page.next;
        self.history_total = page.total_rows;
        self.history_fresh = true;
    }

    pub fn fetch_older(&mut self, page: HistoryPage) {
        for row in page.rows.into_iter().rev() {
            self.history.push_front(row);
        }
        self.history_cursor = page.next;
    }

    pub fn content_row(&self, index: usize) -> Option<&[Run]> {
        if index < self.history.len() {
            self.history.get(index).map(|row| row.runs.as_slice())
        } else {
            self.rows.get(index - self.history.len()).map(Vec::as_slice)
        }
    }

    pub fn search_content_row(&self, row: u64, total_rows: u64) -> Option<usize> {
        if self.history_total != total_rows {
            return None;
        }
        let first = total_rows.checked_sub(self.history.len() as u64)?;
        let index = usize::try_from(row.checked_sub(first)?).ok()?;
        self.content_row(index).map(|_| index)
    }

    pub fn row_text(&self, index: usize) -> String {
        self.rows
            .get(index)
            .map(|runs| runs.iter().map(|run| run.text.as_str()).collect())
            .unwrap_or_default()
    }

    fn replace_rows(&mut self, rows: &[Row]) {
        for row in rows {
            if let Some(target) = self.rows.get_mut(usize::from(row.index)) {
                target.clone_from(&row.runs);
            }
        }
    }
}

fn blank_rows(count: u16) -> Vec<Vec<Run>> {
    vec![Vec::new(); usize::from(count)]
}

#[cfg(test)]
mod tests {
    use muxy_protocol::{ChannelId, ServerPath, Style};

    use super::*;

    fn run(text: &str) -> Run {
        Run {
            text: text.to_owned(),
            width: u16::try_from(text.chars().count()).unwrap_or(u16::MAX),
            style: Style::default(),
        }
    }

    fn row(index: u16, text: &str) -> Row {
        Row {
            index,
            runs: if text.is_empty() {
                vec![]
            } else {
                vec![run(text)]
            },
        }
    }

    fn snapshot() -> AttachSnapshot {
        AttachSnapshot {
            channel: ChannelId(1),
            size: Size { cols: 10, rows: 3 },
            rows: vec![row(0, "first"), row(2, "third")],
            cursor: Cursor {
                row: 2,
                col: 5,
                visible: true,
            },
            modes: Modes::default(),
            title: "shell".into(),
            directory: ServerPath(b"/tmp".to_vec()),
            history: vec![],
            history_cursor: None,
            history_total: 0,
        }
    }

    fn frame(seq: u64, reset: bool, rows: Vec<Row>) -> ScreenFrame {
        ScreenFrame {
            seq,
            reset,
            rows,
            cursor: Cursor {
                row: 0,
                col: 1,
                visible: false,
            },
            modes: Modes {
                application_cursor_keys: true,
                bracketed_paste: true,
            },
        }
    }

    #[test]
    fn snapshot_fills_named_rows_and_leaves_the_rest_blank() {
        let grid = RunGrid::from_snapshot(&snapshot());
        assert_eq!(grid.size, Size { cols: 10, rows: 3 });
        assert_eq!(grid.rows.len(), 3);
        assert_eq!(grid.row_text(0), "first");
        assert_eq!(grid.row_text(1), "");
        assert_eq!(grid.row_text(2), "third");
        assert_eq!(grid.row_text(3), "");
        assert_eq!(grid.cursor.row, 2);
    }

    #[test]
    fn partial_frame_replaces_only_its_rows_and_updates_cursor_and_modes() {
        let mut grid = RunGrid::from_snapshot(&snapshot());
        let frame = frame(
            1,
            false,
            vec![row(1, "second"), row(2, ""), row(9, "ignored")],
        );
        grid.apply(&frame);
        assert_eq!(grid.row_text(0), "first");
        assert_eq!(grid.row_text(1), "second");
        assert_eq!(grid.row_text(2), "");
        assert_eq!(grid.rows.len(), 3);
        assert_eq!(grid.cursor, frame.cursor);
        assert_eq!(grid.modes, frame.modes);
    }

    #[test]
    fn reset_frame_blanks_unnamed_rows_and_grows_to_its_rows() {
        let mut grid = RunGrid::from_snapshot(&snapshot());
        grid.apply(&frame(1, true, vec![row(1, "only")]));
        assert_eq!(grid.rows, vec![vec![], vec![run("only")], vec![]]);
        grid.apply(&frame(2, true, vec![row(4, "fifth")]));
        assert_eq!(grid.rows.len(), 5);
        assert_eq!(grid.row_text(1), "");
        assert_eq!(grid.row_text(4), "fifth");
    }

    #[test]
    fn reset_after_a_delayed_larger_frame_returns_to_the_requested_height() {
        let mut grid = RunGrid::from_snapshot(&snapshot());
        grid.resize(Size { cols: 10, rows: 8 });
        grid.resize(Size { cols: 10, rows: 3 });
        grid.apply(&frame(
            1,
            true,
            (0..8).map(|index| row(index, "")).collect(),
        ));
        grid.apply(&frame(
            2,
            true,
            (0..3).map(|index| row(index, "")).collect(),
        ));
        assert_eq!(grid.rows.len(), 3);
        assert_eq!(grid.history_total, 0);
        assert!(grid.rows.iter().all(Vec::is_empty));
    }

    #[test]
    fn resize_blanks_the_grid_at_the_new_size() {
        let mut grid = RunGrid::from_snapshot(&snapshot());
        grid.history = vec![row(0, "older")].into();
        grid.history_cursor = Some(HistoryCursor(2));
        grid.history_total = 100;
        grid.resize(Size { cols: 4, rows: 2 });
        assert_eq!(grid.size, Size { cols: 4, rows: 2 });
        assert_eq!(grid.rows, vec![Vec::<Run>::new(); 2]);
        assert!(grid.history.is_empty());
        assert!(grid.history_cursor.is_none());
        assert!(!grid.history_fresh);
        assert_eq!(grid.history_total, 100);
    }
    #[test]
    fn older_pages_prepend_in_order_without_overwriting_the_screen() {
        let mut snapshot = snapshot();
        snapshot.history = vec![row(0, "five"), row(1, "six")];
        snapshot.history_cursor = Some(HistoryCursor(2));
        snapshot.history_total = 6;
        let mut grid = RunGrid::from_snapshot(&snapshot);
        let screen = grid.rows.clone();
        for (values, next) in [
            (["three", "four"], Some(HistoryCursor(1))),
            (["one", "two"], None),
        ] {
            grid.fetch_older(HistoryPage {
                rows: vec![row(0, values[0]), row(1, values[1])],
                next,
                total_rows: 6,
                screen: None,
            });
        }
        let rows = (0..6)
            .map(|index| {
                grid.content_row(index)
                    .unwrap_or(&[])
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert_eq!(rows, ["one", "two", "three", "four", "five", "six"]);
        assert_eq!(grid.rows, screen);
        assert_eq!(grid.history_cursor, None);
        assert!(grid.history_fresh);
        grid.apply(&frame(1, false, vec![row(1, "new output")]));
        assert!(!grid.history_fresh);
    }
}
