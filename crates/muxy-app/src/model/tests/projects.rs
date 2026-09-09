use super::*;

fn two_projects() -> (AppState, ProjectId, ProjectId, PaneId, PaneId) {
    let mut state = AppState::bootstrap().expect("state");
    let first = state
        .add_project(std::env::temp_dir())
        .expect("first project");
    state.open_terminal_tab(first).expect("first tab");
    let first_pane = state.current_project().tabs[0].panes[0].id;
    let second = state
        .add_project(std::env::current_dir().expect("cwd"))
        .expect("second project");
    state.open_terminal_tab(second).expect("second tab");
    let second_pane = state.current_project().tabs[0].panes[0].id;
    (state, first, second, first_pane, second_pane)
}

#[gpui::test]
fn switching_projects_detaches_and_reattaches_in_the_owning_directory(cx: &mut TestAppContext) {
    let (mut state, first, second, first_pane, second_pane) = two_projects();
    let first_session = SessionId::new(41).expect("session");
    let second_session = SessionId::new(42).expect("session");
    state
        .set_pane_session(first_pane, Some(first_session))
        .expect("session");
    state
        .set_pane_session(second_pane, Some(second_session))
        .expect("session");
    state.select_project(first).expect("select");
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.receive((1, Update::Connected([first_session, second_session].map(|id| SessionInfo { id, directory: muxy_protocol::ServerPath(b"/tmp".to_vec()) }).to_vec())), cx);
        model.receive((1, Update::Attached { pane: first_pane, session: first_session, attachment: attachment(), created: false }), cx);
        requests.try_iter().for_each(drop);
        model.select_project(second, cx);
        model.start_attach(second_pane, Size { cols: 80, rows: 24 }, cx);
        let work: Vec<_> = requests.try_iter().map(|(_, work)| work).collect();
        assert!(work.iter().any(|work| matches!(work, Work::Detach(_))));
        assert!(work.iter().any(|work| matches!(work, Work::Attach { pane, session: Some(session), directory, .. } if *pane == second_pane && *session == second_session && *directory == std::env::current_dir().expect("cwd"))));
        assert_eq!(model.grids.len(), 1);
        assert!(model.grids.contains_key(&second_pane));
        assert!(model.snapshots.contains_key(&first_pane));
        model.navigate(false, cx);
        assert_eq!(model.state.current_project().id, first);
        model.navigate(true, cx);
        assert_eq!(model.state.current_project().id, second);
        model.select_project(first, cx);
        model.start_attach(first_pane, Size { cols: 80, rows: 24 }, cx);
        assert!(requests.try_iter().any(|(_, work)| matches!(work, Work::Attach { pane, session: Some(session), .. } if pane == first_pane && session == first_session)));
        assert_eq!(model.state.projects().len(), 3);
    });
}

#[gpui::test]
fn hidden_restore_creates_in_each_project_and_removal_discards_late_creations(
    cx: &mut TestAppContext,
) {
    let (mut state, first, second, first_pane, second_pane) = two_projects();
    state.select_project(state.home().id).expect("home");
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.receive((1, Update::Connected(Vec::new())), cx);
        let work: Vec<_> = requests.try_iter().map(|(_, work)| work).collect();
        for (id, pane) in [(first, first_pane), (second, second_pane)] {
            let expected = &model.state.project(id).expect("project").directory;
            assert!(work.iter().any(|work| matches!(work, Work::Attach { pane: target, directory, .. } if *target == pane && directory == expected)));
        }
        model.remove_project_confirmed(first, cx);
        let session = SessionId::new(51).expect("session");
        model.receive((1, Update::Attached { pane: first_pane, session, attachment: attachment(), created: true }), cx);
        assert!(model.state.project(first).is_none());
        assert_eq!(model.state.pending_discards(), [session]);
        assert!(requests.try_iter().any(|(_, work)| matches!(work, Work::Discard(id) if id == session)));
        assert_eq!(model.state.project(second).expect("second").tabs.len(), 1);
        assert!(model.grids.is_empty());
    });
}

