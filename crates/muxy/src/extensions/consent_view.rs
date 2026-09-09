use crate::state::AppState;
use crate::views::window::MainWindow;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div,
};
use muxy_api::extensions::{ConsentChoice, ConsentRequest};
use muxy_core::extensions::state::{ExtensionGatedVerb, ExtensionGrantMatch};

pub(crate) fn layer(
    request: &ConsentRequest,
    block_kind: bool,
    state: &AppState,
    cx: &mut Context<MainWindow>,
) -> AnyElement {
    let metrics = &state.metrics;
    let theme = &state.theme;
    let title = format!("Allow {}?", request.extension_display_name);
    let rule = if block_kind {
        format!("all {}", verb_kind(request.verb))
    } else {
        match_description(&request.suggested_match)
    };
    let details = request
        .payload_details
        .iter()
        .map(|detail| {
            div()
                .text_size(metrics.font_footnote())
                .text_color(theme.fg)
                .child(SharedString::from(detail.clone()))
        })
        .collect::<Vec<_>>();

    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .bg(gpui::hsla(0.0, 0.0, 0.0, 0.35))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .id("extension-consent-dialog")
                .w(metrics.scaled(520.0))
                .max_h(metrics.scaled(620.0))
                .flex()
                .flex_col()
                .p(metrics.spacing6())
                .gap(metrics.spacing5())
                .rounded(metrics.radius_lg())
                .bg(theme.raised())
                .border_1()
                .border_color(theme.border)
                .shadow_lg()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(metrics.spacing2())
                        .child(
                            div()
                                .text_size(metrics.font_body())
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.fg)
                                .child(SharedString::from(title)),
                        )
                        .child(
                            div()
                                .text_size(metrics.font_footnote())
                                .text_color(theme.fg_muted)
                                .child(SharedString::from(verb_description(request.verb))),
                        ),
                )
                .child(
                    div()
                        .id("extension-consent-details")
                        .max_h(metrics.scaled(260.0))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap(metrics.spacing2())
                        .p(metrics.spacing4())
                        .rounded(metrics.radius_sm())
                        .bg(theme.bg)
                        .border_1()
                        .border_color(theme.border)
                        .children(details),
                )
                .child(
                    div()
                        .id("extension-consent-block-kind")
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(metrics.spacing3())
                        .cursor_pointer()
                        .on_click(cx.listener(|window: &mut MainWindow, _, _, cx| {
                            window.toggle_extension_consent_block_kind(cx);
                        }))
                        .child(
                            div()
                                .w(metrics.scaled(16.0))
                                .h(metrics.scaled(16.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(metrics.radius_sm())
                                .border_1()
                                .border_color(if block_kind {
                                    theme.accent
                                } else {
                                    theme.border
                                })
                                .when(block_kind, |box_element| {
                                    box_element.bg(theme.accent).child(
                                        div()
                                            .text_size(metrics.font_caption())
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme.bg)
                                            .child("✓"),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(metrics.spacing1())
                                .child(
                                    div()
                                        .text_size(metrics.font_caption())
                                        .text_color(theme.fg)
                                        .child(SharedString::from(format!(
                                            "Block all {} from this extension",
                                            verb_kind(request.verb)
                                        ))),
                                )
                                .child(
                                    div()
                                        .text_size(metrics.font_caption())
                                        .text_color(theme.fg_muted)
                                        .child(SharedString::from(format!(
                                            "Remember saves rule: {rule}"
                                        ))),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(metrics.spacing3())
                        .child(
                            consent_button(
                                "Deny & remember",
                                "extension-consent-deny-remember",
                                false,
                                true,
                                state,
                            )
                            .on_click(cx.listener(
                                |window: &mut MainWindow, _, _, cx| {
                                    window.resolve_extension_consent(
                                        ConsentChoice::DenyAndRemember,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(metrics.spacing3())
                                .child(
                                    consent_button(
                                        "Cancel",
                                        "extension-consent-cancel",
                                        false,
                                        true,
                                        state,
                                    )
                                    .on_click(cx.listener(
                                        |window: &mut MainWindow, _, _, cx| {
                                            window.resolve_extension_consent(
                                                ConsentChoice::DenyOnce,
                                                cx,
                                            );
                                        },
                                    )),
                                )
                                .child(
                                    consent_button(
                                        "Allow",
                                        "extension-consent-allow",
                                        false,
                                        !block_kind,
                                        state,
                                    )
                                    .when(
                                        !block_kind,
                                        |button| {
                                            button.on_click(cx.listener(
                                                |window: &mut MainWindow, _, _, cx| {
                                                    window.resolve_extension_consent(
                                                        ConsentChoice::AllowOnce,
                                                        cx,
                                                    );
                                                },
                                            ))
                                        },
                                    ),
                                )
                                .child(
                                    consent_button(
                                        "Allow & remember",
                                        "extension-consent-allow-remember",
                                        true,
                                        !block_kind,
                                        state,
                                    )
                                    .when(
                                        !block_kind,
                                        |button| {
                                            button.on_click(cx.listener(
                                                |window: &mut MainWindow, _, _, cx| {
                                                    window.resolve_extension_consent(
                                                        ConsentChoice::AllowAndRemember,
                                                        cx,
                                                    );
                                                },
                                            ))
                                        },
                                    ),
                                ),
                        ),
                ),
        )
        .into_any_element()
}

fn consent_button(
    label: &'static str,
    id: &'static str,
    primary: bool,
    enabled: bool,
    state: &AppState,
) -> gpui::Stateful<gpui::Div> {
    let metrics = &state.metrics;
    let theme = &state.theme;
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(metrics.control_medium())
        .px(metrics.spacing4())
        .rounded(metrics.radius_sm())
        .text_size(metrics.font_footnote())
        .font_weight(FontWeight::MEDIUM)
        .when(enabled, |button| button.cursor_pointer())
        .when(primary && enabled, |button| {
            button.bg(theme.accent).text_color(theme.bg)
        })
        .when(!primary && enabled, |button| {
            button
                .bg(theme.surface)
                .text_color(theme.fg)
                .border_1()
                .border_color(theme.border)
        })
        .when(!enabled, |button| {
            button
                .bg(theme.surface)
                .text_color(theme.fg_dim)
                .border_1()
                .border_color(theme.border)
        })
        .child(SharedString::from(label))
}

fn verb_description(verb: ExtensionGatedVerb) -> &'static str {
    match verb {
        ExtensionGatedVerb::Exec => "wants to run a shell command",
        ExtensionGatedVerb::PanesSend => "wants to type into a terminal",
        ExtensionGatedVerb::PanesSendKeys => "wants to press keys in a terminal",
        ExtensionGatedVerb::PanesReadScreen => "wants to read terminal output",
        ExtensionGatedVerb::TabsOpenForeign => "wants to open another extension's tab",
        ExtensionGatedVerb::TabsRunCommand => "wants to open a terminal that runs a command",
        ExtensionGatedVerb::RemoteInvoke => "wants to serve a mobile request",
        ExtensionGatedVerb::GitWrite => "wants to modify the Git repository",
        ExtensionGatedVerb::FilesWrite => "wants to modify workspace files",
        ExtensionGatedVerb::HttpFetch => "wants to make a network request",
        ExtensionGatedVerb::ProjectsDelete => "wants to delete a project",
    }
}

fn verb_kind(verb: ExtensionGatedVerb) -> &'static str {
    match verb {
        ExtensionGatedVerb::Exec => "shell commands",
        ExtensionGatedVerb::PanesSend => "terminal input",
        ExtensionGatedVerb::PanesSendKeys => "terminal key presses",
        ExtensionGatedVerb::PanesReadScreen => "terminal reads",
        ExtensionGatedVerb::TabsOpenForeign => "foreign extension tabs",
        ExtensionGatedVerb::TabsRunCommand => "startup commands",
        ExtensionGatedVerb::RemoteInvoke => "remote invocations",
        ExtensionGatedVerb::GitWrite => "Git writes",
        ExtensionGatedVerb::FilesWrite => "file writes",
        ExtensionGatedVerb::HttpFetch => "HTTP requests",
        ExtensionGatedVerb::ProjectsDelete => "project deletions",
    }
}

fn match_description(match_rule: &ExtensionGrantMatch) -> String {
    match match_rule {
        ExtensionGrantMatch::Any => "all requests of this kind".to_owned(),
        ExtensionGrantMatch::ArgvExact { value } => format!("argv equals {}", value.join(" ")),
        ExtensionGrantMatch::ArgvPrefix { value } => {
            format!("argv starts with {}", value.join(" "))
        }
        ExtensionGrantMatch::ShellExact { string } => format!("shell equals {string}"),
        ExtensionGrantMatch::PaneEquals { string } => format!("pane equals {string}"),
        ExtensionGrantMatch::ForeignTabEquals { target, string } => {
            format!("tab equals {target}:{string}")
        }
        ExtensionGrantMatch::RemoteActionEquals { string } => {
            format!("remote action equals {string}")
        }
        ExtensionGrantMatch::GitOperationEquals { string } => {
            format!("Git operation equals {string}")
        }
        ExtensionGrantMatch::FileOperationEquals { string } => {
            format!("file operation equals {string}")
        }
        ExtensionGrantMatch::HostEquals { string } => format!("host equals {string}"),
        ExtensionGrantMatch::ProjectNameEquals { string } => {
            format!("project equals {string}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_copy_covers_every_gated_verb_and_match() {
        for verb in [
            ExtensionGatedVerb::Exec,
            ExtensionGatedVerb::PanesSend,
            ExtensionGatedVerb::PanesSendKeys,
            ExtensionGatedVerb::PanesReadScreen,
            ExtensionGatedVerb::TabsOpenForeign,
            ExtensionGatedVerb::TabsRunCommand,
            ExtensionGatedVerb::RemoteInvoke,
            ExtensionGatedVerb::GitWrite,
            ExtensionGatedVerb::FilesWrite,
            ExtensionGatedVerb::HttpFetch,
            ExtensionGatedVerb::ProjectsDelete,
        ] {
            assert!(!verb_description(verb).is_empty());
            assert!(!verb_kind(verb).is_empty());
        }
        assert_eq!(
            match_description(&ExtensionGrantMatch::HostEquals {
                string: "example.com".to_owned()
            }),
            "host equals example.com"
        );
    }
}
