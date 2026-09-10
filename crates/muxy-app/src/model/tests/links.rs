use super::*;
use gpui::{MouseButton, point};
use muxy_protocol::{ChannelId, InputModes, MetadataEvent};

#[gpui::test]
fn terminal_menu_focuses_the_clicked_split_and_routes_clipboard_actions(cx: &mut TestAppContext) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_terminal_tab(state.home().id).expect("tab");
    let first = state.home().tabs[0].panes[0].id;
    let second = state.split_pane(first, Direction::Right).expect("split");
    for (id, session) in [(first, 71), (second, 72)] {
        state
            .set_pane_session(id, SessionId::new(session))
            .expect("session");
    }
    let (boot, requests) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_resize(size(px(1000.0), px(600.0)));
    view.update(cx, |model, cx| {
        model.connection = ConnectionState::Ready;
        for (id, channel) in [(first, 1), (second, 2)] {
            let mut attachment = attachment();
            attachment.channel = ChannelId(channel);
            model.receive(
                (
                    1,
                    Update::Attached {
                        pane: id,
                        session: model.pane_session(id).expect("session"),
                        attachment,
                        created: false,
                    },
                ),
                cx,
            );
        }
    });
    cx.run_until_parked();
    let terminal = view.read_with(cx, |model, _| model.grids[&first].view.clone());
    terminal.update(cx, |pane, cx| {
        pane.apply(
            &muxy_protocol::ScreenFrame {
                seq: 1,
                reset: true,
                rows: saved_screen().rows,
                cursor: saved_screen().cursor,
                modes: muxy_protocol::Modes::default(),
            },
            cx,
        );
    });
    cx.run_until_parked();
    let position = terminal.read_with(cx, |pane, _| {
        let (bounds, cell) = pane.geometry.expect("geometry");
        bounds.origin + point(cell.width, cell.height / 2.0)
    });
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.simulate_mouse_down(position, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(position, MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        assert_eq!(model.active_pane(), Some(first));
        assert!(matches!(model.overlay, Some(Overlay::Menu(_))));
    });
    let all = cx.debug_bounds("menu-item-2").expect("Select All");
    cx.simulate_click(all.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(terminal.read_with(cx, |pane, _| pane.selection.is_some()));
    assert!(view.read_with(cx, |model, cx| {
        model.grids[&second].view.read(cx).selection.is_none()
    }));
    cx.simulate_keystrokes("cmd-c");
    assert!(cx.read(|cx| {
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .is_some_and(|text| text.contains("final marker"))
    }));
    requests.try_iter().for_each(drop);
    cx.simulate_mouse_down(position, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(position, MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
    let paste = cx.debug_bounds("menu-item-1").expect("Paste");
    cx.simulate_click(paste.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(
        requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::Input(ChannelId(1), _)))
    );
    verify_reporting_menu(&view, &terminal, position, cx);
}

fn verify_reporting_menu(
    view: &Entity<AppModel>,
    terminal: &Entity<TerminalPane>,
    position: gpui::Point<gpui::Pixels>,
    cx: &mut VisualTestContext,
) {
    terminal.update(cx, |pane, cx| {
        pane.metadata(
            MetadataEvent::InputModes(InputModes {
                mouse_tracking: true,
                ..InputModes::default()
            }),
            cx,
        );
    });
    cx.simulate_mouse_down(position, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(position, MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
    assert!(view.read_with(cx, |model, _| model.overlay.is_none()));
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    cx.simulate_mouse_down(position, MouseButton::Right, shift);
    cx.simulate_mouse_up(position, MouseButton::Right, shift);
    cx.run_until_parked();
    assert!(view.read_with(cx, |model, _| matches!(
        model.overlay,
        Some(Overlay::Menu(_))
    )));
}

#[gpui::test]
#[allow(clippy::float_cmp)]
fn prompt_shortcuts_and_command_output_menu_act_on_the_focused_terminal(cx: &mut TestAppContext) {
    let (boot, _) = stub_boot(AppState::bootstrap().expect("state"));
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    let terminal = view.update(cx, |model, cx| {
        model.new_tab(cx);
        model.grids[&model.active_pane().expect("pane")]
            .view
            .clone()
    });
    cx.run_until_parked();
    terminal.update(cx, |pane, cx| {
        let mut attached = attachment();
        let size = pane.viewport().unwrap_or(attached.grid.size);
        attached.grid.resize(size);
        attached.grid.history_fresh = true;
        attached.grid.rows = vec![vec![]; usize::from(size.rows)];
        let row = |index, text: &str| muxy_protocol::Row {
            index,
            runs: vec![muxy_protocol::Run {
                text: text.into(),
                width: u16::try_from(text.len()).expect("width"),
                style: muxy_protocol::Style::default(),
            }],
        };
        attached.grid.history = [
            row(0, "$ echo first"),
            row(1, "first"),
            row(2, "$ echo second"),
            row(3, "second"),
        ]
        .into();
        attached.grid.history_total = 4;
        attached.grid.prompts = [0, 2, 4].into();
        attached.grid.rows[0] = row(0, "$ ").runs;
        attached.grid.cursor = muxy_protocol::Cursor {
            row: 0,
            col: 2,
            visible: true,
        };
        pane.attach(attached, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-up");
    assert_eq!(terminal.read_with(cx, |pane, _| pane.scroll.offset), 2.0);
    cx.simulate_keystrokes("cmd-shift-up");
    assert_eq!(terminal.read_with(cx, |pane, _| pane.scroll.offset), 4.0);
    cx.simulate_keystrokes("cmd-shift-down");
    assert_eq!(terminal.read_with(cx, |pane, _| pane.scroll.offset), 2.0);
    cx.simulate_keystrokes("cmd-down");
    assert!(terminal.read_with(cx, |pane, _| pane.scroll.view.is_none()));
    cx.dispatch_action(crate::views::workspace::SelectCommandOutput);
    cx.simulate_keystrokes("cmd-c");
    assert_eq!(
        cx.read(|cx| cx.read_from_clipboard().and_then(|item| item.text())),
        Some("second".into())
    );
    cx.simulate_keystrokes("cmd-up");
    cx.simulate_keystrokes("cmd-up");
    cx.run_until_parked();
    let position = terminal.read_with(cx, |pane, _| {
        let (bounds, cell) = pane.geometry.expect("geometry");
        bounds.origin + point(cell.width, cell.height * 1.5)
    });
    cx.simulate_mouse_down(position, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(position, MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
    let item = cx
        .debug_bounds("menu-item-3")
        .expect("Select Command Output");
    cx.simulate_click(item.center(), Modifiers::default());
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-c");
    assert_eq!(
        cx.read(|cx| cx.read_from_clipboard().and_then(|item| item.text())),
        Some("first".into())
    );
}

#[gpui::test]
#[ignore = "requires a built muxy-server and isolated MUXY_DIR under /tmp/muxy-phase22-"]
fn shell_integration_live_walkthrough(cx: &mut TestAppContext) {
    run_shell_integration_live_walkthrough(cx).expect("phase 22 walkthrough");
}

fn run_shell_integration_live_walkthrough(cx: &mut TestAppContext) -> Result {
    let directory = PathBuf::from(std::env::var("MUXY_DIR")?);
    assert!(
        directory
            .to_string_lossy()
            .starts_with("/tmp/muxy-phase22-")
    );
    assert!(!directory.join("state.json").exists());
    let boot = Boot::load()?;
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, AppModel::new_tab);
    wait_live(cx, &view)?;
    wait(cx, &view, |model, cx| {
        active_grid(model, cx).is_some_and(|grid| !grid.prompts.is_empty())
    })?;
    let terminal = view.read_with(cx, |model, _| {
        model.grids[&model.active_pane().expect("pane")]
            .view
            .clone()
    });
    shell(cx, "seq 1 3000");
    wait(cx, &view, |model, cx| {
        active_grid(model, cx).is_some_and(|grid| {
            (0..grid.rows.len()).any(|row| grid.row_text(row).trim() == "3000")
                && grid.row_text(usize::from(grid.cursor.row)).trim() == "muxy-phase22>"
        })
    })?;
    cx.simulate_keystrokes("cmd-up");
    wait(cx, &view, |_, cx| terminal.read(cx).scroll.offset > 2900.0)?;
    terminal.read_with(cx, |pane, _| {
        let grid = pane.displayed_grid().expect("grid");
        let first = pane.visible_start(grid);
        assert!(grid.prompts.contains(&first));
        let text: String = grid
            .content_row(first)
            .expect("row")
            .iter()
            .map(|run| run.text.as_str())
            .collect();
        assert!(text.contains("seq 1 3000"), "{text:?}");
    });
    cx.simulate_keystrokes("cmd-down");
    wait(cx, &view, |_, cx| terminal.read(cx).scroll.view.is_none())?;
    for text in ["first-output", "second-output"] {
        shell(cx, &format!("printf '{text}\\n'"));
        wait(cx, &view, |model, cx| {
            active_grid(model, cx).is_some_and(|grid| {
                (0..grid.rows.len()).any(|row| grid.row_text(row).trim() == text)
                    && grid.row_text(usize::from(grid.cursor.row)).trim() == "muxy-phase22>"
            })
        })?;
    }
    cx.run_until_parked();
    let position = terminal.read_with(cx, |pane, _| {
        let grid = pane.displayed_grid().expect("grid");
        let row = (0..grid.rows.len())
            .find(|row| grid.row_text(*row).trim() == "first-output")
            .expect("first output");
        let (bounds, cell) = pane.geometry.expect("geometry");
        bounds.origin
            + point(
                cell.width,
                cell.height * (f32::from(u16::try_from(row).expect("row")) + 0.5),
            )
    });
    cx.simulate_mouse_down(position, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(position, MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
    let item = cx
        .debug_bounds("menu-item-3")
        .expect("command output action");
    cx.simulate_click(item.center(), Modifiers::default());
    wait(cx, &view, |_, cx| terminal.read(cx).selection.is_some())?;
    cx.simulate_keystrokes("cmd-c");
    assert_eq!(
        cx.read(|cx| cx.read_from_clipboard().and_then(|item| item.text())),
        Some("first-output".into())
    );
    let probe = Client::connect(&directory.join("server.sock"))?;
    for session in probe.list_sessions()? {
        probe.end_session(session.id)?;
        probe.discard_session(session.id)?;
    }
    report(
        "Phase 22 GPUI/live-server: zsh injection, 3000-row paginated prompt navigation, return to live output, and exact command-output copy PASS",
    )
}
