use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::{ExtensionApiError, ExtensionApiRequest, ExtensionApiServiceGroup};
use crate::execution_environment::ExecutionEnvironment;
use crate::git::command::{RepositoryCommandRequest, repository_command, run_output};
use crate::git::{
    GitOptions, SafeUntrackedDelete, add_worktree, current_branch, is_worktree_dirty,
    remove_worktree, validate_branch,
};
use crate::repository::{MutationControl, RepositoryOptions, RepositoryService};
use crate::subprocess::{
    CancellationSignal, Deadline, EnvironmentMode, StdinMode, SubprocessRequest, run,
};

const OUTPUT_LIMIT: usize = 8 * 1_024 * 1_024;
const ERROR_LIMIT: usize = 1_024 * 1_024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_TIMEOUT: Duration = Duration::from_secs(60 * 60);

#[derive(Clone)]
pub struct ExtensionTaskContext {
    pub worktree: PathBuf,
    pub environment: ExecutionEnvironment,
    pub git: GitOptions,
    pub cancellation: Option<CancellationSignal>,
    pub mutation_boundary: Option<crate::repository::MutationBoundary>,
}

pub fn dispatch_extension_task(
    group: ExtensionApiServiceGroup,
    request: &ExtensionApiRequest,
    context: &ExtensionTaskContext,
) -> Result<Value, ExtensionApiError> {
    match group {
        ExtensionApiServiceGroup::Execution => execute(request, context),
        ExtensionApiServiceGroup::Git => git(request, context),
        ExtensionApiServiceGroup::Worktrees if request.method == "worktrees.refresh" => {
            Ok(Value::Null)
        }
        _ => Err(ExtensionApiError::ServiceUnavailable(
            request.method.clone(),
        )),
    }
}

pub fn is_mutating_extension_task(method: &str) -> bool {
    matches!(
        method,
        "git.init"
            | "git.stage"
            | "git.unstage"
            | "git.discard"
            | "git.commit"
            | "git.push"
            | "git.pull"
            | "git.branch.create"
            | "git.branch.switch"
            | "git.pr.create"
            | "git.pr.merge"
            | "git.pr.close"
            | "git.worktree.add"
            | "git.worktree.remove"
            | "git.branch.delete"
            | "git.branch.deleteRemote"
            | "git.checkout"
            | "git.cherryPick"
            | "git.revert"
            | "git.tag.create"
            | "git.pr.checkout"
            | "git.pr.checkoutWorktree"
            | "worktrees.refresh"
    )
}

fn execute(
    request: &ExtensionApiRequest,
    context: &ExtensionTaskContext,
) -> Result<Value, ExtensionApiError> {
    let timeout = timeout(request.args.get("timeoutMs"));
    let cwd = request
        .args
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| context.worktree.clone());
    if !cwd.is_dir() {
        return Err(ExtensionApiError::InvalidArguments(
            "exec cwd must be an existing directory".to_owned(),
        ));
    }
    let shell = request
        .args
        .get("shell")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    let argv = string_array(&request.args, "argv")?;
    let (executable, args) = if let Some(shell) = shell {
        let executable = context
            .environment
            .get("SHELL".as_ref())
            .map(PathBuf::from)
            .filter(|path| path.is_file())
            .unwrap_or_else(|| PathBuf::from("/bin/sh"));
        (
            executable,
            vec![OsString::from("-lc"), OsString::from(shell)],
        )
    } else {
        let (program, args) = argv.split_first().ok_or_else(|| {
            ExtensionApiError::InvalidArguments("exec requires argv or shell".to_owned())
        })?;
        let executable = context
            .environment
            .resolve_executable(program.as_ref())
            .ok_or_else(|| {
                ExtensionApiError::InvalidArguments(format!(
                    "exec executable '{}' is unavailable",
                    program
                ))
            })?;
        (
            executable,
            args.iter().map(OsString::from).collect::<Vec<_>>(),
        )
    };
    let mut variables = context.environment.variables();
    if let Some(overrides) = request.args.get("env") {
        let overrides = overrides.as_object().ok_or_else(|| {
            ExtensionApiError::InvalidArguments("exec env must be an object".to_owned())
        })?;
        for (key, value) in overrides {
            if key.is_empty() || key.contains('=') || key.contains('\0') {
                return Err(ExtensionApiError::InvalidArguments(
                    "exec env contains an invalid key".to_owned(),
                ));
            }
            let value = value.as_str().ok_or_else(|| {
                ExtensionApiError::InvalidArguments("exec env values must be strings".to_owned())
            })?;
            variables.retain(|(candidate, _)| candidate != key.as_str());
            variables.push((OsString::from(key), OsString::from(value)));
        }
    }
    let stdin = request
        .args
        .get("stdin")
        .and_then(Value::as_str)
        .map(|value| StdinMode::Bytes(value.as_bytes().to_vec()))
        .unwrap_or(StdinMode::Closed);
    let deadline = Deadline::new(timeout);
    let output = run(
        SubprocessRequest {
            executable,
            args,
            current_dir: Some(cwd),
            stdin,
            environment: EnvironmentMode::Replace(variables),
            stdout_limit: OUTPUT_LIMIT,
            stderr_limit: ERROR_LIMIT,
            cancellation: None,
        },
        Some(&deadline),
    )
    .map_err(service_error)?;
    Ok(json!({
        "exitCode": output.status.code().unwrap_or(-1),
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
    }))
}

