use super::*;
use crate::views::terminal::pane::TerminalPane;
use gpui::{
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollDelta, ScrollWheelEvent, point,
};

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

fn drag(cx: &mut VisualTestContext, view: &Entity<AppModel>, start: (u16, u16), end: (u16, u16)) {
    let start = position(view, cx, start.0, start.1);
    let end = position(view, cx, end.0, end.1);
    cx.simulate_event(MouseMoveEvent {
        position: start,
        ..MouseMoveEvent::default()
    });
    cx.simulate_event(MouseDownEvent {
        position: start,
        button: MouseButton::Left,
        click_count: 1,
        ..MouseDownEvent::default()
    });
    cx.simulate_event(MouseMoveEvent {
        position: end,
        pressed_button: Some(MouseButton::Left),
        ..MouseMoveEvent::default()
    });
    cx.simulate_event(MouseUpEvent {
        position: end,
        button: MouseButton::Left,
        ..MouseUpEvent::default()
    });
    cx.run_until_parked();
}

fn click(cx: &mut VisualTestContext, view: &Entity<AppModel>, row: u16, column: u16, count: usize) {
    let position = position(view, cx, row, column);
    cx.simulate_event(MouseMoveEvent {
        position,
        ..MouseMoveEvent::default()
    });
    cx.simulate_event(MouseDownEvent {
        position,
        button: MouseButton::Left,
        click_count: count,
        ..MouseDownEvent::default()
    });
    cx.simulate_event(MouseUpEvent {
        position,
        button: MouseButton::Left,
        ..MouseUpEvent::default()
    });
    cx.run_until_parked();
}

fn copied(cx: &mut VisualTestContext) -> String {
    cx.simulate_keystrokes("cmd-c");
    cx.read(|cx| {
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default()
    })
}

fn find_row(
    view: &Entity<AppModel>,
    cx: &VisualTestContext,
    predicate: impl Fn(&str) -> bool,
) -> u16 {
    view.read_with(cx, |model, cx| {
        let grid = active_grid(model, cx).expect("grid");
        u16::try_from(
            (0..grid.rows.len())
                .find(|index| predicate(grid.row_text(*index).trim_end()))
                .expect("matching row"),
        )
        .expect("row")
    })
}

fn line(view: &Entity<AppModel>, cx: &VisualTestContext, row: u16) -> String {
    view.read_with(cx, |model, cx| {
        active_grid(model, cx)
            .expect("grid")
            .row_text(usize::from(row))
            .trim_end()
            .to_owned()
    })
}

#[gpui::test]
#[ignore = "requires built muxy-server and fresh MUXY_DIR under /tmp/muxy-phase16- with server PID wrapper"]
fn phase16_clipboard_walkthrough(cx: &mut TestAppContext) {
    let result = run(cx);
    assert!(result.is_ok(), "{result:?}");
}

fn run(cx: &mut TestAppContext) -> Result {
    let directory = PathBuf::from(std::env::var("MUXY_DIR")?);
    assert!(
        directory
            .to_string_lossy()
            .starts_with("/tmp/muxy-phase16-")
    );
    assert!(!directory.join("state.json").exists());
    std::fs::create_dir_all(directory.join("listing"))?;
    for name in ["copy-alpha", "copy-beta"] {
        std::fs::write(directory.join("listing").join(name), "clipboard fixture\n")?;
    }
    let boot = Boot::load()?;
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, AppModel::new_tab);
    wait_live(cx, &view)?;
    shell(cx, "exec /bin/zsh -f");
    wait(cx, &view, |model, cx| {
        active_grid(model, cx).is_some_and(|grid| grid.modes.bracketed_paste)
    })?;
    shell(cx, "PROMPT='phase16> '; RPROMPT=''; clear");
    wait_text(cx, &view, "phase16>")?;
    shell(cx, &format!("ls -la {}/listing", directory.display()));
    wait_text(cx, &view, "copy-beta")?;
    let first = find_row(&view, cx, |row| row.ends_with("copy-alpha"));
    let second = find_row(&view, cx, |row| row.ends_with("copy-beta"));
    let a = line(&view, cx, first);
    let b = line(&view, cx, second);
    drag(cx, &view, (first, 0), (second, u16::try_from(b.len())?));
    let copy = copied(cx);
    assert_eq!(copy, format!("{a}\n{b}"));
    report(&format!(
        "16.1: drag + Cmd-C from ls -la, no trailing spaces:\n{copy}"
    ))?;
    let column = u16::try_from(a.find("alpha").ok_or("alpha missing")? + 1)?;
    click(cx, &view, first, column, 2);
    assert_eq!(copied(cx), "alpha");
    click(cx, &view, second, 5, 3);
    assert_eq!(copied(cx), b);
    report("16.2: double click -> alpha; triple click -> entire copy-beta row")?;
    verify_history(cx, &view)?;
    verify_shell_paste(cx, &view, &directory)?;
    verify_vim(cx, &view, &directory)?;
    shell(cx, "exit");
    wait_exited(cx, &view)?;
    wait(cx, &view, |model, cx| {
        active_grid(model, cx).is_some_and(|grid| grid.history_fresh)
    })?;
    let before = screen(&view, cx);
    cx.simulate_keystrokes("cmd-v");
    cx.run_until_parked();
    assert_eq!(before, screen(&view, cx));
    verify_history(cx, &view)?;
    report(
        "16.extra: retained exited pane supports history selection and Cmd-C; Cmd-V leaves its screen unchanged",
    )?;
    view.update(cx, AppModel::disconnect);
    signal_test_server(&directory, "-TERM")?;
    report("Phase 16 GPUI/live-server walkthrough: PASS")
}

