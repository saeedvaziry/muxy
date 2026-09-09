use crate::theme::{Metrics, Theme};
use gpui::{AnyElement, InteractiveElement, IntoElement, ParentElement, RenderOnce, Styled, div};

#[derive(IntoElement)]
pub struct PopoverSurface {
    theme: Theme,
    metrics: Metrics,
    width: f32,
    height: f32,
    content: AnyElement,
}

impl PopoverSurface {
    pub fn new(
        theme: Theme,
        metrics: Metrics,
        width: f32,
        height: f32,
        content: impl IntoElement,
    ) -> Self {
        Self {
            theme,
            metrics,
            width,
            height,
            content: content.into_any_element(),
        }
    }
}

impl RenderOnce for PopoverSurface {
    fn render(self, _window: &mut gpui::Window, _cx: &mut gpui::App) -> impl IntoElement {
        div()
            .occlude()
            .flex()
            .flex_col()
            .w(self.metrics.scaled(self.width))
            .h(self.metrics.scaled(self.height))
            .overflow_hidden()
            .rounded(self.metrics.radius_lg())
            .border_1()
            .border_color(self.theme.border)
            .bg(self.theme.bg)
            .shadow_lg()
            .child(self.content)
    }
}

impl std::fmt::Debug for PopoverSurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PopoverSurface")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}
