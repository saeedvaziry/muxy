use super::*;
use crate::views::terminal::pane::TerminalPane;
use gpui::{
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollDelta, ScrollWheelEvent, point,
};

#[gpui::test]
#[ignore = "requires a built muxy-server, vim, nvim, less, tmux and a fresh MUXY_DIR under /tmp/muxy-phase18-"]
fn phase18_mouse_walkthrough(cx: &mut TestAppContext) {
    let result = run(cx);
    assert!(result.is_ok(), "{result:?}");
}

fn pane<'a>(model: &'a AppModel, cx: &'a gpui::App) -> &'a TerminalPane {
    model.grids[&model.active_pane().expect("pane")]
        .view
        .read(cx)
}

fn position(
    view: &Entity<AppModel>,
    cx: &VisualTestContext,
    row: u16,
    column: u16,
) -> gpui::Point<gpui::Pixels> {
    view.read_with(cx, |model, cx| {
        let (bounds, cell) = pane(model, cx).geometry.expect("geometry");
        bounds.origin
            + point(
                cell.width * (f32::from(column) + 0.1),
                cell.height * (f32::from(row) + 0.5),
            )
    })
}

fn drag(
    cx: &mut VisualTestContext,
    view: &Entity<AppModel>,
    start: (u16, u16),
    end: (u16, u16),
    shift: bool,
) {
    let start = position(view, cx, start.0, start.1);
    let end = position(view, cx, end.0, end.1);
    let modifiers = Modifiers {
        shift,
        ..Modifiers::default()
    };
    cx.simulate_event(MouseMoveEvent {
        position: start,
        modifiers,
        ..MouseMoveEvent::default()
    });
    cx.simulate_event(MouseDownEvent {
        position: start,
        button: MouseButton::Left,
        click_count: 1,
        modifiers,
        ..MouseDownEvent::default()
    });
    cx.simulate_event(MouseMoveEvent {
        position: end,
        pressed_button: Some(MouseButton::Left),
        modifiers,
    });
    cx.simulate_event(MouseUpEvent {
        position: end,
        button: MouseButton::Left,
        modifiers,
        ..MouseUpEvent::default()
    });
    cx.run_until_parked();
}

fn wheel(cx: &mut VisualTestContext, view: &Entity<AppModel>, rows: f32) {
    let position = position(view, cx, 3, 5);
    cx.simulate_event(MouseMoveEvent {
        position,
        ..MouseMoveEvent::default()
    });
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Lines(point(0.0, rows)),
        ..ScrollWheelEvent::default()
    });
    cx.run_until_parked();
}

fn prompt(cx: &mut VisualTestContext, view: &Entity<AppModel>) -> Result {
    wait(cx, view, |model, cx| {
        pane(model, cx).input_modes == muxy_protocol::InputModes::default()
            && active_grid(model, cx).is_some_and(|grid| {
                (0..grid.rows.len()).any(|row| grid.row_text(row).trim() == "phase18>")
            })
    })
}

fn run(cx: &mut TestAppContext) -> Result {
    let directory = PathBuf::from(std::env::var("MUXY_DIR")?);
    assert!(
        directory
            .to_string_lossy()
            .starts_with("/tmp/muxy-phase18-")
    );
    assert!(!directory.join("state.json").exists());
    std::fs::create_dir_all(&directory)?;
    let file = directory.join("lines.txt");
    let mut output = std::fs::File::create(&file)?;
    for row in 1..=500 {
        writeln!(output, "mouse row {row:03}")?;
    }
    let boot = Boot::load()?;
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, AppModel::new_tab);
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    wait_live(cx, &view)?;
    shell(cx, "exec /bin/sh");
    shell(cx, "PS1='phase18> '; clear");
    prompt(cx, &view)?;
    vim(cx, &view, &file)?;
    nvim(cx, &view, &file)?;
    less(cx, &view, &file)?;
    shell(cx, "seq 1 500");
    prompt(cx, &view)?;
    wheel(cx, &view, 10.0);
    wait(cx, &view, |model, cx| pane(model, cx).scroll.view.is_some())?;
    cx.dispatch_action(crate::views::workspace::ScrollToBottom);
    cx.run_until_parked();
    report("18.3: plain-prompt wheel scrolls Muxy history and ScrollToBottom restores the prompt")?;
    tmux(cx, &view, &directory)?;
    focus(cx, &view, &directory)?;
    let probe = Client::connect(&directory.join("server.sock"))?;
    for session in probe.list_sessions()? {
        probe.end_session(session.id)?;
    }
    cx.update(|window, _| window.remove_window());
    report(
        "Phase 18 GPUI/live-server walkthrough: PASS (vim, nvim, less, tmux, Shift-copy, focus); htop requires separate manual verification",
    )
}

fn vim(cx: &mut VisualTestContext, view: &Entity<AppModel>, file: &std::path::Path) -> Result {
    shell(
        cx,
        &format!(
            "vim -Nu NONE -n -i NONE -c 'set mouse=a ttymouse=sgr' {}",
            file.display()
        ),
    );
    wait(cx, view, |model, cx| {
        pane(model, cx).input_modes.mouse_tracking
    })?;
    wait_text(cx, view, "mouse row 001")?;
    drag(cx, view, (3, 5), (3, 5), false);
    wait(cx, view, |model, cx| {
        active_grid(model, cx).is_some_and(|grid| grid.cursor.row == 3 && grid.cursor.col == 5)
    })?;
    drag(cx, view, (1, 0), (3, 5), false);
    wait_text(cx, view, "VISUAL")?;
    assert!(view.read_with(cx, |model, cx| pane(model, cx).selection.is_none()));
    cx.simulate_keystrokes("escape");
    drag(cx, view, (0, 0), (0, 5), true);
    cx.simulate_keystrokes("cmd-c");
    assert_eq!(
        cx.read(|cx| cx.read_from_clipboard().and_then(|item| item.text())),
        Some("mouse".into())
    );
    report(
        "18.1 + 18.5: vim click moved the cursor; drag entered Vim Visual mode; Shift-drag + Cmd-C copied mouse from Muxy selection",
    )?;
    cx.simulate_keystrokes("escape");
    shell(cx, ":q!");
    prompt(cx, view)
}