#[gpui::test]
fn tab_close_confirmation_keeps_its_target_after_project_switch(cx: &mut TestAppContext) {
    let (mut state, first, second, first_pane, _) = two_projects();
    let session = SessionId::new(61).expect("session");
    state
        .set_pane_session(first_pane, Some(session))
        .expect("session");
    state.select_project(first).expect("first");
    let tab = state.current_project().tabs[0].id;
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.connection = ConnectionState::Ready;
        model.close_tab(tab, cx);
        model.select_project(second, cx);
        model.receive_close_checked(tab, session, Ok(None), cx);
        assert!(model.state.project(first).expect("first").tabs.is_empty());
        assert_eq!(model.state.project(second).expect("second").tabs.len(), 1);
        assert_eq!(model.state.current_project().id, second);
        assert!(
            requests
                .try_iter()
                .any(|(_, work)| matches!(work, Work::Discard(id) if id == session))
        );
    });
}

#[gpui::test]
fn project_removal_requires_confirmation_and_persists_offline_cleanup(cx: &mut TestAppContext) {
    let (mut state, first, second, first_pane, _) = two_projects();
    let session = SessionId::new(71).expect("session");
    state
        .set_pane_session(first_pane, Some(session))
        .expect("session");
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| model.confirm_remove_project(first, cx));
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(view.read_with(cx, |model, _| model.state.project(first).is_some()));
    view.update(cx, |model, cx| model.confirm_remove_project(first, cx));
    cx.run_until_parked();
    cx.simulate_prompt_answer("Remove");
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        assert!(model.state.project(first).is_none());
        assert_eq!(model.state.current_project().id, second);
        assert_eq!(model.state.pending_discards(), [session]);
        let saved = store::load(&model.path).expect("load");
        assert!(saved.project(first).is_none());
        assert_eq!(saved.pending_discards(), [session]);
    });
    assert!(
        !requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::Discard(_)))
    );
    view.update(cx, |model, cx| {
        model.receive((1, Update::Connected(vec![])), cx);
    });
    assert!(
        requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::Discard(id) if id == session))
    );
}

#[gpui::test]
fn missing_folder_hides_its_terminal_until_refresh_finds_it_again(cx: &mut TestAppContext) {
    let directory = std::env::temp_dir().join(format!("muxy-project-status-{}", ProjectId::new()));
    std::fs::create_dir(&directory).expect("mkdir");
    let mut state = AppState::bootstrap().expect("state");
    let project = state.add_project(directory.clone()).expect("project");
    state.open_terminal_tab(project).expect("tab");
    let (boot, _) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    std::fs::remove_dir(&directory).expect("remove fixture");
    view.update(cx, |model, cx| {
        model.refresh_project_statuses(cx);
        assert!(model.active_pane().is_none());
        assert!(model.grids.is_empty());
        assert_eq!(model.state.current_project().tabs.len(), 1);
        model.new_tab(cx);
        assert_eq!(model.state.current_project().tabs.len(), 1);
    });
    std::fs::create_dir(&directory).expect("restore fixture");
    view.update(cx, |model, cx| {
        model.refresh_project_statuses(cx);
        assert!(model.active_pane().is_some());
        assert_eq!(model.grids.len(), 1);
    });
    std::fs::remove_dir(&directory).expect("cleanup");
}

#[gpui::test]
fn end_all_includes_hidden_projects_and_keeps_empty_project_records(cx: &mut TestAppContext) {
    let (mut state, first, second, first_pane, second_pane) = two_projects();
    let first_session = SessionId::new(81).expect("session");
    let second_session = SessionId::new(82).expect("session");
    state
        .set_pane_session(first_pane, Some(first_session))
        .expect("session");
    state
        .set_pane_session(second_pane, Some(second_session))
        .expect("session");
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.connection = ConnectionState::Ready;
        model.end_all_and_quit(cx);
        assert!(requests.try_iter().any(|(_, work)| matches!(work, Work::EndAll(sessions) if sessions == [first_session, second_session])));
        model.finish_end_all(Ok(()), cx);
        assert!(model.state.project(first).expect("first").tabs.is_empty());
        assert!(model.state.project(second).expect("second").tabs.is_empty());
    });
}

#[gpui::test]
fn removal_save_failure_keeps_projects_and_never_discards(cx: &mut TestAppContext) {
    let (mut state, first, _, first_pane, _) = two_projects();
    state
        .set_pane_session(first_pane, Some(SessionId::new(91).expect("session")))
        .expect("session");
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.connection = ConnectionState::Ready;
        let temporary = PathBuf::from(format!("{}.tmp", model.path.display()));
        std::fs::create_dir(&temporary).expect("block save");
        let previous = model.state.clone();
        model.remove_project_confirmed(first, cx);
        assert_eq!(model.state, previous);
        assert!(model.error.is_some());
        assert!(
            !requests
                .try_iter()
                .any(|(_, work)| matches!(work, Work::Discard(_)))
        );
        std::fs::remove_dir(temporary).expect("cleanup");
    });
}

