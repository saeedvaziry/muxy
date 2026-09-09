use muxy_terminal::{Color, Run, Size, Style, Terminal, TerminalError};

type TestResult = Result<(), TerminalError>;

const SIZE: Size = Size { cols: 20, rows: 6 };

fn terminal() -> Result<Terminal, TerminalError> {
    Terminal::new(SIZE, 1 << 20)
}

fn run(text: &str, style: Style) -> Run {
    Run {
        text: text.to_owned(),
        width: u16::try_from(text.chars().count()).unwrap_or(u16::MAX),
        style,
    }
}

#[test]
fn plain_text_lands_on_row_zero_as_one_default_run() -> TestResult {
    let mut terminal = terminal()?;
    terminal.feed(b"hello");

    let screen = terminal.screen()?;

    assert_eq!(screen.len(), usize::from(SIZE.rows));
    assert_eq!(screen[0].index, 0);
    assert_eq!(screen[0].runs, vec![run("hello", Style::default())]);
    assert!(screen[1..].iter().all(|row| row.runs.is_empty()));
    Ok(())
}

#[test]
fn red_text_then_reset_produces_two_runs() -> TestResult {
    let mut terminal = terminal()?;
    terminal.feed(b"\x1b[31mred\x1b[0m plain");

    let screen = terminal.screen()?;

    let red = Style {
        fg: Color::Indexed(1),
        ..Style::default()
    };
    assert_eq!(
        screen[0].runs,
        vec![run("red", red), run(" plain", Style::default())]
    );
    Ok(())
}

#[test]
fn wide_character_counts_two_cells_without_a_spacer_run() -> TestResult {
    let mut terminal = terminal()?;
    terminal.feed("a日b".as_bytes());

    let screen = terminal.screen()?;

    assert_eq!(
        screen[0].runs,
        vec![
            run("a", Style::default()),
            Run {
                text: "日".to_owned(),
                width: 2,
                style: Style::default()
            },
            run("b", Style::default()),
        ]
    );
    assert_eq!(terminal.cursor()?.col, 4);
    Ok(())
}

#[test]
fn unicode_runs_preserve_cells_across_grapheme_mode_changes() -> TestResult {
    for (input, expected) in [
        ("لاx", vec![("ل", 1), ("ا", 1), ("x", 1)]),
        ("👩‍💻x", vec![("👩‍", 2), ("💻", 2), ("x", 1)]),
        ("❤️x", vec![("❤️", 1), ("x", 1)]),
        ("\u{1b}[?2027h👩‍💻x", vec![("👩‍💻", 2), ("x", 1)]),
        ("\u{1b}[?2027h❤️x", vec![("❤️", 2), ("x", 1)]),
        (
            "👩‍💻\u{1b}[?2027h👩‍💻x",
            vec![("👩‍", 2), ("💻", 2), ("👩‍💻", 2), ("x", 1)],
        ),
        (
            "abce\u{301}xyz",
            vec![("abc", 3), ("e\u{301}", 1), ("xyz", 3)],
        ),
    ] {
        let mut terminal = terminal()?;
        terminal.feed(input.as_bytes());
        let screen = terminal.screen()?;
        let actual: Vec<_> = screen[0]
            .runs
            .iter()
            .map(|run| (run.text.as_str(), run.width))
            .collect();
        assert_eq!(actual, expected, "{input:?}");
        assert_eq!(
            terminal.cursor()?.col,
            expected.iter().map(|(_, width)| width).sum::<u16>()
        );
    }
    Ok(())
}

#[test]
fn cursor_follows_newlines_and_absolute_moves() -> TestResult {
    let mut terminal = terminal()?;
    terminal.feed(b"ab\r\n");
    let cursor = terminal.cursor()?;
    assert_eq!((cursor.row, cursor.col), (1, 0));
    assert!(cursor.visible);

    terminal.feed(b"\x1b[5;10H");
    let cursor = terminal.cursor()?;
    assert_eq!((cursor.row, cursor.col), (4, 9));

    terminal.feed(b"\x1b[?25l");
    assert!(!terminal.cursor()?.visible);
    Ok(())
}