fn verify_history(cx: &mut VisualTestContext, view: &Entity<AppModel>) -> Result {
    if view.read_with(cx, |model, cx| pane(model, cx).state == PaneState::Live) {
        shell(cx, "seq 1 300");
        wait(cx, view, |model, cx| {
            active_grid(model, cx).is_some_and(|grid| {
                (0..grid.rows.len()).any(|row| grid.row_text(row).trim() == "300")
            })
        })?;
    }
    let position = position(view, cx, 2, 2);
    cx.simulate_event(MouseMoveEvent {
        position,
        ..MouseMoveEvent::default()
    });
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Lines(point(0.0, 2.0)),
        ..ScrollWheelEvent::default()
    });
    wait(cx, view, |model, cx| {
        pane(model, cx)
            .scroll
            .view
            .as_ref()
            .is_some_and(|grid| grid.history_fresh && !grid.history.is_empty())
    })?;
    let (row, expected, width) = view.read_with(cx, |model, cx| {
        let pane = pane(model, cx);
        let grid = pane.displayed_grid().expect("grid");
        let row = u16::try_from(grid.history.len() - 1 - pane.visible_start(grid)).expect("row");
        let a = grid
            .history
            .back()
            .expect("history")
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>();
        let b = grid.row_text(0);
        (
            row,
            format!("{}\n{}", a.trim_end(), b.trim_end()),
            u16::try_from(b.trim_end().len()).expect("columns"),
        )
    });
    drag(cx, view, (row, 0), (row + 1, width));
    assert_eq!(copied(cx), expected);
    assert!(view.read_with(cx, |model, cx| pane(model, cx).scroll.view.is_some()));
    report(&format!(
        "16.3: history/screen boundary copied without jumping to bottom: {expected:?}"
    ))?;
    cx.dispatch_action(crate::views::workspace::ScrollToBottom);
    cx.run_until_parked();
    Ok(())
}

fn verify_shell_paste(
    cx: &mut VisualTestContext,
    view: &Entity<AppModel>,
    directory: &std::path::Path,
) -> Result {
    let output = directory.join("paste-order.txt");
    let commands = format!(
        "echo FIRST >> {}\necho SECOND >> {}",
        output.display(),
        output.display()
    );
    shell(
        cx,
        &format!(
            "printf '%s\\n' 'echo FIRST >> {}' 'echo SECOND >> {}'",
            output.display(),
            output.display()
        ),
    );
    wait(cx, view, |model, cx| {
        active_grid(model, cx).is_some_and(|grid| {
            (0..grid.rows.len()).any(|row| {
                grid.row_text(row).trim_end() == commands.lines().last().unwrap_or_default()
            })
        })
    })?;
    let first = find_row(view, cx, |row| {
        row == commands.lines().next().unwrap_or_default()
    });
    let second = find_row(view, cx, |row| {
        row == commands.lines().last().unwrap_or_default()
    });
    drag(
        cx,
        view,
        (first, 0),
        (
            second,
            u16::try_from(commands.lines().last().unwrap_or_default().len())?,
        ),
    );
    assert_eq!(copied(cx), commands);
    cx.simulate_keystrokes("cmd-v");
    cx.simulate_keystrokes("enter");
    wait(cx, view, |_, _| {
        std::fs::read_to_string(&output).is_ok_and(|text| text == "FIRST\nSECOND\n")
    })?;
    report("16.4: Cmd-V at zsh prompt, then Return -> FIRST\\nSECOND\\n in paste-order.txt")
}

fn verify_vim(
    cx: &mut VisualTestContext,
    view: &Entity<AppModel>,
    directory: &std::path::Path,
) -> Result {
    let output = directory.join("vim-paste.txt");
    shell(
        cx,
        &format!(
            "vim -Nu NONE -n -i NONE -c 'set autoindent' {}",
            output.display()
        ),
    );
    wait(cx, view, |model, cx| {
        active_grid(model, cx)
            .is_some_and(|grid| grid.modes.bracketed_paste && grid.row_text(0).trim().is_empty())
    })?;
    type_text(cx, "i");
    cx.update(|_, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            "  first\n    second\n  third".into(),
        ));
    });
    cx.simulate_keystrokes("cmd-v");
    wait_text(cx, view, "third")?;
    cx.simulate_keystrokes("escape");
    shell(cx, ":wq");
    wait(cx, view, |_, _| {
        std::fs::read_to_string(&output).is_ok_and(|text| text == "  first\n    second\n  third\n")
    })?;
    report(
        "16.5: vim autoindent + insert-mode Cmd-V preserved exactly:\n  first\n    second\n  third",
    )
}
