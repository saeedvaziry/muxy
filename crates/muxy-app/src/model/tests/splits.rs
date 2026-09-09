use super::*;
use muxy_app_core::{Branch, Direction};
use muxy_protocol::ChannelId;

fn split_state() -> (AppState, TabId, [PaneId; 3]) {
    let mut state = AppState::bootstrap().expect("state");
    let tab = state.open_terminal_tab(state.home().id).expect("tab");
    let first = state.home().tabs[0].panes[0].id;
    let second = state.split_pane(first, Direction::Right).expect("split");
    let third = state.split_pane(second, Direction::Down).expect("split");
    for (index, pane) in [first, second, third].into_iter().enumerate() {
        state
            .set_pane_session(pane, SessionId::new(100 + index as u64))
            .expect("session");
    }
    (state, tab, [first, second, third])
}

fn attach_panes(model: &mut AppModel, panes: &[PaneId], cx: &mut Context<AppModel>) {
    model.connection = ConnectionState::Ready;
    for (index, pane) in panes.iter().enumerate() {
        let mut attachment = attachment();
        attachment.channel = ChannelId(u32::try_from(index + 1).expect("channel"));
        model.receive(
            (
                1,
                Update::Attached {
                    pane: *pane,
                    session: model.pane_session(*pane).expect("session"),
                    attachment,
                    created: false,
                },
            ),
            cx,
        );
    }
}

#[gpui::test]
fn zoom_and_tab_switch_detach_only_hidden_leaves_and_restore_all(cx: &mut TestAppContext) {
    let (state, first_tab, panes) = split_state();
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        assert_eq!(model.grids.len(), 3);
        attach_panes(model, &panes, cx);
        let _ = requests.try_iter().collect::<Vec<_>>();
        model.toggle_zoom_pane(cx);
        assert_eq!(model.visible_panes(), [panes[2]]);
        assert_eq!(model.grids.len(), 1);
        let detached: Vec<_> = requests
            .try_iter()
            .filter_map(|(_, work)| match work {
                Work::Detach(channel) => Some(channel),
                _ => None,
            })
            .collect();
        assert_eq!(detached.len(), 2);
        assert!(detached.contains(&ChannelId(1)) && detached.contains(&ChannelId(2)));
        model.toggle_zoom_pane(cx);
        assert_eq!(model.grids.len(), 3);
        assert_eq!(
            model.grids[&panes[2]].view.read(cx).channel(),
            Some(ChannelId(3))
        );
        for pane in &panes {
            model.grids[pane].view.update(cx, |pane, cx| {
                pane.set_viewport(Size { cols: 30, rows: 12 }, cx);
            });
        }
        model.ensure_visible(cx);
        let attached: Vec<_> = requests
            .try_iter()
            .filter_map(|(_, work)| match work {
                Work::Attach { pane, session, .. } => Some((pane, session)),
                _ => None,
            })
            .collect();
        assert_eq!(attached.len(), 2);
        assert!(attached.contains(&(panes[0], model.pane_session(panes[0]))));
        assert!(attached.contains(&(panes[1], model.pane_session(panes[1]))));
        attach_panes(model, &panes, cx);
        let _ = requests.try_iter().collect::<Vec<_>>();
        model.new_tab(cx);
        assert_eq!(model.grids.len(), 1);
        assert_eq!(
            requests
                .try_iter()
                .filter(|(_, work)| matches!(work, Work::Detach(_)))
                .count(),
            3
        );
        model.select_tab(first_tab, cx);
        assert_eq!(model.grids.len(), 3);
        assert_eq!(model.active_pane(), Some(panes[2]));
    });
}