fn git(
    request: &ExtensionApiRequest,
    context: &ExtensionTaskContext,
) -> Result<Value, ExtensionApiError> {
    let service = RepositoryService::new(RepositoryOptions {
        git: context.git.clone(),
        environment: context.environment.clone(),
    });
    match request.method.as_str() {
        "git.status" => status(&service, context, request),
        "git.diff" => diff(context, request),
        "git.repoInfo" => repo_info(&service, context),
        "git.log" => log(context, request),
        "git.branches" => service
            .local_branches(&context.worktree)
            .map(|values| strings_value(values))
            .map_err(service_error),
        "git.remoteBranches" => service
            .remote_branches(&context.worktree)
            .map(|values| strings_value(values))
            .map_err(service_error),
        "git.currentBranch" => current_branch_value(&service, context),
        "git.aheadBehind" => ahead_behind(&service, context),
        "git.pr.info" => pull_request_info(context),
        "git.pr.number" => Ok(pull_request_info(context)?
            .get("number")
            .cloned()
            .unwrap_or(Value::Null)),
        "git.pr.diff" => pull_request_diff(context, request),
        "git.pr.list" => pull_request_list(context, request),
        "git.worktrees" => worktrees(context),
        "git.init" => unit_git(context, vec!["init".into()], false, false),
        "git.stage" => stage(&service, context, request),
        "git.unstage" => unstage(&service, context, request),
        "git.discard" => discard(&service, context, request),
        "git.commit" => commit(&service, context, request),
        "git.push" => push(context, request),
        "git.pull" => unit_git(context, vec!["pull".into()], false, true),
        "git.branch.create" => branch_create(&service, context, request),
        "git.branch.switch" => branch_switch(&service, context, request),
        "git.branch.delete" => branch_delete(&service, context, request),
        "git.branch.deleteRemote" => branch_delete_remote(context, request),
        "git.checkout" => hash_mutation(context, request, "checkout", &["checkout", "--detach"]),
        "git.cherryPick" => hash_mutation(context, request, "cherry-pick", &["cherry-pick"]),
        "git.revert" => hash_mutation(context, request, "revert", &["revert", "--no-edit"]),
        "git.tag.create" => tag_create(context, request),
        "git.pr.create" => pull_request_create(context, request),
        "git.pr.merge" => pull_request_merge(context, request),
        "git.pr.close" => pull_request_close(context, request),
        "git.pr.checkout" => pull_request_checkout(context, request),
        "git.pr.checkoutWorktree" => pull_request_checkout_worktree(context, request),
        "git.worktree.add" => worktree_add(context, request),
        "git.worktree.remove" => worktree_remove(context, request),
        "git.worktree.switch" => Err(ExtensionApiError::ServiceUnavailable(
            request.method.clone(),
        )),
        _ => Err(ExtensionApiError::ServiceUnavailable(
            request.method.clone(),
        )),
    }
}

fn status(
    service: &RepositoryService,
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let summary = service.summary(&context.worktree).map_err(service_error)?;
    let changed = service
        .changed_files(&context.worktree)
        .map_err(service_error)?;
    let branches = service
        .local_branches(&context.worktree)
        .map_err(service_error)?;
    let default_branch = default_branch(context)
        .map(Value::String)
        .unwrap_or(Value::Null);
    let staged_files = changed
        .files
        .iter()
        .filter(|file| file.is_staged)
        .map(|file| changed_file_value(file, true))
        .collect::<Vec<_>>();
    let unstaged_files = changed
        .files
        .iter()
        .filter(|file| file.is_unstaged)
        .map(|file| changed_file_value(file, false))
        .collect::<Vec<_>>();
    let pull_request = if request
        .args
        .get("local")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        Value::Null
    } else {
        pull_request_info(context).unwrap_or(Value::Null)
    };
    Ok(json!({
        "branch": summary.branch,
        "aheadBehind": {
            "ahead": summary.ahead,
            "behind": summary.behind,
            "hasUpstream": summary.upstream.is_some(),
        },
        "defaultBranch": default_branch,
        "branches": branches.into_iter().map(|value| String::from_utf8_lossy(&value).into_owned()).collect::<Vec<_>>(),
        "stagedFiles": staged_files,
        "unstagedFiles": unstaged_files,
        "pullRequest": pull_request,
    }))
}

fn changed_file_value(file: &crate::repository::ChangedFile, staged: bool) -> Value {
    let stat = if staged {
        file.staged_stat
    } else {
        file.unstaged_stat
    };
    let status = if staged { file.x_status } else { file.y_status };
    json!({
        "path": file.display_path(),
        "oldPath": file.display_old_path().map(|value| value.into_owned()),
        "status": char::from(status).to_string(),
        "isStaged": file.is_staged,
        "isUnstaged": file.is_unstaged,
        "isBinary": file.is_binary,
        "additions": stat.and_then(|value| value.additions),
        "deletions": stat.and_then(|value| value.deletions),
    })
}

fn diff(
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let raw = request
        .args
        .get("raw")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut args = vec!["diff".into(), "--no-color".into(), "--no-ext-diff".into()];
    if request
        .args
        .get("staged")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        args.push("--cached".into());
    }
    if let Some(path) = optional_repository_path(&request.args, "filePath")? {
        args.push("--".into());
        args.push(path.into_os_string());
    } else if !raw {
        return Err(ExtensionApiError::InvalidArguments(
            "git.diff requires filePath unless raw is true".to_owned(),
        ));
    }
    let output = git_text(context, args, true, false, OUTPUT_LIMIT)?;
    let (text, truncated) = truncate_lines(
        &output,
        request.args.get("lineLimit").and_then(Value::as_u64),
    );
    if raw {
        return Ok(json!({"diff": text, "truncated": truncated}));
    }
    let mut additions = 0_u64;
    let mut deletions = 0_u64;
    let rows = text
        .lines()
        .map(|line| {
            let (kind, old_text, new_text) = if line.starts_with("@@") {
                ("hunk", None, None)
            } else if line.starts_with('+') && !line.starts_with("+++") {
                additions += 1;
                ("addition", None, Some(line.trim_start_matches('+')))
            } else if line.starts_with('-') && !line.starts_with("---") {
                deletions += 1;
                ("deletion", Some(line.trim_start_matches('-')), None)
            } else {
                ("context", None, None)
            };
            json!({
                "kind": kind,
                "oldLineNumber": Value::Null,
                "newLineNumber": Value::Null,
                "oldText": old_text,
                "newText": new_text,
                "text": line,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "additions": additions,
        "deletions": deletions,
        "truncated": truncated,
        "rows": rows,
    }))
}

