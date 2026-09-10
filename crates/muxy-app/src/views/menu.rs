use muxy_core::shortcuts::ShortcutId;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Pixels, Point,
    SharedString, StatefulInteractiveElement, Styled, Window, actions, div, px,
};
use muxy_ui::components::SymbolGlyph;

use super::overlays::Overlay;
use crate::model::AppModel;

actions!(
    menu,
    [
        DismissMenu,
        HighlightPrevious,
        HighlightNext,
        ConfirmHighlighted
    ]
);

pub(crate) fn register_shortcuts(registry: &mut muxy_ui::shortcuts::Registry<'_>) {
    registry.register(ShortcutId::MenuDismissMenu, &DismissMenu);
    registry.register(ShortcutId::MenuHighlightPrevious, &HighlightPrevious);
    registry.register(ShortcutId::MenuHighlightNext, &HighlightNext);
    registry.register(ShortcutId::MenuConfirmHighlighted, &ConfirmHighlighted);
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Command {
    Dismiss,
    TerminalCopy(muxy_app_core::PaneId),
    TerminalPaste(muxy_app_core::PaneId),
    TerminalSelectAll(muxy_app_core::PaneId),
    TerminalSelectCommandOutput(muxy_app_core::PaneId),
    CopyPath(muxy_app_core::ProjectId),
    RevealPath(muxy_app_core::ProjectId),
    EditProject(muxy_app_core::ProjectId, super::project_editor::Field),
    ProjectColor(muxy_app_core::ProjectId),
    RemoveProject(muxy_app_core::ProjectId),
}

#[derive(Clone, Debug)]
pub(crate) struct Item {
    label: &'static str,
    command: Command,
    disabled: bool,
    checked: bool,
}

impl Item {
    pub(crate) fn action(label: &'static str, command: Command) -> Self {
        Self {
            label,
            command,
            disabled: false,
            checked: false,
        }
    }

    pub(crate) fn checked(mut self) -> Self {
        self.checked = true;
        self
    }
    pub(crate) fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }
}

#[derive(Debug)]
pub(crate) struct Menu {
    pub(crate) items: Vec<Item>,
    pub(crate) position: Point<Pixels>,
    highlighted: Option<usize>,
}

impl Menu {
    pub(crate) fn new(items: Vec<Item>, position: Point<Pixels>) -> Self {
        Self {
            items,
            position,
            highlighted: None,
        }
    }

    fn move_highlight(&mut self, forward: bool) {
        let selectable: Vec<_> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| !item.disabled)
            .map(|(index, _)| index)
            .collect();
        if selectable.is_empty() {
            self.highlighted = None;
            return;
        }
        let current = self
            .highlighted
            .and_then(|index| selectable.iter().position(|item| *item == index));
        let next = match (current, forward) {
            (Some(index), true) => (index + 1) % selectable.len(),
            (Some(index), false) => (index + selectable.len() - 1) % selectable.len(),
            (None, true) => 0,
            (None, false) => selectable.len() - 1,
        };
        self.highlighted = Some(selectable[next]);
    }
}

impl AppModel {
    fn move_menu_highlight(&mut self, forward: bool, cx: &mut Context<Self>) {
        if let Some(Overlay::Menu(menu)) = &mut self.overlay {
            menu.move_highlight(forward);
            cx.notify();
        }
    }

