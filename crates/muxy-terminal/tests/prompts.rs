use muxy_terminal::{Size, Terminal};
use std::error::Error;

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn prompt_rows_survive_history_reads_compression_and_screen_switches() -> TestResult {
    let mut terminal = Terminal::new(Size { cols: 40, rows: 4 }, 1024 * 1024)?;
    for _ in 0..10 {
        terminal
            .feed(b"\x1b]133;A\x07$ \x1b]133;B\x07echo hi\r\n\x1b]133;C\x07hi\r\n\x1b]133;D;0\x07");
    }
    terminal.feed(b"\x1b]133;A\x07$ \x1b]133;B\x07");
    let count = terminal.history_rows()?;
    let screen = terminal.screen()?;
    let prompts = terminal.screen_prompts()?;
    assert_eq!(prompts, [1, 3]);
    terminal.take_changed_rows()?;
    terminal.compress_idle()?;
    let (rows, history_prompts) = terminal.history_with_prompts(3..count)?;
    assert_eq!(rows.len(), count - 3);
    assert_eq!(
        history_prompts,
        (1..count - 3)
            .step_by(2)
            .map(|row| u16::try_from(row).unwrap_or(u16::MAX))
            .collect::<Vec<_>>()
    );
    assert_eq!(terminal.screen()?, screen);
    assert_eq!(terminal.screen_prompts()?, prompts);
    assert!(terminal.take_changed_rows()?.is_empty());
    terminal.feed(b"\x1b[?1049h");
    assert!(terminal.screen_prompts()?.is_empty());
    terminal.feed(b"\x1b[?1049l");
    assert_eq!(terminal.screen_prompts()?, prompts);
    terminal.feed(b"\x1b[2J\x1b[H");
    assert!(terminal.screen_prompts()?.is_empty());
    Ok(())
}

#[test]
fn wrapped_prompt_has_only_one_navigation_stop() -> TestResult {
    let mut terminal = Terminal::new(Size { cols: 8, rows: 8 }, 1024 * 1024)?;
    terminal.feed(b"\x1b]133;A\x07a long prompt> \x1b]133;B\x07echo hi");
    assert_eq!(terminal.screen_prompts()?, [0]);
    terminal.resize(Size { cols: 4, rows: 8 })?;
    assert_eq!(terminal.screen_prompts()?, [0]);
    Ok(())
}