#[gpui::test]
fn zoom_controls_frame_the_pane_and_restore_the_split_layout(cx: &mut TestAppContext) {
    let (mut state, split_tab, panes) = split_state();
    let home = state.home().id;
    for _ in 0..12 {
        state.open_terminal_tab(home).expect("tab");
    }
    let single_tab = state.home().tabs.last().expect("single tab").id;
    let layout = state.home().tabs[0].layout.clone();
    let (boot, _requests) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_resize(size(px(700.0), px(500.0)));
    cx.run_until_parked();
    assert!(cx.debug_bounds("zoomed-pane-frame").is_none());
    assert!(cx.debug_bounds("restore-pane").is_none());
    assert!(cx.debug_bounds("maximize-pane").is_none());
    let single_bounds = cx.debug_bounds("terminal-pane").expect("single terminal");
    view.update(cx, |model, cx| {
        model.select_tab(split_tab, cx);
        attach_panes(model, &panes, cx);
    });
    cx.run_until_parked();
    let original_geometry = view.read_with(cx, |model, cx| {
        model.grids[&panes[2]].view.read(cx).geometry
    });
    assert!(cx.debug_bounds("zoomed-pane-frame").is_none());
    for (selector, zoomed) in [("maximize-pane", true), ("restore-pane", false)] {
        let button = cx.debug_bounds(selector).expect("zoom control");
        assert!(button.left() >= single_bounds.left() && button.right() <= single_bounds.right());
        let position = button.center();
        cx.simulate_event(gpui::MouseDownEvent {
            position,
            button: gpui::MouseButton::Left,
            click_count: 1,
            ..Default::default()
        });
        cx.simulate_event(gpui::MouseUpEvent {
            position,
            button: gpui::MouseButton::Left,
            ..Default::default()
        });
        cx.run_until_parked();
        view.read_with(cx, |model, cx| {
            assert_eq!(
                model.state.home().tabs[0].zoomed,
                zoomed.then_some(panes[2])
            );
            assert_eq!(model.state.home().tabs[0].layout, layout);
            assert_eq!(model.active_pane(), Some(panes[2]));
            assert_eq!(model.grids.len(), if zoomed { 1 } else { 3 });
            assert!(model.grids[&panes[2]].view.read(cx).focused);
            for (id, pane) in &model.grids {
                assert_eq!(
                    pane.view.read(cx).corner_radius,
                    if zoomed && *id == panes[2] {
                        model.metrics.radius_lg() - px(1.0)
                    } else {
                        px(0.0)
                    }
                );
            }
            assert_eq!(store::load(&model.path).expect("saved"), model.state);
        });
        if zoomed {
            let frame = cx.debug_bounds("zoomed-pane-frame").expect("zoom frame");
            let terminal = cx.debug_bounds("terminal-pane").expect("zoomed terminal");
            let inset = view.read_with(cx, |model, _| model.metrics.spacing7()) + px(1.0);
            assert_eq!(terminal.origin, frame.origin + gpui::point(inset, inset));
            assert_eq!(terminal.size, frame.size - size(inset * 2.0, inset * 2.0));
        } else {
            assert!(cx.debug_bounds("split-divider-[]").is_some());
            assert_eq!(
                view.read_with(cx, |model, cx| model.grids[&panes[2]]
                    .view
                    .read(cx)
                    .geometry),
                original_geometry
            );
        }
    }
    cx.simulate_keystrokes("cmd-shift-enter");
    assert!(cx.debug_bounds("zoomed-pane-frame").is_some());
    assert!(cx.debug_bounds("restore-pane").is_some());
    view.update(cx, |model, cx| model.select_tab(single_tab, cx));
    cx.run_until_parked();
    let terminal = cx.debug_bounds("terminal-pane").expect("single terminal");
    assert_eq!(terminal, single_bounds);
}

#[gpui::test]
fn closing_one_pane_discards_only_its_session_and_last_closes_tab(cx: &mut TestAppContext) {
    let (state, _, panes) = split_state();
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        attach_panes(model, &panes, cx);
        let _ = requests.try_iter().collect::<Vec<_>>();
        model.close_pane(panes[1], cx);
        assert_eq!(model.state.home().tabs[0].panes.len(), 2);
        assert_eq!(model.active_pane(), Some(panes[2]));
        assert_eq!(
            model.state.pending_discards(),
            [SessionId::new(101).expect("session")]
        );
        let discarded: Vec<_> = requests
            .try_iter()
            .filter_map(|(_, work)| match work {
                Work::Discard(session) => Some(session),
                _ => None,
            })
            .collect();
        assert_eq!(discarded, model.state.pending_discards());
        let loaded = store::load(&model.path).expect("saved");
        assert_eq!(loaded.home().tabs[0].layout.leaves(), [panes[0], panes[2]]);
        model.close_pane(panes[2], cx);
        assert_eq!(model.active_pane(), Some(panes[0]));
        model.close_pane(panes[0], cx);
        assert!(model.state.home().tabs.is_empty());
        assert!(model.grids.is_empty());
    });
}

