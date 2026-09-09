use muxy_core::shortcuts::ShortcutId;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, Context, FontWeight, InteractiveElement, IntoElement, KeyBinding, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, actions, div, px, relative,
};
use muxy_app_core::Direction;
use muxy_settings::{Action, Keymap};
use muxy_ui::components::IconGlyph;
use muxy_ui::icon::Icon;

use super::{menu, overlays, sidebar, status_bar, tab_strip, titlebar};
use crate::model::AppModel;

actions!(
    muxy,
    [
        NewTab,
        NewHomeTab,
        CloseTab,
        SplitRight,
        SplitDown,
        FocusPaneLeft,
        FocusPaneRight,
        FocusPaneUp,
        FocusPaneDown,
        ToggleZoomPane,
        ClosePane,
        NextTab,
        PreviousTab,
        PreviousProject,
        NextProject,
        AddProject,
        ToggleSidebar,
        ToggleFullScreen,
        ToggleThemePicker,
        NavigateBack,
        NavigateForward,
        Quit,
        EndAllSessionsAndQuit,
        HideApp,
        HideOthers,
        ShowAll,
        Minimize,
        Zoom,
        OpenConfiguration,
        IncreaseFontSize,
        DecreaseFontSize,
        ScrollToBottom,
        Find,
        FindNext,
        FindPrevious
    ]
);

#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = muxy, no_json)]
pub(crate) struct SelectTab {
    pub(crate) index: usize,
}

pub(crate) fn bind_keys(keymap: &Keymap, cx: &mut App) {
    cx.bind_keys(workspace_bindings(keymap));
}

fn workspace_bindings(keymap: &impl muxy_core::shortcuts::ShortcutSettings) -> Vec<KeyBinding> {
    let mut registry = muxy_ui::shortcuts::Registry::new(keymap);
    registry.register(ShortcutId::NewHomeTab, &NewHomeTab);
    registry.register(ShortcutId::ToggleSidebar, &ToggleSidebar);
    registry.register(ShortcutId::ToggleFullScreen, &ToggleFullScreen);
    registry.register(ShortcutId::ToggleThemePicker, &ToggleThemePicker);
    registry.register(ShortcutId::NavigateBack, &NavigateBack);
    registry.register(ShortcutId::NavigateForward, &NavigateForward);
    registry.register(ShortcutId::Quit, &Quit);
    registry.register(ShortcutId::HideApp, &HideApp);
    registry.register(ShortcutId::HideOthers, &HideOthers);
    registry.register(ShortcutId::Minimize, &Minimize);
    registry.register(ShortcutId::NewTab, &NewTab);
    registry.register(ShortcutId::CloseTab, &CloseTab);
    registry.register(ShortcutId::SplitRight, &SplitRight);
    registry.register(ShortcutId::SplitDown, &SplitDown);
    registry.register(ShortcutId::FocusPaneLeft, &FocusPaneLeft);
    registry.register(ShortcutId::FocusPaneRight, &FocusPaneRight);
    registry.register(ShortcutId::FocusPaneUp, &FocusPaneUp);
    registry.register(ShortcutId::FocusPaneDown, &FocusPaneDown);
    registry.register(ShortcutId::ToggleZoomPane, &ToggleZoomPane);
    registry.register(ShortcutId::ClosePane, &ClosePane);
    registry.register(ShortcutId::NextTab, &NextTab);
    registry.register(ShortcutId::PreviousTab, &PreviousTab);
    registry.register(ShortcutId::PreviousProject, &PreviousProject);
    registry.register(ShortcutId::NextProject, &NextProject);
    registry.register(ShortcutId::AddProject, &AddProject);
    registry.register(ShortcutId::SelectTab1, &SelectTab { index: 0 });
    registry.register(ShortcutId::SelectTab2, &SelectTab { index: 1 });
    registry.register(ShortcutId::SelectTab3, &SelectTab { index: 2 });
    registry.register(ShortcutId::SelectTab4, &SelectTab { index: 3 });
    registry.register(ShortcutId::SelectTab5, &SelectTab { index: 4 });
    registry.register(ShortcutId::SelectTab6, &SelectTab { index: 5 });
    registry.register(ShortcutId::SelectTab7, &SelectTab { index: 6 });
    registry.register(ShortcutId::SelectTab8, &SelectTab { index: 7 });
    registry.register(ShortcutId::SelectTab9, &SelectTab { index: 8 });
    registry.register(ShortcutId::Copy, &muxy_ui::text_input::Copy);
    registry.register(ShortcutId::Paste, &muxy_ui::text_input::Paste);
    registry.register(ShortcutId::Find, &Find);
    registry.register(ShortcutId::FindNext, &FindNext);
    registry.register(ShortcutId::FindPrevious, &FindPrevious);
    registry.register(ShortcutId::ScrollToBottom, &ScrollToBottom);
    registry.register(ShortcutId::IncreaseFontSize, &IncreaseFontSize);
    registry.register(ShortcutId::DecreaseFontSize, &DecreaseFontSize);
    muxy_ui::text_input::register_shortcuts(&mut registry);
    muxy_ui::command_popover::register_shortcuts(&mut registry);
    muxy_ui::components::register_shortcuts(&mut registry);
    menu::register_shortcuts(&mut registry);
    super::project_editor::register_shortcuts(&mut registry);
    super::terminal::pane::register_shortcuts(&mut registry);
    registry.register(ShortcutId::EndAllSessionsAndQuit, &EndAllSessionsAndQuit);
    registry.register(ShortcutId::ShowAll, &ShowAll);
    registry.register(ShortcutId::Zoom, &Zoom);
    registry.register(ShortcutId::OpenConfiguration, &OpenConfiguration);
    registry.into_bindings()
}

