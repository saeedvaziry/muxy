use gpui::{Context, PathPromptOptions, PromptButton, PromptLevel, SharedString};
use muxy_api::extensions::{
    ExtensionApiError, ExtensionApiRequest, ExtensionApiServiceGroup, ExtensionTaskContext,
};
use serde_json::{Value, json};
use std::collections::HashSet;

use super::MainWindow;

impl MainWindow {
    pub(crate) fn dispatch_deferred_extension_request(
        &mut self,
        request: ExtensionApiRequest,
        group: ExtensionApiServiceGroup,
        completion: crate::extensions::ExtensionApiCompletion,
        cx: &mut Context<Self>,
    ) {
        if group == ExtensionApiServiceGroup::Dialogs {
            self.dispatch_extension_dialog(request, completion, cx);
            return;
        }
        if request.method == "git.worktree.switch" {
            let result = self.switch_extension_worktree(&request, cx);
            completion(result);
            return;
        }
        if request.method == "worktrees.refresh" {
            let result = self
                .extension_task_project(&request)
                .map(|(project_id, _)| {
                    self.request_worktree_refresh(project_id, None, cx);
                    Value::Null
                });
            completion(result);
            return;
        }
        let (project_id, mut context) = match self.extension_task_context(&request) {
            Ok(context) => context,
            Err(error) => {
                completion(Err(error));
                return;
            }
        };
        let method = request.method.clone();
        let mutating = muxy_api::extensions::is_mutating_extension_task(&method);
        let mut operation = None;
        let mut repository_mutation = false;
        if mutating {
            let token = match self.state.project_operations.begin_operation(
                &project_id,
                crate::project_operations::ProjectOperationKind::RepositoryMutation,
            ) {
                Ok(token) => token,
                Err(_) => {
                    completion(Err(ExtensionApiError::Service(
                        "another project mutation is running".to_owned(),
                    )));
                    return;
                }
            };
            let active_repository = self
                .view
                .repository
                .coordinator
                .key()
                .is_some_and(|key| key.project_id == project_id);
            if active_repository {
                let Some((cancellation, boundary)) = self
                    .view
                    .repository
                    .coordinator
                    .begin_mutation(token.request_id())
                else {
                    let _ = self.state.project_operations.finish_operation(&token);
                    completion(Err(ExtensionApiError::Service(
                        "another repository mutation is running".to_owned(),
                    )));
                    return;
                };
                context.cancellation = Some(cancellation);
                context.mutation_boundary = Some(boundary);
                repository_mutation = true;
            }
            operation = Some(token);
        }
        let task = cx.background_executor().spawn(async move {
            muxy_api::extensions::dispatch_extension_task(group, &request, &context)
        });
        cx.spawn(async move |window, cx| {
            let result = task.await;
            let _ = window.update(cx, |window, cx| {
                if mutating {
                    if let Some(token) = operation {
                        let _ = window.state.project_operations.finish_operation(&token);
                        if repository_mutation {
                            let _ = window.view.repository.coordinator.finish_mutation(
                                token.request_id(),
                                muxy_api::repository::MutationEffect::Uncertain,
                            );
                            window
                                .view
                                .repository
                                .coordinator
                                .request_refresh(crate::repository::RepositoryRefreshSet::all());
                            window.dispatch_repository_refresh(cx);
                        }
                    }
                    window.refresh_project_truth(Some(&HashSet::from([project_id])), cx);
                    window.publish_extension_event(
                        "file.changed",
                        json!({"source": "extension", "method": method}),
                    );
                }
                completion(result);
            });
        })
        .detach();
    }

    fn extension_task_context(
        &self,
        request: &ExtensionApiRequest,
    ) -> Result<(String, ExtensionTaskContext), ExtensionApiError> {
        let (project_id, path) = self.extension_task_project(request)?;
        let worktree = std::fs::canonicalize(&path).map_err(|error| {
            ExtensionApiError::Service(format!("could not resolve project worktree: {error}"))
        })?;
        if !worktree.is_dir() {
            return Err(ExtensionApiError::InvalidArguments(
                "project worktree is not a directory".to_owned(),
            ));
        }
        Ok((
            project_id,
            ExtensionTaskContext {
                worktree,
                environment: self.project_runtime.execution_environment.snapshot(),
                git: self.project_runtime.git_options.clone(),
                cancellation: None,
                mutation_boundary: None,
            },
        ))
    }

    fn extension_task_project(
        &self,
        request: &ExtensionApiRequest,
    ) -> Result<(String, String), ExtensionApiError> {
        let requested = request
            .args
            .get("project")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        let project = match requested {
            Some(identifier) => self.state.workspace.projects.iter().find(|project| {
                project.id.eq_ignore_ascii_case(identifier)
                    || project.name.eq_ignore_ascii_case(identifier)
                    || project.path == identifier
            }),
            None => self.state.active_project(),
        }
        .ok_or_else(|| ExtensionApiError::InvalidArguments("no active project".to_owned()))?;
        if project.is_home() || project.is_remote() {
            return Err(ExtensionApiError::InvalidArguments(
                "extension task requires a local project".to_owned(),
            ));
        }
        let path = if self.state.active_project_id.as_deref() == Some(project.id.as_str()) {
            self.state.active_worktree_path(project)
        } else {
            project.path.clone()
        };
        Ok((project.id.clone(), path))
    }

