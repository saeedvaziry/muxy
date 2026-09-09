use super::*;

#[gpui::test]
fn colors_are_sent_before_attach_and_refreshed_on_theme_change_and_reconnect(
    cx: &mut TestAppContext,
) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_terminal_tab(state.home().id).expect("tab");
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.receive((1, Update::Connected(vec![])), cx);
        let pane = model.active_pane().expect("pane");
        model.start_attach(pane, Size { cols: 80, rows: 24 }, cx);
        let work: Vec<_> = requests.try_iter().collect();
        assert!(matches!(work.first(), Some((1, Work::Colors(colors))) if *colors == model.palette.terminal_colors()));
        assert!(work.iter().any(|(_, work)| matches!(work, Work::Attach { .. })));

        let initial = model.palette.terminal_colors();
        model.dark = !model.dark;
        model.refresh_theme(cx);
        let updated = model.palette.terminal_colors();
        assert_ne!(initial.background, updated.background);
        assert!(matches!(requests.try_recv(), Ok((1, Work::Colors(colors))) if colors == updated));

        model.disconnect(cx);
        model.dark = !model.dark;
        model.refresh_theme(cx);
        assert!(requests.try_iter().next().is_none());
        model.connect(cx);
        assert!(matches!(requests.try_recv(), Ok((2, Work::Connect))));
        model.receive((2, Update::Connected(vec![])), cx);
        assert!(matches!(requests.try_recv(), Ok((2, Work::Colors(colors))) if colors == initial));
    });
}