impl AppModel {
    fn find_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay.is_some() {
            return;
        }
        if let Some(pane) = self.active_pane().and_then(|id| self.grids.get(&id)) {
            pane.view.update(cx, |pane, cx| {
                pane.open_find(&self.theme, self.metrics, window, cx);
            });
        }
    }

    fn step_find(&mut self, previous: bool, cx: &mut Context<Self>) {
        if self.overlay.is_some() {
            return;
        }
        if let Some(pane) = self.active_pane().and_then(|id| self.grids.get(&id)) {
            pane.view
                .update(cx, |pane, cx| pane.step_find(previous, cx));
        }
    }
    fn zoom_terminal(&mut self, delta: f32, cx: &mut Context<Self>) {
        if let Some(pane) = self.active_pane().and_then(|pane| self.grids.get(&pane)) {
            pane.view.update(cx, |pane, cx| {
                pane.terminal.zoom(delta);
                cx.notify();
            });
        }
    }

    pub(crate) fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.appearance.sidebar_expanded = !self.appearance.sidebar_expanded;
        self.save_appearance(cx);
        self.dismiss_overlay(cx);
        self.focus_active(window, cx);
        cx.notify();
    }

    pub(crate) fn focus_active(&self, window: &mut Window, cx: &App) {
        if let Some(pane) = self.active_pane().and_then(|id| self.grids.get(&id)) {
            pane.view.read(cx).focus.focus(window);
        } else {
            self.focus.focus(window);
        }
    }
}

