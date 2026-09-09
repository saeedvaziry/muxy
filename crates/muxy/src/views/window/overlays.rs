use super::*;

fn desktop_authorization_failure(
    result: crate::notifications::desktop::AuthorizationResult,
) -> &'static str {
    match result {
        crate::notifications::desktop::AuthorizationResult::Denied => "Permission was not granted.",
        crate::notifications::desktop::AuthorizationResult::Unavailable => {
            "Desktop notifications are unavailable on this system."
        }
        crate::notifications::desktop::AuthorizationResult::Failed => {
            "Muxy could not update desktop notification permission."
        }
        crate::notifications::desktop::AuthorizationResult::Allowed => "",
    }
}

fn repository_ai_workflow_effect(
    result: &Result<
        muxy_api::repository::RepositoryAiWorkflowOutcome,
        muxy_api::repository::RepositoryAiWorkflowError,
    >,
) -> muxy_api::repository::MutationEffect {
    use muxy_api::repository::{
        MutationEffect, RepositoryAiCompletedBoundary, RepositoryAiWorkflowOutcome,
    };
    match result {
        Ok(RepositoryAiWorkflowOutcome::Committed { .. })
        | Ok(RepositoryAiWorkflowOutcome::PullRequestCreated(_)) => MutationEffect::Uncertain,
        Err(error) => match error.completed {
            RepositoryAiCompletedBoundary::None => MutationEffect::NoMutation,
            RepositoryAiCompletedBoundary::Staged => MutationEffect::PartialSuccess {
                completed: "staging changes",
            },
            RepositoryAiCompletedBoundary::BranchCreated => MutationEffect::PartialSuccess {
                completed: "branch creation",
            },
            RepositoryAiCompletedBoundary::Committed => MutationEffect::PartialSuccess {
                completed: "commit creation",
            },
            RepositoryAiCompletedBoundary::Pushed => {
                MutationEffect::PartialSuccess { completed: "push" }
            }
            RepositoryAiCompletedBoundary::PullRequestCreated => MutationEffect::PartialSuccess {
                completed: "pull request creation",
            },
        },
    }
}

fn current_repository_ai_completion(
    completion: Option<crate::repository::MutationCompletion>,
    current_key: Option<&crate::repository::RepositoryKey>,
    expected_key: &crate::repository::RepositoryKey,
) -> Option<crate::repository::MutationCompletion> {
    completion.filter(|completion| completion.current_identity && current_key == Some(expected_key))
}

fn repository_ai_confirmation_is_current(
    identity: &crate::views::repository::ai::RepositoryAiConfirmationIdentity,
    current_key: Option<&crate::repository::RepositoryKey>,
    summary: &crate::repository::LoadState<muxy_api::repository::RepositorySummary>,
) -> bool {
    let crate::repository::LoadState::Ready(summary) = summary else {
        return false;
    };
    current_key == Some(&identity.key)
        && !summary.is_detached
        && summary.branch == identity.branch
        && summary.head == identity.head
}

impl MainWindow {
    pub(crate) fn open_commit_ai_confirmation(
        &mut self,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_repository_ai_confirmation(
            muxy_core::repository_ai::RepositoryAiAction::Commit,
            anchor,
            window,
            cx,
        );
    }

    pub(crate) fn open_create_pr_ai_confirmation(
        &mut self,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_repository_ai_confirmation(
            muxy_core::repository_ai::RepositoryAiAction::CreatePullRequest,
            anchor,
            window,
            cx,
        );
    }

    pub(crate) fn open_commit_ai_provider_menu(
        &mut self,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_repository_ai_provider_menu(
            muxy_core::repository_ai::RepositoryAiAction::Commit,
            anchor,
            window,
            cx,
        );
    }

    pub(crate) fn open_create_pr_ai_provider_menu(
        &mut self,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_repository_ai_provider_menu(
            muxy_core::repository_ai::RepositoryAiAction::CreatePullRequest,
            anchor,
            window,
            cx,
        );
    }

    fn open_repository_ai_confirmation(
        &mut self,
        action: muxy_core::repository_ai::RepositoryAiAction,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.view.repository.coordinator.state().ai
            == crate::repository::RepositoryAiRunState::Running(action)
        {
            self.view.repository.coordinator.cancel_ai_operation();
            cx.notify();
            return;
        }
        let Some(key) = self.view.repository.coordinator.key().cloned() else {
            return;
        };
        let crate::repository::LoadState::Ready(inventory) =
            &self.view.repository.coordinator.state().providers
        else {
            return;
        };
        let provider = match inventory.resolve_action(&self.state.prefs.repository_ai, action) {
            Ok(provider) => provider.descriptor.display_name.to_owned(),
            Err(error) => {
                self.feedback(
                    "AI Provider Unavailable",
                    error.to_string(),
                    crate::toast::ToastTone::Error,
                    cx,
                );
                return;
            }
        };
        let (branch, head) = match &self.view.repository.coordinator.state().summary {
            crate::repository::LoadState::Ready(summary) if !summary.is_detached => {
                (summary.branch.clone(), summary.head.clone())
            }
            _ => return,
        };
        let theme = self.state.theme.clone();
        let metrics = self.state.metrics;
        let panel = cx.new(|cx| {
            crate::views::repository::ai::RepositoryAiPanel::confirmation(
                crate::views::repository::ai::RepositoryAiConfirmationIdentity {
                    key,
                    branch,
                    head,
                },
                action,
                provider,
                theme,
                metrics,
                cx,
            )
        });
        let subscription = cx.subscribe(&panel, |window: &mut Self, _, event, cx| {
            window.handle_repository_ai_panel_event(event, cx);
        });
        window.focus(&panel.focus_handle(cx));
        self.view.subscriptions = vec![subscription];
        self.view.overlay = Overlay::Repository {
            kind: crate::views::overlay::RepositoryPopoverKind::Ai(Box::new(
                crate::views::repository::ai::RepositoryAiPopover::Panel(panel),
            )),
            anchor,
        };
        cx.notify();
    }

    fn open_repository_ai_provider_menu(
        &mut self,
        action: muxy_core::repository_ai::RepositoryAiAction,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.view.repository.coordinator.key().cloned() else {
            return;
        };
        let crate::repository::LoadState::Ready(inventory) =
            self.view.repository.coordinator.state().providers.clone()
        else {
            return;
        };
        let configured = self
            .state
            .prefs
            .repository_ai
            .action(action)
            .provider
            .clone();
        let theme = self.state.theme.clone();
        let metrics = self.state.metrics;
        let provider_size = crate::views::repository::ai::provider_popover_size();
        let picker = cx.new(|cx| {
            muxy_ui::command_popover::CommandPopover::new(
                muxy_ui::command_popover::CommandPopoverConfig {
                    id: "repository-ai-provider-picker".into(),
                    presentation: muxy_ui::command_popover::CommandPopoverPresentation::Popover,
                    density: muxy_ui::command_popover::CommandPopoverDensity::Compact,
                    tabs: vec![muxy_ui::command_popover::CommandPopoverTab::new(
                        "providers",
                        "Providers",
                    )],
                    placeholder: "Search providers…".into(),
                    footer_actions: if action
                        == muxy_core::repository_ai::RepositoryAiAction::CreatePullRequest
                    {
                        vec![muxy_ui::command_popover::CommandPopoverAction::new(
                            "edit-project-prompt",
                            "Project Prompt",
                        )]
                    } else {
                        Vec::new()
                    },
                    footer_hints: Vec::new(),
                    width: Some(provider_size.0),
                    height: Some(provider_size.1),
                    max_height: None,
                    completion_on_tab: false,
                    confirm_on_click: true,
                },
                theme,
                metrics,
                cx,
            )
        });
        crate::views::repository::ai::sync_provider_picker(
            &picker,
            &configured,
            &inventory,
            "",
            cx,
        );
        let subscription = cx.subscribe(&picker, move |window: &mut Self, picker, event, cx| {
            window.handle_repository_ai_provider_event(action, &picker, event, cx);
        });
        window.focus(&picker.focus_handle(cx));
        self.view.subscriptions = vec![subscription];
        self.view.overlay = Overlay::Repository {
            kind: crate::views::overlay::RepositoryPopoverKind::Ai(Box::new(
                crate::views::repository::ai::RepositoryAiPopover::Provider {
                    key,
                    action,
                    picker,
                },
            )),
            anchor,
        };
        cx.notify();
    }

    fn handle_repository_ai_provider_event(
        &mut self,
        action: muxy_core::repository_ai::RepositoryAiAction,
        picker: &Entity<muxy_ui::command_popover::CommandPopover>,
        event: &muxy_ui::command_popover::CommandPopoverEvent,
        cx: &mut Context<Self>,
    ) {
        use muxy_ui::command_popover::CommandPopoverEvent;
        match event {
            CommandPopoverEvent::QueryChanged { query, .. } => {
                let configured = self
                    .state
                    .prefs
                    .repository_ai
                    .action(action)
                    .provider
                    .clone();
                if let crate::repository::LoadState::Ready(inventory) =
                    &self.view.repository.coordinator.state().providers
                {
                    crate::views::repository::ai::sync_provider_picker(
                        picker,
                        &configured,
                        inventory,
                        query,
                        cx,
                    );
                }
            }
            CommandPopoverEvent::Confirmed(selection)
            | CommandPopoverEvent::SecondaryConfirmed(selection) => {
                let Some(provider) = selection.id.strip_prefix("provider:") else {
                    return;
                };
                let provider = if provider == "auto" { "" } else { provider };
                muxy_core::prefs::Prefs::store_settings_value(
                    action.provider_key(),
                    serde_json::Value::String(provider.to_owned()),
                );
                self.state.prefs.repository_ai.action_mut(action).provider = provider.to_owned();
                self.dismiss_overlay(cx);
            }
            CommandPopoverEvent::FooterAction(action_id)
                if action_id.as_ref() == "edit-project-prompt" =>
            {
                let anchor = match &self.view.overlay {
                    Overlay::Repository { anchor, .. } => *anchor,
                    _ => return,
                };
                self.open_project_pull_request_prompt(anchor, cx);
            }
            CommandPopoverEvent::Dismissed => self.dismiss_overlay(cx),
            _ => {}
        }
    }

    pub(super) fn sync_repository_ai_provider_picker(&mut self, cx: &mut Context<Self>) {
        let Some((action, picker)) = (match &self.view.overlay {
            Overlay::Repository {
                kind: crate::views::overlay::RepositoryPopoverKind::Ai(popover),
                ..
            } => match popover.as_ref() {
                crate::views::repository::ai::RepositoryAiPopover::Provider {
                    action,
                    picker,
                    ..
                } => Some((*action, picker.clone())),
                crate::views::repository::ai::RepositoryAiPopover::Panel(_) => None,
            },
            _ => None,
        }) else {
            return;
        };
        let configured = self
            .state
            .prefs
            .repository_ai
            .action(action)
            .provider
            .clone();
        let query = picker.read(cx).query().to_owned();
        if let crate::repository::LoadState::Ready(inventory) =
            &self.view.repository.coordinator.state().providers
        {
            crate::views::repository::ai::sync_provider_picker(
                &picker,
                &configured,
                inventory,
                &query,
                cx,
            );
        }
    }

    fn open_project_pull_request_prompt(&mut self, anchor: Bounds<Pixels>, cx: &mut Context<Self>) {
        let Some(key) = self.view.repository.coordinator.key().cloned() else {
            return;
        };
        let project_prompt = self
            .state
            .workspace
            .project(&key.project_id)
            .and_then(|project| project.pull_request_prompt.clone());
        let preferences = &self.state.prefs.repository_ai.create_pull_request;
        let fallback = muxy_core::repository_ai::normalized_prompt(Some(&preferences.prompt))
            .unwrap_or(muxy_core::repository_ai::CREATE_PULL_REQUEST_PROMPT)
            .to_owned();
        let theme = self.state.theme.clone();
        let metrics = self.state.metrics;
        let panel = cx.new(|cx| {
            crate::views::repository::ai::RepositoryAiPanel::project_prompt(
                key,
                project_prompt,
                fallback,
                theme,
                metrics,
                cx,
            )
        });
        let subscription = cx.subscribe(&panel, |window: &mut Self, _, event, cx| {
            window.handle_repository_ai_panel_event(event, cx);
        });
        self.view.subscriptions = vec![subscription];
        self.view.overlay = Overlay::Repository {
            kind: crate::views::overlay::RepositoryPopoverKind::Ai(Box::new(
                crate::views::repository::ai::RepositoryAiPopover::Panel(panel),
            )),
            anchor,
        };
        cx.notify();
    }