#[gpui::test]
fn closing_tab_checks_every_hidden_pane_and_cancel_keeps_all(cx: &mut TestAppContext) {
    let (state, tab, panes) = split_state();
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        attach_panes(model, &panes, cx);
        model.toggle_zoom_pane(cx);
        model.close_tab(tab, cx);
        assert_eq!(model.pending_close, Some(tab));
        assert!(requests.try_iter().any(|(_, work)| matches!(work, Work::CheckClose { session, .. } if Some(session) == SessionId::new(100))));
        model.receive_close_checked(tab, SessionId::new(100).expect("session"), Ok(None), cx);
        assert!(requests.try_iter().any(|(_, work)| matches!(work, Work::CheckClose { session, .. } if Some(session) == SessionId::new(101))));
        model.receive_close_checked(tab, SessionId::new(101).expect("session"), Ok(Some(muxy_protocol::ForegroundProcess { name: "vim".into(), is_shell: false })), cx);
    });
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        assert_eq!(model.state.home().tabs[0].panes.len(), 3);
        assert!(model.state.pending_discards().is_empty());
    });
}

#[gpui::test]
fn end_all_includes_unfocused_and_zoom_hidden_sessions(cx: &mut TestAppContext) {
    let (state, tab, panes) = split_state();
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        attach_panes(model, &panes, cx);
        model
            .state
            .set_ratio(tab, &[Branch::Second], 0.3)
            .expect("ratio");
        model.toggle_zoom_pane(cx);
        model.end_all_and_quit(cx);
        let sessions = requests
            .try_iter()
            .find_map(|(_, work)| match work {
                Work::EndAll(sessions) => Some(sessions),
                _ => None,
            })
            .expect("end all");
        assert_eq!(
            sessions,
            [100, 101, 102].map(|id| SessionId::new(id).expect("session"))
        );
    });
}

#[gpui::test]
fn shortcuts_split_focus_zoom_and_close_the_expected_pane(cx: &mut TestAppContext) {
    let (boot, requests) = stub_boot(AppState::bootstrap().expect("state"));
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.receive((1, Update::Connected(vec![])), cx);
    });
    cx.simulate_keystrokes("cmd-t");
    let first = view.read_with(cx, |model, _| model.active_pane().expect("first"));
    cx.simulate_keystrokes("cmd-d");
    let second = view.read_with(cx, |model, _| model.active_pane().expect("second"));
    cx.simulate_keystrokes("cmd-shift-d");
    let third = view.read_with(cx, |model, _| model.active_pane().expect("third"));
    view.update(cx, |model, cx| {
        assert_eq!(model.visible_panes(), [first, second, third]);
        let work: Vec<_> = requests.try_iter().map(|(_, work)| work).collect();
        assert_eq!(
            work.iter()
                .filter(|work| matches!(work, Work::Attach { session: None, .. }))
                .count(),
            3
        );
        for (index, pane) in [first, second, third].into_iter().enumerate() {
            model
                .state
                .set_pane_session(pane, SessionId::new(100 + index as u64))
                .expect("session");
        }
        attach_panes(model, &[first, second, third], cx);
    });
    cx.run_until_parked();
    for (shortcut, expected) in [
        ("cmd-alt-up", second),
        ("cmd-alt-left", first),
        ("cmd-alt-right", second),
        ("cmd-alt-down", third),
    ] {
        cx.simulate_keystrokes(shortcut);
        view.read_with(cx, |model, cx| {
            assert_eq!(model.active_pane(), Some(expected));
            for (id, pane) in &model.grids {
                assert_eq!(pane.view.read(cx).focused, *id == expected);
            }
        });
    }
    cx.simulate_keystrokes("cmd-shift-enter");
    assert_eq!(
        view.read_with(cx, |model, _| model.visible_panes()),
        [third]
    );
    cx.simulate_keystrokes("cmd-shift-enter");
    view.update(cx, |model, cx| {
        attach_panes(model, &[first, second, third], cx);
    });
    cx.simulate_keystrokes("cmd-w");
    view.read_with(cx, |model, _| {
        assert_eq!(model.visible_panes(), [first, second]);
    });
    cx.simulate_keystrokes("cmd-shift-w");
    view.read_with(cx, |model, _| assert!(model.state.home().tabs.is_empty()));
}

