use muxy_terminal::{Size, Terminal, TerminalEvent};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn title_callbacks_drain_the_latest_value_and_support_clearing() -> TestResult {
    let mut terminal = Terminal::new(Size { cols: 80, rows: 24 }, 1024)?;
    assert!(terminal.take_events().is_empty());
    terminal.feed(b"\x1b]0;first\x07\x1b]2;hello\x1b\\");
    assert_eq!(
        terminal.take_events(),
        [TerminalEvent::Title("hello".into())]
    );
    assert!(terminal.take_events().is_empty());
    terminal.feed(b"\x1b]0;\x07");
    assert_eq!(
        terminal.take_events(),
        [TerminalEvent::Title(String::new())]
    );
    Ok(())
}

#[test]
fn split_osc_directory_is_delivered_by_the_engine() -> TestResult {
    let mut terminal = Terminal::new(Size { cols: 80, rows: 24 }, 1024)?;
    terminal.feed(b"\x1b]7;file://localhost/tmp");
    assert!(terminal.take_events().is_empty());
    terminal.feed(b"\x1b\\");
    assert_eq!(
        terminal.take_events(),
        [TerminalEvent::Directory("file://localhost/tmp".into())]
    );
    assert!(terminal.take_events().is_empty());
    Ok(())
}

#[test]
fn bell_is_coalesced_and_not_triggered_by_osc_terminators() -> TestResult {
    let mut terminal = Terminal::new(Size { cols: 80, rows: 24 }, 1024)?;
    terminal.feed(b"\x07\x07");
    assert_eq!(terminal.take_events(), [TerminalEvent::Bell]);
    assert!(terminal.take_events().is_empty());
    terminal.feed(b"\x1b]2;hello\x07");
    assert_eq!(
        terminal.take_events(),
        [TerminalEvent::Title("hello".into())]
    );
    terminal.feed(b"\x07");
    assert_eq!(terminal.take_events(), [TerminalEvent::Bell]);
    Ok(())
}
