use muxy_core::shortcuts::ShortcutId;

use gpui::{
    AnyElement, AppContext, Context, Entity, Focusable, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Pixels, Point, StatefulInteractiveElement, Styled, Window, actions,
    div, px, size,
};
use muxy_app_core::{PROJECT_COLORS, ProjectId, ProjectStatus};
use muxy_ui::text_input::{InputEvent, InputStyle, TextInput};
use muxy_ui::theme::parse_hex;

use super::overlays::{Overlay, clamp};
use crate::model::AppModel;

actions!(
    project_colors,
    [PreviousColor, NextColor, ChooseColor, DismissColors]
);

pub(crate) fn register_shortcuts(registry: &mut muxy_ui::shortcuts::Registry<'_>) {
    registry.register(ShortcutId::ProjectColorsPreviousColor, &PreviousColor);
    registry.register(ShortcutId::ProjectColorsNextColor, &NextColor);
    registry.register(ShortcutId::ProjectColorsChooseColor, &ChooseColor);
    registry.register(ShortcutId::ProjectColorsDismissColors, &DismissColors);
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Field {
    Name,
    Icon,
}

pub(crate) struct Editor {
    project: ProjectId,
    field: Field,
    input: Entity<TextInput>,
    position: Point<Pixels>,
    error: Option<String>,
}

pub(crate) struct Colors {
    project: ProjectId,
    position: Point<Pixels>,
    selected: usize,
}

impl AppModel {
    pub(crate) fn open_project_editor(
        &mut self,
        id: ProjectId,
        field: Field,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self
            .state
            .project(id)
            .filter(|project| project.status() == ProjectStatus::Available)
        else {
            return;
        };
        let text = match field {
            Field::Name => project.name.clone(),
            Field::Icon => project.icon.clone().unwrap_or_default(),
        };
        let input = cx.new(|cx| {
            TextInput::new(InputStyle::field(&self.theme, &self.metrics), cx).with_text(text)
        });
        input.update(cx, TextInput::select_all_text);
        input.focus_handle(cx).focus(window);
        self.overlay_subscription = Some(cx.subscribe(&input, |model, _, event, cx| match event {
            InputEvent::Submitted => model.submit_project_editor(cx),
            InputEvent::Cancelled => model.dismiss_overlay(cx),
            InputEvent::Changed => {
                if let Some(Overlay::ProjectEditor(editor)) = &mut model.overlay {
                    editor.error = None;
                }
                cx.notify();
            }
        }));
        self.overlay = Some(Overlay::ProjectEditor(Editor {
            project: id,
            field,
            input,
            position,
            error: None,
        }));
        cx.notify();
    }

    fn submit_project_editor(&mut self, cx: &mut Context<Self>) {
        let Some(Overlay::ProjectEditor(editor)) = &self.overlay else {
            return;
        };
        let id = editor.project;
        let field = editor.field;
        let text = editor.input.read(cx).text().trim().to_owned();
        if self.edit_project(
            |state| match field {
                Field::Name => state.rename_project(id, &text),
                Field::Icon => state.set_project_icon(id, (!text.is_empty()).then_some(text)),
            },
            cx,
        ) {
            self.dismiss_overlay(cx);
        } else if let Some(Overlay::ProjectEditor(editor)) = &mut self.overlay {
            editor.error.clone_from(&self.error);
        }
        cx.notify();
    }

    pub(crate) fn open_project_colors(
        &mut self,
        id: ProjectId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self
            .state
            .project(id)
            .filter(|project| project.status() == ProjectStatus::Available)
        else {
            return;
        };
        let selected = PROJECT_COLORS
            .iter()
            .position(|(_, color)| *color == project.color.as_str())
            .unwrap_or(0);
        self.overlay_subscription = None;
        self.overlay = Some(Overlay::ProjectColors(Colors {
            project: id,
            position,
            selected,
        }));
        self.overlay_focus.focus(window);
        cx.notify();
    }

    fn move_color(&mut self, forward: bool, cx: &mut Context<Self>) {
        if let Some(Overlay::ProjectColors(colors)) = &mut self.overlay {
            colors.selected = (colors.selected
                + if forward { 1 } else { PROJECT_COLORS.len() - 1 })
                % PROJECT_COLORS.len();
            cx.notify();
        }
    }

    fn choose_color(&mut self, selected: Option<usize>, cx: &mut Context<Self>) {
        let Some(Overlay::ProjectColors(colors)) = &self.overlay else {
            return;
        };
        let id = colors.project;
        let index = selected.unwrap_or(colors.selected);
        if let Some((_, hex)) = PROJECT_COLORS.get(index)
            && let Ok(color) = hex.parse()
            && self.edit_project(|state| state.set_project_color(id, color), cx)
        {
            self.dismiss_overlay(cx);
        }
    }
}

pub(crate) fn render(
    editor: &Editor,
    model: &AppModel,
    window: &Window,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let m = model.metrics;
    let theme = &model.theme;
    let origin = clamp(
        editor.position,
        size(px(300.0), px(190.0)),
        window.viewport_size(),
    );
    div()
        .absolute()
        .left(origin.x)
        .top(origin.y)
        .w(px(300.0))
        .flex()
        .flex_col()
        .gap(m.spacing4())
        .p(m.spacing5())
        .rounded(m.radius_lg())
        .bg(theme.raised())
        .border_1()
        .border_color(theme.border)
        .shadow_lg()
        .occlude()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .text_size(m.font_body())
                .font_weight(FontWeight::SEMIBOLD)
                .child(match editor.field {
                    Field::Name => "Rename Project",
                    Field::Icon => "Change Icon",
                }),
        )
        .child(editor.input.clone())
        .children(matches!(editor.field, Field::Icon).then(|| {
            div()
                .text_size(m.font_footnote())
                .text_color(theme.fg_muted)
                .child("Enter one emoji, or leave empty to use the default.")
        }))
        .children(editor.error.as_ref().map(|error| {
            div()
                .text_size(m.font_footnote())
                .text_color(theme.danger)
                .child(error.clone())
        }))
        .child(
            div().flex().justify_end().child(
                div()
                    .id("save-project-editor")
                    .cursor_pointer()
                    .px(m.spacing4())
                    .py(m.spacing2())
                    .rounded(m.radius_md())
                    .bg(theme.accent)
                    .text_color(theme.accent_foreground)
                    .text_size(m.font_body())
                    .child("Save")
                    .on_click(cx.listener(|model, _, _, cx| model.submit_project_editor(cx))),
            ),
        )
        .into_any_element()
}

