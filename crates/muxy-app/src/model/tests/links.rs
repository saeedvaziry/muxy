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