    fn switch_extension_worktree(
        &mut self,
        request: &ExtensionApiRequest,
        cx: &mut Context<Self>,
    ) -> Result<Value, ExtensionApiError> {
        let identifier = request
            .args
            .get("identifier")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ExtensionApiError::InvalidArguments(
                    "git.worktree.switch requires identifier".to_owned(),
                )
            })?;
        let (project_id, _) = self.extension_task_project(request)?;
        let worktree_id = self
            .state
            .worktrees
            .get(&project_id)
            .and_then(|worktrees| {
                worktrees.iter().find(|worktree| {
                    worktree.id.eq_ignore_ascii_case(identifier)
                        || worktree.name.eq_ignore_ascii_case(identifier)
                        || worktree.path == identifier
                })
            })
            .map(|worktree| worktree.id.clone())
            .ok_or_else(|| {
                ExtensionApiError::InvalidArguments(format!(
                    "worktree '{identifier}' is not registered"
                ))
            })?;
        if !self.state.try_select_worktree(&project_id, &worktree_id) {
            return Err(ExtensionApiError::Service(
                "could not persist selected worktree".to_owned(),
            ));
        }
        self.sync_repository_context(cx);
        self.publish_extension_event(
            "worktree.switched",
            json!({"projectID": project_id, "worktreeID": worktree_id}),
        );
        cx.notify();
        Ok(Value::Null)
    }

    fn dispatch_extension_dialog(
        &mut self,
        request: ExtensionApiRequest,
        completion: crate::extensions::ExtensionApiCompletion,
        cx: &mut Context<Self>,
    ) {
        if request.method == "dialog.pickFolder" {
            let receiver = cx.prompt_for_paths(PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: request
                    .args
                    .get("title")
                    .and_then(Value::as_str)
                    .map(|value| SharedString::from(value.to_owned())),
            });
            cx.spawn(async move |_, _| {
                let result = receiver
                    .await
                    .map_err(|error| ExtensionApiError::Service(error.to_string()))
                    .and_then(|result| {
                        result.map_err(|error| ExtensionApiError::Service(error.to_string()))
                    })
                    .map(|paths| {
                        paths
                            .and_then(|paths| paths.into_iter().next())
                            .map(|path| Value::String(path.to_string_lossy().into_owned()))
                            .unwrap_or(Value::Null)
                    });
                completion(result);
            })
            .detach();
            return;
        }
        if request.method == "dialog.prompt" {
            completion(Err(ExtensionApiError::ServiceUnavailable(request.method)));
            return;
        }
        let title = request
            .args
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let message = request
            .args
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let level = match request.args.get("style").and_then(Value::as_str) {
            Some("critical") => PromptLevel::Critical,
            Some("info") => PromptLevel::Info,
            _ => PromptLevel::Warning,
        };
        let labels = if request.method == "dialog.alert" {
            vec!["OK".to_owned()]
        } else {
            request
                .args
                .get("buttons")
                .and_then(Value::as_array)
                .map(|buttons| {
                    buttons
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .filter(|buttons| !buttons.is_empty())
                .unwrap_or_else(|| vec!["OK".to_owned(), "Cancel".to_owned()])
        };
        let cancel = request
            .args
            .get("cancel")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let buttons = labels
            .iter()
            .map(|label| {
                if cancel.as_deref() == Some(label) {
                    PromptButton::cancel(label.clone())
                } else {
                    PromptButton::new(label.clone())
                }
            })
            .collect::<Vec<_>>();
        let handle = cx.active_window();
        let is_alert = request.method == "dialog.alert";
        cx.spawn(async move |_, cx| {
            let Some(handle) = handle else {
                completion(Err(ExtensionApiError::Service(
                    "dialog window is unavailable".to_owned(),
                )));
                return;
            };
            let receiver = match cx.update(|cx| {
                handle
                    .update(cx, |_, window, cx| {
                        window.prompt(level, &title, Some(&message), &buttons, cx)
                    })
                    .map_err(|error| ExtensionApiError::Service(error.to_string()))
            }) {
                Ok(Ok(receiver)) => receiver,
                Ok(Err(error)) => {
                    completion(Err(ExtensionApiError::Service(error.to_string())));
                    return;
                }
                Err(error) => {
                    completion(Err(ExtensionApiError::Service(error.to_string())));
                    return;
                }
            };
            let result = receiver
                .await
                .map_err(|error| ExtensionApiError::Service(error.to_string()))
                .map(|index| {
                    if is_alert {
                        Value::Null
                    } else {
                        labels
                            .get(index)
                            .cloned()
                            .map(Value::String)
                            .unwrap_or(Value::Null)
                    }
                });
            completion(result);
        })
        .detach();
    }
}