    fn handle_repository_ai_panel_event(
        &mut self,
        event: &crate::views::repository::ai::RepositoryAiPanelEvent,
        cx: &mut Context<Self>,
    ) {
        use crate::views::repository::ai::RepositoryAiPanelEvent;
        match event {
            RepositoryAiPanelEvent::Dismiss => self.dismiss_overlay(cx),
            RepositoryAiPanelEvent::SaveProjectPrompt(prompt) => {
                self.persist_project_pull_request_prompt(Some(prompt), cx)
            }
            RepositoryAiPanelEvent::UseGlobalPrompt => {
                self.persist_project_pull_request_prompt(None, cx)
            }
            RepositoryAiPanelEvent::Confirm {
                action,
                identity,
                additional_prompt,
            } => self.start_repository_ai_workflow(
                *action,
                identity.clone(),
                additional_prompt.clone(),
                cx,
            ),
        }
    }

    fn persist_project_pull_request_prompt(
        &mut self,
        prompt: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.view.repository.coordinator.key().cloned() else {
            return;
        };
        if self
            .state
            .workspace
            .set_pull_request_prompt(&key.project_id, prompt)
        {
            self.dismiss_overlay(cx);
        } else {
            self.feedback(
                "Could Not Save Project Prompt",
                "Muxy could not persist the project pull request prompt.",
                crate::toast::ToastTone::Error,
                cx,
            );
        }
    }

    fn start_repository_ai_workflow(
        &mut self,
        action: muxy_core::repository_ai::RepositoryAiAction,
        identity: crate::views::repository::ai::RepositoryAiConfirmationIdentity,
        additional_prompt: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if !repository_ai_confirmation_is_current(
            &identity,
            self.view.repository.coordinator.key(),
            &self.view.repository.coordinator.state().summary,
        ) {
            self.dismiss_overlay(cx);
            self.feedback(
                "Repository Context Changed",
                "The branch or commit changed after confirmation. Review the action and try again.",
                crate::toast::ToastTone::Warning,
                cx,
            );
            return;
        }
        let key = identity.key.clone();
        let project_prompt = self
            .state
            .workspace
            .project(&key.project_id)
            .and_then(|project| project.pull_request_prompt.clone());
        let request = muxy_api::repository::RepositoryAiWorkflowRequest {
            preferences: self.state.prefs.repository_ai.clone(),
            project_prompt,
            additional_prompt,
            home: muxy_core::prefs::home_dir(),
            normal_local_profile: true,
            expected_context: Some(muxy_api::repository::RepositoryAiExpectedContext {
                branch: identity.branch,
                head: identity.head,
            }),
        };
        let acquisition = self.view.repository.coordinator.begin_ai_operation(
            &mut self.state.project_operations,
            &key.project_id,
            action,
        );
        let (token, control) = match acquisition {
            Ok(acquisition) => acquisition,
            Err(error) => {
                self.feedback(
                    "Repository Action Unavailable",
                    error,
                    crate::toast::ToastTone::Warning,
                    cx,
                );
                return;
            }
        };
        self.dismiss_overlay(cx);
        let service = self.repository_service();
        let path = key.normalized_path.clone();
        cx.spawn(async move |window, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match action {
                        muxy_core::repository_ai::RepositoryAiAction::Commit => {
                            service.ai_commit_and_push(&path, &request, &control)
                        }
                        muxy_core::repository_ai::RepositoryAiAction::CreatePullRequest => {
                            service.ai_create_pull_request(&path, &request, &control)
                        }
                    }
                })
                .await;
            let _ =
                window.update(cx, |window, cx| {
                    let effect = repository_ai_workflow_effect(&result);
                    let completion = window.view.repository.coordinator.finish_ai_operation(
                        &mut window.state.project_operations,
                        &token,
                        effect,
                    );
                    if let Some(completion) = current_repository_ai_completion(
                        completion,
                        window.view.repository.coordinator.key(),
                        &key,
                    ) {
                        window
                            .view
                            .repository
                            .coordinator
                            .request_refresh(completion.refresh);
                        window.dispatch_repository_refresh(cx);
                        match result {
                        Ok(muxy_api::repository::RepositoryAiWorkflowOutcome::Committed {
                            ..
                        }) => window.feedback(
                            "Repository AI Action Complete",
                            "Changes were committed and pushed.",
                            crate::toast::ToastTone::Success,
                            cx,
                        ),
                        Ok(muxy_api::repository::RepositoryAiWorkflowOutcome::PullRequestCreated(
                            _,
                        )) => window.feedback(
                            "Repository AI Action Complete",
                            "The pull request was created.",
                            crate::toast::ToastTone::Success,
                            cx,
                        ),
                        Err(error) => window.feedback(
                            "Repository AI Action Failed",
                            error.to_string(),
                            crate::toast::ToastTone::Error,
                            cx,
                        ),
                    }
                    }
                    cx.notify();
                });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn open_pull_request_popover(
        &mut self,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.view.repository.coordinator.key().cloned() else {
            return;
        };
        if matches!(
            self.view.repository.coordinator.state().pull_request,
            crate::repository::PullRequestLoadState::Unavailable(_)
        ) {
            self.view
                .repository
                .coordinator
                .request_refresh(crate::repository::RepositoryRefreshSet::pull_request());
            self.dispatch_repository_refresh(cx);
            return;
        }
        if matches!(
            &self.view.overlay,
            Overlay::Repository {
                kind: crate::views::overlay::RepositoryPopoverKind::PullRequest(popover),
                ..
            } if popover.key == key
        ) {
            self.dismiss_overlay(cx);
            return;
        }
        let (resolved, summary) = match (
            &self.view.repository.coordinator.state().pull_request,
            &self.view.repository.coordinator.state().summary,
        ) {
            (
                crate::repository::PullRequestLoadState::Found {
                    info: _,
                    resolved: Some(resolved),
                },
                crate::repository::LoadState::Ready(summary),
            ) => (resolved.clone(), summary.clone()),
            _ => return,
        };
        let theme = self.state.theme.clone();
        let metrics = self.state.metrics;
        let panel_key = key.clone();
        let repository_identity = resolved.repository_identity().clone();
        let info = resolved.info.clone();
        let panel = cx.new(move |cx| {
            crate::views::repository::pull_request::PullRequestPanel::new(
                panel_key,
                repository_identity,
                &info,
                &summary,
                theme,
                metrics,
                cx,
            )
        });
        let subscription = cx.subscribe(&panel, |window: &mut Self, _, event, cx| {
            window.handle_pull_request_popover_event(event, cx);
        });
        window.focus(&panel.focus_handle(cx));
        self.view.subscriptions = vec![subscription];
        self.view.overlay = Overlay::Repository {
            kind: crate::views::overlay::RepositoryPopoverKind::PullRequest(Box::new(
                crate::views::repository::pull_request::PullRequestPopover {
                    key,
                    resolved,
                    panel,
                    operation_error: None,
                    operation_message: None,
                },
            )),
            anchor,
        };
        self.sync_pull_request_popover(cx);
        cx.notify();
    }

    fn refresh_pull_request_popover(&mut self, cx: &mut Context<Self>) {
        if let Overlay::Repository {
            kind: crate::views::overlay::RepositoryPopoverKind::PullRequest(popover),
            ..
        } = &self.view.overlay
        {
            popover
                .panel
                .update(cx, |panel, cx| panel.set_busy(true, cx));
        }
        self.view
            .repository
            .coordinator
            .request_refresh(crate::repository::RepositoryRefreshSet::pull_request());
        self.dispatch_repository_refresh(cx);
    }

    fn handle_pull_request_popover_event(
        &mut self,
        event: &crate::views::repository::pull_request::PullRequestPopoverEvent,
        cx: &mut Context<Self>,
    ) {
        use crate::views::repository::pull_request::{
            PullRequestConfirmedAction, PullRequestMutationKind, PullRequestPopoverEvent,
        };
        match event {
            PullRequestPopoverEvent::Refresh => {
                self.refresh_pull_request_popover(cx);
            }
            PullRequestPopoverEvent::Open => self.open_current_pull_request_url(cx),
            PullRequestPopoverEvent::Update(identity) => {
                self.start_pull_request_mutation(PullRequestMutationKind::Update, identity, cx)
            }
            PullRequestPopoverEvent::Execute {
                action: PullRequestConfirmedAction::Merge(method),
                identity,
            } => self.start_pull_request_mutation(
                PullRequestMutationKind::Merge(*method),
                identity,
                cx,
            ),
            PullRequestPopoverEvent::Execute {
                action: PullRequestConfirmedAction::Close,
                identity,
            } => self.start_pull_request_mutation(PullRequestMutationKind::Close, identity, cx),
            PullRequestPopoverEvent::Dismiss => self.dismiss_overlay(cx),
        }
    }

