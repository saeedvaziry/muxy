use std::ops::Range;

use muxy_client::RunGrid;
use muxy_protocol::Run;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Point {
    pub(crate) row: isize,
    pub(crate) column: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Selection {
    pub(crate) anchor: Point,
    pub(crate) head: Point,
}

impl Selection {
    pub(crate) fn normalized(self) -> Range<Point> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub(crate) fn columns(self, row: isize, grid: &RunGrid) -> Range<u16> {
        let range = self.normalized();
        if range.is_empty() || row < range.start.row || row > range.end.row {
            return 0..0;
        }
        let start = if row == range.start.row {
            range.start.column
        } else {
            0
        };
        let end = if row == range.end.row {
            range.end.column
        } else {
            grid.size.cols
        };
        let mut columns = start..end;
        for (cell, _) in cells(row_runs(grid, row).unwrap_or_default()) {
            if cell.start < end && cell.end > start {
                columns.start = columns.start.min(cell.start);
                columns.end = columns.end.max(cell.end);
            }
        }
        columns
    }

    pub(crate) fn text(self, grid: &RunGrid) -> String {
        if self.normalized().is_empty() {
            return String::new();
        }
        let range = self.normalized();
        (range.start.row..=range.end.row)
            .filter(|row| *row != range.end.row || range.end.column > 0)
            .map(|row| {
                let columns = self.columns(row, grid);
                let mut text = String::new();
                for (cell, value) in cells(row_runs(grid, row).unwrap_or_default()) {
                    if cell.start < columns.end && cell.end > columns.start {
                        text.push_str(value);
                    }
                }
                text.trim_end_matches(' ').to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(crate) fn rows(self, grid: &RunGrid) -> Vec<Option<Vec<Run>>> {
        let range = self.normalized();
        (range.start.row..=range.end.row)
            .filter(|row| !self.columns(*row, grid).is_empty())
            .map(|row| row_runs(grid, row).map(<[Run]>::to_vec))
            .collect()
    }

    pub(crate) fn word(point: Point, grid: &RunGrid) -> Self {
        let cells = cells(row_runs(grid, point.row).unwrap_or_default());
        let Some(index) = cells
            .iter()
            .position(|(range, _)| range.contains(&point.column))
        else {
            return Self {
                anchor: point,
                head: Point {
                    column: point.column.saturating_add(1).min(grid.size.cols),
                    ..point
                },
            };
        };
        let class = word_class(cells[index].1);
        let mut start = index;
        let mut end = index + 1;
        if class != 2 {
            while start > 0 && word_class(cells[start - 1].1) == class {
                start -= 1;
            }
            while end < cells.len() && word_class(cells[end].1) == class {
                end += 1;
            }
        }
        Self {
            anchor: Point {
                column: cells[start].0.start,
                ..point
            },
            head: Point {
                column: cells[end - 1].0.end,
                ..point
            },
        }
    }

    pub(crate) fn row(point: Point, grid: &RunGrid) -> Self {
        Self {
            anchor: Point { column: 0, ..point },
            head: Point {
                column: grid.size.cols,
                ..point
            },
        }
    }
}

fn row_runs(grid: &RunGrid, row: isize) -> Option<&[Run]> {
    grid.content_row(grid.history.len().checked_add_signed(row)?)
}

pub(super) fn cells(runs: &[Run]) -> Vec<(Range<u16>, &str)> {
    let mut result = Vec::new();
    let mut column = 0_u16;
    for run in runs {
        if run.text.is_ascii() && run.text.len() == usize::from(run.width) {
            for index in 0..run.text.len() {
                result.push((column..column.saturating_add(1), &run.text[index..=index]));
                column = column.saturating_add(1);
            }
        } else {
            let end = column.saturating_add(run.width);
            result.push((column..end, run.text.as_str()));
            column = end;
        }
    }
    result
}

fn word_class(text: &str) -> u8 {
    match text.chars().next() {
        Some(character) if character.is_whitespace() => 0,
        Some(character) if character.is_alphanumeric() => 1,
        _ => 2,
    }
}
