use gpui::{Context, InteractiveElement, ParentElement, StatefulInteractiveElement, Styled, div};
use muxy_protocol::ExitReason;

use super::terminal::pane::PaneState;
use crate::model::AppModel;

pub(crate) fn label(state: PaneState) -> Option<String> {
    match state {
        PaneState::Live => None,
        PaneState::Connecting => Some("Connecting…".into()),
        PaneState::Disconnected => Some("Disconnected".into()),
        PaneState::Exited {
            unavailable: true, ..
        } => Some("Session exited · Saved output unavailable".into()),
        PaneState::Exited { reason, .. } => Some(match reason {
            Some(ExitReason::Exited(code)) => format!("Session exited · Exit code {code}"),
            Some(ExitReason::Signaled(signal)) => format!("Session exited · Signal {signal}"),
            Some(ExitReason::Ended | ExitReason::ServerStopped) | None => "Session exited".into(),
        }),
    }
}

pub(crate) fn status(model: &AppModel, cx: &mut Context<AppModel>) -> Option<gpui::Div> {
    let state = model.status(cx);
    let text = label(state)?;
    let m = model.metrics;
    let theme = &model.theme;
    let connect = (state == PaneState::Disconnected).then(|| {
        div()
            .id("connect-server")
            .cursor_pointer()
            .text_color(theme.fg)
            .on_click(cx.listener(|model, _, _, cx| model.connect(cx)))
            .child("Connect")
    });
    Some(
        div()
            .flex()
            .items_center()
            .gap(m.spacing4())
            .text_color(theme.fg_muted)
            .text_size(m.font_footnote())
            .child(text)
            .children(connect),
    )
}