    fn open_current_pull_request_url(&mut self, cx: &mut Context<Self>) {
        let url = match &self.view.repository.coordinator.state().pull_request {
            crate::repository::PullRequestLoadState::Found { info, .. } => info.url.clone(),
            _ => return,
        };
        cx.spawn(async move |window, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::platform::open_external_url(&url) })
                .await;
            if let Err(error) = result {
                let _ = window.update(cx, |window, cx| {
                    window.feedback(
                        "Could Not Open Pull Request",
                        error.to_string(),
                        crate::toast::ToastTone::Error,
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    pub(super) fn sync_pull_request_popover(&mut self, cx: &mut Context<Self>) {
        let key = self.view.repository.coordinator.key().cloned();
        let busy = key
            .as_ref()
            .is_some_and(|key| self.state.project_operations.is_mutating(&key.project_id));
        let mut close = false;
        if let Overlay::Repository {
            kind: crate::views::overlay::RepositoryPopoverKind::PullRequest(popover),
            ..
        } = &mut self.view.overlay
        {
            if key.as_ref() != Some(&popover.key) {
                close = true;
            } else {
                match (
                    &self.view.repository.coordinator.state().pull_request,
                    &self.view.repository.coordinator.state().summary,
                ) {
                    (
                        crate::repository::PullRequestLoadState::Found {
                            info,
                            resolved: Some(resolved),
                        },
                        crate::repository::LoadState::Ready(summary),
                    ) => {
                        popover.resolved = resolved.clone();
                        let error = popover.operation_error.clone();
                        let message = popover.operation_message.clone();
                        let key = key.expect("matching pull request popover requires a key");
                        let repository_identity = resolved.repository_identity().clone();
                        popover.panel.update(cx, |panel, cx| {
                            panel.sync(
                                key,
                                repository_identity,
                                info,
                                summary,
                                crate::views::repository::pull_request::PullRequestOperationState {
                                    busy,
                                    error,
                                    message,
                                },
                                cx,
                            )
                        });
                    }
                    (crate::repository::PullRequestLoadState::Loading, _)
                    | (_, crate::repository::LoadState::Loading) => {
                        popover
                            .panel
                            .update(cx, |panel, cx| panel.set_busy(true, cx));
                    }
                    _ => close = true,
                }
            }
        }
        if close {
            self.dismiss_overlay(cx);
        }
    }

    pub(crate) fn open_branch_popover(
        &mut self,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.view
            .repository
            .coordinator
            .set_changes_monitoring(false);
        let Some(key) = self.view.repository.coordinator.key().cloned() else {
            return;
        };
        if matches!(
            &self.view.overlay,
            Overlay::Repository {
                kind: crate::views::overlay::RepositoryPopoverKind::Branch(popover),
                ..
            } if popover.key == key
        ) {
            self.dismiss_overlay(cx);
            return;
        }
        let theme = self.state.theme.clone();
        let metrics = self.state.metrics;
        let picker = cx.new(move |cx| {
            muxy_ui::command_popover::CommandPopover::new(
                muxy_ui::command_popover::CommandPopoverConfig {
                    id: "repository-picker".into(),
                    presentation: muxy_ui::command_popover::CommandPopoverPresentation::Popover,
                    density: muxy_ui::command_popover::CommandPopoverDensity::Comfortable,
                    tabs: vec![
                        muxy_ui::command_popover::CommandPopoverTab::new("branches", "Branches"),
                        muxy_ui::command_popover::CommandPopoverTab::new("stashes", "Stashes"),
                    ],
                    placeholder: "Switch, create, or search…".into(),
                    footer_actions: Vec::new(),
                    footer_hints: Vec::new(),
                    width: Some(520.0),
                    height: Some(400.0),
                    max_height: None,
                    completion_on_tab: false,
                    confirm_on_click: true,
                },
                theme,
                metrics,
                cx,
            )
        });
        let subscription = cx.subscribe(&picker, |window: &mut Self, picker, event, cx| {
            window.handle_repository_picker_event(&picker, event, cx);
        });
        window.focus(&picker.focus_handle(cx));
        self.view.subscriptions = vec![subscription];
        self.view.overlay = Overlay::Repository {
            kind: crate::views::overlay::RepositoryPopoverKind::Branch(Box::new(
                crate::views::repository::branch::BranchPopover {
                    key,
                    load_request: 0,
                    picker,
                    deletion: Default::default(),
                    operation_error: None,
                    branch_entries: crate::repository::LoadState::Loading,
                    stashes: crate::repository::LoadState::Loading,
                },
            )),
            anchor,
        };
        self.view
            .repository
            .coordinator
            .request_refresh(crate::repository::RepositoryRefreshSet::summary_and_branches());
        self.dispatch_repository_refresh(cx);
        self.load_repository_picker_data(cx);
        cx.notify();
    }

    pub(crate) fn open_changes_popover(
        &mut self,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.view.repository.coordinator.key().cloned() else {
            return;
        };
        if matches!(
            &self.view.overlay,
            Overlay::Repository {
                kind: crate::views::overlay::RepositoryPopoverKind::Changes(popover),
                ..
            } if popover.key == key
        ) {
            self.dismiss_overlay(cx);
            return;
        }
        let removable_worktree = self
            .state
            .workspace
            .project(&key.project_id)
            .and_then(|project| {
                self.state
                    .worktrees
                    .get(&key.project_id)
                    .and_then(|worktrees| {
                        worktrees
                            .iter()
                            .find(|worktree| worktree.id.eq_ignore_ascii_case(&key.worktree_id))
                    })
                    .filter(|worktree| project.can_remove_worktree(worktree))
            })
            .map(|worktree| (key.project_id.clone(), worktree.id.clone()));
        let theme = self.state.theme.clone();
        let metrics = self.state.metrics;
        let picker = cx.new(move |cx| {
            muxy_ui::command_popover::CommandPopover::new(
                muxy_ui::command_popover::CommandPopoverConfig {
                    id: "repository-changes-picker".into(),
                    presentation: muxy_ui::command_popover::CommandPopoverPresentation::Popover,
                    density: muxy_ui::command_popover::CommandPopoverDensity::Comfortable,
                    tabs: vec![muxy_ui::command_popover::CommandPopoverTab::new(
                        "changes", "Changes",
                    )],
                    placeholder: "Search changed files…".into(),
                    footer_actions: Vec::new(),
                    footer_hints: Vec::new(),
                    width: Some(
                        crate::views::repository::changes::changes_overlay_policy().target_width,
                    ),
                    height: Some(
                        crate::views::repository::changes::changes_overlay_policy().target_height,
                    ),
                    max_height: None,
                    completion_on_tab: false,
                    confirm_on_click: false,
                },
                theme,
                metrics,
                cx,
            )
        });
        let subscription = cx.subscribe(&picker, |window: &mut Self, picker, event, cx| {
            window.handle_changes_picker_event(&picker, event, cx);
        });
        window.focus(&picker.focus_handle(cx));
        self.view.subscriptions = vec![subscription];
        self.view.overlay = Overlay::Repository {
            kind: crate::views::overlay::RepositoryPopoverKind::Changes(Box::new(
                crate::views::repository::changes::ChangesPopover {
                    key,
                    picker,
                    selection: Default::default(),
                    discard: Default::default(),
                    operation_error: None,
                    line_counts: Default::default(),
                    removable_worktree,
                    discard_in_flight: false,
                },
            )),
            anchor,
        };
        self.view
            .repository
            .coordinator
            .set_changes_monitoring(true);
        self.view
            .repository
            .coordinator
            .request_refresh(crate::repository::RepositoryRefreshSet::summary_and_changes());
        self.dispatch_repository_refresh(cx);
        self.sync_changes_picker(cx);
        cx.notify();
    }

    fn handle_changes_picker_event(
        &mut self,
        picker: &Entity<muxy_ui::command_popover::CommandPopover>,
        event: &muxy_ui::command_popover::CommandPopoverEvent,
        cx: &mut Context<Self>,
    ) {
        use muxy_ui::command_popover::CommandPopoverEvent;
        match event {
            CommandPopoverEvent::QueryChanged { .. } => {
                if let Overlay::Repository {
                    kind: crate::views::overlay::RepositoryPopoverKind::Changes(popover),
                    ..
                } = &mut self.view.overlay
                {
                    popover.discard.cancel();
                }
                self.sync_changes_picker(cx);
            }
            CommandPopoverEvent::RowClicked {
                row,
                shift,
                platform,
            } => {
                let gesture = if *shift {
                    crate::views::repository::changes::ChangeSelectionGesture::Range
                } else if *platform {
                    crate::views::repository::changes::ChangeSelectionGesture::Toggle
                } else {
                    crate::views::repository::changes::ChangeSelectionGesture::Plain
                };
                self.select_changes_row(row.as_ref(), gesture, cx);
            }
            CommandPopoverEvent::Confirmed(selection) => self.select_changes_row(
                selection.id.as_ref(),
                crate::views::repository::changes::ChangeSelectionGesture::Plain,
                cx,
            ),
            CommandPopoverEvent::RowAction { row, action } => {
                self.perform_changes_row_action(picker, row.as_ref(), action.as_ref(), cx);
            }
            CommandPopoverEvent::FooterAction(action) => {
                self.perform_changes_footer_action(picker, action.as_ref(), cx);
            }
            CommandPopoverEvent::Dismissed => self.dismiss_overlay(cx),
            CommandPopoverEvent::InlineActionDismissed => {
                if let Overlay::Repository {
                    kind: crate::views::overlay::RepositoryPopoverKind::Changes(popover),
                    ..
                } = &mut self.view.overlay
                {
                    popover.discard.cancel();
                }
            }
            CommandPopoverEvent::TabChanged(_)
            | CommandPopoverEvent::SelectionChanged(_)
            | CommandPopoverEvent::SecondaryConfirmed(_)
            | CommandPopoverEvent::Submitted { .. }
            | CommandPopoverEvent::CompletionRequested
            | CommandPopoverEvent::NavigateBackRequested => {}
        }
    }

    fn select_changes_row(
        &mut self,
        row: &str,
        gesture: crate::views::repository::changes::ChangeSelectionGesture,
        cx: &mut Context<Self>,
    ) {
        let changes = self.view.repository.coordinator.state().changes.clone();
        let Overlay::Repository {
            kind: crate::views::overlay::RepositoryPopoverKind::Changes(popover),
            ..
        } = &mut self.view.overlay
        else {
            return;
        };
        let query = popover.picker.read(cx).query().to_owned();
        let Some(selected) = crate::views::repository::changes::find_row(
            &changes,
            &query,
            &popover.line_counts,
            row,
        ) else {
            return;
        };
        let ordered =
            crate::views::repository::changes::ordered_keys(&changes, &query, &popover.line_counts);
        popover.selection.apply(selected.key, gesture, &ordered);
        self.sync_changes_picker(cx);
    }

    fn perform_changes_row_action(
        &mut self,
        picker: &Entity<muxy_ui::command_popover::CommandPopover>,
        row: &str,
        action: &str,
        cx: &mut Context<Self>,
    ) {
        let changes = self.view.repository.coordinator.state().changes.clone();
        let Some((key, query, line_counts)) = (match &self.view.overlay {
            Overlay::Repository {
                kind: crate::views::overlay::RepositoryPopoverKind::Changes(popover),
                ..
            } => Some((
                popover.key.clone(),
                popover.picker.read(cx).query().to_owned(),
                popover.line_counts.clone(),
            )),
            _ => None,
        }) else {
            return;
        };
        let Some(presented) =
            crate::views::repository::changes::find_row(&changes, &query, &line_counts, row)
        else {
            return;
        };
        match action {
            "stage" => self.start_changes_mutation(
                crate::views::repository::changes::ChangesMutationKind::Stage(vec![presented.file]),
                cx,
            ),
            "unstage" => self.start_changes_mutation(
                crate::views::repository::changes::ChangesMutationKind::Unstage(vec![
                    presented.file,
                ]),
                cx,
            ),
            "stats" => self.count_untracked_lines(key, presented.file, cx),
            "discard" => {
                let files = vec![presented.file];
                let warning = crate::views::repository::changes::discard_warning(&files);
                if let Overlay::Repository {
                    kind: crate::views::overlay::RepositoryPopoverKind::Changes(popover),
                    ..
                } = &mut self.view.overlay
                    && popover.discard.request(key, &files)
                {
                    let _ = picker.update(cx, |picker, cx| {
                        picker.open_confirmation_with_message(
                            row,
                            "discard",
                            Some(warning.into()),
                            cx,
                        )
                    });
                }
            }
            "confirm:discard" => self.confirm_changes_discard(cx),
            _ => {}
        }
    }

    fn perform_changes_footer_action(
        &mut self,
        picker: &Entity<muxy_ui::command_popover::CommandPopover>,
        action: &str,
        cx: &mut Context<Self>,
    ) {
        match action {
            "stage-all" => self.start_changes_mutation(
                crate::views::repository::changes::ChangesMutationKind::StageAll,
                cx,
            ),
            "unstage-all" => self.start_changes_mutation(
                crate::views::repository::changes::ChangesMutationKind::UnstageAll,
                cx,
            ),
            "stage-selected" => {
                let files = self.selected_change_files(|row| row.can_stage);
                self.start_changes_mutation(
                    crate::views::repository::changes::ChangesMutationKind::Stage(files),
                    cx,
                );
            }
            "unstage-selected" => {
                let files = self.selected_change_files(|row| row.can_unstage);
                self.start_changes_mutation(
                    crate::views::repository::changes::ChangesMutationKind::Unstage(files),
                    cx,
                );
            }
            "discard-selected" => {
                let files = self.selected_change_files(|row| row.can_discard);
                let warning = crate::views::repository::changes::discard_warning(&files);
                let first_row = self.first_selected_change_row();
                if let Overlay::Repository {
                    kind: crate::views::overlay::RepositoryPopoverKind::Changes(popover),
                    ..
                } = &mut self.view.overlay
                    && popover.discard.request(popover.key.clone(), &files)
                    && let Some(first_row) = first_row
                {
                    let _ = picker.update(cx, |picker, cx| {
                        picker.open_confirmation_with_message(
                            &first_row,
                            "discard",
                            Some(warning.into()),
                            cx,
                        )
                    });
                }
            }
            "retry" => {
                if let Overlay::Repository {
                    kind: crate::views::overlay::RepositoryPopoverKind::Changes(popover),
                    ..
                } = &mut self.view.overlay
                {
                    popover.operation_error = None;
                }
                self.view.repository.coordinator.request_refresh(
                    crate::repository::RepositoryRefreshSet::summary_and_changes(),
                );
                self.dispatch_repository_refresh(cx);
                self.sync_changes_picker(cx);
            }
            "remove-worktree" => {
                let target = match &self.view.overlay {
                    Overlay::Repository {
                        kind: crate::views::overlay::RepositoryPopoverKind::Changes(popover),
                        ..
                    } => popover.removable_worktree.clone(),
                    _ => None,
                };
                if let Some((project_id, worktree_id)) = target {
                    self.close_repository_overlay(cx);
                    self.request_worktree_removal_inspection(project_id, worktree_id, cx);
                }
            }
            _ => {}
        }
    }

    fn selected_change_files(
        &self,
        predicate: impl Fn(&crate::views::repository::changes::PresentedChangeRow) -> bool,
    ) -> Vec<muxy_api::repository::ChangedFile> {
        let Overlay::Repository {
            kind: crate::views::overlay::RepositoryPopoverKind::Changes(popover),
            ..
        } = &self.view.overlay
        else {
            return Vec::new();
        };
        crate::views::repository::changes::selected_files(
            &self.view.repository.coordinator.state().changes,
            "",
            &popover.line_counts,
            popover.selection.selected(),
            predicate,
        )
    }

    fn first_selected_change_row(&self) -> Option<String> {
        let Overlay::Repository {
            kind: crate::views::overlay::RepositoryPopoverKind::Changes(popover),
            ..
        } = &self.view.overlay
        else {
            return None;
        };
        crate::views::repository::changes::present_changes(
            &self.view.repository.coordinator.state().changes,
            "",
            &popover.line_counts,
        )
        .sections
        .into_iter()
        .flat_map(|section| section.rows)
        .filter(|row| popover.selection.contains(&row.key) && row.can_discard)
        .max_by_key(|row| row.file.is_untracked)
        .map(|row| crate::views::repository::changes::row_id(&row.key))
    }

    fn confirm_changes_discard(&mut self, cx: &mut Context<Self>) {
        let changes = match &self.view.repository.coordinator.state().changes {
            crate::repository::LoadState::Ready(changes) => changes.files.clone(),
            _ => return,
        };
        let files = match &mut self.view.overlay {
            Overlay::Repository {
                kind: crate::views::overlay::RepositoryPopoverKind::Changes(popover),
                ..
            } => {
                let Some(files) = popover.discard.take(&popover.key, &changes) else {
                    return;
                };
                popover.discard_in_flight = true;
                files
            }
            _ => return,
        };
        self.start_changes_mutation(
            crate::views::repository::changes::ChangesMutationKind::Discard(files),
            cx,
        );
    }

    fn handle_repository_picker_event(
        &mut self,
        picker: &Entity<muxy_ui::command_popover::CommandPopover>,
        event: &muxy_ui::command_popover::CommandPopoverEvent,
        cx: &mut Context<Self>,
    ) {
        use muxy_ui::command_popover::CommandPopoverEvent;
        match event {
            CommandPopoverEvent::QueryChanged { .. } | CommandPopoverEvent::TabChanged(_) => {
                self.sync_repository_picker(cx);
            }
            CommandPopoverEvent::SelectionChanged(_) | CommandPopoverEvent::RowClicked { .. } => {}
            CommandPopoverEvent::Confirmed(selection) => {
                self.confirm_repository_picker_selection(picker, selection.id.as_ref(), cx);
            }
            CommandPopoverEvent::SecondaryConfirmed(selection) => {
                self.confirm_repository_picker_selection(picker, selection.id.as_ref(), cx);
            }
            CommandPopoverEvent::RowAction { row, action } => {
                self.perform_repository_picker_row_action(row.as_ref(), action.as_ref(), cx);
            }
            CommandPopoverEvent::Dismissed => self.dismiss_overlay(cx),
            CommandPopoverEvent::FooterAction(_)
            | CommandPopoverEvent::InlineActionDismissed
            | CommandPopoverEvent::Submitted { .. }
            | CommandPopoverEvent::CompletionRequested
            | CommandPopoverEvent::NavigateBackRequested => {}
        }
    }

    pub(super) fn sync_repository_picker(&mut self, cx: &mut Context<Self>) {
        let busy = self
            .view
            .repository
            .coordinator
            .key()
            .is_some_and(|key| self.state.project_operations.is_mutating(&key.project_id));
        if let Overlay::Repository {
            kind: crate::views::overlay::RepositoryPopoverKind::Branch(popover),
            ..
        } = &self.view.overlay
        {
            crate::views::repository::branch::sync_picker(popover, busy, cx);
        }
    }

    pub(super) fn sync_changes_picker(&mut self, cx: &mut Context<Self>) {
        let changes = self.view.repository.coordinator.state().changes.clone();
        let busy = self
            .view
            .repository
            .coordinator
            .key()
            .is_some_and(|key| self.state.project_operations.is_mutating(&key.project_id));
        if let Overlay::Repository {
            kind: crate::views::overlay::RepositoryPopoverKind::Changes(popover),
            ..
        } = &mut self.view.overlay
        {
            crate::views::repository::changes::sync_picker(popover, &changes, busy, cx);
        }
    }

    pub(super) fn load_repository_picker_data(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.view.repository.coordinator.key().cloned() else {
            return;
        };
        let request = match &mut self.view.overlay {
            Overlay::Repository {
                kind: crate::views::overlay::RepositoryPopoverKind::Branch(popover),
                ..
            } if popover.key == key => {
                popover.load_request = popover.load_request.wrapping_add(1).max(1);
                popover.load_request
            }
            _ => return,
        };
        let service = self.repository_service();
        let path = key.normalized_path.clone();
        cx.spawn(async move |window, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let branches = service.branch_entries(&path);
                    let stashes = service.stash_entries(&path);
                    (branches, stashes)
                })
                .await;
            let _ = window.update(cx, |window, cx| {
                let Overlay::Repository {
                    kind: crate::views::overlay::RepositoryPopoverKind::Branch(popover),
                    ..
                } = &mut window.view.overlay
                else {
                    return;
                };
                if popover.key != key || popover.load_request != request {
                    return;
                }
                popover.branch_entries = match result.0 {
                    Ok(entries) => crate::repository::LoadState::Ready(entries),
                    Err(error) => crate::repository::LoadState::Error(error.to_string()),
                };
                popover.stashes = match result.1 {
                    Ok(entries) => crate::repository::LoadState::Ready(entries),
                    Err(error) => crate::repository::LoadState::Error(error.to_string()),
                };
                window.sync_repository_picker(cx);
            });
        })
        .detach();
    }

    fn confirm_repository_picker_selection(
        &mut self,
        picker: &Entity<muxy_ui::command_popover::CommandPopover>,
        selection: &str,
        cx: &mut Context<Self>,
    ) {
        let active_tab = picker.read(cx).active_tab().to_owned();
        if active_tab == "branches" {
            if selection == "create-branch" {
                let branch = picker.read(cx).query().trim().to_owned();
                self.create_repository_branch(branch, cx);
                return;
            }
            let entry = self.repository_picker_branch(selection);
            if let Some(entry) = entry {
                match entry.kind {
                    muxy_api::repository::BranchKind::Local => {
                        self.switch_repository_branch(entry.name, cx)
                    }
                    muxy_api::repository::BranchKind::Remote => {
                        self.switch_repository_remote_branch(entry.name, cx)
                    }
                }
            }
            return;
        }
        if selection == "create-stash" {
            self.create_repository_stash(cx);
            return;
        }
        if let Some(entry) = self.repository_picker_stash(selection) {
            self.preview_repository_stash(entry, cx);
        }
    }

    fn perform_repository_picker_row_action(
        &mut self,
        row: &str,
        action: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(branch) = self.repository_picker_branch(row) {
            if action == "delete" {
                self.open_repository_picker_confirmation(row, action, cx);
            } else if action == "confirm:delete" {
                let Some(key) = self.view.repository.coordinator.key().cloned() else {
                    return;
                };
                self.request_branch_deletion(key, branch.name.clone(), cx);
                self.confirm_branch_deletion(branch.name, cx);
            }
            return;
        }
        let Some(stash) = self.repository_picker_stash(row) else {
            return;
        };
        match action {
            "preview" => self.preview_repository_stash(stash, cx),
            "apply" => self.apply_repository_stash(stash, cx),
            "pop" => self.pop_repository_stash(stash, cx),
            "drop" => self.open_repository_picker_confirmation(row, action, cx),
            "confirm:drop" => self.drop_repository_stash(stash, cx),
            _ => {}
        }
    }

    fn repository_picker_stash(&self, row: &str) -> Option<muxy_api::repository::StashEntry> {
        let Overlay::Repository {
            kind: crate::views::overlay::RepositoryPopoverKind::Branch(popover),
            ..
        } = &self.view.overlay
        else {
            return None;
        };
        match &popover.stashes {
            crate::repository::LoadState::Ready(entries) => entries
                .iter()
                .find(|entry| crate::views::repository::branch::stash_row_id(entry) == row)
                .cloned(),
            _ => None,
        }
    }

    fn repository_picker_branch(&self, row: &str) -> Option<muxy_api::repository::BranchEntry> {
        let Overlay::Repository {
            kind: crate::views::overlay::RepositoryPopoverKind::Branch(popover),
            ..
        } = &self.view.overlay
        else {
            return None;
        };
        match &popover.branch_entries {
            crate::repository::LoadState::Ready(entries) => entries
                .iter()
                .find(|entry| crate::views::repository::branch::branch_row_id(entry) == row)
                .cloned(),
            _ => None,
        }
    }

    fn open_repository_picker_confirmation(
        &mut self,
        row: &str,
        action: &str,
        cx: &mut Context<Self>,
    ) {
        if let Overlay::Repository {
            kind: crate::views::overlay::RepositoryPopoverKind::Branch(popover),
            ..
        } = &self.view.overlay
        {
            let _ = popover
                .picker
                .update(cx, |picker, cx| picker.open_confirmation(row, action, cx));
        }
    }

    pub(super) fn close_repository_overlay(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.view.overlay, Overlay::Repository { .. }) {
            return;
        }
        self.view
            .repository
            .coordinator
            .set_changes_monitoring(false);
        self.view.subscriptions.clear();
        self.view.overlay = Overlay::None;
        self.view.pending_focus = Some(self.view.workspace_focus.clone());
        cx.notify();
    }

