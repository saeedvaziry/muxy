use super::*;
use crate::views::terminal::find::Results;
use crate::views::terminal::pane::TerminalPane;
use muxy_protocol::{ErrorCode, HistoryCursor, SearchMatch, SearchPage};

fn found(row: u64) -> SearchMatch {
    SearchMatch {
        row,
        start: 0,
        end: 1,
    }
}

fn page(matches: Vec<SearchMatch>, next: Option<HistoryCursor>, total_rows: u64) -> SearchPage {
    SearchPage {
        matches,
        next,
        total_rows,
        scanned_rows: 2000,
    }
}

#[test]
fn search_replies_ignore_old_queries_continue_empty_pages_and_restart_stale_cursors() {
    let mut results = Results::default();
    results.query = "a".into();
    let old = results.restart().expect("request");
    results.query = "b".into();
    let fresh = results.restart().expect("request");
    assert!(
        results
            .receive(&old, Ok(page(vec![found(1)], None, 50)))
            .is_none()
    );
    assert!(results.matches.is_empty());
    let older = results
        .receive(&fresh, Ok(page(vec![], Some(HistoryCursor(42)), 50)))
        .expect("continue empty page");
    let restart = results
        .receive(
            &older,
            Err(muxy_client::ClientError::Server(
                muxy_protocol::ErrorReply {
                    code: ErrorCode::StaleHistoryCursor,
                    message: "changed".into(),
                },
            )),
        )
        .expect("restart");
    assert_eq!(restart.before, HistoryCursor(0));
    assert_ne!(restart.token, older.token);
    results.receive(&restart, Ok(page(vec![found(49), found(2)], None, 50)));
    assert_eq!(results.counter(), "1 of 2");
    results.step(true);
    assert_eq!(results.current, Some(1));
    results.step(false);
    assert_eq!(results.current, Some(0));
}

#[test]
fn search_maps_reply_totals_and_keeps_frozen_highlights_separate_from_live_output() {
    let mut grid = attachment().grid;
    grid.history_total = 100;
    grid.history = (0..10)
        .map(|index| muxy_protocol::Row {
            index,
            runs: vec![muxy_protocol::Run {
                text: "a".into(),
                width: 1,
                style: muxy_protocol::Style::default(),
            }],
        })
        .collect();
    assert_eq!(grid.search_content_row(92, 100), Some(2));
    assert_eq!(grid.search_content_row(89, 100), None);
    assert_eq!(grid.search_content_row(100, 100), Some(10));
    let frozen = grid.clone();
    let mut results = Results::default();
    results.query = "a".into();
    results.matches = vec![found(92)];
    results.total_rows = 100;
    results.current = Some(0);
    assert_eq!(results.highlights(&frozen, 2), vec![(found(92), true)]);
    grid.history_total = 101;
    assert!(results.highlights(&grid, 2).is_empty());
    assert_eq!(results.highlights(&frozen, 2).len(), 1);
    grid.history_total = 100;
    grid.history[2].runs[0].text = "b".into();
    assert!(results.highlights(&grid, 2).is_empty());
}

fn pane<'a>(model: &'a AppModel, cx: &'a gpui::App) -> &'a TerminalPane {
    model.grids[&model.active_pane().expect("pane")]
        .view
        .read(cx)
}

#[gpui::test]
fn find_searches_each_edit_immediately_ignores_case_and_keeps_input_out_of_the_shell(
    cx: &mut TestAppContext,
) {
    let (boot, requests) = stub_boot(AppState::bootstrap().expect("state"));
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.receive((1, Update::Connected(vec![])), cx);
        model.new_tab(cx);
        let pane = model.active_pane().expect("pane");
        model.receive(
            (
                1,
                Update::Attached {
                    pane,
                    session: SessionId::new(42).expect("ID"),
                    attachment: attachment(),
                    created: true,
                },
            ),
            cx,
        );
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-f");
    cx.run_until_parked();
    requests.try_iter().for_each(drop);
    for expected in ["f", "fi", "fin", "fina", "final"] {
        type_text(cx, &expected[expected.len() - 1..]);
        let work: Vec<_> = requests.try_iter().map(|(_, work)| work).collect();
        assert!(!work.iter().any(|work| matches!(work, Work::Input(..))));
        let (pane_id, request) = work
            .into_iter()
            .find_map(|work| match work {
                Work::Search { pane, request, .. } => Some((pane, request)),
                _ => None,
            })
            .expect("typing sends a search without Enter or advancing the clock");
        assert_eq!(request.query, expected);
        assert!(request.ignore_case);
        view.update(cx, |model, cx| {
            model.receive(
                (
                    1,
                    Update::Search {
                        pane: pane_id,
                        request,
                        result: Ok(page(vec![found(0)], None, 0)),
                    },
                ),
                cx,
            );
        });
        assert_eq!(
            view.read_with(cx, |model, cx| pane(model, cx)
                .find
                .as_ref()
                .expect("find")
                .results
                .counter()),
            "1 of 1"
        );
    }
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(view.read_with(cx, |model, cx| pane(model, cx).find.is_none()));
    type_text(cx, "x");
    assert!(
        requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::Input(_, bytes) if bytes == b"x"))
    );
}