#[gpui::test]
fn split_directory_inherits_only_when_configured_and_falls_back_to_project(
    cx: &mut TestAppContext,
) {
    for (setting, reported, inherit) in [
        (
            muxy_settings::NewPaneDirectory::Project,
            b"/tmp".as_slice(),
            false,
        ),
        (
            muxy_settings::NewPaneDirectory::Current,
            b"/tmp".as_slice(),
            true,
        ),
        (
            muxy_settings::NewPaneDirectory::Current,
            b"".as_slice(),
            false,
        ),
        (
            muxy_settings::NewPaneDirectory::Current,
            b"relative".as_slice(),
            false,
        ),
    ] {
        let mut state = AppState::bootstrap().expect("state");
        state.open_terminal_tab(state.home().id).expect("tab");
        let original = state.home().tabs[0].panes[0].id;
        state
            .set_pane_session(original, SessionId::new(100))
            .expect("session");
        let (mut boot, requests) = stub_boot(state);
        boot.settings.panes.new_pane_directory = setting;
        let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
        view.update(cx, |model, cx| {
            model.connection = ConnectionState::Ready;
            let mut data = attachment();
            data.directory = muxy_protocol::ServerPath(reported.to_vec());
            model.receive(
                (
                    1,
                    Update::Attached {
                        pane: original,
                        session: SessionId::new(100).expect("session"),
                        attachment: data,
                        created: false,
                    },
                ),
                cx,
            );
            let _ = requests.try_iter().collect::<Vec<_>>();
            model.split_pane(Direction::Right, cx);
            let new = model.active_pane().expect("split");
            let directory = requests
                .try_iter()
                .find_map(|(_, work)| match work {
                    Work::Attach {
                        pane,
                        directory,
                        session: None,
                        ..
                    } if pane == new => Some(directory),
                    _ => None,
                })
                .expect("creation request");
            assert_eq!(
                directory,
                if inherit {
                    PathBuf::from("/tmp")
                } else {
                    model.state.current_project().directory.clone()
                }
            );
            model.close_pane(new, cx);
            model.receive(
                (
                    1,
                    Update::Attached {
                        pane: new,
                        session: SessionId::new(101).expect("session"),
                        attachment: attachment(),
                        created: true,
                    },
                ),
                cx,
            );
            assert!(
                model
                    .state
                    .pending_discards()
                    .contains(&SessionId::new(101).expect("session"))
            );
            assert!(model.initial_directories.is_empty());
        });
    }
}

fn drag_divider(
    cx: &mut VisualTestContext,
    selector: &'static str,
    offset: gpui::Point<gpui::Pixels>,
) {
    let bounds = cx.debug_bounds(selector).expect("divider");
    let start = bounds.center() + gpui::point(px(2.0), px(0.0));
    cx.simulate_event(gpui::MouseMoveEvent {
        position: start,
        ..Default::default()
    });
    cx.simulate_event(gpui::MouseDownEvent {
        position: start,
        button: gpui::MouseButton::Left,
        click_count: 1,
        ..Default::default()
    });
    cx.simulate_event(gpui::MouseMoveEvent {
        position: start + offset,
        pressed_button: Some(gpui::MouseButton::Left),
        ..Default::default()
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position: start + offset,
        button: gpui::MouseButton::Left,
        ..Default::default()
    });
    cx.run_until_parked();
}