fn repo_info(
    service: &RepositoryService,
    context: &ExtensionTaskContext,
) -> Result<Value, ExtensionApiError> {
    let identity = service
        .repository_identity(&context.worktree)
        .map_err(service_error)?;
    let branch = current_branch(
        &context.git,
        &context.worktree,
        &Deadline::new(DEFAULT_TIMEOUT),
    )
    .map_err(service_error)?
    .unwrap_or_default();
    let common = git_text(
        context,
        vec!["rev-parse".into(), "--git-common-dir".into()],
        true,
        false,
        OUTPUT_LIMIT,
    )?;
    let git_dir = identity.git_dir.to_string_lossy().into_owned();
    let is_worktree = Path::new(common.trim())
        .file_name()
        .is_none_or(|name| name != ".git");
    Ok(json!({
        "root": identity.worktree_root.to_string_lossy(),
        "gitDir": git_dir,
        "isWorktree": is_worktree,
        "currentBranch": branch,
    }))
}

fn log(
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let count = request
        .args
        .get("maxCount")
        .and_then(Value::as_u64)
        .unwrap_or(100)
        .min(1000);
    let skip = request
        .args
        .get("skip")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = git_text(
        context,
        vec![
            "log".into(),
            format!("--max-count={count}").into(),
            format!("--skip={skip}").into(),
            "--date=iso-strict".into(),
            "--format=%H%x1f%h%x1f%s%x1f%an%x1f%aI%x1f%P%x1f%D%x1e".into(),
        ],
        true,
        false,
        OUTPUT_LIMIT,
    )?;
    let values = output
        .split('\u{1e}')
        .filter_map(|record| {
            let fields = record.trim().split('\u{1f}').collect::<Vec<_>>();
            (fields.len() == 7).then(|| {
                let parents = fields[5]
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                let refs = fields[6]
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| {
                        let name = value.strip_prefix("HEAD -> ").unwrap_or(value);
                        let kind = if value.starts_with("tag: ") {
                            "tag"
                        } else if value.starts_with("origin/") {
                            "remoteBranch"
                        } else if value.starts_with("HEAD") {
                            "head"
                        } else {
                            "localBranch"
                        };
                        json!({"name": name.trim_start_matches("tag: "), "kind": kind})
                    })
                    .collect::<Vec<_>>();
                json!({
                    "hash": fields[0],
                    "shortHash": fields[1],
                    "subject": fields[2],
                    "authorName": fields[3],
                    "authorDate": fields[4],
                    "isMerge": parents.len() > 1,
                    "parentHashes": parents,
                    "refs": refs,
                })
            })
        })
        .collect::<Vec<_>>();
    Ok(Value::Array(values))
}

fn current_branch_value(
    service: &RepositoryService,
    context: &ExtensionTaskContext,
) -> Result<Value, ExtensionApiError> {
    let summary = service.summary(&context.worktree).map_err(service_error)?;
    Ok(if summary.is_detached {
        Value::String(String::new())
    } else {
        Value::String(summary.branch)
    })
}

fn ahead_behind(
    service: &RepositoryService,
    context: &ExtensionTaskContext,
) -> Result<Value, ExtensionApiError> {
    let summary = service.summary(&context.worktree).map_err(service_error)?;
    Ok(json!({
        "ahead": summary.ahead,
        "behind": summary.behind,
        "hasUpstream": summary.upstream.is_some(),
    }))
}

fn stage(
    service: &RepositoryService,
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let paths = repository_paths(&request.args, "paths")?;
    let control = mutation_control(context);
    if paths.is_empty() {
        service
            .stage_all(&context.worktree, &control)
            .map_err(service_error)?;
    } else {
        let changed = service
            .changed_files(&context.worktree)
            .map_err(service_error)?;
        for path in paths {
            let file = changed
                .files
                .iter()
                .find(|file| file.path == path.as_os_str().as_encoded_bytes())
                .ok_or_else(|| {
                    ExtensionApiError::InvalidArguments(format!(
                        "'{}' is not a changed file",
                        path.display()
                    ))
                })?;
            service
                .stage(&context.worktree, file, &control)
                .map_err(service_error)?;
        }
    }
    Ok(Value::Null)
}

fn unstage(
    service: &RepositoryService,
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let paths = repository_paths(&request.args, "paths")?;
    let control = mutation_control(context);
    if paths.is_empty() {
        service
            .unstage_all(&context.worktree, &control)
            .map_err(service_error)?;
    } else {
        let changed = service
            .changed_files(&context.worktree)
            .map_err(service_error)?;
        for path in paths {
            let file = changed
                .files
                .iter()
                .find(|file| file.path == path.as_os_str().as_encoded_bytes())
                .ok_or_else(|| {
                    ExtensionApiError::InvalidArguments(format!(
                        "'{}' is not a staged file",
                        path.display()
                    ))
                })?;
            service
                .unstage(&context.worktree, file, &control)
                .map_err(service_error)?;
        }
    }
    Ok(Value::Null)
}