    pub(crate) fn open_sort_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items =
            crate::views::workspace_switcher::sort_menu_items(self.state.workspace.sort_mode());
        self.open_menu(items, position, cx);
        window.focus(&self.view.menu_focus);
    }

    pub(crate) fn open_status_path_menu(
        &mut self,
        path: String,
        remote: bool,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = crate::views::status_bar::path_menu_items(path, remote);
        self.open_menu(items, position, cx);
        window.focus(&self.view.menu_focus);
    }

    pub(crate) fn open_project_menu(
        &mut self,
        project_id: &str,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.state.workspace.project(project_id) else {
            return;
        };
        let groups: Vec<&muxy_core::store::Group> =
            self.state.workspace.groups.all().iter().collect();
        let worktrees = self
            .state
            .worktrees
            .get(project_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let active = self
            .state
            .prefs
            .active_worktree_ids
            .get(project_id)
            .map(String::as_str);
        let items = project_menu::items(project, &groups, worktrees, active);
        self.open_menu(items, position, cx);
        window.focus(&self.view.menu_focus);
    }

    pub(crate) fn open_worktree_menu(
        &mut self,
        project_id: &str,
        worktree_id: &str,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.state.workspace.project(project_id) else {
            return;
        };
        let Some(worktree) = self.state.worktrees.get(project_id).and_then(|worktrees| {
            worktrees
                .iter()
                .find(|worktree| worktree.id.eq_ignore_ascii_case(worktree_id))
        }) else {
            return;
        };
        let mut items = vec![Item::action(
            "Switch Worktree",
            Command::SelectWorktree {
                project_id: project_id.to_owned(),
                worktree_id: worktree.id.clone(),
            },
        )];
        if project.can_remove_worktree(worktree) {
            items.push(Item::Separator);
            items.push(
                Item::action(
                    "Remove Worktree…",
                    Command::RemoveWorktree {
                        project_id: project_id.to_owned(),
                        worktree_id: worktree.id.clone(),
                    },
                )
                .destructive(),
            );
        }
        self.open_menu(items, position, cx);
        window.focus(&self.view.menu_focus);
    }

    pub(crate) fn open_menu(
        &mut self,
        items: Vec<Item>,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.composer.set_menu_open(false);
        self.view.subscriptions.clear();
        self.view.overlay = Overlay::Menu(Menu::new(items, position));
        cx.notify();
    }

    pub(crate) fn move_menu_highlight(&mut self, delta: i32, cx: &mut Context<Self>) {
        if let Overlay::Menu(menu) = &mut self.view.overlay {
            menu.move_highlight(delta);
            cx.notify();
        }
    }

    pub(crate) fn confirm_menu_highlight(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Overlay::Menu(menu) = &self.view.overlay else {
            return;
        };
        let Some(command) = menu.highlighted_command() else {
            return;
        };
        self.perform(command, window, cx);
    }

    pub(crate) fn open_submenu(&mut self, index: Option<usize>) {
        if let Overlay::Menu(menu) = &mut self.view.overlay {
            menu.open_submenu = index;
        }
    }

    pub(crate) fn dismiss_overlay(&mut self, cx: &mut Context<Self>) {
        let restore_composer_focus =
            matches!(self.view.overlay, Overlay::Menu(_)) && self.composer.menu_open();
        if matches!(self.view.overlay, Overlay::TerminalConfirm { .. }) {
            self.resolve_terminal_confirmation(false, cx);
            return;
        }
        if let Overlay::CreateWorktree(modal) = &self.view.overlay
            && !modal.read(cx).dismissible()
        {
            return;
        }
        if matches!(self.view.overlay, Overlay::CreateWorktree(_)) {
            self.view.worktrees.clear_create();
        }
        if matches!(
            &self.view.overlay,
            Overlay::Repository {
                kind: crate::views::overlay::RepositoryPopoverKind::Changes(popover),
                ..
            } if popover.discard_in_flight
        ) {
            return;
        }
        if matches!(
            &self.view.overlay,
            Overlay::Repository {
                kind: crate::views::overlay::RepositoryPopoverKind::PullRequest(popover),
                ..
            } if self.state.project_operations.is_mutating(&popover.key.project_id)
        ) {
            return;
        }
        if let Overlay::Repository {
            kind: crate::views::overlay::RepositoryPopoverKind::Branch(popover),
            ..
        } = &mut self.view.overlay
        {
            if popover.deletion.escape()
                == crate::views::repository::branch::BranchEscapeAction::CancelDeletion
            {
                cx.notify();
                return;
            }
            self.view.pending_focus = Some(self.view.workspace_focus.clone());
        }
        if matches!(
            self.view.overlay,
            Overlay::Repository {
                kind: crate::views::overlay::RepositoryPopoverKind::Changes(_),
                ..
            }
        ) {
            self.view
                .repository
                .coordinator
                .set_changes_monitoring(false);
            self.view.pending_focus = Some(self.view.workspace_focus.clone());
        }
        if matches!(
            self.view.overlay,
            Overlay::Repository {
                kind: crate::views::overlay::RepositoryPopoverKind::PullRequest(_),
                ..
            }
        ) {
            self.view.pending_focus = Some(self.view.workspace_focus.clone());
        }
        if matches!(self.view.overlay, Overlay::Notifications { .. }) {
            self.view.pending_focus = Some(self.view.workspace_focus.clone());
        }
        self.view.subscriptions.clear();
        self.view.overlay = Overlay::None;
        if restore_composer_focus {
            self.composer.set_menu_open(false);
            if let Some(input) = self.composer.input() {
                self.view.pending_focus = Some(input.focus_handle(cx));
            }
        }
        cx.notify();
    }

    pub(super) fn menu_anchor(&self) -> Point<Pixels> {
        match &self.view.overlay {
            Overlay::Menu(menu) => menu.position,
            Overlay::Rename { anchor, .. }
            | Overlay::GroupRename { anchor, .. }
            | Overlay::Symbols { anchor, .. }
            | Overlay::Colors { anchor, .. }
            | Overlay::TabColors { anchor, .. } => *anchor,
            Overlay::Repository { anchor, .. } | Overlay::Notifications { anchor } => anchor.origin,
            Overlay::TabRename { bounds, .. } => bounds.origin,
            Overlay::None
            | Overlay::Picker(_)
            | Overlay::Omnibox(_)
            | Overlay::Settings(_)
            | Overlay::ThemePicker { .. }
            | Overlay::CreateWorktree(_)
            | Overlay::TerminalConfirm { .. } => gpui::point(px(0.0), px(0.0)),
        }
    }

    pub(super) fn input_style(&self) -> InputStyle {
        let theme = &self.state.theme;
        let metrics = &self.state.metrics;
        InputStyle::compact(theme, metrics)
    }

    pub(crate) fn set_icon(
        &mut self,
        project_id: &str,
        icon: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.state.workspace.update(project_id, |project| {
            project.icon = icon.clone();
        });
        self.dismiss_overlay(cx);
    }

    pub(crate) fn set_icon_color(
        &mut self,
        project_id: &str,
        color: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.state.workspace.update(project_id, |project| {
            project.icon_color = color.clone();
        });
        self.dismiss_overlay(cx);
    }

    pub(crate) fn open_terminal_menu(
        &mut self,
        tab_id: &str,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let has_selection = self
            .terminal_runtime
            .surfaces
            .handle(tab_id)
            .is_some_and(|handle| handle.has_selection());
        let items = vec![
            Item::action("Copy", Command::TerminalCopy(tab_id.to_owned())).disabled(!has_selection),
            Item::action("Paste", Command::TerminalPaste(tab_id.to_owned())),
        ];
        self.open_menu(items, position, cx);
        window.focus(&self.view.menu_focus);
    }

    pub(super) fn open_project_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self
            .state
            .prefs
            .project_search_root
            .clone()
            .unwrap_or_else(|| muxy_core::prefs::home_dir().to_string_lossy().into_owned());
        let paths = self.state.workspace.project_paths();
        let theme = self.state.theme.clone();
        let metrics = self.state.metrics;

        let search = self.picker_search.clone();
        let picker = cx.new(|cx| ProjectPicker::new(search, root, paths, theme, metrics, cx));
        let subscription = cx.subscribe(&picker, |window: &mut Self, _, event, cx| match event {
            PickerEvent::Dismiss => window.dismiss_overlay(cx),
            PickerEvent::Confirm {
                path,
                create_if_missing,
            } => window.confirm_project_path(path, *create_if_missing, cx),
            PickerEvent::ChooseFinder { directory } => {
                window.choose_project_with_finder(directory.clone(), cx)
            }
            PickerEvent::EditSearchLocation { directory } => {
                window.edit_search_location(directory.clone(), cx)
            }
        });

        window.focus(&picker.focus_handle(cx));
        self.view.subscriptions = vec![subscription];
        self.view.overlay = Overlay::Picker(picker);
        cx.notify();
    }

    pub(crate) fn open_omnibox(
        &mut self,
        scope: omnibox::Scope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Overlay::Omnibox(existing) = &self.view.overlay {
            let existing = existing.clone();
            if existing.read(cx).scope() == scope {
                self.dismiss_overlay(cx);
                return;
            }
            existing.update(cx, |view, cx| view.apply_scope(scope, cx));
            self.refresh_omnibox(true, cx);
            cx.notify();
            return;
        }

        let theme = self.state.theme.clone();
        let metrics = self.state.metrics;
        let view = cx.new(|cx| omnibox::Omnibox::new(scope, theme, metrics, cx));
        let subscription = cx.subscribe(&view, |window: &mut Self, _, event, cx| match event {
            omnibox::OmniboxEvent::Dismiss => window.dismiss_overlay(cx),
            omnibox::OmniboxEvent::QueryChanged => window.refresh_omnibox(true, cx),
            omnibox::OmniboxEvent::Confirm(action) => {
                let action = action.clone();
                window.perform_omnibox(action, cx);
            }
        });

        window.focus(&view.focus_handle(cx));
        self.view.subscriptions = vec![subscription];
        self.view.overlay = Overlay::Omnibox(view);
        self.refresh_omnibox(true, cx);
        cx.notify();
    }

    pub(crate) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings_category(None, window, cx);
    }

    pub(crate) fn open_quick_terminal_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_settings_category(Some(settings::Category::QuickTerminal), window, cx);
    }

    fn open_settings_category(
        &mut self,
        category: Option<settings::Category>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let theme = self.state.theme.clone();
        let metrics = self.state.metrics;
        let appearance = self.state.appearance;
        let extension_options = self.extension_settings_options();
        let modal = cx.new(|cx| {
            settings::SettingsModal::new(theme, metrics, appearance, extension_options, cx)
        });
        if let Some(category) = category {
            modal.update(cx, |modal, cx| modal.select_category(category, cx));
        }
        let subscription = cx.subscribe(&modal, |window: &mut Self, _, event, cx| match event {
            settings::SettingsEvent::Dismiss => window.dismiss_overlay(cx),
            settings::SettingsEvent::Applied(effect) => window.apply_settings(*effect, cx),
            settings::SettingsEvent::SetDesktopNotifications(enabled) => {
                window.set_desktop_notifications(*enabled, cx)
            }
            settings::SettingsEvent::PreviewNotificationSound(name) => {
                window.preview_notification_sound(name)
            }
        });

        window.focus(&modal.focus_handle(cx));
        self.view.subscriptions = vec![subscription];
        self.view.overlay = Overlay::Settings(modal);
        cx.notify();
    }

    fn set_desktop_notifications(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if !enabled {
            self.notification_coordinator.cancel_desktop_authorization();
            muxy_core::prefs::Prefs::store_settings_value(
                "muxy.notifications.desktopEnabled",
                serde_json::Value::Bool(false),
            );
            self.finish_desktop_authorization_modal(cx);
            return;
        }
        let Some((generation, authorization)) =
            self.notification_coordinator.begin_desktop_authorization()
        else {
            return;
        };
        let task = cx.spawn(async move |window, cx| {
            let result = authorization
                .recv()
                .await
                .unwrap_or(crate::notifications::desktop::AuthorizationResult::Failed);
            let _ = window.update(cx, |window, cx| {
                let Some(enabled) = window
                    .notification_coordinator
                    .complete_desktop_authorization(generation, result)
                else {
                    return;
                };
                muxy_core::prefs::Prefs::store_settings_value(
                    "muxy.notifications.desktopEnabled",
                    serde_json::Value::Bool(enabled),
                );
                window.finish_desktop_authorization_modal(cx);
                if !enabled {
                    window.feedback(
                        "Desktop Notifications Unavailable",
                        desktop_authorization_failure(result),
                        crate::toast::ToastTone::Error,
                        cx,
                    );
                }
            });
        });
        self.notification_coordinator.set_authorization_task(task);
    }

    fn finish_desktop_authorization_modal(&mut self, cx: &mut Context<Self>) {
        if let Overlay::Settings(modal) = &self.view.overlay {
            modal.update(cx, |modal, cx| {
                modal.set_desktop_authorization_pending(false, cx);
            });
        }
        cx.notify();
    }

    fn preview_notification_sound(&mut self, name: &str) {
        self.notification_coordinator.play_sound(name);
    }

    pub(crate) fn open_theme_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.view.overlay, Overlay::ThemePicker { .. }) {
            self.dismiss_overlay(cx);
            return;
        }
        let theme = self.state.theme.clone();
        let metrics = self.state.metrics;
        let appearance = self.state.appearance;
        let browser = cx.new(|cx| {
            settings::theme_picker::ThemeBrowser::new(
                settings::theme_picker::ThemeMode::CurrentAppearance,
                appearance,
                theme,
                metrics,
                cx,
            )
        });
        let subscription = cx.subscribe(&browser, |window: &mut Self, _, event, cx| match event {
            settings::theme_picker::ThemeBrowserEvent::Applied => window.apply_theme_setting(cx),
            settings::theme_picker::ThemeBrowserEvent::Dismiss => window.dismiss_overlay(cx),
        });

        window.focus(&browser.focus_handle(cx));
        self.view.subscriptions = vec![subscription];
        self.view.overlay = Overlay::ThemePicker {
            browser,
            anchor: self.view.theme_picker_anchor,
        };
        cx.notify();
    }

    pub(crate) fn record_theme_picker_anchor(&mut self, bounds: Bounds<Pixels>) {
        self.view.theme_picker_anchor = Some(bounds);
    }

    pub(crate) fn open_create_worktree(
        &mut self,
        project_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.view.overlay, Overlay::CreateWorktree(_))
            || self.view.worktrees.create_request().is_some()
            || self.state.project_operations.is_mutating(project_id)
        {
            return;
        }
        let Some(project) = self.state.workspace.project(project_id).cloned() else {
            return;
        };
        if project.is_home() || project.is_remote() || !project.is_git_repo {
            self.feedback(
                "New Worktree",
                "Worktrees require an existing local Git project.",
                crate::toast::ToastTone::Warning,
                cx,
            );
            return;
        }
        let generation = self.state.project_operations.generation(project_id);
        let identity = self.view.worktrees.begin_create_for(project_id, generation);
        let git_options = self.project_runtime.git_options.clone();
        let home = muxy_core::prefs::home_dir();
        let location_context = muxy_api::worktree_location::LocationContext {
            home: home.clone(),
            profile_worktree_root: muxy_core::prefs::app_support_dir().join("worktree-checkouts"),
            default_path_template: self.state.prefs.default_worktree_path_template.clone(),
            default_parent_path: self.state.prefs.default_worktree_parent_path.clone(),
        };
        let global_config = muxy_api::worktree_config::global_config_path(
            &home,
            std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        );
        let project_path = project.path.clone();
        let expected_path = project.path.clone();
        cx.spawn(async move |window, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    let deadline = muxy_api::subprocess::Deadline::new(Duration::from_secs(30));
                    let branches = muxy_api::git::list_local_branches(
                        &git_options,
                        std::path::Path::new(&project_path),
                        &deadline,
                    );
                    let current = muxy_api::git::current_branch(
                        &git_options,
                        std::path::Path::new(&project_path),
                        &deadline,
                    );
                    let commands = muxy_api::worktree_config::resolved_commands(
                        muxy_api::worktree_config::HookKind::Setup,
                        std::path::Path::new(&project_path),
                        &global_config,
                        true,
                    );
                    (branches, current, commands)
                })
                .await;
            let _ = window.update(cx, |window, cx| {
                if !window.view.worktrees.matches_create(&identity)
                    || window
                        .state
                        .project_operations
                        .generation(&identity.project_id)
                        != identity.generation
                    || window
                        .state
                        .workspace
                        .project(&identity.project_id)
                        .is_none_or(|current| current.path != expected_path)
                {
                    return;
                }
                let (branches, current, commands) = loaded;
                let mut initial_error = None;
                let branches = branches.unwrap_or_else(|error| {
                    initial_error = Some(error.to_string());
                    Vec::new()
                });
                let current = current.unwrap_or_else(|error| {
                    initial_error.get_or_insert_with(|| error.to_string());
                    None
                });
                let commands = commands.unwrap_or_else(|error| {
                    initial_error.get_or_insert_with(|| error.to_string());
                    Vec::new()
                });
                let mut form = create_worktree_overlay::CreateWorktreeForm::new(
                    project,
                    location_context,
                    commands,
                );
                form.set_error(initial_error);
                let theme = window.state.theme.clone();
                let metrics = window.state.metrics;
                let modal = cx.new(|cx| {
                    create_worktree_overlay::CreateWorktreeModal::new(
                        form, branches, current, theme, metrics, cx,
                    )
                });
                let subscription =
                    cx.subscribe(&modal, |window: &mut Self, modal, event, cx| match event {
                        create_worktree_overlay::CreateWorktreeEvent::Dismiss => {
                            window.view.worktrees.clear_create();
                            window.dismiss_overlay(cx);
                        }
                        create_worktree_overlay::CreateWorktreeEvent::ChooseFolder => {
                            window.choose_worktree_folder(modal, cx);
                        }
                        create_worktree_overlay::CreateWorktreeEvent::Submit {
                            request,
                            path_template,
                            parent_path,
                        } => window.request_native_worktree_creation(
                            *request.clone(),
                            path_template.clone(),
                            parent_path.clone(),
                            modal,
                            cx,
                        ),
                    });
                window.view.pending_focus = Some(modal.focus_handle(cx));
                window.view.subscriptions = vec![subscription];
                window.view.overlay = Overlay::CreateWorktree(modal);
                cx.notify();
            });
        })
        .detach();
        window.focus(&self.view.workspace_focus);
    }

    fn choose_worktree_folder(
        &mut self,
        modal: Entity<create_worktree_overlay::CreateWorktreeModal>,
        cx: &mut Context<Self>,
    ) {
        let Some(identity) = self.view.worktrees.create_request().cloned() else {
            return;
        };
        let directory = self
            .state
            .workspace
            .project(&identity.project_id)
            .map(|project| project.path.clone());
        cx.spawn(async move |window, cx| {
            let selected =
                crate::views::file_dialog::pick_folder(crate::views::file_dialog::FolderRequest {
                    message: "Select where new worktrees for this project should be created",
                    directory,
                })
                .await;
            let _ = window.update(cx, |window, cx| {
                if !window.view.worktrees.matches_create(&identity)
                    || window
                        .state
                        .project_operations
                        .generation(&identity.project_id)
                        != identity.generation
                {
                    return;
                }
                if let Some(path) = selected {
                    modal.update(cx, |modal, cx| modal.set_folder(path, cx));
                }
            });
        })
        .detach();
    }

    pub(super) fn refresh_omnibox(&mut self, reset_highlight: bool, cx: &mut Context<Self>) {
        let Overlay::Omnibox(view) = &self.view.overlay else {
            return;
        };
        let view = view.clone();
        let (scope, query) = {
            let read = view.read(cx);
            (read.scope(), read.query(cx))
        };
        let mut candidates =
            omnibox::items::items(&self.state, &self.terminal_runtime.surfaces, scope);
        if scope == omnibox::Scope::CommandShortcuts {
            candidates.extend(omnibox::items::extension_command_items(
                self.extension_runtime.catalog(),
            ));
        }
        let rows = omnibox::items::ranked(candidates, scope, &query);
        view.update(cx, |view, cx| view.set_rows(rows, reset_highlight, cx));
    }

    pub(super) fn perform_omnibox(&mut self, action: omnibox::ItemAction, cx: &mut Context<Self>) {
        self.dismiss_overlay(cx);
        match action {
            omnibox::ItemAction::SelectProject(id) => {
                self.state.select_project(&id);
                cx.notify();
            }
            omnibox::ItemAction::RestoreProject(id) => self.restore_removed_project(&id, cx),
            omnibox::ItemAction::SelectWorktree {
                project_id,
                worktree_id,
            } => {
                self.state.select_worktree(&project_id, &worktree_id);
                cx.notify();
            }
            omnibox::ItemAction::SelectGroup(group_id) => {
                self.state.workspace.select_group(group_id);
                if let Some(first) = self
                    .state
                    .workspace
                    .visible_projects()
                    .first()
                    .map(|project| project.id.clone())
                {
                    self.state.select_project(&first);
                }
                cx.notify();
            }
            omnibox::ItemAction::SelectTab {
                project_id,
                worktree_path,
                tab_id,
            } => self.select_omnibox_tab(&project_id, &worktree_path, &tab_id, cx),
            omnibox::ItemAction::RunCommand(id) => self.create_command_tab(&id, cx),
            omnibox::ItemAction::RunExtensionCommand {
                extension_id,
                command_id,
            } => self.run_extension_command(&extension_id, &command_id, None, cx),
        }
    }

    pub(super) fn restore_removed_project(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(project) = muxy_core::store::load_recently_removed()
            .into_iter()
            .find(|entry| entry.project.id.eq_ignore_ascii_case(id))
            .map(|entry| entry.project)
        else {
            self.alert(
                "Restore Project".to_owned(),
                "This project is no longer available in Recently Removed.".to_owned(),
                cx,
            );
            return;
        };
        let path = project.path.clone();
        match muxy_api::picker::path_service::directory_state(&path) {
            muxy_api::picker::path_service::DirectoryState::Missing => {
                self.alert(
                    "Restore Project".to_owned(),
                    format!("The project folder no longer exists at {path}."),
                    cx,
                );
                return;
            }
            muxy_api::picker::path_service::DirectoryState::NotDirectory => {
                self.alert(
                    "Restore Project".to_owned(),
                    format!("The project path is no longer a folder: {path}."),
                    cx,
                );
                return;
            }
            muxy_api::picker::path_service::DirectoryState::Directory => {}
        }

        if let Some(existing_id) = self
            .state
            .workspace
            .contains_path(&path)
            .map(|project| project.id.clone())
        {
            let _ = muxy_core::store::take_recently_removed(id);
            self.state.select_project(&existing_id);
            cx.notify();
            return;
        }

        let restored_id = project.id.clone();
        if !self.state.workspace.restore(project) {
            self.alert(
                "Restore Project".to_owned(),
                "Muxy could not save the restored project.".to_owned(),
                cx,
            );
            return;
        }
        if muxy_core::store::take_recently_removed(id).is_none() {
            log::warn!("restored project but failed to update recently removed projects");
        }
        if let Some(group_id) = self.state.workspace.active_group_id.clone() {
            self.state
                .workspace
                .groups
                .add_project(&restored_id, &group_id);
        }
        self.state.select_project(&restored_id);
        self.refresh_project_truth(None, cx);
        cx.notify();
    }

    pub(super) fn select_omnibox_tab(
        &mut self,
        project_id: &str,
        worktree_path: &str,
        tab_id: &str,
        cx: &mut Context<Self>,
    ) {
        let worktree_id = self.state.worktrees.get(project_id).and_then(|list| {
            list.iter()
                .find(|worktree| worktree.path == worktree_path)
                .map(|worktree| worktree.id.clone())
        });
        match worktree_id {
            Some(worktree_id) => self.state.select_worktree(project_id, &worktree_id),
            None => self.state.select_project(project_id),
        }
        let target = self.state.active_tab_workspace().and_then(|workspace| {
            let root = workspace.root.as_ref()?;
            let area_id = root.area_ids().into_iter().find(|area_id| {
                root.area_by_id(area_id)
                    .is_some_and(|area| area.tabs.iter().any(|tab| tab.id == tab_id))
            })?;
            Some((
                area_id,
                workspace.root_id_for_tab(tab_id).map(str::to_owned),
            ))
        });
        if let Some((area_id, root_id)) = target
            && let Some(workspace) = self.state.active_tab_workspace_mut()
        {
            if let Some(root_id) = root_id {
                workspace.select_root_tab(&root_id);
            }
            workspace.select_tab(&area_id, tab_id);
            let _ = self.state.persist_tab_workspaces();
        }
        cx.notify();
    }

    pub(super) fn choose_project_with_finder(&mut self, directory: String, cx: &mut Context<Self>) {
        self.dismiss_overlay(cx);
        cx.spawn(async move |window, cx| {
            let request = crate::views::file_dialog::FolderRequest {
                message: "Select a project folder",
                directory: Some(directory),
            };
            let Some(path) = crate::views::file_dialog::pick_folder(request).await else {
                return;
            };
            let path = path.to_string_lossy().into_owned();
            let _ = window.update(cx, |window, cx| {
                window.confirm_project_path(&path, false, cx);
            });
        })
        .detach();
    }

    pub(super) fn edit_search_location(&mut self, directory: String, cx: &mut Context<Self>) {
        self.dismiss_overlay(cx);
        cx.spawn(async move |window, cx| {
            let request = crate::views::file_dialog::FolderRequest {
                message: "Select where Muxy searches for project folders",
                directory: Some(directory),
            };
            let Some(path) = crate::views::file_dialog::pick_folder(request).await else {
                return;
            };
            let path = path.to_string_lossy().into_owned();
            Prefs::store_settings_value(
                "muxy.projectPicker.defaultDirectory",
                Value::String(path.clone()),
            );
            let _ = window.update(cx, |window, cx| {
                window.state.prefs.project_search_root = Some(path);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn confirm_project_path(
        &mut self,
        path: &str,
        create_if_missing: bool,
        cx: &mut Context<Self>,
    ) {
        let standardized = muxy_api::picker::path_service::standardize(path);
        let missing = matches!(
            muxy_api::picker::path_service::directory_state(&standardized),
            muxy_api::picker::path_service::DirectoryState::Missing
        );

        if missing && create_if_missing {
            let message = format!("Muxy will create \"{standardized}\" and add it as a project.");
            let answer = self.ask(
                "Create Project Folder?".to_owned(),
                message,
                &["Create & Add", "Cancel"],
                cx,
            );
            cx.spawn(async move |window, cx| {
                if answer.await != Some(0) {
                    return;
                }
                let _ = window.update(cx, |window, cx| {
                    window.add_project_path(&standardized, true, cx);
                });
            })
            .detach();
            return;
        }

        self.add_project_path(&standardized, create_if_missing, cx);
    }

    pub(super) fn add_project_path(
        &mut self,
        path: &str,
        create_if_missing: bool,
        cx: &mut Context<Self>,
    ) {
        let result = muxy_api::picker::confirm::ensure_directory(path, create_if_missing);
        if result != muxy_api::picker::confirm::ConfirmResult::Success {
            self.alert(result.title().to_owned(), result.message(path), cx);
            return;
        }

        let existing = self
            .state
            .workspace
            .contains_path(path)
            .map(|project| project.id.clone());
        let id = match existing {
            Some(id) => Some(id),
            None => {
                let name = muxy_api::picker::path_service::last_component(path);
                self.state.workspace.add(name, path.to_owned())
            }
        };

        if let Some(id) = id {
            if let Some(group_id) = self.state.workspace.active_group_id.clone() {
                self.state.workspace.groups.add_project(&id, &group_id);
            }
            self.state.workspace.sort();
            self.state.select_project(&id);
            self.refresh_project_truth(None, cx);
        }

        self.dismiss_overlay(cx);
    }

    pub(super) fn confirm_delete_group(&mut self, group_id: String, cx: &mut Context<Self>) {
        let name = self.state.workspace.groups.name_for(Some(&group_id));
        self.dismiss_overlay(cx);

        let answer = self.ask(
            format!("Delete \"{name}\"?"),
            "Projects in this workspace will not be deleted.".to_owned(),
            &["Delete", "Cancel"],
            cx,
        );

        cx.spawn(async move |window, cx| {
            if answer.await != Some(0) {
                return;
            }
            let _ = window.update(cx, |window, cx| {
                if window.state.workspace.active_group_id.as_deref() == Some(group_id.as_str()) {
                    window.state.workspace.select_group(None);
                }
                window.state.workspace.groups.remove(&group_id);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn confirm_remove(&mut self, project_id: String, cx: &mut Context<Self>) {
        let Some(project) = self.state.workspace.project(&project_id) else {
            return;
        };
        let title = format!("Remove \"{}\"?", project.name);
        self.dismiss_overlay(cx);

        let answer = self.ask(
            title,
            "This will remove the project from Muxy. Project files on disk will not be deleted."
                .to_owned(),
            &["Remove", "Cancel"],
            cx,
        );

        cx.spawn(async move |window, cx| {
            if answer.await != Some(0) {
                return;
            }
            let _ = window.update(cx, |window, cx| {
                window.remove_project(&project_id, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn remove_project(&mut self, project_id: &str, cx: &mut Context<Self>) {
        if self.state.project_operations.is_mutating(project_id) {
            self.alert(
                "Project Is Busy".to_owned(),
                "Wait for the active worktree operation to finish before removing this project."
                    .to_owned(),
                cx,
            );
            return;
        }
        self.state.remove_project(project_id);
    }

    pub(super) fn ask(
        &self,
        title: String,
        message: String,
        buttons: &'static [&'static str],
        cx: &mut Context<Self>,
    ) -> gpui::Task<Option<usize>> {
        let handle = cx.active_window();
        cx.spawn(async move |_, cx| {
            let handle = handle?;
            let answer = cx
                .update(|cx| {
                    handle
                        .update(cx, |_, window, cx| {
                            window.prompt(
                                gpui::PromptLevel::Warning,
                                &title,
                                Some(&message),
                                buttons,
                                cx,
                            )
                        })
                        .ok()
                })
                .ok()
                .flatten()?;
            answer.await.ok()
        })
    }

    pub(super) fn alert(&mut self, title: String, message: String, cx: &mut Context<Self>) {
        self.ask(title, message, &["OK"], cx).detach();
    }

    pub(super) fn start_rename(&mut self, project_id: String, cx: &mut Context<Self>) {
        let Some(project) = self.state.workspace.project(&project_id) else {
            return;
        };
        let name = project.name.clone();
        let anchor = self.menu_anchor();
        let style = self.input_style();
        let input = cx.new(|cx| {
            TextInput::new(style, cx)
                .with_placeholder("Project name")
                .with_text(name)
        });
        input.update(cx, |input: &mut TextInput, cx| input.select_all_text(cx));

        let committed = project_id.clone();
        let subscription = cx.subscribe(
            &input,
            move |window: &mut Self, input: Entity<TextInput>, event, cx| match event {
                InputEvent::Submitted => {
                    let name = input.read(cx).text().trim().to_owned();
                    if !name.is_empty() {
                        window.state.workspace.update(&committed, |project| {
                            project.name = name.clone();
                        });
                        window.state.workspace.sort();
                    }
                    window.dismiss_overlay(cx);
                }
                InputEvent::Cancelled => window.dismiss_overlay(cx),
                InputEvent::Changed => {}
            },
        );

        self.view.pending_focus = Some(input.focus_handle(cx));
        self.view.subscriptions = vec![subscription];
        self.view.overlay = Overlay::Rename { input, anchor };
        cx.notify();
    }

    pub(super) fn start_group_rename(&mut self, group_id: Option<String>, cx: &mut Context<Self>) {
        let anchor = self.menu_anchor();
        let style = self.input_style();
        let existing = group_id
            .as_deref()
            .map(|id| self.state.workspace.groups.name_for(Some(id)))
            .unwrap_or_default();
        let input = cx.new(|cx| {
            TextInput::new(style, cx)
                .with_placeholder("Workspace name")
                .with_text(existing)
        });
        if group_id.is_some() {
            input.update(cx, |input: &mut TextInput, cx| input.select_all_text(cx));
        }

        let committed = group_id.clone();
        let subscription = cx.subscribe(
            &input,
            move |window: &mut Self, input: Entity<TextInput>, event, cx| match event {
                InputEvent::Submitted => {
                    let name = input.read(cx).text().trim().to_owned();
                    if !name.is_empty() {
                        match committed.as_deref() {
                            Some(id) => window.state.workspace.groups.rename(id, name),
                            None => {
                                window.state.workspace.groups.add(name);
                            }
                        }
                    }
                    window.dismiss_overlay(cx);
                }
                InputEvent::Cancelled => window.dismiss_overlay(cx),
                InputEvent::Changed => {}
            },
        );

        self.view.pending_focus = Some(input.focus_handle(cx));
        self.view.subscriptions = vec![subscription];
        self.view.overlay = Overlay::GroupRename {
            group_id,
            input,
            anchor,
        };
        cx.notify();
    }

    pub(crate) fn open_add_project_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let removed = muxy_core::store::load_recently_removed();
        if removed.is_empty() {
            self.perform(Command::OpenProjectPicker, window, cx);
            return;
        }
        let mut items = vec![
            Item::action("Local", Command::OpenProjectPicker),
            Item::Separator,
            Item::label("Recently Removed"),
        ];
        items.extend(removed.into_iter().map(|entry| {
            Item::action(
                entry.project.name,
                Command::RestoreRecentlyRemoved(entry.project.id),
            )
        }));
        self.open_menu(items, position, cx);
        window.focus(&self.view.menu_focus);
    }

    pub(crate) fn open_layout_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_menu(app_layout_menu_items(), position, cx);
        window.focus(&self.view.menu_focus);
    }

    pub(crate) fn open_terminal_layout_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items: Vec<Item> = self
            .state
            .layouts()
            .into_iter()
            .map(|descriptor| {
                Item::action(
                    descriptor.name,
                    Command::ApplyLayout(descriptor.path.to_string_lossy().into_owned()),
                )
            })
            .collect();
        if items.is_empty() {
            return;
        }
        self.open_menu(items, position, cx);
        window.focus(&self.view.menu_focus);
    }

    pub(crate) fn open_workspace_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = self.state.workspace.active_group_id.clone();
        let groups: Vec<(String, String, bool)> = self
            .state
            .workspace
            .groups
            .all()
            .iter()
            .map(|group| (group.id.clone(), group.name.clone(), group.is_local))
            .collect();

        let mut items = vec![
            Item::action("All Projects", Command::SelectWorkspaceGroup(None))
                .checked(active.is_none()),
        ];
        if !groups.is_empty() {
            items.push(Item::Separator);
            for (id, name, _) in &groups {
                items.push(
                    Item::action(name, Command::SelectWorkspaceGroup(Some(id.clone())))
                        .checked(active.as_deref() == Some(id.as_str())),
                );
            }
        }

        let renameable: Vec<Item> = groups
            .iter()
            .filter(|(_, _, is_local)| *is_local)
            .map(|(id, name, _)| Item::action(name, Command::RenameWorkspaceGroup(id.clone())))
            .collect();
        let deletable: Vec<Item> = groups
            .iter()
            .map(|(id, name, _)| {
                Item::action(name, Command::DeleteWorkspaceGroup(id.clone())).destructive()
            })
            .collect();

        items.push(Item::Separator);
        let rename_empty = renameable.is_empty();
        let delete_empty = deletable.is_empty();
        items.push(Item::submenu("Rename Workspace", renameable).disabled(rename_empty));
        items.push(Item::submenu("Delete Workspace", deletable).disabled(delete_empty));
        items.push(Item::Separator);
        items.push(Item::action("New Workspace", Command::CreateWorkspaceGroup));

        self.open_menu(items, position, cx);
        window.focus(&self.view.menu_focus);
    }

    pub(super) fn open_project_color_picker(&mut self, project_id: String, cx: &mut Context<Self>) {
        let anchor = self.menu_anchor();
        let selected = self
            .state
            .workspace
            .project(&project_id)
            .and_then(|project| project.icon_color.clone());
        let picker = self.new_color_picker("project-color-picker", "Project Colors", cx);
        sync_color_picker(&picker, "", selected.as_deref(), cx);
        let target = project_id.clone();
        let subscription = cx.subscribe(&picker, move |window: &mut Self, picker, event, cx| {
            use muxy_ui::command_popover::CommandPopoverEvent;
            match event {
                CommandPopoverEvent::QueryChanged { query, .. } => {
                    let selected = window
                        .state
                        .workspace
                        .project(&target)
                        .and_then(|project| project.icon_color.as_deref());
                    sync_color_picker(&picker, query.as_ref(), selected, cx);
                }
                CommandPopoverEvent::Confirmed(selection)
                | CommandPopoverEvent::SecondaryConfirmed(selection) => {
                    if let Some(color) = selection.id.strip_prefix("color-") {
                        window.set_icon_color(&target, Some(color.to_owned()), cx);
                    }
                }
                CommandPopoverEvent::FooterAction(action) if action.as_ref() == "reset" => {
                    window.set_icon_color(&target, None, cx)
                }
                CommandPopoverEvent::Dismissed => window.dismiss_overlay(cx),
                _ => {}
            }
        });
        self.view.subscriptions = vec![subscription];
        self.view.overlay = Overlay::Colors { picker, anchor };
        cx.notify();
    }

    pub(super) fn open_tab_color_picker(&mut self, tab_id: String, cx: &mut Context<Self>) {
        let anchor = self.menu_anchor();
        let selected = self
            .state
            .active_tab_workspace()
            .and_then(|workspace| workspace.tab(&tab_id))
            .and_then(|tab| tab.color_id.clone());
        let picker = self.new_color_picker("tab-color-picker", "Tab Colors", cx);
        sync_color_picker(&picker, "", selected.as_deref(), cx);
        let target = tab_id.clone();
        let subscription = cx.subscribe(&picker, move |window: &mut Self, picker, event, cx| {
            use muxy_ui::command_popover::CommandPopoverEvent;
            match event {
                CommandPopoverEvent::QueryChanged { query, .. } => {
                    let selected = window
                        .state
                        .active_tab_workspace()
                        .and_then(|workspace| workspace.tab(&target))
                        .and_then(|tab| tab.color_id.as_deref());
                    sync_color_picker(&picker, query.as_ref(), selected, cx);
                }
                CommandPopoverEvent::Confirmed(selection)
                | CommandPopoverEvent::SecondaryConfirmed(selection) => {
                    if let Some(color) = selection.id.strip_prefix("color-") {
                        window.set_tab_color(&target, Some(color.to_owned()), cx);
                    }
                }
                CommandPopoverEvent::FooterAction(action) if action.as_ref() == "reset" => {
                    window.set_tab_color(&target, None, cx)
                }
                CommandPopoverEvent::Dismissed => window.dismiss_overlay(cx),
                _ => {}
            }
        });
        self.view.subscriptions = vec![subscription];
        self.view.overlay = Overlay::TabColors { picker, anchor };
        cx.notify();
    }

    fn new_color_picker(
        &self,
        id: &'static str,
        title: &'static str,
        cx: &mut Context<Self>,
    ) -> Entity<muxy_ui::command_popover::CommandPopover> {
        let theme = self.state.theme.clone();
        let metrics = self.state.metrics;
        cx.new(move |cx| {
            muxy_ui::command_popover::CommandPopover::new(
                muxy_ui::command_popover::CommandPopoverConfig {
                    id: id.into(),
                    presentation: muxy_ui::command_popover::CommandPopoverPresentation::Popover,
                    density: muxy_ui::command_popover::CommandPopoverDensity::Compact,
                    tabs: vec![muxy_ui::command_popover::CommandPopoverTab::new(
                        "colors", title,
                    )],
                    placeholder: "Search colors…".into(),
                    footer_actions: vec![muxy_ui::command_popover::CommandPopoverAction::new(
                        "reset",
                        "Reset to Default",
                    )],
                    footer_hints: Vec::new(),
                    width: Some(400.0),
                    height: None,
                    max_height: Some(480.0),
                    completion_on_tab: false,
                    confirm_on_click: true,
                },
                theme,
                metrics,
                cx,
            )
        })
    }

    pub(super) fn open_symbol_picker(&mut self, project_id: String, cx: &mut Context<Self>) {
        let anchor = self.menu_anchor();
        let theme = self.state.theme.clone();
        let metrics = self.state.metrics;
        let picker = cx.new(move |cx| {
            muxy_ui::command_popover::CommandPopover::new(
                muxy_ui::command_popover::CommandPopoverConfig {
                    id: "symbol-picker".into(),
                    presentation: muxy_ui::command_popover::CommandPopoverPresentation::Popover,
                    density: muxy_ui::command_popover::CommandPopoverDensity::Compact,
                    tabs: vec![muxy_ui::command_popover::CommandPopoverTab::new(
                        "symbols", "Icons",
                    )],
                    placeholder: "Search icons…".into(),
                    footer_actions: vec![muxy_ui::command_popover::CommandPopoverAction::new(
                        "remove",
                        "Remove Icon",
                    )],
                    footer_hints: Vec::new(),
                    width: Some(400.0),
                    height: None,
                    max_height: Some(480.0),
                    completion_on_tab: false,
                    confirm_on_click: true,
                },
                theme,
                metrics,
                cx,
            )
        });
        let selected = self
            .state
            .workspace
            .project(&project_id)
            .and_then(|project| project.icon.as_deref());
        sync_symbol_picker(&picker, "", selected, cx);
        let target = project_id.clone();
        let subscription = cx.subscribe(&picker, move |window: &mut Self, picker, event, cx| {
            use muxy_ui::command_popover::CommandPopoverEvent;
            match event {
                CommandPopoverEvent::QueryChanged { query, .. } => {
                    let selected = window
                        .state
                        .workspace
                        .project(&target)
                        .and_then(|project| project.icon.as_deref());
                    sync_symbol_picker(&picker, query.as_ref(), selected, cx);
                }
                CommandPopoverEvent::Confirmed(selection)
                | CommandPopoverEvent::SecondaryConfirmed(selection) => {
                    if let Some(symbol) = selection.id.strip_prefix("symbol-") {
                        window.set_icon(&target, Some(symbol.to_owned()), cx);
                    }
                }
                CommandPopoverEvent::FooterAction(action) if action.as_ref() == "remove" => {
                    window.set_icon(&target, None, cx)
                }
                CommandPopoverEvent::Dismissed => window.dismiss_overlay(cx),
                _ => {}
            }
        });
        self.view.subscriptions = vec![subscription];
        self.view.overlay = Overlay::Symbols { picker, anchor };
        cx.notify();
    }

    pub(super) fn pick_logo(&mut self, project_id: String, cx: &mut Context<Self>) {
        let directory = self
            .state
            .workspace
            .project(&project_id)
            .map(|project| project.path.clone());
        self.dismiss_overlay(cx);

        cx.spawn(async move |window, cx| {
            let request = crate::views::file_dialog::ImageRequest {
                title: "Choose a Logo Image",
                directory,
            };
            let Some(source) = crate::views::file_dialog::pick_image(request).await else {
                return;
            };
            let Some(filename) = logo::store(&source, &project_id) else {
                let _ = window.update(cx, |window, cx| {
                    window.alert(
                        "Could Not Read Image".to_owned(),
                        "Muxy couldn't read that image. Choose a different file.".to_owned(),
                        cx,
                    );
                });
                return;
            };
            let _ = window.update(cx, |window, cx| {
                window.state.workspace.update(&project_id, |project| {
                    project.logo = Some(filename.clone());
                });
                cx.notify();
            });
        })
        .detach();
    }
}

fn sync_color_picker(
    picker: &Entity<muxy_ui::command_popover::CommandPopover>,
    query: &str,
    selected: Option<&str>,
    cx: &mut Context<MainWindow>,
) {
    let query = query.trim().to_lowercase();
    let items = muxy_core::store::ICON_PALETTE
        .iter()
        .filter(|swatch| query.is_empty() || swatch.id.contains(&query))
        .map(|swatch| {
            let mut row = muxy_ui::command_popover::CommandPopoverRow::new(
                format!("color-{}", swatch.id),
                title_case(swatch.id),
            );
            row.subtitle = Some(swatch.hex.into());
            row.leading = crate::views::swatches::icon_color(Some(swatch.id))
                .map(Into::into)
                .map(muxy_ui::command_popover::CommandPopoverLeading::Swatch);
            row.current = selected == Some(swatch.id);
            muxy_ui::command_popover::CommandPopoverItem::Row(row)
        })
        .collect::<Vec<_>>();
    let status = if items.is_empty() {
        muxy_ui::command_popover::CommandPopoverStatus::Empty("No colors found".into())
    } else {
        muxy_ui::command_popover::CommandPopoverStatus::Ready
    };
    picker.update(cx, |picker, cx| {
        picker.set_items(items, cx);
        picker.set_status(status, cx);
    });
}

fn title_case(value: &str) -> String {
    let mut characters = value.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

fn sync_symbol_picker(
    picker: &Entity<muxy_ui::command_popover::CommandPopover>,
    query: &str,
    selected: Option<&str>,
    cx: &mut Context<MainWindow>,
) {
    let items = muxy_ui::symbols::matching(query)
        .into_iter()
        .filter(|symbol| {
            cfg!(target_os = "macos") || muxy_ui::icon::Icon::from_symbol(symbol.symbol).is_some()
        })
        .map(|symbol| {
            let mut row = muxy_ui::command_popover::CommandPopoverRow::new(
                format!("symbol-{}", symbol.symbol),
                symbol.name,
            );
            row.subtitle = Some(symbol.symbol.into());
            row.leading = Some(muxy_ui::command_popover::CommandPopoverLeading::Symbol(
                symbol.symbol.into(),
            ));
            row.current = selected == Some(symbol.symbol);
            muxy_ui::command_popover::CommandPopoverItem::Row(row)
        })
        .collect::<Vec<_>>();
    let status = if items.is_empty() {
        muxy_ui::command_popover::CommandPopoverStatus::Empty("No icons found".into())
    } else {
        muxy_ui::command_popover::CommandPopoverStatus::Ready
    };
    picker.update(cx, |picker, cx| {
        picker.set_items(items, cx);
        picker.set_status(status, cx);
    });
}

fn app_layout_menu_items() -> Vec<Item> {
    vec![
        Item::action("Project Focused", Command::DismissOverlay).checked(true),
        Item::action("Tab Focused", Command::DismissOverlay).disabled(true),
        Item::action("Agents Focused", Command::DismissOverlay).disabled(true),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_ai_outcomes_preserve_exact_partial_effect_boundaries() {
        use muxy_api::repository::{
            MutationEffect, RepositoryAiCompletedBoundary, RepositoryAiWorkflowError,
            RepositoryAiWorkflowOutcome,
        };

        assert_eq!(
            repository_ai_workflow_effect(&Ok(RepositoryAiWorkflowOutcome::Committed {
                head_oid: "a".repeat(40),
            })),
            MutationEffect::Uncertain
        );
        let cases = [
            (RepositoryAiCompletedBoundary::None, None),
            (
                RepositoryAiCompletedBoundary::Staged,
                Some("staging changes"),
            ),
            (
                RepositoryAiCompletedBoundary::BranchCreated,
                Some("branch creation"),
            ),
            (
                RepositoryAiCompletedBoundary::Committed,
                Some("commit creation"),
            ),
            (RepositoryAiCompletedBoundary::Pushed, Some("push")),
            (
                RepositoryAiCompletedBoundary::PullRequestCreated,
                Some("pull request creation"),
            ),
        ];
        for (completed, expected) in cases {
            let effect = repository_ai_workflow_effect(&Err(RepositoryAiWorkflowError {
                completed,
                message: "failed".to_owned(),
            }));
            match expected {
                None => assert_eq!(effect, MutationEffect::NoMutation),
                Some(expected) => assert_eq!(
                    effect,
                    MutationEffect::PartialSuccess {
                        completed: expected
                    }
                ),
            }
        }
    }

    #[test]
    fn stale_repository_ai_completion_cannot_refresh_or_alert_the_new_context() {
        let key = crate::repository::RepositoryKey {
            project_id: "one".to_owned(),
            worktree_id: "primary".to_owned(),
            normalized_path: std::path::PathBuf::from("/one"),
        };
        let other = crate::repository::RepositoryKey {
            project_id: "two".to_owned(),
            worktree_id: "primary".to_owned(),
            normalized_path: std::path::PathBuf::from("/two"),
        };
        let completion = || crate::repository::MutationCompletion {
            effect: muxy_api::repository::MutationEffect::Uncertain,
            current_identity: true,
            refresh: crate::repository::RepositoryRefreshSet::repository_truth(),
        };
        assert!(current_repository_ai_completion(Some(completion()), Some(&key), &key).is_some());
        assert!(current_repository_ai_completion(Some(completion()), Some(&other), &key).is_none());
        let mut stale = completion();
        stale.current_identity = false;
        assert!(current_repository_ai_completion(Some(stale), Some(&key), &key).is_none());
    }

    #[test]
    fn repository_ai_confirmation_rejects_key_branch_and_head_drift() {
        let key = crate::repository::RepositoryKey {
            project_id: "one".to_owned(),
            worktree_id: "primary".to_owned(),
            normalized_path: std::path::PathBuf::from("/one"),
        };
        let identity = crate::views::repository::ai::RepositoryAiConfirmationIdentity {
            key: key.clone(),
            branch: "topic".to_owned(),
            head: muxy_api::repository::RepositoryHead::Commit("a".repeat(40)),
        };
        let mut summary = muxy_api::repository::RepositorySummary {
            branch: "topic".to_owned(),
            head: muxy_api::repository::RepositoryHead::Commit("a".repeat(40)),
            is_detached: false,
            upstream: None,
            ahead: 0,
            behind: 0,
            changed_count: 1,
            staged_count: 0,
            unstaged_count: 1,
            untracked_count: 0,
            conflicted_count: 0,
        };
        assert!(repository_ai_confirmation_is_current(
            &identity,
            Some(&key),
            &crate::repository::LoadState::Ready(summary.clone())
        ));
        summary.branch = "other".to_owned();
        assert!(!repository_ai_confirmation_is_current(
            &identity,
            Some(&key),
            &crate::repository::LoadState::Ready(summary.clone())
        ));
        summary.branch = "topic".to_owned();
        summary.head = muxy_api::repository::RepositoryHead::Commit("b".repeat(40));
        assert!(!repository_ai_confirmation_is_current(
            &identity,
            Some(&key),
            &crate::repository::LoadState::Ready(summary)
        ));
    }

    #[test]
    fn chrome_app_layout_menu_matches_supported_sidebar_layouts() {
        let items = app_layout_menu_items();
        let entries: Vec<(&str, bool, bool)> = items
            .iter()
            .map(|item| match item {
                Item::Action {
                    label,
                    disabled,
                    checked,
                    ..
                } => (label.as_ref(), *disabled, *checked),
                _ => panic!("app layout menu must contain actions only"),
            })
            .collect();

        assert_eq!(
            entries,
            vec![
                ("Project Focused", false, true),
                ("Tab Focused", true, false),
                ("Agents Focused", true, false),
            ]
        );
    }
}
