use std::error::Error;
use std::fmt::Write;

use muxy_terminal::{Row, Size, Terminal};

type TestResult = Result<(), Box<dyn Error>>;
const SIZE: Size = Size { cols: 20, rows: 24 };

fn numbered(terminal: &mut Terminal, start: usize, end: usize) -> TestResult {
    let mut output = String::new();
    for number in start..=end {
        write!(output, "{number}\r\n")?;
    }
    terminal.feed(output.as_bytes());
    Ok(())
}

fn texts(rows: &[Row]) -> Vec<String> {
    rows.iter()
        .map(|row| {
            row.runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

#[test]
fn five_thousand_lines_have_exact_bounded_history_and_leave_the_screen_unchanged() -> TestResult {
    let mut terminal = Terminal::new(SIZE, 16 * 1024 * 1024)?;
    numbered(&mut terminal, 1, 5000)?;
    let count = terminal.history_rows()?;
    assert_eq!(count, 5001 - usize::from(SIZE.rows));
    let screen = terminal.screen()?;
    let cursor = terminal.cursor()?;
    for range in [0..10, 2499..2509, count - 10..usize::MAX] {
        let rows = terminal.history(range.clone())?;
        assert_eq!(
            texts(&rows),
            (range.start + 1..=range.end.min(count))
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
        );
        assert!(
            rows.iter()
                .enumerate()
                .all(|(index, row)| usize::from(row.index) == index)
        );
    }
    assert!(terminal.history(10..10)?.is_empty());
    assert!(terminal.history(count..usize::MAX)?.is_empty());
    assert_eq!(terminal.screen()?, screen);
    assert_eq!(terminal.cursor()?, cursor);
    assert_eq!(terminal.take_changed_rows()?, screen);
    terminal.history(0..500)?;
    assert!(terminal.take_changed_rows()?.is_empty());
    terminal.feed(b"\x1b[2;1Hchanged");
    terminal.history(0..500)?;
    let changed = terminal.take_changed_rows()?;
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].index, 1);
    Ok(())
}

#[test]
fn generation_survives_append_and_changes_after_resize_clear_and_screen_switch() -> TestResult {
    let mut terminal = Terminal::new(SIZE, 16 * 1024 * 1024)?;
    numbered(&mut terminal, 1, 100)?;
    let generation = terminal.history_generation()?;
    numbered(&mut terminal, 101, 120)?;
    assert_eq!(terminal.history_generation()?, generation);
    terminal.resize(Size { cols: 10, rows: 24 })?;
    let resized = terminal.history_generation()?;
    assert_ne!(resized, generation);
    terminal.feed(b"\x1b[3J");
    let cleared = terminal.history_generation()?;
    assert_ne!(cleared, resized);
    numbered(&mut terminal, 1, 100)?;
    let primary = terminal.history(0..10)?;
    let generation = terminal.history_generation()?;
    terminal.feed(b"\x1b[?1049h");
    assert_ne!(terminal.history_generation()?, generation);
    assert!(terminal.history(0..10)?.is_empty());
    terminal.feed(b"\x1b[?1049l");
    assert_eq!(terminal.history(0..10)?, primary);
    Ok(())
}

#[test]
fn eviction_invalidates_generation_even_when_the_oldest_rows_have_equal_text() -> TestResult {
    let mut terminal = Terminal::new(SIZE, 32 * 1024)?;
    for _ in 0..200 {
        terminal.feed(b"same\r\n");
    }
    let generation = terminal.history_generation()?;
    for _ in 0..10_000 {
        terminal.feed(b"same\r\n");
    }
    assert_ne!(terminal.history_generation()?, generation);
    assert!(terminal.history_rows()? < 10_000);
    Ok(())
}

#[test]
fn clear_and_refill_in_one_write_invalidates_old_boundaries() -> TestResult {
    let mut terminal = Terminal::new(SIZE, 16 * 1024 * 1024)?;
    numbered(&mut terminal, 1, 100)?;
    let generation = terminal.history_generation()?;
    let mut output = String::from("\x1b[3J");
    for number in 1..=200 {
        write!(output, "{number}\r\n")?;
    }
    terminal.feed(output.as_bytes());
    assert_ne!(terminal.history_generation()?, generation);
    Ok(())
}