#[test]
fn changed_rows_report_only_touched_rows_then_nothing() -> TestResult {
    let mut terminal = terminal()?;
    terminal.feed(b"first");
    let initial = terminal.take_changed_rows()?;
    assert_eq!(initial.len(), usize::from(SIZE.rows));

    terminal.feed(b"\r\nsecond");
    let changed = terminal.take_changed_rows()?;
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].index, 1);
    assert_eq!(changed[0].runs, vec![run("second", Style::default())]);

    assert!(terminal.take_changed_rows()?.is_empty());
    Ok(())
}

#[test]
fn resize_returns_every_row_and_clamps_the_cursor() -> TestResult {
    let mut terminal = terminal()?;
    terminal.feed(b"one\r\ntwo\r\nthree\r\nfour");
    terminal.take_changed_rows()?;
    assert_eq!(terminal.cursor()?.row, 3);

    let smaller = Size { cols: 3, rows: 2 };
    terminal.resize(smaller)?;

    let changed = terminal.take_changed_rows()?;
    assert_eq!(changed.len(), usize::from(smaller.rows));
    let cursor = terminal.cursor()?;
    assert!(cursor.row < smaller.rows, "{cursor:?}");
    assert!(cursor.col < smaller.cols, "{cursor:?}");
    assert!(terminal.take_changed_rows()?.is_empty());
    Ok(())
}

#[test]
fn long_output_keeps_history_and_compresses() -> TestResult {
    let mut terminal = terminal()?;
    for line in 0..20_000_u32 {
        terminal.feed(format!("line {line}\r\n").as_bytes());
    }

    assert!(terminal.history_rows()? > 0);
    terminal.compress_idle()?;
    assert!(terminal.history_rows()? > 0);
    Ok(())
}

#[test]
fn modes_flip_with_their_sequences() -> TestResult {
    let mut terminal = terminal()?;
    let modes = terminal.modes()?;
    assert!(!modes.bracketed_paste);
    assert!(!modes.application_cursor_keys);

    terminal.feed(b"\x1b[?2004h\x1b[?1h");
    let modes = terminal.modes()?;
    assert!(modes.bracketed_paste);
    assert!(modes.application_cursor_keys);

    terminal.feed(b"\x1b[?2004l\x1b[?1l");
    let modes = terminal.modes()?;
    assert!(!modes.bracketed_paste);
    assert!(!modes.application_cursor_keys);
    Ok(())
}

#[test]
fn cursor_position_request_writes_a_report_to_the_pty() -> TestResult {
    let mut terminal = terminal()?;
    assert!(terminal.take_pty_output().is_empty());

    terminal.feed(b"\x1b[2;5H\x1b[6n");

    assert_eq!(terminal.take_pty_output(), b"\x1b[2;5R");
    assert!(terminal.take_pty_output().is_empty());
    Ok(())
}

#[test]
fn styled_blank_cells_keep_their_background() -> TestResult {
    let mut terminal = terminal()?;
    terminal.feed(b"\x1b[44m  \x1b[0m");

    let screen = terminal.screen()?;

    let blue = Style {
        bg: Color::Indexed(4),
        ..Style::default()
    };
    assert_eq!(screen[0].runs, vec![run("  ", blue)]);
    Ok(())
}