#[gpui::test]
fn divider_drag_persists_ratios_and_click_focus_routes_input(cx: &mut TestAppContext) {
    let (state, _, panes) = split_state();
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_resize(size(px(1100.0), px(800.0)));
    view.update(cx, |model, cx| attach_panes(model, &panes, cx));
    cx.run_until_parked();
    drag_divider(cx, "split-divider-[]", gpui::point(px(60.0), px(0.0)));
    drag_divider(
        cx,
        "split-divider-[Second]",
        gpui::point(px(0.0), px(-60.0)),
    );
    view.read_with(cx, |model, _| {
        let muxy_app_core::Layout::Split { ratio, second, .. } = &model.state.home().tabs[0].layout
        else {
            panic!("split");
        };
        assert!(*ratio > 0.5);
        assert!(
            matches!(second.as_ref(), muxy_app_core::Layout::Split { ratio, .. } if *ratio < 0.5)
        );
        assert_eq!(store::load(&model.path).expect("saved"), model.state);
    });
    let position = view.read_with(cx, |model, cx| {
        model.grids[&panes[0]]
            .view
            .read(cx)
            .geometry
            .expect("geometry")
            .0
            .origin
            + gpui::point(px(10.0), px(10.0))
    });
    cx.simulate_event(gpui::MouseDownEvent {
        position,
        button: gpui::MouseButton::Left,
        click_count: 1,
        ..Default::default()
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position,
        button: gpui::MouseButton::Left,
        ..Default::default()
    });
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |model, _| model.active_pane()),
        Some(panes[0])
    );
    let _ = requests.try_iter().collect::<Vec<_>>();
    cx.simulate_keystrokes("x");
    assert!(
        requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::Input(ChannelId(1), bytes) if bytes == b"x"))
    );
    cx.update(|window, cx| view.update(cx, |model, cx| model.open_theme_picker(window, cx)));
    cx.run_until_parked();
    view.read_with(cx, |model, cx| {
        assert!(
            model
                .grids
                .values()
                .all(|pane| !pane.view.read(cx).native_visible)
        );
    });
}

#[gpui::test]
#[ignore = "requires a built muxy-server, top, vim, and a fresh MUXY_DIR under /tmp/muxy-phase23-"]
fn phase23_split_walkthrough(cx: &mut TestAppContext) {
    let result = run_split_walkthrough(cx);
    assert!(result.is_ok(), "{result:?}");
}

fn wait_all_panes(cx: &mut VisualTestContext, view: &Entity<AppModel>, count: usize) -> Result {
    wait(cx, view, |model, cx| {
        model.grids.len() == count
            && model.pending.is_empty()
            && model
                .grids
                .values()
                .all(|pane| pane.view.read(cx).channel().is_some())
    })
}