pub(crate) fn render_colors(
    colors: &Colors,
    model: &AppModel,
    window: &Window,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let theme = &model.theme;
    let m = model.metrics;
    let origin = clamp(
        colors.position,
        size(px(284.0), px(84.0)),
        window.viewport_size(),
    );
    div()
        .key_context("ProjectColors")
        .track_focus(&model.overlay_focus)
        .on_action(cx.listener(|model, _: &PreviousColor, _, cx| model.move_color(false, cx)))
        .on_action(cx.listener(|model, _: &NextColor, _, cx| model.move_color(true, cx)))
        .on_action(cx.listener(|model, _: &ChooseColor, _, cx| model.choose_color(None, cx)))
        .on_action(cx.listener(|model, _: &DismissColors, _, cx| model.dismiss_overlay(cx)))
        .absolute()
        .left(origin.x)
        .top(origin.y)
        .flex()
        .flex_col()
        .gap(m.spacing3())
        .p(m.spacing4())
        .rounded(m.radius_lg())
        .bg(theme.raised())
        .border_1()
        .border_color(theme.border)
        .shadow_lg()
        .occlude()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(div().text_size(m.font_body()).child(format!(
            "Project Color · {}",
            PROJECT_COLORS[colors.selected].0
        )))
        .child(
            div()
                .flex()
                .gap(px(4.0))
                .children(PROJECT_COLORS.iter().enumerate().map(|(index, (_, hex))| {
                    div()
                        .id(("project-color", index))
                        .cursor_pointer()
                        .size(px(28.0))
                        .rounded(m.radius_md())
                        .border(px(2.0))
                        .border_color(if colors.selected == index {
                            theme.fg
                        } else {
                            theme.border
                        })
                        .bg(parse_hex(hex).unwrap_or(gpui::rgb(0)))
                        .hover(|style| style.opacity(0.8))
                        .on_click(
                            cx.listener(move |model, _, _, cx| model.choose_color(Some(index), cx)),
                        )
                })),
        )
        .into_any_element()
}