fn nvim(cx: &mut VisualTestContext, view: &Entity<AppModel>, file: &std::path::Path) -> Result {
    shell(
        cx,
        &format!(
            "nvim -u NONE -n -i NONE -c 'set mouse=a' {}",
            file.display()
        ),
    );
    wait(cx, view, |model, cx| {
        pane(model, cx).input_modes.mouse_tracking
    })?;
    wait_text(cx, view, "mouse row 001")?;
    drag(cx, view, (3, 5), (3, 5), false);
    wait(cx, view, |model, cx| {
        active_grid(model, cx).is_some_and(|grid| grid.cursor.row == 3 && grid.cursor.col == 5)
    })?;
    drag(cx, view, (1, 0), (3, 5), false);
    wait_text(cx, view, "VISUAL")?;
    assert!(view.read_with(cx, |model, cx| pane(model, cx).selection.is_none()));
    cx.simulate_keystrokes("escape");
    cx.update(|_, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("before Neovim copy".into()));
    });
    drag(cx, view, (0, 0), (0, 5), true);
    assert!(view.read_with(cx, |model, cx| pane(model, cx).selection.is_some()));
    cx.simulate_keystrokes("cmd-c");
    assert_eq!(
        cx.read(|cx| cx.read_from_clipboard().and_then(|item| item.text())),
        Some("mouse".into())
    );
    wheel(cx, view, -2.0);
    wait(cx, view, |model, cx| {
        active_grid(model, cx).is_some_and(|grid| grid.row_text(0).trim() == "mouse row 007")
    })?;
    assert!(view.read_with(cx, |model, cx| pane(model, cx).scroll.view.is_none()));
    report(
        "Neovim: click moved the cursor; drag entered Visual mode; Shift-drag copied Muxy selection; wheel scrolled six lines",
    )?;
    cx.simulate_keystrokes("escape");
    shell(cx, ":q!");
    prompt(cx, view)
}

fn less(cx: &mut VisualTestContext, view: &Entity<AppModel>, file: &std::path::Path) -> Result {
    shell(cx, &format!("less {}", file.display()));
    wait(cx, view, |model, cx| {
        pane(model, cx).input_modes.alternate_scroll
    })?;
    wait_text(cx, view, "mouse row 001")?;
    wheel(cx, view, -2.0);
    wait(cx, view, |model, cx| {
        active_grid(model, cx).is_some_and(|grid| grid.row_text(0).trim() == "mouse row 007")
    })?;
    assert!(view.read_with(cx, |model, cx| pane(model, cx).scroll.view.is_none()));
    report("18.3: less wheel advanced six lines through alternate-scroll cursor keys")?;
    type_text(cx, "q");
    prompt(cx, view)
}

fn tmux(
    cx: &mut VisualTestContext,
    view: &Entity<AppModel>,
    directory: &std::path::Path,
) -> Result {
    let socket = directory.join("tmux.sock");
    let config = directory.join("tmux.conf");
    std::fs::write(&config, "set -g mouse on\nset -g status off\n")?;
    shell(
        cx,
        &format!(
            "tmux -S {} -f {} new-session -s phase18 /bin/sh",
            socket.display(),
            config.display()
        ),
    );
    let result = (|| {
        wait(cx, view, |model, cx| {
            pane(model, cx).input_modes.mouse_tracking
        })?;
        shell(cx, "seq 1 500; printf 'TMUX_%s\\n' READY");
        wait_text(cx, view, "TMUX_READY")?;
        wheel(cx, view, 3.0);
        wait(cx, view, |_, _| {
            std::process::Command::new("tmux")
                .args(["-S"])
                .arg(&socket)
                .args([
                    "display-message",
                    "-p",
                    "-t",
                    "phase18:0.0",
                    "#{scroll_position}",
                ])
                .output()
                .ok()
                .is_some_and(|output| {
                    String::from_utf8_lossy(&output.stdout)
                        .trim()
                        .parse::<u16>()
                        .is_ok_and(|position| position > 0)
                })
        })?;
        assert!(view.read_with(cx, |model, cx| pane(model, cx).scroll.view.is_none()));
        report("18.4: tmux mouse wheel entered copy mode and scrolled tmux history")
    })();
    let _ = std::process::Command::new("tmux")
        .arg("-S")
        .arg(socket)
        .arg("kill-server")
        .output();
    result?;
    prompt(cx, view)
}

fn focus(
    cx: &mut VisualTestContext,
    view: &Entity<AppModel>,
    directory: &std::path::Path,
) -> Result {
    let output = directory.join("focus.bin");
    shell(
        cx,
        &format!(
            "stty -echo -icanon min 1 time 0; printf '\\033[?1004h'; dd bs=1 count=6 of={} 2>/dev/null; printf '\\033[?1004l'; stty sane",
            output.display()
        ),
    );
    wait(cx, view, |model, cx| {
        pane(model, cx).input_modes.focus_events
    })?;
    cx.deactivate_window();
    cx.run_until_parked();
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    prompt(cx, view)?;
    assert_eq!(std::fs::read(output)?, b"\x1b[O\x1b[I");
    report(
        "18.6: window deactivation and activation delivered exact ESC[O and ESC[I bytes to the PTY",
    )
}
