use gpui::{AnyWindowHandle, AsyncApp, Context};
use muxy_ui::dialog::ConfirmationResponse;

use crate::model::AppModel;

pub(crate) const TITLE: &str = "Close Tab?";
pub(crate) const MESSAGE: &str =
    "A process is still running in this tab.\nAre you sure you want to close it?";

pub(crate) const PANE_TITLE: &str = "Close Pane?";
pub(crate) const PANE_MESSAGE: &str =
    "A process is still running in this pane.\nAre you sure you want to close it?";

impl AppModel {
    pub(crate) fn confirm_close(&mut self, tab: muxy_app_core::TabId, cx: &mut Context<Self>) {
        if self.close_prompt.is_some() {
            return;
        }
        self.dismiss_overlay(cx);
        let window = self.window;
        let (title, message) = if self.closing_one_pane() {
            (PANE_TITLE, PANE_MESSAGE)
        } else {
            (TITLE, MESSAGE)
        };
        self.close_prompt = Some(cx.spawn(async move |model, cx| {
            let response = prompt(window, title, message, cx).await;
            let _ = model.update(cx, |model, cx| {
                model.finish_close_prompt(tab, response, cx);
            });
        }));
    }
}

#[cfg(not(test))]
async fn prompt(
    window: AnyWindowHandle,
    title: &'static str,
    message: &'static str,
    cx: &mut AsyncApp,
) -> Result<ConfirmationResponse, String> {
    let (sender, receiver) = async_channel::bounded(1);
    let _dialog = window
        .update(cx, |_, _, _| {
            muxy_ui::dialog::confirm(
                title,
                message,
                "Close",
                Some("Don't ask again"),
                move |response| {
                    let _ = sender.try_send(response);
                },
            )
        })
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    Ok(receiver.recv().await.unwrap_or_default())
}

#[cfg(test)]
pub(crate) const CLOSE_WITHOUT_ASKING: &str = "Close and don't ask again";

#[cfg(test)]
async fn prompt(
    window: AnyWindowHandle,
    title: &'static str,
    message: &'static str,
    cx: &mut AsyncApp,
) -> Result<ConfirmationResponse, String> {
    let answer = window
        .update(cx, |_, window, cx| {
            window.prompt(
                gpui::PromptLevel::Warning,
                title,
                Some(message),
                &["Close", "Cancel", CLOSE_WITHOUT_ASKING],
                cx,
            )
        })
        .map_err(|error| error.to_string())?;
    Ok(match answer.await {
        Ok(0) => ConfirmationResponse::Confirmed {
            dont_ask_again: false,
        },
        Ok(2) => ConfirmationResponse::Confirmed {
            dont_ask_again: true,
        },
        _ => ConfirmationResponse::Cancelled,
    })
}