fn discard(
    service: &RepositoryService,
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let paths = repository_paths(&request.args, "paths")?;
    let untracked_paths = repository_paths(&request.args, "untrackedPaths")?;
    let control = mutation_control(context);
    let changed = service
        .changed_files(&context.worktree)
        .map_err(service_error)?;
    for path in paths {
        let file = changed
            .files
            .iter()
            .find(|file| file.path == path.as_os_str().as_encoded_bytes())
            .ok_or_else(|| {
                ExtensionApiError::InvalidArguments(format!(
                    "'{}' is not a changed file",
                    path.display()
                ))
            })?;
        service
            .discard(&context.worktree, file, &control)
            .map_err(service_error)?;
    }
    for path in untracked_paths {
        SafeUntrackedDelete::delete(&context.worktree, path.as_os_str().as_encoded_bytes())
            .map_err(service_error)?;
    }
    Ok(Value::Null)
}

fn commit(
    service: &RepositoryService,
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let message = required_string(&request.args, "message")?.trim();
    if message.is_empty() {
        return Err(ExtensionApiError::InvalidArguments(
            "commit message is required".to_owned(),
        ));
    }
    let control = mutation_control(context);
    if request
        .args
        .get("stageAll")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        service
            .stage_all(&context.worktree, &control)
            .map_err(service_error)?;
    }
    service
        .commit(&context.worktree, message, &control)
        .map_err(service_error)?;
    let hash = git_text(
        context,
        vec!["rev-parse".into(), "HEAD".into()],
        true,
        false,
        256,
    )?;
    Ok(json!({"hash": hash.trim()}))
}

fn push(
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let set_upstream = request
        .args
        .get("setUpstream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let has_upstream = git_output(
        context,
        vec![
            "rev-parse".into(),
            "--abbrev-ref".into(),
            "--symbolic-full-name".into(),
            "@{upstream}".into(),
        ],
        true,
        false,
        1024,
    )
    .is_ok_and(|output| output.status.success());
    if set_upstream || !has_upstream {
        let branch = required_current_branch(context)?;
        unit_git(
            context,
            vec![
                "push".into(),
                "--set-upstream".into(),
                "origin".into(),
                branch.into(),
            ],
            false,
            true,
        )
    } else {
        unit_git(context, vec!["push".into()], false, true)
    }
}

fn branch_create(
    service: &RepositoryService,
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let branch = validated_branch(required_string(&request.args, "name")?)?;
    service
        .create_branch(
            &context.worktree,
            branch.as_bytes(),
            &mutation_control(context),
        )
        .map_err(service_error)?;
    Ok(Value::Null)
}

fn branch_switch(
    service: &RepositoryService,
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let branch = validated_branch(required_string(&request.args, "branch")?)?;
    service
        .switch_branch(
            &context.worktree,
            branch.as_bytes(),
            &mutation_control(context),
        )
        .map_err(service_error)?;
    Ok(Value::Null)
}

fn branch_delete(
    service: &RepositoryService,
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let branch = validated_branch(required_string(&request.args, "name")?)?;
    let control = mutation_control(context);
    let intent = service
        .prepare_branch_deletion(&context.worktree, branch.as_bytes(), &control)
        .map_err(service_error)?;
    if !request
        .args
        .get("force")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let merged = git_output(
            context,
            vec!["branch".into(), "--merged".into(), "HEAD".into()],
            true,
            false,
            OUTPUT_LIMIT,
        )?;
        if !String::from_utf8_lossy(&merged.stdout)
            .lines()
            .map(|line| line.trim().trim_start_matches("* "))
            .any(|candidate| candidate == branch)
        {
            return Err(ExtensionApiError::Service(format!(
                "branch '{branch}' is not fully merged"
            )));
        }
    }
    service
        .delete_branch(&context.worktree, &intent, &control)
        .map_err(service_error)?;
    Ok(Value::Null)
}

fn branch_delete_remote(
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let branch = validated_branch(required_string(&request.args, "branch")?)?;
    let (remote, name) = branch.split_once('/').unwrap_or(("origin", &branch));
    unit_git(
        context,
        vec!["push".into(), remote.into(), "--delete".into(), name.into()],
        false,
        true,
    )
}

fn hash_mutation(
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
    label: &str,
    prefix: &[&str],
) -> Result<Value, ExtensionApiError> {
    let hash = required_string(&request.args, "hash")?.trim();
    if !valid_revision(hash) {
        return Err(ExtensionApiError::InvalidArguments(format!(
            "{label} requires a valid hash"
        )));
    }
    let mut args = prefix.iter().map(OsString::from).collect::<Vec<_>>();
    args.push(hash.into());
    unit_git(context, args, false, false)
}

fn tag_create(
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let name = validated_branch(required_string(&request.args, "name")?)?;
    let hash = required_string(&request.args, "hash")?.trim();
    if !valid_revision(hash) {
        return Err(ExtensionApiError::InvalidArguments(
            "tag.create requires a valid hash".to_owned(),
        ));
    }
    unit_git(
        context,
        vec!["tag".into(), name.into(), hash.into()],
        false,
        false,
    )
}