    fn confirm_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let command = match &self.overlay {
            Some(Overlay::Menu(menu)) => menu
                .highlighted
                .and_then(|index| menu.items.get(index))
                .filter(|item| !item.disabled)
                .map(|item| item.command),
            _ => None,
        };
        if let Some(command) = command {
            self.perform_menu(command, window, cx);
        }
    }

    fn perform_menu(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        let position = match &self.overlay {
            Some(Overlay::Menu(menu)) => menu.position,
            _ => gpui::point(px(8.0), px(40.0)),
        };
        self.dismiss_overlay(cx);
        match command {
            Command::Dismiss => {}
            Command::TerminalCopy(id)
            | Command::TerminalPaste(id)
            | Command::TerminalSelectAll(id)
            | Command::TerminalSelectCommandOutput(id) => {
                if let Some(pane) = self.grids.get(&id) {
                    pane.view.update(cx, |pane, cx| match command {
                        Command::TerminalCopy(_) => pane.copy_selection(cx),
                        Command::TerminalPaste(_) => pane.paste_clipboard(cx),
                        Command::TerminalSelectAll(_) => pane.select_all(cx),
                        Command::TerminalSelectCommandOutput(_) => {
                            pane.select_command_output(Some(position), cx);
                        }
                        _ => {}
                    });
                }
            }
            Command::CopyPath(id) => {
                if let Some(project) = self.state.project(id)
                    && project.status() == muxy_app_core::ProjectStatus::Available
                {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                        project.directory.to_string_lossy().into_owned(),
                    ));
                }
            }
            Command::RevealPath(id) => {
                if let Some(project) = self.state.project(id)
                    && project.status() == muxy_app_core::ProjectStatus::Available
                {
                    cx.reveal_path(&project.directory);
                }
            }
            Command::EditProject(id, field) => {
                self.open_project_editor(id, field, position, window, cx);
                return;
            }
            Command::ProjectColor(id) => {
                self.open_project_colors(id, position, window, cx);
                return;
            }
            Command::RemoveProject(id) => self.confirm_remove_project(id, cx),
        }
        self.focus_active(window, cx);
    }
}

pub(crate) fn render(
    menu: &Menu,
    model: &AppModel,
    window: &Window,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let m = model.metrics;
    let theme = &model.theme;
    let count = f32::from(u16::try_from(menu.items.len()).unwrap_or(u16::MAX));
    let origin = super::overlays::clamp(
        menu.position,
        gpui::size(px(180.0), px(count * 22.0 + 10.0)),
        window.viewport_size(),
    );
    let mut panel = div()
        .key_context("Menu")
        .track_focus(&model.overlay_focus)
        .on_action(cx.listener(|model, _: &DismissMenu, _, cx| model.dismiss_overlay(cx)))
        .on_action(
            cx.listener(|model, _: &HighlightPrevious, _, cx| model.move_menu_highlight(false, cx)),
        )
        .on_action(
            cx.listener(|model, _: &HighlightNext, _, cx| model.move_menu_highlight(true, cx)),
        )
        .on_action(
            cx.listener(|model, _: &ConfirmHighlighted, window, cx| model.confirm_menu(window, cx)),
        )
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .occlude()
        .absolute()
        .left(origin.x)
        .top(origin.y)
        .flex()
        .flex_col()
        .min_w(px(180.0))
        .py(m.spacing2())
        .rounded(m.radius_lg())
        .bg(theme.raised())
        .border_1()
        .border_color(theme.border)
        .shadow_lg();
    for (index, item) in menu.items.iter().enumerate() {
        let command = item.command;
        let mut mark = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(px(12.0));
        if item.checked {
            mark = mark.child(SymbolGlyph::new("checkmark", m.font_caption(), theme.fg));
        }
        panel = panel.child(
            div()
                .id(SharedString::from(format!("menu-item-{index}")))
                .debug_selector(move || format!("menu-item-{index}"))
                .flex()
                .items_center()
                .gap(m.spacing2())
                .h(px(22.0))
                .px(m.spacing3())
                .mx(m.spacing2())
                .rounded(m.radius_sm())
                .font_weight(FontWeight::NORMAL)
                .when(menu.highlighted == Some(index) && !item.disabled, |row| {
                    row.bg(theme.fg_alpha(0.1))
                })
                .child(mark)
                .child(
                    div()
                        .flex_grow()
                        .text_size(m.font_emphasis())
                        .text_color(if item.disabled {
                            theme.fg_dim
                        } else {
                            theme.fg
                        })
                        .child(item.label),
                )
                .when(!item.disabled, |row| {
                    row.cursor_pointer()
                        .hover(|style| style.bg(theme.fg_alpha(0.1)))
                        .on_click(cx.listener(move |model, _, window, cx| {
                            model.perform_menu(command, window, cx);
                        }))
                }),
        );
    }
    panel.into_any_element()
}