#[gpui::test]
#[ignore = "requires a built muxy-server and a fresh MUXY_DIR under /tmp/muxy-phase24-"]
fn projects_live_walkthrough(cx: &mut TestAppContext) {
    run_projects_live_walkthrough(cx).expect("phase 24 walkthrough");
}

fn run_projects_live_walkthrough(cx: &mut TestAppContext) -> Result {
    let directory = PathBuf::from(std::env::var("MUXY_DIR")?);
    assert!(
        directory
            .to_string_lossy()
            .starts_with("/tmp/muxy-phase24-")
    );
    assert!(!directory.join("state.json").exists());
    let first_path = directory.join("Alpha");
    let second_path = directory.join("Beta");
    std::fs::create_dir_all(&first_path)?;
    std::fs::create_dir_all(&second_path)?;
    std::fs::write(first_path.join("keep.txt"), "keep")?;
    let boot = Boot::load()?;
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    let first = view.update(cx, |model, cx| {
        assert!(model.add_project(first_path.clone(), cx));
        assert!(model.state.current_project().tabs.is_empty());
        model.new_tab(cx);
        model.state.current_project().id
    });
    wait_live(cx, &view)?;
    shell(cx, "printf 'PROJECT_DIR_%s\\n' \"$PWD\"");
    wait_text(
        cx,
        &view,
        &format!("PROJECT_DIR_{}", first_path.canonicalize()?.display()),
    )?;
    let first_session = active_session(&view, cx)?;
    let second = view.update(cx, |model, cx| {
        assert!(model.add_project(second_path.clone(), cx));
        assert!(model.state.current_project().tabs.is_empty());
        model.new_tab(cx);
        model.state.current_project().id
    });
    wait_live(cx, &view)?;
    let second_session = active_session(&view, cx)?;
    shell(cx, "printf 'PROJECT_DIR_%s\\n' \"$PWD\"");
    wait_text(
        cx,
        &view,
        &format!("PROJECT_DIR_{}", second_path.canonicalize()?.display()),
    )?;
    let probe = Client::connect(&directory.join("server.sock"))?;
    assert_eq!(probe.list_sessions()?.len(), 2);
    report("24.1: two projects start empty; new tabs run in their respective folders")?;
    cx.simulate_keystrokes("cmd-alt-[");
    wait(cx, &view, |model, cx| {
        model.state.current_project().id == first && active_grid(model, cx).is_some()
    })?;
    assert_eq!(active_session(&view, cx)?, first_session);
    view.update(cx, |model, cx| {
        model.edit_project(
            |state| {
                state.rename_project(first, "Frontend")?;
                state.set_project_icon(first, Some("👩🏽‍💻".into()))?;
                state.set_project_color(first, "#bb9af7".parse()?)?;
                state.move_project(second, 1)
            },
            cx,
        );
    });
    let saved = view.read_with(cx, |model, _| store::load(&model.path))?;
    assert_eq!(saved.projects()[1].id, second);
    assert_eq!(saved.project(first).ok_or("first")?.name, "Frontend");
    assert_eq!(
        saved.project(first).ok_or("first")?.icon.as_deref(),
        Some("👩🏽‍💻")
    );
    report("24.2: switching restores the same session; name, emoji, color and order persist")?;
    view.update(cx, |model, _| {
        model.work.send((model.generation, Work::Stop))
    })?;
    cx.update(|window, _| window.remove_window());
    drop(view);
    cx.run_until_parked();
    let boot = Boot::load()?;
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    wait_live(cx, &view)?;
    assert_eq!(active_session(&view, cx)?, first_session);
    view.update(cx, |model, cx| model.select_project(second, cx));
    wait_live(cx, &view)?;
    assert_eq!(active_session(&view, cx)?, second_session);
    assert_eq!(probe.list_sessions()?.len(), 2);
    report("24.3: relaunch restores both projects and attaches to their original sessions")?;
    verify_removal(cx, &view, &probe, &directory, first, second, first_session)
}