fn worktrees(context: &ExtensionTaskContext) -> Result<Value, ExtensionApiError> {
    let output = git_text(
        context,
        vec!["worktree".into(), "list".into(), "--porcelain".into()],
        true,
        false,
        OUTPUT_LIMIT,
    )?;
    let mut result = Vec::new();
    let mut current = Map::new();
    for line in output.lines().chain(std::iter::once("")) {
        if let Some(value) = line.strip_prefix("worktree ") {
            if !current.is_empty() {
                result.push(Value::Object(std::mem::take(&mut current)));
            }
            current.insert("path".to_owned(), Value::String(value.to_owned()));
            current.insert("branch".to_owned(), Value::Null);
            current.insert("head".to_owned(), Value::Null);
            current.insert("isBare".to_owned(), Value::Bool(false));
            current.insert("isDetached".to_owned(), Value::Bool(false));
            current.insert("isPrunable".to_owned(), Value::Bool(false));
        } else if let Some(value) = line.strip_prefix("HEAD ") {
            current.insert("head".to_owned(), Value::String(value.to_owned()));
        } else if let Some(value) = line.strip_prefix("branch ") {
            current.insert(
                "branch".to_owned(),
                Value::String(
                    value
                        .strip_prefix("refs/heads/")
                        .unwrap_or(value)
                        .to_owned(),
                ),
            );
        } else if line == "bare" {
            current.insert("isBare".to_owned(), Value::Bool(true));
        } else if line == "detached" {
            current.insert("isDetached".to_owned(), Value::Bool(true));
        } else if line.starts_with("prunable") {
            current.insert("isPrunable".to_owned(), Value::Bool(true));
        } else if line.is_empty() && !current.is_empty() {
            result.push(Value::Object(std::mem::take(&mut current)));
        }
    }
    Ok(Value::Array(result))
}

fn worktree_add(
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let path = required_string(&request.args, "path")?.trim();
    if path.is_empty() {
        return Err(ExtensionApiError::InvalidArguments(
            "worktree.add requires path".to_owned(),
        ));
    }
    let branch = validated_branch(required_string(&request.args, "branch")?)?;
    let base = request
        .args
        .get("baseBranch")
        .and_then(Value::as_str)
        .map(validated_branch)
        .transpose()?;
    add_worktree(
        &context.git,
        &context.worktree,
        &PathBuf::from(path),
        &branch,
        request
            .args
            .get("createBranch")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        base.as_deref(),
        &Deadline::new(DEFAULT_TIMEOUT),
    )
    .map_err(service_error)?;
    Ok(Value::Null)
}

fn worktree_remove(
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let path = PathBuf::from(required_string(&request.args, "path")?);
    let timeout = timeout(request.args.get("timeoutMs"));
    let deadline = Deadline::new(timeout);
    let dirty = is_worktree_dirty(&context.git, &path, &deadline).map_err(service_error)?;
    if dirty
        && !request
            .args
            .get("force")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        return Err(ExtensionApiError::Service(
            "worktree has uncommitted changes".to_owned(),
        ));
    }
    let outcome = remove_worktree(&context.git, &context.worktree, &path, &deadline)
        .map_err(service_error)?;
    Ok(json!({
        "path": path.to_string_lossy(),
        "dirRemoved": !path.exists() || outcome.reconciled,
    }))
}

fn pull_request_info(context: &ExtensionTaskContext) -> Result<Value, ExtensionApiError> {
    let output = gh_json(
        context,
        vec![
            "pr".into(),
            "view".into(),
            "--json".into(),
            "url,number,state,isDraft,baseRefName,mergeable,mergeStateStatus,statusCheckRollup,isCrossRepository".into(),
        ],
    )?;
    Ok(pr_info_value(output))
}

fn pull_request_diff(
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let number = required_positive_integer(&request.args, "number")?;
    let output = external_text(
        context,
        "gh",
        vec!["pr".into(), "diff".into(), number.to_string().into()],
        OUTPUT_LIMIT,
    )?;
    let (diff, truncated) = truncate_lines(
        &output,
        request.args.get("lineLimit").and_then(Value::as_u64),
    );
    Ok(json!({"diff": diff, "truncated": truncated}))
}

fn pull_request_list(
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let limit = request
        .args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(100)
        .clamp(1, 200);
    let state = request
        .args
        .get("filter")
        .and_then(Value::as_str)
        .unwrap_or("open");
    if !matches!(state, "open" | "closed" | "merged" | "all") {
        return Err(ExtensionApiError::InvalidArguments(
            "git.pr.list filter is invalid".to_owned(),
        ));
    }
    let output = gh_json(
        context,
        vec![
            "pr".into(),
            "list".into(),
            "--state".into(),
            state.into(),
            "--limit".into(),
            limit.to_string().into(),
            "--json".into(),
            "number,title,author,headRefName,baseRefName,state,isDraft,url,updatedAt,mergeable,mergeStateStatus,statusCheckRollup".into(),
        ],
    )?;
    let items = output.as_array().ok_or_else(|| {
        ExtensionApiError::Service("GitHub CLI returned an invalid pull request list".to_owned())
    })?;
    Ok(Value::Array(
        items
            .iter()
            .map(|item| {
                let checks = check_summary(item.get("statusCheckRollup"));
                json!({
                    "number": item.get("number").cloned().unwrap_or(Value::Null),
                    "title": item.get("title").cloned().unwrap_or(Value::String(String::new())),
                    "author": item.get("author").and_then(|value| value.get("login")).cloned().unwrap_or(Value::String(String::new())),
                    "headBranch": item.get("headRefName").cloned().unwrap_or(Value::String(String::new())),
                    "baseBranch": item.get("baseRefName").cloned().unwrap_or(Value::String(String::new())),
                    "state": lowercase_value(item.get("state")),
                    "isDraft": item.get("isDraft").cloned().unwrap_or(Value::Bool(false)),
                    "url": item.get("url").cloned().unwrap_or(Value::String(String::new())),
                    "updatedAt": item.get("updatedAt").cloned().unwrap_or(Value::Null),
                    "mergeable": nullable_lowercase_value(item.get("mergeable")),
                    "mergeStateStatus": lowercase_value(item.get("mergeStateStatus")),
                    "checks": checks,
                })
            })
            .collect(),
    ))
}