#[test]
fn color_queries_follow_selected_defaults() -> TestResult {
    let mut terminal = terminal()?;
    for (foreground, background) in [
        ([0xc9, 0xc2, 0xd9], [0x19, 0x17, 0x1f]),
        ([0x1e, 0x1e, 0x2e], [0xf0, 0xf0, 0xf5]),
        ([0xab, 0xcd, 0xef], [0x12, 0x34, 0x56]),
    ] {
        let cursor = [0xc3, 0x70, 0xd3];
        let ansi = std::array::from_fn(|index| [u8::try_from(index).unwrap_or(0), 0x55, 0xaa]);
        terminal.set_colors(foreground, background, cursor, ansi)?;
        for (query, color) in [("10", foreground), ("11", background), ("12", cursor)] {
            terminal.feed(format!("\x1b]{query};?\x07").as_bytes());
            assert_eq!(terminal.take_pty_output(), color_reply(query, color));
        }
        for (index, color) in ansi.into_iter().enumerate() {
            let query = format!("4;{index}");
            terminal.feed(format!("\x1b]{query};?\x07").as_bytes());
            assert_eq!(terminal.take_pty_output(), color_reply(&query, color));
        }
        terminal.feed(b"\x1b]4;196;?\x07");
        assert_eq!(
            terminal.take_pty_output(),
            color_reply("4;196", [255, 0, 0])
        );
    }
    Ok(())
}

fn color_reply(query: &str, [r, g, b]: [u8; 3]) -> Vec<u8> {
    format!("\x1b]{query};rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}\x07").into_bytes()
}

#[test]
fn theme_defaults_do_not_replace_program_color_overrides() -> TestResult {
    let mut terminal = terminal()?;
    terminal.set_colors([200; 3], [20; 3], [200; 3], [[40; 3]; 16])?;
    terminal.feed(b"\x1b]10;#123456\x07\x1b]11;#123456\x07\x1b]12;#123456\x07\x1b]4;1;#123456\x07");
    terminal.set_colors([210; 3], [30; 3], [220; 3], [[50; 3]; 16])?;
    for query in ["10", "11", "12", "4;1"] {
        terminal.feed(format!("\x1b]{query};?\x07").as_bytes());
        assert_eq!(
            terminal.take_pty_output(),
            color_reply(query, [0x12, 0x34, 0x56])
        );
    }
    terminal.feed(b"\x1b]110\x07\x1b]111\x07\x1b]112\x07\x1b]104;1\x07");
    for (query, color) in [
        ("10", [210; 3]),
        ("11", [30; 3]),
        ("12", [220; 3]),
        ("4;1", [50; 3]),
    ] {
        terminal.feed(format!("\x1b]{query};?\x07").as_bytes());
        assert_eq!(terminal.take_pty_output(), color_reply(query, color));
    }
    Ok(())
}

#[test]
fn theme_defaults_preserve_indexed_colors_and_truecolor_blank_backgrounds() -> TestResult {
    let mut terminal = terminal()?;
    terminal.set_colors([200; 3], [20; 3], [200; 3], [[40; 3]; 16])?;
    terminal.feed(b"plain\x1b[36mcyan\x1b[0;48;2;53;51;58m  \x1b[0m");
    let screen = terminal.screen()?;
    assert_eq!(
        screen[0].runs,
        vec![
            run("plain", Style::default()),
            run(
                "cyan",
                Style {
                    fg: Color::Indexed(6),
                    ..Style::default()
                }
            ),
            run(
                "  ",
                Style {
                    bg: Color::Rgb(53, 51, 58),
                    ..Style::default()
                }
            ),
        ]
    );
    Ok(())
}

#[test]
fn cursor_blinks_by_default_and_honors_application_modes() -> TestResult {
    let mut terminal = terminal()?;
    assert!(terminal.cursor_blinking()?);
    for (sequence, blinking) in [
        ("\x1b[2 q", false),
        ("\x1b[1 q", true),
        ("\x1b[4 q", false),
        ("\x1b[3 q", true),
        ("\x1b[6 q", false),
        ("\x1b[5 q", true),
        ("\x1b[?12l", false),
        ("\x1b[?12h", true),
        ("\x1b[2 q", false),
        ("\x1b[0 q", true),
    ] {
        terminal.feed(sequence.as_bytes());
        assert_eq!(terminal.cursor_blinking()?, blinking, "{sequence:?}");
    }
    terminal.feed(b"\x1b[?25l");
    assert!(!terminal.cursor()?.visible);
    assert!(terminal.cursor_blinking()?);
    terminal.feed(b"\x1b[?25h");
    assert!(terminal.cursor()?.visible);
    Ok(())
}