fn verify_removal(
    cx: &mut VisualTestContext,
    view: &Entity<AppModel>,
    probe: &Client,
    directory: &std::path::Path,
    first: ProjectId,
    second: ProjectId,
    first_session: SessionId,
) -> Result {
    let first_path = directory.join("Alpha");
    let second_path = directory.join("Beta");
    let moved = directory.join("Alpha-moved");
    std::fs::rename(&first_path, &moved)?;
    view.update(cx, AppModel::refresh_project_statuses);
    assert_eq!(
        view.read_with(cx, |model, _| model
            .state
            .project(first)
            .map(muxy_app_core::Project::status)),
        Some(ProjectStatus::Missing)
    );
    view.update(cx, |model, cx| model.confirm_remove_project(first, cx));
    cx.run_until_parked();
    cx.simulate_prompt_answer("Remove");
    wait(cx, view, |model, _| {
        model.state.project(first).is_none() && model.state.pending_discards().is_empty()
    })?;
    assert_eq!(std::fs::read_to_string(moved.join("keep.txt"))?, "keep");
    assert!(
        probe
            .list_sessions()?
            .iter()
            .all(|session| session.id != first_session)
    );
    report(
        "24.4: moved folder becomes Missing; confirmed removal preserves its files and ends its session",
    )?;
    shell(cx, "sleep 60");
    wait(cx, view, |model, cx| {
        active_process(model, cx).is_some_and(|process| process.name == "sleep")
    })?;
    view.update(cx, |model, cx| model.confirm_remove_project(second, cx));
    cx.run_until_parked();
    cx.simulate_prompt_answer("Remove");
    wait(cx, view, |model, _| {
        model.state.project(second).is_none() && model.state.pending_discards().is_empty()
    })?;
    assert!(probe.list_sessions()?.is_empty());
    assert!(second_path.is_dir());
    report(
        "24.5: confirmed removal ends a running sleep session, discards its saved output, and keeps the folder",
    )?;
    report("Phase 24 GPUI/live-server walkthrough: PASS")
}

#[gpui::test]
fn project_editor_and_color_shortcuts_apply_to_the_requested_project(cx: &mut TestAppContext) {
    let (state, first, second, _, _) = two_projects();
    let (boot, _) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.update(|window, cx| {
        view.update(cx, |model, cx| {
            model.open_project_editor(
                first,
                crate::views::project_editor::Field::Name,
                gpui::point(px(10.0), px(60.0)),
                window,
                cx,
            );
        });
    });
    cx.simulate_input("Frontend");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        assert_eq!(model.state.project(first).expect("first").name, "Frontend");
        assert_eq!(model.state.current_project().id, second);
        assert!(model.overlay.is_none());
    });
    cx.update(|window, cx| {
        view.update(cx, |model, cx| {
            model.open_project_editor(
                first,
                crate::views::project_editor::Field::Icon,
                gpui::point(px(10.0), px(60.0)),
                window,
                cx,
            );
        });
    });
    cx.simulate_input("👩🏽‍💻");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |model, _| model
            .state
            .project(first)
            .expect("first")
            .icon
            .clone()),
        Some("👩🏽‍💻".into())
    );
    cx.update(|window, cx| {
        view.update(cx, |model, cx| {
            model.open_project_colors(first, gpui::point(px(10.0), px(60.0)), window, cx);
        });
    });
    cx.simulate_keystrokes("right enter");
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |model, _| model
            .state
            .project(first)
            .expect("first")
            .color
            .to_string()),
        "#7dcfff"
    );
    cx.simulate_keystrokes("cmd-alt-[");
    assert_eq!(
        view.read_with(cx, |model, _| model.state.current_project().id),
        first
    );
    cx.simulate_keystrokes("cmd-alt-]");
    assert_eq!(
        view.read_with(cx, |model, _| model.state.current_project().id),
        second
    );
}