fn pull_request_create(
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let title = required_string(&request.args, "title")?.trim();
    if title.is_empty() {
        return Err(ExtensionApiError::InvalidArguments(
            "pull request title is required".to_owned(),
        ));
    }
    let mut args = vec![
        "pr".into(),
        "create".into(),
        "--title".into(),
        title.into(),
        "--body".into(),
        request
            .args
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
    ];
    if let Some(base) = request.args.get("baseBranch").and_then(Value::as_str) {
        args.push("--base".into());
        args.push(validated_branch(base)?.into());
    }
    if request
        .args
        .get("draft")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        args.push("--draft".into());
    }
    let url = external_text(context, "gh", args, OUTPUT_LIMIT)?;
    let output = gh_json(
        context,
        vec![
            "pr".into(),
            "view".into(),
            url.trim().into(),
            "--json".into(),
            "url,number,state,isDraft,baseRefName,mergeable,mergeStateStatus,statusCheckRollup,isCrossRepository".into(),
        ],
    )?;
    Ok(pr_info_value(output))
}

fn pull_request_merge(
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let number = required_positive_integer(&request.args, "number")?;
    let method = request
        .args
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("merge");
    let flag = match method {
        "merge" => "--merge",
        "squash" => "--squash",
        "rebase" => "--rebase",
        _ => {
            return Err(ExtensionApiError::InvalidArguments(
                "pull request merge method is invalid".to_owned(),
            ));
        }
    };
    let mut args = vec![
        "pr".into(),
        "merge".into(),
        number.to_string().into(),
        flag.into(),
    ];
    if request
        .args
        .get("deleteBranch")
        .and_then(Value::as_bool)
        .unwrap_or(true)
    {
        args.push("--delete-branch".into());
    }
    external_text(context, "gh", args, OUTPUT_LIMIT)?;
    Ok(Value::Null)
}

fn pull_request_close(
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let number = required_positive_integer(&request.args, "number")?;
    external_text(
        context,
        "gh",
        vec!["pr".into(), "close".into(), number.to_string().into()],
        OUTPUT_LIMIT,
    )?;
    Ok(Value::Null)
}

fn pull_request_checkout(
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let number = required_positive_integer(&request.args, "number")?;
    external_text(
        context,
        "gh",
        vec!["pr".into(), "checkout".into(), number.to_string().into()],
        OUTPUT_LIMIT,
    )?;
    Ok(Value::Null)
}

fn pull_request_checkout_worktree(
    context: &ExtensionTaskContext,
    request: &ExtensionApiRequest,
) -> Result<Value, ExtensionApiError> {
    let number = required_positive_integer(&request.args, "number")?;
    let path = required_string(&request.args, "path")?.trim();
    if path.is_empty() {
        return Err(ExtensionApiError::InvalidArguments(
            "pull request worktree path is required".to_owned(),
        ));
    }
    let branch = format!("muxy/pr-{number}");
    unit_git(
        context,
        vec![
            "fetch".into(),
            "origin".into(),
            format!("pull/{number}/head:refs/heads/{branch}").into(),
        ],
        false,
        true,
    )?;
    add_worktree(
        &context.git,
        &context.worktree,
        Path::new(path),
        &branch,
        false,
        None,
        &Deadline::new(DEFAULT_TIMEOUT),
    )
    .map_err(service_error)?;
    Ok(json!({"branch": branch}))
}

fn pr_info_value(value: Value) -> Value {
    let checks = check_summary(value.get("statusCheckRollup"));
    json!({
        "url": value.get("url").cloned().unwrap_or(Value::String(String::new())),
        "number": value.get("number").cloned().unwrap_or(Value::Null),
        "state": lowercase_value(value.get("state")),
        "isDraft": value.get("isDraft").cloned().unwrap_or(Value::Bool(false)),
        "baseBranch": value.get("baseRefName").cloned().unwrap_or(Value::String(String::new())),
        "mergeable": nullable_lowercase_value(value.get("mergeable")),
        "mergeStateStatus": lowercase_value(value.get("mergeStateStatus")),
        "isCrossRepository": value.get("isCrossRepository").cloned().unwrap_or(Value::Bool(false)),
        "checks": checks,
    })
}

fn check_summary(value: Option<&Value>) -> Value {
    let Some(checks) = value.and_then(Value::as_array) else {
        return json!({"status": "none", "passing": 0, "failing": 0, "pending": 0, "total": 0});
    };
    let mut passing = 0_usize;
    let mut failing = 0_usize;
    let mut pending = 0_usize;
    for check in checks {
        let conclusion = check
            .get("conclusion")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let status = check
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if matches!(conclusion, "SUCCESS" | "NEUTRAL" | "SKIPPED") {
            passing += 1;
        } else if matches!(conclusion, "FAILURE" | "ERROR" | "CANCELLED" | "TIMED_OUT") {
            failing += 1;
        } else if status != "COMPLETED" || conclusion.is_empty() {
            pending += 1;
        }
    }
    let state = if failing > 0 {
        "failure"
    } else if pending > 0 {
        "pending"
    } else if passing > 0 {
        "success"
    } else {
        "none"
    };
    json!({
        "status": state,
        "passing": passing,
        "failing": failing,
        "pending": pending,
        "total": checks.len(),
    })
}

fn default_branch(context: &ExtensionTaskContext) -> Option<String> {
    git_text(
        context,
        vec![
            "symbolic-ref".into(),
            "--short".into(),
            "refs/remotes/origin/HEAD".into(),
        ],
        true,
        false,
        1024,
    )
    .ok()
    .map(|value| value.trim().trim_start_matches("origin/").to_owned())
    .filter(|value| !value.is_empty())
    .or_else(|| {
        git_text(
            context,
            vec!["config".into(), "--get".into(), "init.defaultBranch".into()],
            true,
            false,
            1024,
        )
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    })
}

