use muxy_terminal::{LinkSpan, MAX_LINK_SPANS, MAX_LINK_URI, Size, Terminal};

type Result = std::result::Result<(), Box<dyn std::error::Error>>;

fn osc(uri: &str, text: &str) -> String {
    format!("\x1b]8;;{uri}\x1b\\{text}\x1b]8;;\x1b\\")
}

#[test]
fn links_track_uri_only_changes_clear_resize_and_history_reads() -> Result {
    let mut terminal = Terminal::new(Size { cols: 40, rows: 3 }, 1_000_000)?;
    terminal.feed(osc("https://example.com", "link").as_bytes());
    terminal.take_changed_rows()?;
    assert_eq!(
        terminal.screen_links()[0].spans,
        vec![LinkSpan {
            start: 0,
            end: 4,
            uri: "https://example.com".into()
        }]
    );
    terminal.feed(format!("\r{}", osc("https://other.test", "link")).as_bytes());
    assert_eq!(terminal.take_changed_rows()?.len(), 1);
    assert_eq!(
        terminal.screen_links()[0].spans[0].uri,
        "https://other.test"
    );
    terminal.history(0..10)?;
    assert!(terminal.take_changed_rows()?.is_empty());
    assert_eq!(
        terminal.screen_links()[0].spans[0].uri,
        "https://other.test"
    );
    terminal.resize(Size { cols: 2, rows: 3 })?;
    terminal.take_changed_rows()?;
    assert!(
        terminal
            .screen_links()
            .iter()
            .flat_map(|row| &row.spans)
            .all(|span| span.end <= 2)
    );
    terminal.feed(b"\x1b[2J\x1b[H");
    terminal.take_changed_rows()?;
    assert!(terminal.screen_links().is_empty());
    Ok(())
}

#[test]
fn wide_cells_combining_text_and_adjacent_uris_keep_cell_boundaries() -> Result {
    let mut terminal = Terminal::new(Size { cols: 40, rows: 3 }, 1_000_000)?;
    terminal.feed(
        format!(
            "a{}{}",
            osc("https://wide.test", "界e\u{301}"),
            osc("https://next.test", "next")
        )
        .as_bytes(),
    );
    terminal.take_changed_rows()?;
    assert_eq!(
        terminal.screen_links()[0].spans,
        vec![
            LinkSpan {
                start: 1,
                end: 4,
                uri: "https://wide.test".into()
            },
            LinkSpan {
                start: 4,
                end: 8,
                uri: "https://next.test".into()
            },
        ]
    );
    Ok(())
}

#[test]
fn uri_and_screen_budgets_are_bounded_and_recover_after_eviction() -> Result {
    let mut terminal = Terminal::new(
        Size {
            cols: 2048,
            rows: 2,
        },
        1_000_000,
    )?;
    terminal.feed(
        osc(
            &format!("https://example.com/{}", "x".repeat(MAX_LINK_URI)),
            "ignored",
        )
        .as_bytes(),
    );
    terminal.take_changed_rows()?;
    assert!(terminal.screen_links().is_empty());
    terminal.feed(b"\x1b[2J\x1b[H");
    for i in 0..MAX_LINK_SPANS + 10 {
        terminal.feed(osc(&format!("https://test/{i}"), "x").as_bytes());
    }
    terminal.take_changed_rows()?;
    assert_eq!(
        terminal
            .screen_links()
            .iter()
            .map(|row| row.spans.len())
            .sum::<usize>(),
        MAX_LINK_SPANS
    );
    terminal.feed(b"\x1b[H\x1b[2K\x1b[2;1H");
    terminal.feed(osc("https://recovered.test", "yes").as_bytes());
    terminal.take_changed_rows()?;
    assert_eq!(terminal.screen_links().len(), 1);
    assert_eq!(terminal.screen_links()[0].row, 1);
    assert_eq!(
        terminal.screen_links()[0].spans[0].uri,
        "https://recovered.test"
    );
    Ok(())
}