#[gpui::test]
#[ignore = "requires a built server and fresh MUXY_DIR under /tmp/muxy-phase21- with a server PID wrapper"]
fn phase21_find_walkthrough(cx: &mut TestAppContext) {
    let result = walkthrough(cx);
    assert!(result.is_ok(), "{result:?}");
}

fn query(cx: &mut VisualTestContext, view: &Entity<AppModel>, text: &str) -> Result {
    cx.simulate_keystrokes("cmd-f");
    cx.run_until_parked();
    cx.simulate_input(text);
    wait(cx, view, |model, cx| {
        pane(model, cx).find.as_ref().is_some_and(|find| {
            find.results.query == text
                && !find.results.matches.is_empty()
                && !find.results.loading_history
                && !find.results.counter().ends_with('…')
        })
    })
}

fn wait(
    cx: &mut VisualTestContext,
    view: &Entity<AppModel>,
    condition: impl Fn(&AppModel, &gpui::App) -> bool,
) -> Result {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        cx.executor().advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        if view.read_with(cx, &condition) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(view
                .read_with(cx, |model, cx| {
                    let pane = pane(model, cx);
                    format!(
                        "Find timed out: {:?}; scroll offset: {}; history: {:?}",
                        pane.find
                            .as_ref()
                            .map(|find| (&find.results.query, find.results.counter())),
                        pane.scroll.offset,
                        pane.displayed_grid()
                            .map(|grid| (grid.history.len(), grid.history_total))
                    )
                })
                .into());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn walkthrough(cx: &mut TestAppContext) -> Result {
    let directory = PathBuf::from(std::env::var("MUXY_DIR")?);
    assert!(
        directory
            .to_string_lossy()
            .starts_with("/tmp/muxy-phase21-")
    );
    assert!(!directory.join("state.json").exists());
    let boot = Boot::load()?;
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, AppModel::new_tab);
    wait_live(cx, &view)?;
    shell(cx, "exec /bin/sh");
    shell(cx, "PS1='find> '; clear; seq 1 5000; echo Seq");
    wait_text(cx, &view, "Seq")?;
    query(cx, &view, "4999")?;
    report("21.1: Cmd-F 4999 found screen output")?;
    query(cx, &view, "1")?;
    cx.simulate_keystrokes("cmd-shift-g");
    wait(cx, &view, |model, cx| {
        let pane = pane(model, cx);
        pane.find.as_ref().is_some_and(|find| {
            find.results.current == Some(find.results.matches.len() - 1)
                && !find.results.loading_history
                && pane
                    .scroll
                    .view
                    .as_ref()
                    .is_some_and(|grid| grid.history_cursor.is_none())
        })
    })?;
    report("21.1: Previous wrapped to the oldest match and paged through 5,000 rows")?;
    cx.simulate_keystrokes("cmd-g");
    wait(cx, &view, |model, cx| {
        pane(model, cx)
            .find
            .as_ref()
            .is_some_and(|find| find.results.current == Some(0) && !find.results.loading_history)
    })?;
    query(cx, &view, "Seq")?;
    let button = cx
        .debug_bounds("find-ignore-case")
        .ok_or("missing case toggle")?;
    cx.simulate_click(button.center(), Modifiers::default());
    assert!(view.read_with(cx, |model, cx| {
        !pane(model, cx)
            .find
            .as_ref()
            .expect("find")
            .results
            .ignore_case
    }));
    query(cx, &view, "Seq")?;
    cx.simulate_click(button.center(), Modifiers::default());
    query(cx, &view, "seq")?;
    assert!(view.read_with(cx, |model, cx| {
        pane(model, cx)
            .find
            .as_ref()
            .expect("find")
            .results
            .ignore_case
    }));
    report("21.2: ignore-case finds Seq with seq")?;
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    shell(cx, "exit");
    wait_exited(cx, &view)?;
    signal_test_server(&directory, "-TERM")?;
    wait(cx, &view, |model, _| {
        model.connection == ConnectionState::Disconnected
    })?;
    reload_model(cx, &view)?;
    wait_exited(cx, &view)?;
    query(cx, &view, "4999")?;
    report("21.4: exited pane searched successfully after server restart")?;
    signal_test_server(&directory, "-TERM")?;
    report("Phase 21 GPUI/live-server walkthrough: PASS")
}