#[gpui::test]
fn long_project_paths_leave_connection_controls_inside_a_narrow_window(cx: &mut TestAppContext) {
    let root = std::env::temp_dir().join(format!("muxy-long-path-{}", ProjectId::new()));
    let mut directory = root.clone();
    for _ in 0..12 {
        directory.push("a-long-project-directory-name");
    }
    std::fs::create_dir_all(&directory).expect("mkdir");
    let mut state = AppState::bootstrap().expect("state");
    state.add_project(directory).expect("project");
    let (mut boot, _) = stub_boot(state);
    boot.settings.appearance.sidebar_expanded = true;
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, AppModel::disconnect);
    cx.simulate_resize(size(px(640.0), px(400.0)));
    cx.run_until_parked();
    let bar = cx.debug_bounds("project-status-bar").expect("status bar");
    let controls = cx
        .debug_bounds("project-connection-status")
        .expect("connection controls");
    assert!(controls.left() >= bar.left());
    assert!(controls.right() <= bar.right());
    assert!(controls.size.width > px(0.0));
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[gpui::test]
fn existing_shortcuts_dispatch_the_original_action_when_project_defaults_collide(
    cx: &mut TestAppContext,
) {
    for chord in ["cmd-o", "cmd-alt-[", "cmd-alt-]"] {
        let (state, _, project, _, _) = two_projects();
        let (mut boot, _requests) = stub_boot(state);
        let directory = boot
            .state_path
            .parent()
            .expect("state directory")
            .to_owned();
        std::fs::create_dir_all(&directory).expect("mkdir");
        let settings_path = directory.join("settings.toml");
        std::fs::write(&settings_path, format!("[keymap]\nnew_tab = '{chord}'\n"))
            .expect("settings");
        boot.settings = muxy_settings::Settings::load(&settings_path).expect("legacy keymap");
        cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
        let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
        view.update(cx, |model, _| model.connection = ConnectionState::Ready);
        cx.simulate_keystrokes(chord);
        cx.run_until_parked();
        view.read_with(cx, |model, _| {
            assert_eq!(model.state.current_project().id, project);
            assert_eq!(model.state.current_project().tabs.len(), 2);
            assert!(model.overlay.is_none());
        });
        std::fs::remove_dir_all(directory).expect("cleanup");
    }
}

#[gpui::test]
fn command_o_opens_the_legacy_sized_picker_and_existing_paths_select_the_project(
    cx: &mut TestAppContext,
) {
    let root = std::env::temp_dir().join(format!("muxy-open-project-{}", ProjectId::new()));
    std::fs::create_dir_all(root.join("Alpha")).expect("mkdir");
    let mut state = AppState::bootstrap().expect("state");
    let project = state.add_project(root.join("Alpha")).expect("project");
    state.select_project(state.home().id).expect("home");
    let (mut boot, requests) = stub_boot(state);
    boot.settings.projects.search_root = Some(root.clone());
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_resize(size(px(1200.0), px(800.0)));
    cx.simulate_keystrokes("cmd-o");
    cx.run_until_parked();
    assert!(view.read_with(cx, |model, _| matches!(
        model.overlay,
        Some(Overlay::Projects(_))
    )));
    let panel = cx.debug_bounds("project-picker").expect("picker panel");
    assert_eq!(panel.size, size(px(640.0), px(460.0)));
    assert_eq!(panel.left(), px(280.0));
    assert_eq!(panel.top(), px(48.0));
    cx.simulate_input(&root.join("Alpha").to_string_lossy());
    cx.executor().advance_clock(Duration::from_millis(125));
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-enter");
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        assert_eq!(model.state.current_project().id, project);
        assert_eq!(model.state.projects().len(), 2);
        assert!(model.overlay.is_none());
    });
    assert!(
        !requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::Attach { .. }))
    );
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[gpui::test]
fn creating_a_project_folder_requires_the_explicit_create_confirmation(cx: &mut TestAppContext) {
    let root = std::env::temp_dir().join(format!("muxy-create-project-{}", ProjectId::new()));
    std::fs::create_dir_all(&root).expect("root");
    let (mut boot, _) = stub_boot(AppState::bootstrap().expect("state"));
    boot.settings.projects.search_root = Some(root.clone());
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_keystrokes("cmd-o");
    cx.simulate_input(&root.join("New").to_string_lossy());
    cx.executor().advance_clock(Duration::from_millis(125));
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-enter");
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    assert!(!root.join("New").exists());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(!root.join("New").exists());
    assert_eq!(
        view.read_with(cx, |model, _| model.state.projects().len()),
        1
    );
    cx.simulate_keystrokes("cmd-enter");
    cx.run_until_parked();
    cx.simulate_prompt_answer("Create & Add");
    cx.run_until_parked();
    assert!(root.join("New").is_dir());
    view.read_with(cx, |model, _| {
        assert_eq!(model.state.current_project().directory, root.join("New"));
        assert!(model.state.current_project().tabs.is_empty());
        assert!(model.overlay.is_none());
    });
    std::fs::remove_dir_all(root).expect("cleanup");
}