fn required_current_branch(context: &ExtensionTaskContext) -> Result<String, ExtensionApiError> {
    current_branch(
        &context.git,
        &context.worktree,
        &Deadline::new(DEFAULT_TIMEOUT),
    )
    .map_err(service_error)?
    .ok_or_else(|| ExtensionApiError::Service("repository has a detached HEAD".to_owned()))
}

fn unit_git(
    context: &ExtensionTaskContext,
    args: Vec<OsString>,
    read_only: bool,
    network: bool,
) -> Result<Value, ExtensionApiError> {
    git_text(context, args, read_only, network, OUTPUT_LIMIT)?;
    Ok(Value::Null)
}

fn git_text(
    context: &ExtensionTaskContext,
    args: Vec<OsString>,
    read_only: bool,
    network: bool,
    limit: usize,
) -> Result<String, ExtensionApiError> {
    let output = git_output(context, args, read_only, network, limit)?;
    if !output.status.success() {
        return Err(command_error("git", &output.stderr));
    }
    if output.stdout_truncated || output.stderr_truncated {
        return Err(ExtensionApiError::Service(
            "git output exceeded the allowed size".to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn git_output(
    context: &ExtensionTaskContext,
    args: Vec<OsString>,
    read_only: bool,
    network: bool,
    limit: usize,
) -> Result<crate::subprocess::SubprocessOutput, ExtensionApiError> {
    let command = repository_command(
        &context.environment,
        RepositoryCommandRequest {
            args,
            read_only,
            network,
            stdin: StdinMode::Closed,
            stdout_limit: limit,
            stderr_limit: ERROR_LIMIT,
            cancellation: context.cancellation.clone(),
        },
    );
    run_output(
        &context.git,
        &context.worktree,
        command,
        &Deadline::new(DEFAULT_TIMEOUT),
    )
    .map_err(service_error)
}

fn mutation_control(context: &ExtensionTaskContext) -> MutationControl {
    match (
        context.cancellation.clone(),
        context.mutation_boundary.clone(),
    ) {
        (Some(cancellation), Some(boundary)) => {
            MutationControl::with_cancellation_and_boundary(cancellation, boundary)
        }
        (Some(cancellation), None) => MutationControl::with_cancellation(cancellation),
        (None, _) => MutationControl::with_timeout(DEFAULT_TIMEOUT),
    }
}

fn gh_json(
    context: &ExtensionTaskContext,
    args: Vec<OsString>,
) -> Result<Value, ExtensionApiError> {
    let text = external_text(context, "gh", args, OUTPUT_LIMIT)?;
    serde_json::from_str(&text).map_err(|error| {
        ExtensionApiError::Service(format!("invalid GitHub CLI response: {error}"))
    })
}

fn external_text(
    context: &ExtensionTaskContext,
    executable: &str,
    args: Vec<OsString>,
    limit: usize,
) -> Result<String, ExtensionApiError> {
    let executable = context
        .environment
        .resolve_executable(executable.as_ref())
        .ok_or_else(|| ExtensionApiError::Service(format!("{executable} is unavailable")))?;
    let output = run(
        SubprocessRequest {
            executable,
            args,
            current_dir: Some(context.worktree.clone()),
            stdin: StdinMode::Closed,
            environment: EnvironmentMode::Replace(context.environment.variables()),
            stdout_limit: limit,
            stderr_limit: ERROR_LIMIT,
            cancellation: None,
        },
        Some(&Deadline::new(DEFAULT_TIMEOUT)),
    )
    .map_err(service_error)?;
    if !output.status.success() {
        return Err(command_error("command", &output.stderr));
    }
    if output.stdout_truncated || output.stderr_truncated {
        return Err(ExtensionApiError::Service(
            "command output exceeded the allowed size".to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn repository_paths(
    args: &Map<String, Value>,
    key: &str,
) -> Result<Vec<PathBuf>, ExtensionApiError> {
    string_array(args, key)?
        .into_iter()
        .map(|path| validate_repository_path(&path))
        .collect()
}

fn optional_repository_path(
    args: &Map<String, Value>,
    key: &str,
) -> Result<Option<PathBuf>, ExtensionApiError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(validate_repository_path)
        .transpose()
}

fn validate_repository_path(value: &str) -> Result<PathBuf, ExtensionApiError> {
    let path = PathBuf::from(value);
    if value.is_empty()
        || value.contains('\0')
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
        || !path
            .components()
            .any(|component| matches!(component, Component::Normal(_)))
    {
        return Err(ExtensionApiError::InvalidArguments(format!(
            "invalid repository path '{value}'"
        )));
    }
    Ok(path)
}

fn validated_branch(value: &str) -> Result<String, ExtensionApiError> {
    let value = value.trim();
    validate_branch(value).map_err(|_| {
        ExtensionApiError::InvalidArguments(format!("invalid branch name '{value}'"))
    })?;
    Ok(value.to_owned())
}

fn valid_revision(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || matches!(byte, b'/' | b'_' | b'-' | b'.'))
}

fn required_string<'a>(
    args: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, ExtensionApiError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ExtensionApiError::InvalidArguments(format!("{key} is required")))
}

fn required_positive_integer(
    args: &Map<String, Value>,
    key: &str,
) -> Result<u64, ExtensionApiError> {
    args.get(key)
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| ExtensionApiError::InvalidArguments(format!("{key} is required")))
}

fn string_array(args: &Map<String, Value>, key: &str) -> Result<Vec<String>, ExtensionApiError> {
    let Some(value) = args.get(key) else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| ExtensionApiError::InvalidArguments(format!("{key} must be an array")))?;
    values
        .iter()
        .map(|value| {
            value.as_str().map(str::to_owned).ok_or_else(|| {
                ExtensionApiError::InvalidArguments(format!("{key} must contain strings"))
            })
        })
        .collect()
}

fn strings_value(values: Vec<Vec<u8>>) -> Value {
    Value::Array(
        values
            .into_iter()
            .map(|value| Value::String(String::from_utf8_lossy(&value).into_owned()))
            .collect(),
    )
}

fn truncate_lines(value: &str, limit: Option<u64>) -> (String, bool) {
    let Some(limit) = limit.map(|value| value.min(100_000) as usize) else {
        return (value.to_owned(), false);
    };
    let mut lines = value.lines();
    let text = lines.by_ref().take(limit).collect::<Vec<_>>().join("\n");
    let truncated = lines.next().is_some();
    (text, truncated)
}

fn lowercase_value(value: Option<&Value>) -> Value {
    Value::String(
        value
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase(),
    )
}

fn nullable_lowercase_value(value: Option<&Value>) -> Value {
    let value = value.and_then(Value::as_str).unwrap_or_default();
    if value.is_empty() || value == "UNKNOWN" {
        Value::Null
    } else {
        Value::String(value.to_ascii_lowercase())
    }
}

fn timeout(value: Option<&Value>) -> Duration {
    value
        .and_then(Value::as_u64)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_TIMEOUT)
        .clamp(Duration::from_millis(1), MAX_TIMEOUT)
}

fn command_error(command: &str, stderr: &[u8]) -> ExtensionApiError {
    let detail = crate::subprocess::bounded_error_text(stderr);
    ExtensionApiError::Service(if detail.trim().is_empty() {
        format!("{command} failed")
    } else {
        detail
    })
}

fn service_error(error: impl std::fmt::Display) -> ExtensionApiError {
    ExtensionApiError::Service(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxy_core::extensions::manifest::ExtensionPermission;
    use std::collections::BTreeSet;
    use std::collections::HashMap;
    use std::process::Command;

    fn request(method: &str, args: Value) -> ExtensionApiRequest {
        ExtensionApiRequest::new(
            "git",
            method,
            args.as_object().cloned().unwrap_or_default(),
            super::super::ExtensionApiSource::Webview,
            BTreeSet::from([
                ExtensionPermission::GitRead.as_str().to_owned(),
                ExtensionPermission::GitWrite.as_str().to_owned(),
            ]),
        )
    }

    #[test]
    fn repository_paths_reject_absolute_and_parent_traversal() {
        for path in ["/tmp/outside", "../outside", "a/../../outside", ""] {
            assert!(validate_repository_path(path).is_err(), "{path}");
        }
        assert_eq!(
            validate_repository_path("src/main.rs").unwrap(),
            PathBuf::from("src/main.rs")
        );
    }

    #[test]
    fn mutating_task_catalog_separates_reads_and_writes() {
        assert!(!is_mutating_extension_task("git.status"));
        assert!(!is_mutating_extension_task("git.diff"));
        assert!(is_mutating_extension_task("git.commit"));
        assert!(is_mutating_extension_task("git.worktree.remove"));
    }

    #[test]
    fn invalid_hash_and_branch_arguments_fail_before_execution() {
        assert!(!valid_revision("--upload-pack=bad"));
        assert!(validated_branch("--bad").is_err());
        assert!(required_positive_integer(&Map::new(), "number").is_err());
        let request = request("git.checkout", json!({"hash": "--bad"}));
        assert_eq!(request.method, "git.checkout");
    }

    #[test]
    fn git_extension_status_stage_commit_and_branch_use_repository_services() {
        let directory = tempfile::tempdir().unwrap();
        let repository = directory.path().join("repo");
        std::fs::create_dir(&repository).unwrap();
        command(&repository, &["init", "-q"]);
        command(&repository, &["config", "user.name", "Muxy Test"]);
        command(
            &repository,
            &["config", "user.email", "muxy@example.invalid"],
        );
        std::fs::write(repository.join("file.txt"), "one\n").unwrap();
        command(&repository, &["add", "file.txt"]);
        command(&repository, &["commit", "-qm", "initial"]);
        std::fs::write(repository.join("file.txt"), "one\ntwo\n").unwrap();

        let environment = ExecutionEnvironment::from_current_process();
        let context = ExtensionTaskContext {
            worktree: repository,
            git: GitOptions {
                executable: environment.resolve_executable("git".as_ref()).unwrap(),
                environment: HashMap::new(),
            },
            environment,
            cancellation: None,
            mutation_boundary: None,
        };
        let status = dispatch_extension_task(
            ExtensionApiServiceGroup::Git,
            &request("git.status", json!({"local": true})),
            &context,
        )
        .unwrap();
        assert_eq!(status["unstagedFiles"][0]["path"], "file.txt");

        dispatch_extension_task(
            ExtensionApiServiceGroup::Git,
            &request("git.stage", json!({"paths": ["file.txt"]})),
            &context,
        )
        .unwrap();
        let status = dispatch_extension_task(
            ExtensionApiServiceGroup::Git,
            &request("git.status", json!({"local": true})),
            &context,
        )
        .unwrap();
        assert_eq!(status["stagedFiles"][0]["path"], "file.txt");

        let committed = dispatch_extension_task(
            ExtensionApiServiceGroup::Git,
            &request(
                "git.commit",
                json!({"message": "extension commit", "stageAll": false}),
            ),
            &context,
        )
        .unwrap();
        assert_eq!(committed["hash"].as_str().unwrap().len(), 40);

        dispatch_extension_task(
            ExtensionApiServiceGroup::Git,
            &request("git.branch.create", json!({"name": "extension-branch"})),
            &context,
        )
        .unwrap();
        let branch = dispatch_extension_task(
            ExtensionApiServiceGroup::Git,
            &request("git.currentBranch", json!({})),
            &context,
        )
        .unwrap();
        assert_eq!(branch, "extension-branch");
    }

    fn command(repository: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }
}