#[gpui::test]
#[ignore = "requires a built server, 256 KiB history budget and fresh MUXY_DIR under /tmp/muxy-phase21-"]
fn phase21_find_eviction_walkthrough(cx: &mut TestAppContext) {
    let result = eviction_walkthrough(cx);
    assert!(result.is_ok(), "{result:?}");
}

fn eviction_walkthrough(cx: &mut TestAppContext) -> Result {
    let directory = PathBuf::from(std::env::var("MUXY_DIR")?);
    assert!(
        directory
            .to_string_lossy()
            .starts_with("/tmp/muxy-phase21-")
    );
    assert!(!directory.join("state.json").exists());
    let boot = Boot::load()?;
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, AppModel::new_tab);
    wait_live(cx, &view)?;
    shell(cx, "exec /bin/sh");
    shell(cx, "PS1='find> '; clear; printf 'pulse-start\\n'");
    wait_text(cx, &view, "pulse-start")?;
    query(cx, &view, "pulse")?;
    let probe = Client::connect(&directory.join("server.sock"))?;
    let session = active_session(&view, cx)?;
    let size = view.read_with(cx, |model, cx| active_grid(model, cx).expect("grid").size);
    let attachment = probe.attach(session, size)?;
    probe.send_input(attachment.channel, b"i=0; while [ \"$i\" -lt 6 ]; do seq 1 5000; printf 'pulse-%s\\n' \"$i\"; i=$((i+1)); sleep .15; done; printf 'pulse-done\\n'\n")?;
    wait(cx, &view, |model, cx| {
        let pane = pane(model, cx);
        let Some(grid) = pane.grid.as_ref() else {
            return false;
        };
        let Some(find) = pane.find.as_ref() else {
            return false;
        };
        grid.rows
            .iter()
            .flatten()
            .any(|run| run.text.contains("pulse-done"))
            && !find.results.counter().ends_with('…')
            && !find.results.matches.is_empty()
            && find.results.total_rows == grid.history_total
    })?;
    view.read_with(cx, |model, cx| {
        let pane = pane(model, cx);
        let grid = pane.displayed_grid().expect("grid");
        assert!(grid.history_total < 30000, "small budget must evict output");
        assert!(pane.scroll.view.is_none());
        let results = &pane.find.as_ref().expect("find").results;
        for found in &results.matches {
            if let Some(index) = grid.search_content_row(found.row, results.total_rows) {
                let text: String = grid
                    .content_row(index)
                    .expect("row")
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect();
                assert_eq!(
                    &text[usize::from(found.start)..usize::from(found.end)],
                    "pulse"
                );
            }
        }
    });
    report(
        "21.3: live search refreshed during 30,000 rows with a 256 KiB budget; visible match coordinates remained correct",
    )?;
    verify_frozen(cx, &view, &probe, attachment.channel)?;
    probe.end_session(session)?;
    signal_test_server(&directory, "-TERM")?;
    report("21.3: scrolled output and its search results remained frozen during further eviction")
}

fn verify_frozen(
    cx: &mut VisualTestContext,
    view: &Entity<AppModel>,
    probe: &Client,
    channel: muxy_protocol::ChannelId,
) -> Result {
    view.update(cx, |model, cx| {
        let id = model.active_pane().expect("pane");
        model.grids[&id]
            .view
            .update(cx, |pane, cx| pane.scroll_rows(3.0, cx));
    });
    wait(cx, view, |model, cx| {
        pane(model, cx)
            .scroll
            .view
            .as_ref()
            .is_some_and(|grid| grid.history_fresh)
    })?;
    let frozen = view.read_with(cx, |model, cx| {
        let pane = pane(model, cx);
        (
            pane.scroll.view.clone(),
            pane.find.as_ref().expect("find").results.matches.clone(),
        )
    });
    probe.send_input(channel, b"seq 1 10000; printf 'FROZEN_DONE\\n'\n")?;
    wait(cx, view, |model, cx| {
        active_grid(model, cx).is_some_and(|grid| {
            grid.rows
                .iter()
                .flatten()
                .any(|run| run.text.contains("FROZEN_DONE"))
        })
    })?;
    view.read_with(cx, |model, cx| {
        let pane = pane(model, cx);
        assert_eq!(pane.scroll.view, frozen.0);
        assert_eq!(pane.find.as_ref().expect("find").results.matches, frozen.1);
    });
    Ok(())
}