fn action_handlers(cx: &mut Context<AppModel>) -> gpui::Div {
    div()
        .on_action(cx.listener(|model, _: &Find, window, cx| model.find_terminal(window, cx)))
        .on_action(cx.listener(|model, _: &FindNext, _, cx| model.step_find(false, cx)))
        .on_action(cx.listener(|model, _: &FindPrevious, _, cx| model.step_find(true, cx)))
        .key_context("WorkspaceTabs")
        .on_mouse_down(
            gpui::MouseButton::Navigate(gpui::NavigationDirection::Back),
            cx.listener(|model, _, _, cx| model.navigate(false, cx)),
        )
        .on_mouse_down(
            gpui::MouseButton::Navigate(gpui::NavigationDirection::Forward),
            cx.listener(|model, _, _, cx| model.navigate(true, cx)),
        )
        .on_action(cx.listener(|model, _: &NewTab, _, cx| model.new_tab(cx)))
        .on_action(
            cx.listener(|model, _: &SplitRight, _, cx| model.split_pane(Direction::Right, cx)),
        )
        .on_action(cx.listener(|model, _: &SplitDown, _, cx| model.split_pane(Direction::Down, cx)))
        .on_action(cx.listener(|model, _: &ToggleZoomPane, _, cx| model.toggle_zoom_pane(cx)))
        .on_action(cx.listener(|model, _: &ClosePane, _, cx| {
            if let Some(pane) = model.active_pane() {
                model.close_pane(pane, cx);
            }
        }))
        .on_action(cx.listener(|model, _: &FocusPaneLeft, _, cx| {
            model.focus_direction(Direction::Left, cx);
        }))
        .on_action(cx.listener(|model, _: &FocusPaneRight, _, cx| {
            model.focus_direction(Direction::Right, cx);
        }))
        .on_action(
            cx.listener(|model, _: &FocusPaneUp, _, cx| model.focus_direction(Direction::Up, cx)),
        )
        .on_action(cx.listener(|model, _: &FocusPaneDown, _, cx| {
            model.focus_direction(Direction::Down, cx);
        }))
        .on_action(cx.listener(|model, _: &Quit, _, cx| {
            model.quit(cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|model, _: &EndAllSessionsAndQuit, _, cx| {
            model.end_all_and_quit(cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|model, _: &NewHomeTab, _, cx| {
            model.select_project(model.state.home().id, cx);
            model.new_tab(cx);
        }))
        .on_action(cx.listener(|model, _: &PreviousProject, _, cx| model.cycle_project(false, cx)))
        .on_action(cx.listener(|model, _: &NextProject, _, cx| model.cycle_project(true, cx)))
        .on_action(
            cx.listener(|model, _: &AddProject, window, cx| model.open_project_picker(window, cx)),
        )
        .on_action(cx.listener(|model, _: &CloseTab, _, cx| {
            if let Some(tab) = model.active_tab() {
                model.close_tab(tab, cx);
            }
        }))
        .on_action(cx.listener(|model, _: &NextTab, _, cx| model.cycle_tab(true, cx)))
        .on_action(cx.listener(|model, _: &PreviousTab, _, cx| model.cycle_tab(false, cx)))
        .on_action(cx.listener(|model, action: &SelectTab, _, cx| {
            if let Some(tab) = model.state.current_project().tabs.get(action.index) {
                model.select_tab(tab.id, cx);
            }
        }))
        .on_action(
            cx.listener(|model, _: &ToggleSidebar, window, cx| model.toggle_sidebar(window, cx)),
        )
        .on_action(cx.listener(|_, _: &ToggleFullScreen, window, _| window.toggle_fullscreen()))
        .on_action(cx.listener(|model, _: &ToggleThemePicker, window, cx| {
            model.open_theme_picker(window, cx);
        }))
        .on_action(cx.listener(|model, _: &NavigateBack, _, cx| model.navigate(false, cx)))
        .on_action(cx.listener(|model, _: &NavigateForward, _, cx| model.navigate(true, cx)))
        .on_action(cx.listener(|_, _: &Minimize, window, _| window.minimize_window()))
        .on_action(cx.listener(|_, _: &Zoom, window, _| window.zoom_window()))
        .on_action(cx.listener(|model, _: &IncreaseFontSize, _, cx| model.zoom_terminal(1.0, cx)))
        .on_action(cx.listener(|model, _: &DecreaseFontSize, _, cx| model.zoom_terminal(-1.0, cx)))
}

impl Render for AppModel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.overlay.is_none() && (self.focus_requested || self.active_pane().is_none()) {
            self.focus_active(window, cx);
            self.focus_requested = false;
        }
        let active = self.active_pane();
        let zoomed = self
            .state
            .current_project()
            .tabs
            .iter()
            .find(|tab| Some(tab.id) == self.active_tab())
            .and_then(|tab| tab.zoomed);
        let split = self.visible_panes().len() > 1;
        if self.overlay.is_some() || self.close_prompt.is_some() {
            self.split_resize.end();
        }
        for (id, pane) in &self.grids {
            pane.view.update(cx, |pane, cx| {
                pane.set_focused(Some(*id) == active, cx);
                let border = (pane.focused && split).then_some(self.theme.accent);
                let radius = if Some(*id) == zoomed {
                    (self.metrics.radius_lg() - px(1.0)).max(px(0.0))
                } else {
                    px(0.0)
                };
                if pane.focus_border != border || pane.corner_radius != radius {
                    pane.focus_border = border;
                    pane.corner_radius = radius;
                    cx.notify();
                }
                pane.native_visible = self.overlay.is_none() && self.close_prompt.is_none();
                #[cfg(target_os = "macos")]
                if let Some(scroll) = &pane.native_scroll {
                    scroll.set_visible(pane.native_visible && pane.grid.is_some());
                }
            });
        }
        let theme = &self.theme;
        let sidebar_width = if self.appearance.sidebar_expanded {
            220.0
        } else {
            44.0
        };
        let content = super::splits::render(self, cx).unwrap_or_else(|| empty(self, cx));
        let error = self.error.as_ref().map(|error| {
            div()
                .px(px(12.0))
                .py(px(8.0))
                .bg(theme.surface)
                .text_color(theme.fg)
                .text_size(px(12.0))
                .child(error.clone())
        });
        action_handlers(cx)
            .track_focus(&self.focus)
            .relative()
            .flex()
            .size_full()
            .bg(theme.bg)
            .text_color(theme.fg)
            .font_family(".SystemUIFont")
            .line_height(relative(1.2))
            .child(sidebar::sidebar(self, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .child(tab_strip::tab_strip(self, sidebar_width, window, cx))
                    .child(div().h(px(1.0)).flex_none().bg(theme.border))
                    .children(error)
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_hidden()
                            .child(content),
                    )
                    .when(self.appearance.status_bar_visible, |column| {
                        column.child(status_bar::status_bar(self, cx))
                    }),
            )
            .child(
                div()
                    .absolute()
                    .top(px(if sidebar_width >= 153.0 { 0.0 } else { 33.0 }))
                    .bottom_0()
                    .left(px(sidebar_width - 1.0))
                    .w(px(1.0))
                    .bg(theme.border),
            )
            .child(titlebar::navigation(self, sidebar_width, cx))
            .child(overlays::layer(self, window, cx))
    }
}

fn empty(model: &AppModel, cx: &mut Context<AppModel>) -> gpui::AnyElement {
    let theme = &model.theme;
    let m = model.metrics;
    let missing = model.state.current_project().status() == muxy_app_core::ProjectStatus::Missing;
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(m.spacing7())
        .child(IconGlyph::new(
            Icon::AppWindow,
            m.icon_xxl(),
            theme.fg_muted,
        ))
        .child(
            div()
                .text_size(m.font_headline())
                .font_weight(FontWeight::SEMIBOLD)
                .child(if missing { "Project folder is missing".to_owned() } else { format!("No tabs in {}", model.state.current_project().name) }),
        )
        .child(
            div()
                .max_w(m.scaled(360.0))
                .text_center()
                .text_size(m.font_body())
                .text_color(theme.fg_muted)
                .child(if missing { "This project’s folder could not be found. Remove the project from the sidebar to clear its tabs." } else { "Open a new terminal tab to start working in this project." }),
        )
        .when(!missing, |view| view.child(
            div()
                .id("empty-new-tab")
                .flex()
                .items_center()
                .gap(m.spacing4())
                .px(m.spacing6())
                .py(m.spacing3())
                .rounded(m.radius_md())
                .bg(gpui::rgb(0x0a_7c_ff))
                .text_color(gpui::white())
                .text_size(m.font_body())
                .cursor_pointer()
                .hover(|style| style.opacity(0.85))
                .on_click(cx.listener(|model, _, _, cx| model.new_tab(cx)))
                .child("New Tab")
                .when_some(model.settings.keymap.chord(Action::NewTab), |element, chord| element.child(
                    div()
                        .text_size(m.font_footnote())
                        .font_weight(FontWeight::MEDIUM)
                        .opacity(0.72)
                        .child(chord.to_string()),
                )),
        ))
        .into_any_element()
}

#[cfg(test)]
mod shortcut_tests {
    use super::*;
    use muxy_core::shortcuts::{ALL, Defaults, ShortcutSettings};
    use std::cell::RefCell;
    use std::collections::BTreeSet;

    struct CatalogProbe(RefCell<BTreeSet<String>>);

    impl ShortcutSettings for CatalogProbe {
        fn keys(&self, id: &str, context: Option<&str>) -> Vec<String> {
            self.0.borrow_mut().insert(id.to_owned());
            Defaults.keys(id, context)
        }
    }

    #[test]
    fn every_catalog_action_has_a_registered_handler_and_uses_configured_bindings() {
        let probe = CatalogProbe(RefCell::default());
        let bindings = workspace_bindings(&probe);
        let registered = probe.0.into_inner();
        assert_eq!(
            registered,
            ALL.iter().map(|shortcut| shortcut.id.to_owned()).collect()
        );
        let expected: usize = ALL
            .iter()
            .flat_map(|shortcut| shortcut.key_contexts)
            .map(|scopes| scopes.len())
            .sum();
        assert_eq!(bindings.len(), expected);
        assert_eq!(workspace_bindings(&Keymap::default()).len(), expected);
    }
}

#[cfg(test)]
mod clipboard_shortcut_tests {
    use super::*;
    use gpui::{AppContext, ClipboardItem, Entity, Focusable, Render, TestAppContext};
    use muxy_core::shortcuts::{Defaults, ShortcutSettings};
    use muxy_ui::text_input::{InputStyle, TextInput};
    use muxy_ui::theme::{Metrics, Theme};

    struct Remapped;
    impl ShortcutSettings for Remapped {
        fn keys(&self, id: &str, context: Option<&str>) -> Vec<String> {
            match id {
                "text_input.copy" => vec!["ctrl-k".into()],
                "text_input.paste" => vec!["ctrl-j".into()],
                _ => Defaults.keys(id, context),
            }
        }
    }

    struct TextWorkspace(Entity<TextInput>);
    impl Render for TextWorkspace {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().key_context("WorkspaceTabs").child(self.0.clone())
        }
    }

    #[gpui::test]
    fn component_clipboard_remaps_replace_old_workspace_keys(cx: &mut TestAppContext) {
        cx.update(|cx| cx.bind_keys(workspace_bindings(&Remapped)));
        for context in [
            muxy_ui::text_input::DEFAULT_CONTEXT,
            muxy_ui::text_input::BARE_CONTEXT,
            muxy_ui::text_input::MULTILINE_CONTEXT,
            muxy_ui::text_input::SEARCH_CONTEXT,
        ] {
            assert_clipboard_remap(cx, context);
        }
    }

    fn assert_clipboard_remap(cx: &mut TestAppContext, context: &'static str) {
        let (workspace, cx) = cx.add_window_view(|window, cx| {
            let input = cx.new(|cx| {
                TextInput::new(
                    InputStyle::field(
                        &Theme::from_scheme(&muxy_ui::theme::ColorScheme::default()),
                        &Metrics::new(1.0),
                    ),
                    cx,
                )
                .with_text("original")
                .with_key_context(context)
            });
            input.read(cx).focus_handle(cx).focus(window);
            TextWorkspace(input)
        });
        let input = workspace.read_with(cx, |workspace, _| workspace.0.clone());
        cx.simulate_keystrokes("cmd-a");
        cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into())));
        cx.simulate_keystrokes("cmd-c");
        assert_eq!(
            cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text())),
            Some("sentinel".into())
        );
        cx.simulate_keystrokes("ctrl-k");
        assert_eq!(
            cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text())),
            Some("original".into())
        );
        cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("replacement".into())));
        cx.simulate_keystrokes("cmd-v");
        assert_eq!(
            input.read_with(cx, |input, _| input.text().to_owned()),
            "original"
        );
        cx.simulate_keystrokes("ctrl-j");
        assert_eq!(
            input.read_with(cx, |input, _| input.text().to_owned()),
            "replacement"
        );
    }
}