fn run_split_walkthrough(cx: &mut TestAppContext) -> Result {
    let directory = PathBuf::from(std::env::var("MUXY_DIR")?);
    assert!(
        directory
            .to_string_lossy()
            .starts_with("/tmp/muxy-phase23-")
    );
    assert!(!directory.join("state.json").exists());
    let boot = Boot::load()?;
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_resize(size(px(1200.0), px(800.0)));
    cx.simulate_keystrokes("cmd-t");
    wait_live(cx, &view)?;
    let first = view.read_with(cx, |model, _| model.active_pane().expect("first"));
    let top_pid = record_shell_pid(cx, &view, &directory, "top-shell")?;
    shell(cx, "top -s 1");
    wait(cx, &view, |model, cx| {
        active_process(model, cx).is_some_and(|process| !process.is_shell)
    })?;
    wait_text(cx, &view, "Processes:")?;
    cx.simulate_keystrokes("cmd-d");
    wait_all_panes(cx, &view, 2)?;
    let second = view.read_with(cx, |model, _| model.active_pane().expect("second"));
    let vim_pid = record_shell_pid(cx, &view, &directory, "vim-shell")?;
    shell(cx, "vim -u NONE -i NONE");
    wait(cx, &view, |model, cx| {
        active_process(model, cx).is_some_and(|process| !process.is_shell)
    })?;
    cx.simulate_keystrokes("cmd-shift-d");
    wait_all_panes(cx, &view, 3)?;
    let third = view.read_with(cx, |model, _| model.active_pane().expect("third"));
    let shell_pid = record_shell_pid(cx, &view, &directory, "shell")?;
    shell(cx, "printf 'PHASE23_%s\\n' READY");
    wait_text(cx, &view, "PHASE23_READY")?;
    let probe = Client::connect(&directory.join("server.sock"))?;
    assert_eq!(probe.list_sessions()?.len(), 3);
    assert_eq!(
        view.read_with(cx, |model, _| model.visible_panes()),
        [first, second, third]
    );
    report(
        "23.1: Cmd-D and Cmd-Shift-D create three visible sessions running top, vim, and a shell",
    )?;

    drag_divider(cx, "split-divider-[]", gpui::point(px(50.0), px(0.0)));
    drag_divider(
        cx,
        "split-divider-[Second]",
        gpui::point(px(0.0), px(-40.0)),
    );
    cx.simulate_keystrokes("cmd-alt-up");
    assert_eq!(
        view.read_with(cx, |model, _| model.active_pane()),
        Some(second)
    );
    cx.simulate_keystrokes("cmd-shift-enter");
    wait_all_panes(cx, &view, 1)?;
    assert_eq!(
        view.read_with(cx, |model, _| model.visible_panes()),
        [second]
    );
    cx.simulate_keystrokes("cmd-shift-enter");
    wait_all_panes(cx, &view, 3)?;
    report(
        "23.2: both dividers drag; directional focus and vim zoom/unzoom attach the expected leaves",
    )?;

    let focus_before = view.read_with(cx, |model, _| model.active_pane());
    let before = view.read_with(cx, |model, _| model.state.home().tabs[0].clone());
    cx.simulate_keystrokes("cmd-q");
    cx.run_until_parked();
    let (empty, _) = stub_boot(AppState::bootstrap()?);
    cx.update(|window, cx| view.update(cx, |model, cx| *model = AppModel::new(empty, window, cx)));
    assert_eq!(probe.list_sessions()?.len(), 3);
    assert!(process_exists(top_pid) && process_exists(vim_pid) && process_exists(shell_pid));
    reload_model(cx, &view)?;
    wait_all_panes(cx, &view, 3)?;
    view.read_with(cx, |model, _| {
        let after = &model.state.home().tabs[0];
        assert_eq!(after.id, before.id);
        assert_eq!(after.layout, before.layout);
        assert_eq!(model.active_pane(), focus_before);
        assert_eq!(after.zoomed, before.zoomed);
        assert_eq!(
            after
                .panes
                .iter()
                .map(|pane| (pane.id, pane.content.clone()))
                .collect::<Vec<_>>(),
            before
                .panes
                .iter()
                .map(|pane| (pane.id, pane.content.clone()))
                .collect::<Vec<_>>()
        );
    });
    report(
        "23.3: quit, teardown, and Boot reload preserve layout, ratios, focus, and all three live session identities",
    )?;

    verify_close_panes(cx, &view, &probe, [top_pid, vim_pid, shell_pid])
}

fn verify_close_panes(
    cx: &mut VisualTestContext,
    view: &Entity<AppModel>,
    probe: &Client,
    [top_pid, vim_pid, shell_pid]: [u32; 3],
) -> Result {
    cx.simulate_keystrokes("cmd-w");
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Close");
    wait(cx, view, |model, _| {
        model.state.home().tabs[0].panes.len() == 2
            && model.state.pending_discards().is_empty()
            && !process_exists(vim_pid)
    })?;
    assert!(process_exists(top_pid) && process_exists(shell_pid));
    assert_eq!(probe.list_sessions()?.len(), 2);
    cx.simulate_keystrokes("cmd-alt-right");
    shell(cx, "printf 'SIBLING_%s\\n' ALIVE");
    wait_text(cx, view, "SIBLING_ALIVE")?;
    report(
        "23.4: confirmed Cmd-W closes only vim's pane/session; ps and shell input verify both siblings remain alive",
    )?;
    cx.simulate_keystrokes("cmd-alt-left cmd-w");
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Close");
    wait(cx, view, |model, _| {
        model.state.home().tabs[0].panes.len() == 1 && model.state.pending_discards().is_empty()
    })?;
    cx.simulate_keystrokes("cmd-w");
    wait(cx, view, |model, _| {
        model.state.home().tabs.is_empty() && model.state.pending_discards().is_empty()
    })?;
    assert!(probe.list_sessions()?.is_empty());
    assert!(!process_exists(top_pid) && !process_exists(shell_pid));
    report(
        "23.5: closing remaining panes collapses the tree and closes the tab; all test sessions are discarded",
    )?;
    report("Phase 23 GPUI/live-server walkthrough: PASS")
}
