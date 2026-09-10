#[path = "../src/views/terminal/selection.rs"]
mod selection;

use muxy_client::RunGrid;
use muxy_protocol::{
    AttachSnapshot, ChannelId, Cursor, HistoryPage, Modes, Row, Run, ServerPath, Size, Style,
};
use selection::{Point, Selection};

fn run(text: &str, width: u16) -> Run {
    Run {
        text: text.into(),
        width,
        style: Style::default(),
    }
}

fn grid() -> RunGrid {
    RunGrid::from_snapshot(&AttachSnapshot {
        prompts: Vec::new(),
        channel: ChannelId(1),
        size: Size { cols: 20, rows: 3 },
        rows: vec![
            Row {
                index: 0,
                runs: vec![run("one, two.three   ", 17)],
            },
            Row {
                index: 1,
                runs: vec![
                    run("A", 1),
                    run("界", 2),
                    run("e\u{301}", 1),
                    run("👩‍💻", 2),
                    run(" Z   ", 5),
                ],
            },
            Row {
                index: 2,
                runs: vec![run("last   ", 7)],
            },
        ],
        cursor: Cursor {
            row: 0,
            col: 0,
            visible: true,
        },
        modes: Modes::default(),
        title: String::new(),
        directory: ServerPath(Vec::new()),
        history: vec![Row {
            index: 0,
            runs: vec![run("older   ", 8)],
        }],
        history_cursor: None,
        history_total: 1,
    })
}

fn selection(a: (isize, u16), b: (isize, u16)) -> Selection {
    Selection {
        anchor: Point {
            row: a.0,
            column: a.1,
        },
        head: Point {
            row: b.0,
            column: b.1,
        },
    }
}

#[test]
fn normalization_and_extraction_match_in_both_directions_across_history() {
    let forward = selection((-1, 0), (0, 8));
    let backward = selection((0, 8), (-1, 0));
    assert_eq!(forward.normalized(), backward.normalized());
    assert_eq!(forward.text(&grid()), "older\none, two");
    assert_eq!(forward.text(&grid()), backward.text(&grid()));
    assert_eq!(selection((0, 0), (0, 0)).text(&grid()), "");
    assert_eq!(selection((0, 0), (1, 0)).text(&grid()), "one, two.three");
}

#[test]
fn wide_cells_and_combining_clusters_are_selected_atomically() {
    let grid = grid();
    for (start, end, expected, columns) in [
        (1, 2, "界", 1..3),
        (2, 3, "界", 1..3),
        (3, 4, "e\u{301}", 3..4),
        (5, 6, "👩‍💻", 4..6),
    ] {
        let selected = selection((1, start), (1, end));
        assert_eq!(selected.text(&grid), expected);
        assert_eq!(selected.columns(1, &grid), columns);
    }
    assert_eq!(
        selection((1, 0), (2, 20)).text(&grid),
        "A界e\u{301}👩‍💻 Z\nlast"
    );
}

#[test]
fn words_stop_at_whitespace_and_punctuation_and_rows_trim_padding() {
    let grid = grid();
    for (column, expected) in [
        (0, "one"),
        (3, ","),
        (4, ""),
        (6, "two"),
        (8, "."),
        (10, "three"),
    ] {
        assert_eq!(
            Selection::word(Point { row: 0, column }, &grid).text(&grid),
            expected
        );
    }
    assert_eq!(
        Selection::word(Point { row: 1, column: 2 }, &grid).text(&grid),
        "A界e\u{301}"
    );
    assert_eq!(
        Selection::row(Point { row: 1, column: 5 }, &grid).text(&grid),
        "A界e\u{301}👩‍💻 Z"
    );
}

#[test]
fn older_pages_do_not_move_selection_and_selected_rows_detect_changes() {
    let mut grid = grid();
    let selected = selection((-1, 0), (0, 3));
    let before = selected.rows(&grid);
    grid.fetch_older(HistoryPage {
        prompts: Vec::new(),
        rows: vec![Row {
            index: 0,
            runs: vec![run("oldest", 6)],
        }],
        next: None,
        total_rows: 2,
        screen: None,
    });
    assert_eq!(selected.text(&grid), "older\none");
    assert_eq!(selected.rows(&grid), before);
    grid.rows[2] = vec![run("elsewhere", 9)];
    assert_eq!(selected.rows(&grid), before);
    grid.rows[0] = vec![run("changed", 7)];
    assert_ne!(selected.rows(&grid), before);
}
