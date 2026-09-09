use super::MainWindow;
use crate::composer::{ComposerTarget, TargetTransition};
use gpui::{
    App, AppContext, Context, Entity, Focusable, Pixels, Point, SharedString, Window, point, px,
};
use muxy_core::composer::submission::{
    ImageSubmissionStrategy, SubmissionSnapshot, plan_submission,
};
use muxy_core::composer::{ComposerStore, DraftId};
use muxy_core::prefs::{
    COMPOSER_FONT_SIZE_MAX, COMPOSER_FONT_SIZE_MIN, ComposerPanelMode, ComposerPanelPosition,
    ComposerPreferences,
};
use muxy_core::workspace::TabKind;
use muxy_terminal::input::{TerminalInputResult, TerminalInputTransaction};
use muxy_ui::panel::{PanelMode, PanelPlacement, PanelPosition};
use muxy_ui::text_input::{InputEvent, InputStyle, TextInput};
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

fn panel_position(position: ComposerPanelPosition) -> PanelPosition {
    match position {
        ComposerPanelPosition::Right => PanelPosition::Right,
        ComposerPanelPosition::Bottom => PanelPosition::Bottom,
    }
}

fn panel_mode(mode: ComposerPanelMode) -> PanelMode {
    match mode {
        ComposerPanelMode::Pinned => PanelMode::Pinned,
        ComposerPanelMode::Floating => PanelMode::Floating,
    }
}

pub(crate) fn composer_font_shortcut_delta(key: &str, platform: bool) -> Option<f64> {
    if !platform {
        return None;
    }
    match key {
        "+" | "=" | "plus" => Some(1.0),
        "-" | "minus" => Some(-1.0),
        _ => None,
    }
}

fn publish_composer_release(
    store: &mut ComposerStore,
    id: DraftId,
    text: String,
    attachments: Vec<String>,
    clear: bool,
) -> std::io::Result<()> {
    store.edit_content(id.clone(), text.clone(), attachments.clone())?;
    if !clear {
        return store.flush().map(|_| ());
    }
    store.edit_content(id.clone(), String::new(), Vec::new())?;
    if let Err(error) = store.flush() {
        if let Err(restore_error) = store.edit_content(id, text, attachments) {
            return Err(std::io::Error::other(format!(
                "{error}; failed to retain the Composer editor after publication failure: {restore_error}"
            )));
        }
        return Err(error);
    }
    Ok(())
}

fn composer_input_style(window: &MainWindow) -> InputStyle {
    let font_size = window.state.prefs.composer.font_size as f32;
    let multiplier =
        muxy_core::prefs::settings::f64_value("editor.richInputLineHeightMultiplier", 1.2)
            .clamp(1.1, 2.0) as f32;
    InputStyle {
        font_size: gpui::px(font_size),
        line_height: gpui::px(font_size * multiplier),
        ..InputStyle::field(&window.state.theme, &window.state.metrics)
    }
}

fn phase_4_status_path(
    is_test_process: bool,
    case_name: Option<&str>,
    app_support: &Path,
    injected_app_support: Option<&Path>,
    home: &Path,
) -> Option<PathBuf> {
    if !is_test_process
        || case_name != Some("phase-4")
        || injected_app_support != Some(app_support)
        || !app_support.is_absolute()
        || app_support.starts_with(home)
        || app_support
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        || std::fs::symlink_metadata(app_support)
            .ok()
            .is_none_or(|metadata| !metadata.is_dir() || metadata.file_type().is_symlink())
        || std::fs::canonicalize(app_support).ok().as_deref() != Some(app_support)
    {
        return None;
    }
    Some(app_support.join(".muxy-p7-panel-status.json"))
}

pub(crate) fn current_phase_4_status_path() -> Option<PathBuf> {
    let app_support = muxy_core::prefs::app_support_dir();
    let injected = std::env::var_os("MUXY_TEST_APPLICATION_SUPPORT_DIRECTORY").map(PathBuf::from);
    let case_name = std::env::var("MUXY_TEST_P7_COMPOSER_CASE").ok();
    phase_4_status_path(
        muxy_core::prefs::is_test_process(),
        case_name.as_deref(),
        &app_support,
        injected.as_deref(),
        &muxy_core::prefs::home_dir(),
    )
}

fn current_phase_5_status_path() -> Option<PathBuf> {
    let app_support = muxy_core::prefs::app_support_dir();
    let injected = std::env::var_os("MUXY_TEST_APPLICATION_SUPPORT_DIRECTORY").map(PathBuf::from);
    let case_name = std::env::var("MUXY_TEST_P7_COMPOSER_CASE").ok();
    phase_4_status_path(
        muxy_core::prefs::is_test_process(),
        (case_name.as_deref() == Some("phase-5")).then_some("phase-4"),
        &app_support,
        injected.as_deref(),
        &muxy_core::prefs::home_dir(),
    )
    .map(|_| app_support.join(".muxy-p7-submission-status.json"))
}

fn current_phase_6_status_path() -> Option<PathBuf> {
    let app_support = muxy_core::prefs::app_support_dir();
    let injected = std::env::var_os("MUXY_TEST_APPLICATION_SUPPORT_DIRECTORY").map(PathBuf::from);
    let case_name = std::env::var("MUXY_TEST_P7_COMPOSER_CASE").ok();
    phase_4_status_path(
        muxy_core::prefs::is_test_process(),
        (case_name.as_deref() == Some("phase-6")).then_some("phase-4"),
        &app_support,
        injected.as_deref(),
        &muxy_core::prefs::home_dir(),
    )
    .map(|_| app_support.join(".muxy-p7-images-status.json"))
}

fn current_phase_7_status_path() -> Option<PathBuf> {
    let app_support = muxy_core::prefs::app_support_dir();
    let injected = std::env::var_os("MUXY_TEST_APPLICATION_SUPPORT_DIRECTORY").map(PathBuf::from);
    let case_name = std::env::var("MUXY_TEST_P7_COMPOSER_CASE").ok();
    phase_4_status_path(
        muxy_core::prefs::is_test_process(),
        (case_name.as_deref() == Some("phase-7")).then_some("phase-4"),
        &app_support,
        injected.as_deref(),
        &muxy_core::prefs::home_dir(),
    )
    .map(|_| app_support.join(".muxy-p7-drops-status.json"))
}

fn submission_target_ids(
    active: &str,
    broadcast: bool,
    visible: impl IntoIterator<Item = (String, bool, bool)>,
) -> Vec<String> {
    if !broadcast {
        return vec![active.to_owned()];
    }
    let mut seen = HashSet::new();
    visible
        .into_iter()
        .filter(|(_, terminal, live)| *terminal && *live)
        .map(|(pane_id, _, _)| pane_id)
        .filter(|pane_id| seen.insert(pane_id.clone()))
        .collect()
}

async fn submit_to_targets(
    target_pane_ids: &[String],
    mut enqueue: impl FnMut(&str) -> Option<async_channel::Receiver<TerminalInputResult>>,
) -> Option<Vec<String>> {
    let mut failures = Vec::new();
    for pane_id in target_pane_ids {
        let completion = enqueue(pane_id)?;
        let result = completion
            .recv()
            .await
            .unwrap_or(Err(muxy_terminal::input::TerminalInputError::Cancelled));
        if let Err(error) = result {
            failures.push(format!("{pane_id}: {error:?}"));
        }
    }
    Some(failures)
}

fn clear_completed_submission(
    store: &mut ComposerStore,
    draft_id: &DraftId,
    revision: u64,
    all_succeeded: bool,
    clear_after: bool,
) -> std::io::Result<bool> {
    if !crate::composer::submission::should_clear_submission(
        all_succeeded,
        clear_after,
        revision,
        store.draft_revision(draft_id),
    ) {
        return Ok(false);
    }
    let retained = store.draft(draft_id).cloned();
    if !store.clear_if_revision(draft_id, revision)? {
        return Ok(false);
    }
    if let Err(error) = store.flush() {
        if let Some(retained) = retained
            && let Err(restore_error) = store.replace_draft(draft_id.clone(), retained)
        {
            return Err(std::io::Error::other(format!(
                "{error}; failed to restore the Composer draft: {restore_error}"
            )));
        }
        return Err(error);
    }
    Ok(true)
}

impl MainWindow {
    pub(crate) fn composer_is_open(&self) -> bool {
        self.composer.is_open()
    }

    pub(crate) fn composer_input_is_focused(&self, window: &Window, cx: &App) -> bool {
        self.composer
            .input()
            .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window))
    }

    fn active_composer_target(&self) -> Option<ComposerTarget> {
        let navigation = self.state.current_navigation_entry()?;
        let pane_id = navigation.tab_id?;
        let workspace = self.state.active_tab_workspace()?;
        let tab = workspace.tab(&pane_id)?;
        if tab.kind != TabKind::Terminal
            || self.terminal_runtime.surfaces.handle(&pane_id).is_none()
        {
            return None;
        }
        ComposerTarget::new(navigation.project_id, navigation.worktree_id, pane_id)
    }

    fn create_composer_input(
        &mut self,
        target: &ComposerTarget,
        cx: &mut Context<Self>,
    ) -> (Entity<TextInput>, Vec<String>, gpui::Subscription) {
        let draft = self
            .composer_store
            .draft(&target.draft_id())
            .cloned()
            .unwrap_or_default();
        let style = composer_input_style(self);
        let family =
            muxy_core::prefs::settings::string_value("editor.richInputFontFamily", "SF Mono");
        let view = cx.weak_entity();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(style, cx)
                .multiline()
                .with_placeholder("Type…")
                .with_text(draft.text.clone())
                .with_paste_delegate(move |_, cx| match crate::pasteboard::read_content() {
                    Ok(crate::pasteboard::PasteboardContent::Text(_))
                    | Ok(crate::pasteboard::PasteboardContent::Empty)
                    | Err(_) => false,
                    Ok(crate::pasteboard::PasteboardContent::Files(paths)) => {
                        let _ = view.update(cx, |window, cx| window.add_composer_files(paths, cx));
                        true
                    }
                    Ok(crate::pasteboard::PasteboardContent::Image(contents)) => {
                        let _ = view.update(cx, |window, cx| {
                            window.paste_composer_image(contents, cx);
                        });
                        true
                    }
                });
            input.set_font_family(Some(SharedString::from(family)), cx);
            input
        });
        let subscription = cx.subscribe(&input, |window, input, event, cx| {
            if matches!(event, InputEvent::Changed) {
                window.composer_input_changed(&input, cx);
            }
        });
        (input, draft.file_attachments, subscription)
    }

    fn open_composer_for(&mut self, target: ComposerTarget, cx: &mut Context<Self>) {
        let (input, attachments, subscription) = self.create_composer_input(&target, cx);
        let placement = PanelPlacement::new(
            muxy_core::composer::PANEL_ID,
            panel_position(self.state.prefs.composer.position),
            panel_mode(self.state.prefs.composer.panel_mode),
        );
        self.place_panel(placement);
        let focus = input.focus_handle(cx);
        self.composer.open(target, input, attachments, subscription);
        self.view.pending_focus = Some(focus);
        cx.notify();
    }

    pub(crate) fn toggle_composer(&mut self, cx: &mut Context<Self>) {
        if self.composer.is_open() {
            self.close_composer(cx);
        } else if let Some(target) = self.active_composer_target() {
            self.open_composer_for(target, cx);
        }
    }

    fn visible_terminal_panes(&self) -> Vec<(String, bool, bool)> {
        self.state
            .active_tab_workspace()
            .map(|workspace| {
                workspace
                    .visible_area_tabs()
                    .into_iter()
                    .map(|(_, pane_id)| {
                        let terminal = workspace
                            .tab(&pane_id)
                            .is_some_and(|tab| tab.kind == TabKind::Terminal);
                        let live = self.terminal_runtime.surfaces.handle(&pane_id).is_some();
                        (pane_id, terminal, live)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn composer_submission_targets(&self, target: &ComposerTarget) -> Vec<String> {
        submission_target_ids(
            &target.pane_id,
            self.state.prefs.composer.broadcast,
            self.visible_terminal_panes(),
        )
    }

    fn staged_broadcast_targets_ready(&self, target: &ComposerTarget) -> bool {
        submission_target_ids(&target.pane_id, true, self.visible_terminal_panes()).len() == 2
    }

    pub(crate) fn submit_composer(&mut self, append_return: bool, cx: &mut Context<Self>) {
        self.start_composer_submission(append_return, None, cx);
    }

    fn submit_composer_with_completion(
        &mut self,
        append_return: bool,
        cx: &mut Context<Self>,
    ) -> async_channel::Receiver<bool> {
        let (sender, receiver) = async_channel::bounded(1);
        self.start_composer_submission(append_return, Some(sender), cx);
        receiver
    }

    fn start_composer_submission(
        &mut self,
        append_return: bool,
        staged_completion: Option<async_channel::Sender<bool>>,
        cx: &mut Context<Self>,
    ) {
        if !self.composer.is_open() {
            if let Some(target) = self.active_composer_target() {
                let _ = self.enqueue_terminal_input(
                    target.pane_id,
                    TerminalInputTransaction::new(Vec::new(), true),
                    cx,
                );
            }
            return;
        }
        if let Err(error) = self.store_composer_editor(cx) {
            self.feedback(
                "Composer submission",
                error.to_string(),
                crate::toast::ToastTone::Error,
                cx,
            );
            return;
        }
        let Some(target) = self.composer.target().cloned() else {
            return;
        };
        let Some(input) = self.composer.input().cloned() else {
            return;
        };
        let target_pane_ids = self.composer_submission_targets(&target);
        if target_pane_ids.is_empty() {
            self.feedback(
                "Composer submission",
                "No visible terminal panes are available",
                crate::toast::ToastTone::Error,
                cx,
            );
            return;
        }
        let draft_id = target.draft_id();
        let revision = self.composer_store.draft_revision(&draft_id);
        let draft = self
            .composer_store
            .draft(&draft_id)
            .cloned()
            .unwrap_or_default();
        let (selected_text, text) = {
            let input = input.read(cx);
            (
                Some(input.selected_text().to_owned()),
                input.text().to_owned(),
            )
        };
        let image_strategy = match muxy_core::prefs::settings::string_value(
            "editor.richInputImageStrategy",
            "clipboard",
        )
        .as_str()
        {
            "inlinePath" => ImageSubmissionStrategy::InlinePath,
            _ => ImageSubmissionStrategy::Clipboard,
        };
        let plan = plan_submission(SubmissionSnapshot {
            text,
            revision,
            selected_text,
            file_paths: self.composer.file_attachments().to_vec(),
            image_attachments: draft.image_attachments,
            append_return,
            image_strategy,
            target_pane_ids,
        });
        let filenames = crate::composer::submission::copied_image_filenames(&plan);
        let mut image_sources = Vec::with_capacity(filenames.len());
        if !filenames.is_empty() {
            let Some(storage) = self.composer_store.image_storage() else {
                self.feedback(
                    "Composer submission",
                    "Composer image storage is unavailable",
                    crate::toast::ToastTone::Error,
                    cx,
                );
                if let Some(completion) = staged_completion {
                    let _ = completion.try_send(false);
                }
                return;
            };
            for filename in filenames {
                let contents = match storage.read(&filename) {
                    Ok(contents) => contents,
                    Err(_) => {
                        let error =
                            crate::composer::submission::SubmissionError::ImageReadFailed(filename);
                        self.feedback(
                            "Composer submission",
                            error.to_string(),
                            crate::toast::ToastTone::Error,
                            cx,
                        );
                        if let Some(completion) = staged_completion {
                            let _ = completion.try_send(false);
                        }
                        return;
                    }
                };
                let path = match storage.path_for(&filename) {
                    Ok(path) => path.to_string_lossy().into_owned(),
                    Err(_) => {
                        let error =
                            crate::composer::submission::SubmissionError::ImageReadFailed(filename);
                        self.feedback(
                            "Composer submission",
                            error.to_string(),
                            crate::toast::ToastTone::Error,
                            cx,
                        );
                        if let Some(completion) = staged_completion {
                            let _ = completion.try_send(false);
                        }
                        return;
                    }
                };
                image_sources.push((filename, path, contents));
            }
        }
        let normalization = cx.background_executor().spawn(async move {
            let mut images = std::collections::HashMap::with_capacity(image_sources.len());
            for (filename, path, contents) in image_sources {
                let png =
                    muxy_core::composer::image_storage::normalize_png(&contents).map_err(|_| {
                        crate::composer::submission::SubmissionError::ImageNormalizationFailed(
                            filename.clone(),
                        )
                    })?;
                images.insert(
                    filename,
                    crate::composer::submission::SubmissionImage { path, png },
                );
            }
            Ok::<_, crate::composer::submission::SubmissionError>(images)
        });
        let clear_after =
            muxy_core::prefs::settings::bool_value("muxy.richInput.clearAfterSending", false);
        cx.spawn(async move |window, cx| {
            let images = match normalization.await {
                Ok(images) => images,
                Err(error) => {
                    let _ = window.update(cx, |window, cx| {
                        window.feedback(
                            "Composer submission",
                            error.to_string(),
                            crate::toast::ToastTone::Error,
                            cx,
                        );
                    });
                    if let Some(completion) = staged_completion {
                        let _ = completion.try_send(false);
                    }
                    return;
                }
            };
            let resolved = match crate::composer::submission::resolve_submission(plan, &images) {
                Ok(resolved) => resolved,
                Err(error) => {
                    let _ = window.update(cx, |window, cx| {
                        window.feedback(
                            "Composer submission",
                            error.to_string(),
                            crate::toast::ToastTone::Error,
                            cx,
                        );
                    });
                    if let Some(completion) = staged_completion {
                        let _ = completion.try_send(false);
                    }
                    return;
                }
            };
            if resolved.transaction.steps.is_empty() {
                if let Some(completion) = staged_completion {
                    let _ = completion.try_send(false);
                }
                return;
            }
            let Some(failures) = submit_to_targets(&resolved.target_pane_ids, |pane_id| {
                window
                    .update(cx, |window, cx| {
                        window.enqueue_terminal_input(
                            pane_id.to_owned(),
                            resolved.transaction.clone(),
                            cx,
                        )
                    })
                    .ok()
            })
            .await
            else {
                return;
            };
            let succeeded = failures.is_empty();
            let _ = window.update(cx, |window, cx| {
                window.finish_composer_submission(
                    draft_id,
                    resolved.revision,
                    clear_after,
                    failures,
                    cx,
                );
            });
            if let Some(completion) = staged_completion {
                let _ = completion.try_send(succeeded);
            }
        })
        .detach();
    }

    fn finish_composer_submission(
        &mut self,
        draft_id: DraftId,
        revision: u64,
        clear_after: bool,
        failures: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let all_succeeded = failures.is_empty();
        if !all_succeeded {
            self.feedback(
                "Composer submission",
                format!("Failed to send to {}", failures.join(", ")),
                crate::toast::ToastTone::Error,
                cx,
            );
        }
        let cleared = match clear_completed_submission(
            &mut self.composer_store,
            &draft_id,
            revision,
            all_succeeded,
            clear_after,
        ) {
            Ok(cleared) => cleared,
            Err(error) => {
                self.feedback(
                    "Composer draft",
                    error.to_string(),
                    crate::toast::ToastTone::Error,
                    cx,
                );
                return;
            }
        };
        if !cleared {
            return;
        }
        self.composer_save_generation = self.composer_save_generation.saturating_add(1);
        self._composer_save_task = None;
        if self
            .composer
            .target()
            .is_some_and(|target| target.draft_id() == draft_id)
        {
            self.composer.replace_file_attachments(Vec::new());
            if let Some(input) = self.composer.input().cloned() {
                input.update(cx, |input, cx| input.set_text("", cx));
            }
        }
        cx.notify();
    }

    fn store_composer_editor(&mut self, cx: &mut Context<Self>) -> std::io::Result<()> {
        let Some(target) = self.composer.target().cloned() else {
            return Ok(());
        };
        let Some(input) = self.composer.input().cloned() else {
            return Ok(());
        };
        let text = input.read(cx).text().to_owned();
        let attachments = self.composer.file_attachments().to_vec();
        self.composer_store
            .edit_content(target.draft_id(), text, attachments)?;
        Ok(())
    }

    fn composer_input_changed(&mut self, input: &Entity<TextInput>, cx: &mut Context<Self>) {
        if self.composer.input().map(Entity::entity_id) != Some(input.entity_id()) {
            return;
        }
        if let Err(error) = self.store_composer_editor(cx) {
            self.feedback(
                "Composer draft",
                error.to_string(),
                crate::toast::ToastTone::Error,
                cx,
            );
            return;
        }
        self.composer.allow_release();
        self.schedule_composer_save(cx);
    }

    fn release_composer_target(&mut self, clear: bool, cx: &mut Context<Self>) -> bool {
        let Some(target) = self.composer.target().cloned() else {
            return true;
        };
        let Some(input) = self.composer.input().cloned() else {
            return true;
        };
        let text = input.read(cx).text().to_owned();
        let attachments = self.composer.file_attachments().to_vec();
        self.composer_save_generation = self.composer_save_generation.saturating_add(1);
        self._composer_save_task = None;
        if let Err(error) = publish_composer_release(
            &mut self.composer_store,
            target.draft_id(),
            text,
            attachments,
            clear,
        ) {
            self.composer.block_release();
            self.feedback(
                "Composer draft",
                error.to_string(),
                crate::toast::ToastTone::Error,
                cx,
            );
            return false;
        }
        if self.composer.menu_open() {
            self.dismiss_overlay(cx);
        }
        self.composer.close();
        self.panels.close(&muxy_ui::panel::PanelId::from(
            muxy_core::composer::PANEL_ID,
        ));
        true
    }

    pub(crate) fn close_composer(&mut self, cx: &mut Context<Self>) {
        if !self.composer.is_open() {
            return;
        }
        let clear = muxy_core::prefs::settings::bool_value("muxy.richInput.clearOnClose", false);
        if self.release_composer_target(clear, cx) {
            self.view.pending_focus = Some(self.view.workspace_focus.clone());
            cx.notify();
        }
    }

    pub(crate) fn close_floating_composer_from_outside(&mut self, cx: &mut Context<Self>) {
        if self
            .panels
            .host()
            .placement(&muxy_ui::panel::PanelId::from(
                muxy_core::composer::PANEL_ID,
            ))
            .is_some_and(|placement| placement.mode == PanelMode::Floating)
        {
            self.close_composer(cx);
        }
    }

    pub(crate) fn reconcile_composer_target(&mut self, cx: &mut Context<Self>) {
        if !self.composer.is_open() {
            self.prepare_staged_phase_4(cx);
            self.prepare_staged_phase_5(cx);
            self.prepare_staged_phase_6(cx);
            self.prepare_staged_phase_7(cx);
            return;
        }
        let next = self.active_composer_target();
        let transition = crate::composer::target_transition(self.composer.target(), next.as_ref());
        match transition {
            TargetTransition::Unchanged => self.composer.allow_release(),
            TargetTransition::RebindPane => self.composer.rebind_pane(next.unwrap()),
            TargetTransition::TransferWorktree if !self.composer.release_blocked() => {
                let clear_on_close =
                    muxy_core::prefs::settings::bool_value("muxy.richInput.clearOnClose", false);
                let clear = crate::composer::clear_on_target_transition(transition, clear_on_close);
                if self.release_composer_target(clear, cx) {
                    self.open_composer_for(next.unwrap(), cx);
                }
            }
            TargetTransition::Close if !self.composer.release_blocked() => {
                let clear_on_close =
                    muxy_core::prefs::settings::bool_value("muxy.richInput.clearOnClose", false);
                let clear = crate::composer::clear_on_target_transition(transition, clear_on_close);
                if self.release_composer_target(clear, cx) {
                    self.view.pending_focus = Some(self.view.workspace_focus.clone());
                    cx.notify();
                }
            }
            TargetTransition::TransferWorktree | TargetTransition::Close => {}
        }
        self.prepare_staged_phase_4(cx);
        self.prepare_staged_phase_5(cx);
        self.prepare_staged_phase_6(cx);
        self.prepare_staged_phase_7(cx);
    }

    pub(crate) fn move_composer_panel(&mut self, cx: &mut Context<Self>) {
        let next = match self.state.prefs.composer.position {
            ComposerPanelPosition::Right => ComposerPanelPosition::Bottom,
            ComposerPanelPosition::Bottom => ComposerPanelPosition::Right,
        };
        if let Err(error) = self.set_composer_position(next) {
            self.feedback(
                "Composer panel",
                error.to_string(),
                crate::toast::ToastTone::Error,
                cx,
            );
            return;
        }
        cx.notify();
    }

    pub(crate) fn toggle_composer_panel_mode(&mut self, cx: &mut Context<Self>) {
        let next = match self.state.prefs.composer.panel_mode {
            ComposerPanelMode::Floating => ComposerPanelMode::Pinned,
            ComposerPanelMode::Pinned => ComposerPanelMode::Floating,
        };
        if let Err(error) = self.set_composer_panel_mode(next) {
            self.feedback(
                "Composer panel",
                error.to_string(),
                crate::toast::ToastTone::Error,
                cx,
            );
            return;
        }
        cx.notify();
    }

    pub(crate) fn toggle_composer_broadcast(&mut self, cx: &mut Context<Self>) {
        let next = !self.state.prefs.composer.broadcast;
        if let Err(error) = self.set_composer_broadcast(next) {
            self.feedback(
                "Composer",
                error.to_string(),
                crate::toast::ToastTone::Error,
                cx,
            );
            return;
        }
        cx.notify();
    }

    pub(crate) fn open_composer_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let clear_after =
            muxy_core::prefs::settings::bool_value("muxy.richInput.clearAfterSending", false);
        let clear_on_close =
            muxy_core::prefs::settings::bool_value("muxy.richInput.clearOnClose", false);
        let items = vec![
            crate::views::menu::Item::action(
                if self.state.prefs.composer.broadcast {
                    "Send to Active Pane"
                } else {
                    "Send to All Split Panes"
                },
                crate::command::Command::ToggleComposerBroadcast,
            ),
            crate::views::menu::Item::action(
                "Send Without Enter",
                crate::command::Command::SubmitComposerWithoutReturn,
            ),
            crate::views::menu::Item::Separator,
            crate::views::menu::Item::action(
                "Clear After Sending",
                crate::command::Command::ToggleComposerClearAfterSending,
            )
            .checked(clear_after),
            crate::views::menu::Item::action(
                "Clear on Close",
                crate::command::Command::ToggleComposerClearOnClose,
            )
            .checked(clear_on_close),
        ];
        let height = items
            .iter()
            .fold(self.state.metrics.spacing2() * 2.0, |height, item| {
                height + crate::views::menu::item_height(item, &self.state)
            });
        let margin = self.state.metrics.spacing4();
        let width = self.state.metrics.scaled(180.0);
        let anchor = point(
            px((f32::from(position.x) - f32::from(width)).max(f32::from(margin))),
            px(
                (f32::from(position.y) - f32::from(height) - f32::from(margin))
                    .max(f32::from(margin)),
            ),
        );
        self.open_menu(items, anchor, cx);
        self.composer.set_menu_open(true);
        window.focus(&self.view.menu_focus);
    }

    pub(crate) fn toggle_composer_boolean_setting(
        &mut self,
        key: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
    ) {
        let next = !muxy_core::prefs::settings::bool_value(key, false);
        if let Err(error) = muxy_core::prefs::settings::try_set(key, serde_json::Value::Bool(next))
        {
            self.feedback(label, error.to_string(), crate::toast::ToastTone::Error, cx);
            return;
        }
        cx.notify();
    }

    pub(crate) fn change_composer_font_size(&mut self, delta: f64, cx: &mut Context<Self>) {
        let next = (self.state.prefs.composer.font_size + delta)
            .clamp(COMPOSER_FONT_SIZE_MIN, COMPOSER_FONT_SIZE_MAX);
        if let Err(error) = self.set_composer_font_size(next, cx) {
            self.feedback(
                "Composer",
                error.to_string(),
                crate::toast::ToastTone::Error,
                cx,
            );
            return;
        }
        cx.notify();
    }

    pub(crate) fn resize_composer_panel(&mut self, dimension: f32, cx: &mut Context<Self>) {
        let result = match self.state.prefs.composer.position {
            ComposerPanelPosition::Right => {
                ComposerPreferences::try_store_panel_width(dimension as f64)
                    .map(|_| self.state.prefs.composer.panel_width = dimension as f64)
            }
            ComposerPanelPosition::Bottom => {
                ComposerPreferences::try_store_panel_height(dimension as f64)
                    .map(|_| self.state.prefs.composer.panel_height = dimension as f64)
            }
        };
        if let Err(error) = result {
            self.feedback(
                "Composer panel",
                error.to_string(),
                crate::toast::ToastTone::Error,
                cx,
            );
            return;
        }
        cx.notify();
    }

    pub(crate) fn choose_composer_files(&mut self, cx: &mut Context<Self>) {
        let Some(draft_id) = self.composer.target().map(ComposerTarget::draft_id) else {
            return;
        };
        let directory = self
            .state
            .active_project()
            .map(|project| self.state.active_worktree_path(project));
        cx.spawn(async move |window, cx| {
            let paths =
                crate::views::file_dialog::pick_files(crate::views::file_dialog::FilesRequest {
                    title: "Attach files",
                    directory,
                })
                .await;
            if paths.is_empty() {
                return;
            }
            let _ = window.update(cx, |window, cx| {
                if !crate::composer::picker_target_matches(&draft_id, window.composer.target()) {
                    return;
                }
                window.add_composer_files(
                    paths
                        .into_iter()
                        .filter(|path| path.is_absolute())
                        .map(|path| path.to_string_lossy().into_owned()),
                    cx,
                );
            });
        })
        .detach();
    }

    fn paste_composer_image(&mut self, contents: Vec<u8>, cx: &mut Context<Self>) {
        if let Err(error) = self.store_composer_editor(cx) {
            self.feedback(
                "Composer image",
                error.to_string(),
                crate::toast::ToastTone::Error,
                cx,
            );
            return;
        }
        let Some(target) = self.composer.target().cloned() else {
            return;
        };
        let Some(input) = self.composer.input().cloned() else {
            return;
        };
        let draft_id = target.draft_id();
        let revision = self.composer_store.draft_revision(&draft_id);
        let selection = input.read(cx).selected_range();
        let preparation = cx.background_executor().spawn(async move {
            muxy_core::composer::image_storage::prepare_image_source(contents)
        });
        cx.spawn(async move |window, cx| {
            let prepared = preparation.await;
            let _ = window.update(cx, |window, cx| {
                let prepared = match prepared {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        window.feedback(
                            "Composer image",
                            error.to_string(),
                            crate::toast::ToastTone::Error,
                            cx,
                        );
                        return;
                    }
                };
                if !crate::composer::picker_target_matches(&draft_id, window.composer.target())
                    || window.composer_store.draft_revision(&draft_id) != revision
                {
                    return;
                }
                let Some(current_input) = window.composer.input().cloned() else {
                    return;
                };
                if current_input.entity_id() != input.entity_id()
                    || current_input.read(cx).selected_range() != selection
                {
                    return;
                }
                match window
                    .composer_store
                    .attach_prepared_image(draft_id, &prepared, selection)
                {
                    Ok((number, _)) => {
                        current_input.update(cx, |input, cx| {
                            input.insert_at_selection(&format!("[Image {number}]"), cx);
                        });
                        window.composer.allow_release();
                        window.composer_save_generation =
                            window.composer_save_generation.saturating_add(1);
                        window._composer_save_task = None;
                        cx.notify();
                    }
                    Err(error) => window.feedback(
                        "Composer image",
                        error.to_string(),
                        crate::toast::ToastTone::Error,
                        cx,
                    ),
                }
            });
        })
        .detach();
    }

    pub(super) fn add_composer_files(
        &mut self,
        paths: impl IntoIterator<Item = String>,
        cx: &mut Context<Self>,
    ) {
        let mut attachments = self.composer.file_attachments().to_vec();
        let mut seen = attachments.iter().cloned().collect::<HashSet<_>>();
        attachments.extend(paths.into_iter().filter(|path| seen.insert(path.clone())));
        self.composer.replace_file_attachments(attachments);
        self.composer_input_changed_from_attachments(cx);
    }

    pub(crate) fn remove_composer_file(&mut self, path: &str, cx: &mut Context<Self>) {
        let attachments = self
            .composer
            .file_attachments()
            .iter()
            .filter(|candidate| candidate.as_str() != path)
            .cloned()
            .collect();
        self.composer.replace_file_attachments(attachments);
        self.composer_input_changed_from_attachments(cx);
    }

    fn composer_input_changed_from_attachments(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.store_composer_editor(cx) {
            self.feedback(
                "Composer draft",
                error.to_string(),
                crate::toast::ToastTone::Error,
                cx,
            );
            return;
        }
        self.composer.allow_release();
        self.schedule_composer_save(cx);
        cx.notify();
    }

    fn set_composer_position(&mut self, position: ComposerPanelPosition) -> std::io::Result<()> {
        ComposerPreferences::try_store_position(position)?;
        self.state.prefs.composer.position = position;
        self.place_panel(PanelPlacement::new(
            muxy_core::composer::PANEL_ID,
            panel_position(position),
            panel_mode(self.state.prefs.composer.panel_mode),
        ));
        Ok(())
    }

    fn set_composer_panel_mode(&mut self, mode: ComposerPanelMode) -> std::io::Result<()> {
        ComposerPreferences::try_store_panel_mode(mode)?;
        self.state.prefs.composer.panel_mode = mode;
        self.place_panel(PanelPlacement::new(
            muxy_core::composer::PANEL_ID,
            panel_position(self.state.prefs.composer.position),
            panel_mode(mode),
        ));
        Ok(())
    }

    fn set_composer_broadcast(&mut self, broadcast: bool) -> std::io::Result<()> {
        ComposerPreferences::try_store_broadcast(broadcast)?;
        self.state.prefs.composer.broadcast = broadcast;
        Ok(())
    }

    fn set_composer_font_size(
        &mut self,
        font_size: f64,
        cx: &mut Context<Self>,
    ) -> std::io::Result<()> {
        ComposerPreferences::try_store_font_size(font_size)?;
        self.state.prefs.composer.font_size = font_size;
        let style = composer_input_style(self);
        if let Some(input) = self.composer.input().cloned() {
            input.update(cx, |input, cx| input.set_style(style, cx));
        }
        Ok(())
    }

    fn prepare_staged_phase_7(&mut self, cx: &mut Context<Self>) {
        let Some(status_path) = current_phase_7_status_path() else {
            return;
        };
        if self.composer.staged_submission_started() {
            return;
        }
        if !self.composer.is_open() {
            let Some(target) = self.active_composer_target() else {
                return;
            };
            self.open_composer_for(target, cx);
        }
        let Some(target) = self.composer.target().cloned() else {
            return;
        };
        let app_support = muxy_core::prefs::app_support_dir();
        let first_file = app_support.join("drop first.txt");
        let image_file = app_support.join("drop-image.png");
        let existing_directory = app_support.join("project");
        let new_directory = app_support.join("drop-project");
        self.composer.mark_staged_submission_started();
        self.handle_composer_drop(
            &[first_file.clone(), image_file.clone(), first_file.clone()],
            cx,
        );
        let composer_attachments = self.composer.file_attachments().to_vec();
        self.terminal_runtime.surfaces.reset_staged_input_bytes();
        let file_url = format!(
            "file://{}",
            first_file.to_string_lossy().replace(' ', "%20")
        );
        let terminal_injected = self.terminal_runtime.surfaces.inject_staged_external_drop(
            &target.pane_id,
            muxy_terminal::backend::ExternalDrop {
                file_values: vec![file_url, image_file.to_string_lossy().into_owned()],
                plain_text: Some("ignored text".to_owned()),
            },
        );
        cx.spawn(async move |window, cx| {
            let mut terminal_bytes = Vec::new();
            for _ in 0..100 {
                terminal_bytes = window
                    .update(cx, |window, _| {
                        window
                            .terminal_runtime
                            .surfaces
                            .take_staged_input_bytes(&target.pane_id)
                    })
                    .unwrap_or_default();
                if !terminal_bytes.is_empty() {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
            }
            let terminal_focused = window
                .update(cx, |window, _| {
                    window.focused_tab_id().as_deref() == Some(&target.pane_id)
                })
                .unwrap_or(false);
            let sidebar = window
                .update(cx, |window, cx| {
                    let initial_project_count = window.state.workspace.projects.len();
                    window.handle_sidebar_drop(std::slice::from_ref(&existing_directory), cx);
                    let existing_project_selected =
                        window.state.active_project().is_some_and(|project| {
                            project.path == existing_directory.to_string_lossy()
                        });
                    let existing_project_count = window.state.workspace.projects.len();
                    window.handle_sidebar_drop(&[first_file.clone(), new_directory.clone()], cx);
                    let project_count = window.state.workspace.projects.len();
                    let active_project_path = window
                        .state
                        .active_project()
                        .map(|project| project.path.clone());
                    let file_added_as_project = window
                        .state
                        .workspace
                        .contains_path(&first_file.to_string_lossy())
                        .is_some();
                    let new_project_added = window
                        .state
                        .workspace
                        .contains_path(&new_directory.to_string_lossy())
                        .is_some();
                    (
                        initial_project_count,
                        existing_project_selected,
                        existing_project_count,
                        project_count,
                        active_project_path,
                        file_added_as_project,
                        new_project_added,
                    )
                })
                .unwrap_or_default();
            let copied_image_count = std::fs::read_dir(app_support.join("RichInputImages"))
                .map(|entries| entries.filter_map(Result::ok).count())
                .unwrap_or(0);
            let value = serde_json::json!({
                "composerAttachments": composer_attachments,
                "copiedImageCount": copied_image_count,
                "terminalInjected": terminal_injected,
                "terminalSucceeded": !terminal_bytes.is_empty(),
                "terminalFocused": terminal_focused,
                "terminalBytes": terminal_bytes,
                "initialProjectCount": sidebar.0,
                "existingProjectSelected": sidebar.1,
                "existingProjectCount": sidebar.2,
                "projectCount": sidebar.3,
                "activeProjectPath": sidebar.4,
                "fileAddedAsProject": sidebar.5,
                "newProjectAdded": sidebar.6,
            });
            if let Ok(contents) = serde_json::to_vec_pretty(&value)
                && let Err(error) =
                    crate::composer::submission::write_staged_status(&status_path, &contents)
            {
                log::warn!("failed to write P7 drop status: {error}");
            }
        })
        .detach();
    }

    fn prepare_staged_phase_6(&mut self, cx: &mut Context<Self>) {
        let Some(status_path) = current_phase_6_status_path() else {
            return;
        };
        if self.composer.staged_submission_started() {
            return;
        }
        if !self.composer.is_open() {
            let Some(target) = self.active_composer_target() else {
                return;
            };
            self.open_composer_for(target, cx);
        }
        let Some(target) = self.composer.target().cloned() else {
            return;
        };
        if !self.staged_broadcast_targets_ready(&target) {
            return;
        }
        let Some(filename) = self
            .composer_store
            .draft(&target.draft_id())
            .and_then(|draft| draft.image_attachments.first())
            .map(|attachment| attachment.filename.clone())
        else {
            return;
        };
        let app_support = muxy_core::prefs::app_support_dir();
        let inline_output = app_support.join("phase-6-inline.txt");
        self.composer.mark_staged_submission_started();
        cx.spawn(async move |window, cx| {
            let before_clipboard = window
                .update(cx, |_, _| crate::pasteboard::capture().ok())
                .ok()
                .flatten();
            let _ = window.update(cx, |window, _| {
                window.terminal_runtime.surfaces.reset_staged_input_bytes();
            });
            let clipboard = window
                .update(cx, |window, cx| {
                    Some(window.submit_composer_with_completion(false, cx))
                })
                .ok()
                .flatten();
            let clipboard_succeeded = match clipboard {
                Some(completion) => completion.recv().await == Ok(true),
                None => false,
            };
            let after_clipboard = window
                .update(cx, |_, _| crate::pasteboard::capture().ok())
                .ok()
                .flatten();
            let clipboard_bytes = window
                .update(cx, |window, _| {
                    let bytes = window
                        .terminal_runtime
                        .surfaces
                        .take_staged_input_bytes(&target.pane_id);
                    window.terminal_runtime.surfaces.reset_staged_input_bytes();
                    bytes
                })
                .unwrap_or_default();
            let cleanup = window
                .update(cx, |window, cx| {
                    window.enqueue_terminal_input(
                        target.pane_id.clone(),
                        TerminalInputTransaction::new(
                            vec![muxy_terminal::input::TerminalInputStep::RawBytes(vec![
                                0x1b, 0x03,
                            ])],
                            false,
                        ),
                        cx,
                    )
                })
                .ok();
            if let Some(completion) = cleanup {
                let _ = completion.recv().await;
            }
            let _ = window.update(cx, |window, _| {
                window.terminal_runtime.surfaces.reset_staged_input_bytes();
            });
            let failure = window
                .update(cx, |window, cx| {
                    muxy_core::prefs::settings::set(
                        "muxy.richInput.clearAfterSending",
                        serde_json::Value::Bool(false),
                    );
                    window.set_composer_broadcast(true).ok()?;
                    let pane_ids = window.composer_submission_targets(&target);
                    let first = pane_ids.first()?;
                    window
                        .terminal_runtime
                        .surfaces
                        .arm_staged_image_failure(first);
                    let input = window.composer.input()?.clone();
                    input.update(cx, |input, cx| {
                        input.set_text("first\nsecond [Image 1]", cx)
                    });
                    let completion = window.submit_composer_with_completion(true, cx);
                    Some((completion, pane_ids))
                })
                .ok()
                .flatten();
            let (failure_handled, pane_ids) = match failure {
                Some((completion, pane_ids)) => (completion.recv().await == Ok(false), pane_ids),
                None => (false, Vec::new()),
            };
            let after_failure_clipboard = window
                .update(cx, |_, _| crate::pasteboard::capture().ok())
                .ok()
                .flatten();
            let (failure_bytes, draft_retained_after_failure, image_retained_after_failure) = window
                .update(cx, |window, _| {
                    let bytes = pane_ids
                        .iter()
                        .map(|pane_id| {
                            (
                                pane_id.clone(),
                                window
                                    .terminal_runtime
                                    .surfaces
                                    .take_staged_input_bytes(pane_id),
                            )
                        })
                        .collect::<std::collections::HashMap<_, _>>();
                    let draft_retained = window.composer_store.draft(&target.draft_id()).is_some();
                    let image_retained = window
                        .composer_store
                        .image_storage()
                        .is_some_and(|storage| storage.read(&filename).is_ok());
                    (bytes, draft_retained, image_retained)
                })
                .unwrap_or_default();
            for pane_id in &pane_ids {
                let cleanup = window
                    .update(cx, |window, cx| {
                        window.enqueue_terminal_input(
                            pane_id.clone(),
                            TerminalInputTransaction::new(
                                vec![muxy_terminal::input::TerminalInputStep::RawBytes(vec![
                                    0x1b, 0x03,
                                ])],
                                false,
                            ),
                            cx,
                        )
                    })
                    .ok();
                if let Some(completion) = cleanup {
                    let _ = completion.recv().await;
                }
            }
            let _ = window.update(cx, |window, _| {
                window.terminal_runtime.surfaces.reset_staged_input_bytes();
            });
            let _ = window.update(cx, |window, cx| {
                muxy_core::prefs::settings::set_editor_setting(
                    "richInputImageStrategy",
                    serde_json::Value::String("inlinePath".to_owned()),
                );
                window.set_composer_broadcast(false).ok()?;
                muxy_core::prefs::settings::set(
                    "muxy.richInput.clearAfterSending",
                    serde_json::Value::Bool(true),
                );
                let command = format!(
                    "printf '%s' [Image 1] > {}",
                    muxy_terminal::backend::shell_escape(&inline_output.to_string_lossy())
                );
                let input = window.composer.input()?.clone();
                input.update(cx, |input, cx| input.set_text(command, cx));
                Some(())
            });
            let inline = window
                .update(cx, |window, cx| {
                    Some(window.submit_composer_with_completion(true, cx))
                })
                .ok()
                .flatten();
            let inline_succeeded = match inline {
                Some(completion) => completion.recv().await == Ok(true),
                None => false,
            };
            for _ in 0..100 {
                if inline_output.exists() {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
            }
            let (inline_bytes, draft_retained, image_retained) = window
                .update(cx, |window, _| {
                    let bytes = window
                        .terminal_runtime
                        .surfaces
                        .take_staged_input_bytes(&target.pane_id);
                    let draft_retained = window.composer_store.draft(&target.draft_id()).is_some();
                    let image_retained = window
                        .composer_store
                        .image_storage()
                        .is_some_and(|storage| storage.read(&filename).is_ok());
                    (bytes, draft_retained, image_retained)
                })
                .unwrap_or_default();
            let value = serde_json::json!({
                "clipboardSucceeded": clipboard_succeeded,
                "clipboardRestored": before_clipboard.is_some() && before_clipboard == after_clipboard,
                "clipboardBytes": clipboard_bytes,
                "failureHandled": failure_handled,
                "failureClipboardRestored": before_clipboard.is_some() && before_clipboard == after_failure_clipboard,
                "paneIds": pane_ids,
                "failureBytes": failure_bytes,
                "draftRetainedAfterFailure": draft_retained_after_failure,
                "imageRetainedAfterFailure": image_retained_after_failure,
                "inlineSucceeded": inline_succeeded,
                "inlineBytes": inline_bytes,
                "inlineOutput": std::fs::read_to_string(&inline_output).unwrap_or_default(),
                "draftRetained": draft_retained,
                "imageRetained": image_retained,
            });
            if let Ok(contents) = serde_json::to_vec_pretty(&value)
                && let Err(error) =
                    crate::composer::submission::write_staged_status(&status_path, &contents)
            {
                log::warn!("failed to write P7 Composer image status: {error}");
            }
        })
        .detach();
    }

    fn prepare_staged_phase_5(&mut self, cx: &mut Context<Self>) {
        let Some(status_path) = current_phase_5_status_path() else {
            return;
        };
        if self.composer.staged_submission_started() {
            return;
        }
        if !self.composer.is_open() {
            let Some(target) = self.active_composer_target() else {
                return;
            };
            self.open_composer_for(target, cx);
        }
        let Some(target) = self.composer.target().cloned() else {
            return;
        };
        if !self.staged_broadcast_targets_ready(&target) {
            return;
        }
        let app_support = muxy_core::prefs::app_support_dir();
        let active_output = app_support.join("phase-5-active.txt");
        let selected_output = app_support.join("phase-5-selected.txt");
        let local_output = app_support.join("phase-5-local.txt");
        let no_return_output = app_support.join("phase-5-no-return.txt");
        let broadcast_output = app_support.join("phase-5-broadcast.txt");
        let attachment = app_support.join("attached file's script.sh");
        self.composer.mark_staged_submission_started();
        cx.spawn(async move |window, cx| {
            let _ = window.update(cx, |window, _| {
                window.terminal_runtime.surfaces.reset_staged_input_bytes();
            });
            let active_command = format!(
                "printf 'ACTIVE_SCREEN\\n' | tee {}",
                muxy_terminal::backend::shell_escape(&active_output.to_string_lossy())
            );
            let active = window
                .update(cx, |window, cx| {
                    let input = window.composer.input()?.clone();
                    input.update(cx, |input, cx| input.set_text(active_command, cx));
                    window.composer.replace_file_attachments(Vec::new());
                    Some(window.submit_composer_with_completion(true, cx))
                })
                .ok()
                .flatten();
            let active_succeeded = match active {
                Some(completion) => completion.recv().await == Ok(true),
                None => false,
            };
            let active_bytes = window
                .update(cx, |window, _| {
                    let bytes = window
                        .terminal_runtime
                        .surfaces
                        .take_staged_input_bytes(&target.pane_id);
                    window.terminal_runtime.surfaces.reset_staged_input_bytes();
                    bytes
                })
                .unwrap_or_default();

            let selected_command = format!(
                "printf 'SELECTED_SCREEN\\n' | tee {}",
                muxy_terminal::backend::shell_escape(&selected_output.to_string_lossy())
            );
            let attachment_path = attachment.to_string_lossy().into_owned();
            let selected = window
                .update(cx, |window, cx| {
                    let input = window.composer.input()?.clone();
                    input.update(cx, |input, cx| {
                        input.set_text(selected_command, cx);
                        input.select_all_text(cx);
                    });
                    window
                        .composer
                        .replace_file_attachments(vec![attachment_path]);
                    Some(window.submit_composer_with_completion(true, cx))
                })
                .ok()
                .flatten();
            let selected_succeeded = match selected {
                Some(completion) => completion.recv().await == Ok(true),
                None => false,
            };
            let selected_bytes = window
                .update(cx, |window, _| {
                    let bytes = window
                        .terminal_runtime
                        .surfaces
                        .take_staged_input_bytes(&target.pane_id);
                    window.terminal_runtime.surfaces.reset_staged_input_bytes();
                    bytes
                })
                .unwrap_or_default();

            let attachment_path = attachment.to_string_lossy().into_owned();
            let local_path = window
                .update(cx, |window, cx| {
                    let input = window.composer.input()?.clone();
                    input.update(cx, |input, cx| input.set_text("", cx));
                    window
                        .composer
                        .replace_file_attachments(vec![attachment_path]);
                    Some(window.submit_composer_with_completion(true, cx))
                })
                .ok()
                .flatten();
            let local_path_succeeded = match local_path {
                Some(completion) => completion.recv().await == Ok(true),
                None => false,
            };
            let local_path_bytes = window
                .update(cx, |window, _| {
                    let bytes = window
                        .terminal_runtime
                        .surfaces
                        .take_staged_input_bytes(&target.pane_id);
                    window.terminal_runtime.surfaces.reset_staged_input_bytes();
                    bytes
                })
                .unwrap_or_default();

            let no_return_command = format!(
                "printf 'NO_RETURN_SCREEN\\n' | tee {}",
                muxy_terminal::backend::shell_escape(&no_return_output.to_string_lossy())
            );
            let no_return = window
                .update(cx, |window, cx| {
                    let input = window.composer.input()?.clone();
                    input.update(cx, |input, cx| input.set_text(no_return_command, cx));
                    window.composer.replace_file_attachments(Vec::new());
                    Some(window.submit_composer_with_completion(false, cx))
                })
                .ok()
                .flatten();
            let no_return_succeeded = match no_return {
                Some(completion) => completion.recv().await == Ok(true),
                None => false,
            };
            cx.background_executor()
                .timer(std::time::Duration::from_millis(250))
                .await;
            let no_return_before_return = !no_return_output.exists();
            let no_return_bytes = window
                .update(cx, |window, _| {
                    let bytes = window
                        .terminal_runtime
                        .surfaces
                        .take_staged_input_bytes(&target.pane_id);
                    window.terminal_runtime.surfaces.reset_staged_input_bytes();
                    bytes
                })
                .unwrap_or_default();
            let return_completion = window
                .update(cx, |window, cx| {
                    window.enqueue_terminal_input(
                        target.pane_id.clone(),
                        TerminalInputTransaction::new(Vec::new(), true),
                        cx,
                    )
                })
                .ok();
            if let Some(completion) = return_completion {
                let _ = completion.recv().await;
            }
            let _ = window.update(cx, |window, _| {
                window.terminal_runtime.surfaces.reset_staged_input_bytes();
            });

            let broadcast_command = format!(
                "printf 'BROADCAST:%s\\n' \"$MUXY_PANE_ID\" | tee -a {}",
                muxy_terminal::backend::shell_escape(&broadcast_output.to_string_lossy())
            );
            let broadcast = window
                .update(cx, |window, cx| {
                    if window.set_composer_broadcast(true).is_err() {
                        return None;
                    }
                    let input = window.composer.input()?.clone();
                    input.update(cx, |input, cx| input.set_text(broadcast_command, cx));
                    window.composer.replace_file_attachments(Vec::new());
                    let pane_ids = window.composer_submission_targets(&target);
                    let completion = window.submit_composer_with_completion(true, cx);
                    Some((completion, pane_ids))
                })
                .ok()
                .flatten();
            let (broadcast_succeeded, pane_ids) = match broadcast {
                Some((completion, pane_ids)) => (completion.recv().await == Ok(true), pane_ids),
                None => (false, Vec::new()),
            };
            let broadcast_bytes = window
                .update(cx, |window, _| {
                    pane_ids
                        .iter()
                        .map(|pane_id| {
                            (
                                pane_id.clone(),
                                window
                                    .terminal_runtime
                                    .surfaces
                                    .take_staged_input_bytes(pane_id),
                            )
                        })
                        .collect::<std::collections::HashMap<_, _>>()
                })
                .unwrap_or_default();

            for _ in 0..100 {
                let broadcast_lines = std::fs::read_to_string(&broadcast_output)
                    .map(|value| value.lines().count())
                    .unwrap_or(0);
                if active_output.exists()
                    && selected_output.exists()
                    && local_output.exists()
                    && no_return_output.exists()
                    && broadcast_lines == pane_ids.len()
                {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
            }

            let screens = window
                .update(cx, |window, _| {
                    pane_ids
                        .iter()
                        .map(|pane_id| {
                            (
                                pane_id.clone(),
                                serde_json::Value::String(
                                    window
                                        .terminal_runtime
                                        .surfaces
                                        .handle(pane_id)
                                        .and_then(|handle| handle.read_screen_text(200))
                                        .unwrap_or_default(),
                                ),
                            )
                        })
                        .collect::<serde_json::Map<String, serde_json::Value>>()
                })
                .unwrap_or_default();
            let read = |path: &Path| std::fs::read_to_string(path).unwrap_or_default();
            let value = serde_json::json!({
                "activeSucceeded": active_succeeded,
                "selectedSucceeded": selected_succeeded,
                "localPathSucceeded": local_path_succeeded,
                "noReturnSucceeded": no_return_succeeded,
                "noReturnBeforeReturn": no_return_before_return,
                "broadcastSucceeded": broadcast_succeeded,
                "paneIds": pane_ids,
                "activeOutput": read(&active_output),
                "selectedOutput": read(&selected_output),
                "localPathOutput": read(&local_output),
                "noReturnOutput": read(&no_return_output),
                "broadcastOutput": read(&broadcast_output),
                "activeBytes": active_bytes,
                "selectedBytes": selected_bytes,
                "localPathBytes": local_path_bytes,
                "noReturnBytes": no_return_bytes,
                "broadcastBytes": broadcast_bytes,
                "screens": screens,
            });
            if let Ok(contents) = serde_json::to_vec_pretty(&value)
                && let Err(error) =
                    crate::composer::submission::write_staged_status(&status_path, &contents)
            {
                log::warn!("failed to write P7 Composer submission status: {error}");
            }
        })
        .detach();
    }

    fn prepare_staged_phase_4(&mut self, cx: &mut Context<Self>) {
        if current_phase_4_status_path().is_none() || self.composer.staged_prepared() {
            return;
        }
        if !self.composer.is_open() {
            let Some(target) = self.active_composer_target() else {
                return;
            };
            self.open_composer_for(target, cx);
        }
        let Some(input) = self.composer.input().cloned() else {
            return;
        };
        let dimension = match self.state.prefs.composer.position {
            ComposerPanelPosition::Right => self.state.prefs.composer.panel_width,
            ComposerPanelPosition::Bottom => self.state.prefs.composer.panel_height,
        };
        self.composer
            .set_staged_restore(crate::composer::StagedComposerRestore {
                text: input.read(cx).text().to_owned(),
                position: self.state.prefs.composer.position,
                mode: self.state.prefs.composer.panel_mode,
                dimension,
                broadcast: self.state.prefs.composer.broadcast,
                font_size: self.state.prefs.composer.font_size,
            });
        input.update(cx, |input, cx| input.set_text("phase-4 draft", cx));
        self.add_composer_files(["/tmp/p7-phase-4.txt".to_owned()], cx);
        let result = self
            .set_composer_position(ComposerPanelPosition::Bottom)
            .and_then(|_| self.set_composer_panel_mode(ComposerPanelMode::Pinned))
            .and_then(|_| self.set_composer_broadcast(true))
            .and_then(|_| {
                ComposerPreferences::try_store_panel_height(260.0)?;
                self.state.prefs.composer.panel_height = 260.0;
                Ok(())
            })
            .and_then(|_| self.set_composer_font_size(15.0, cx))
            .and_then(|_| self.store_composer_editor(cx))
            .and_then(|_| self.composer_store.flush().map(|_| ()));
        if let Err(error) = result {
            self.feedback(
                "Composer phase 4",
                error.to_string(),
                crate::toast::ToastTone::Error,
                cx,
            );
            return;
        }
        self.composer.mark_staged_prepared();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        clear_completed_submission, composer_font_shortcut_delta, phase_4_status_path,
        publish_composer_release, submission_target_ids, submit_to_targets,
    };
    use muxy_core::composer::{ComposerStore, DraftId};
    use muxy_terminal::input::{TerminalInputError, TerminalInputResult};
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::future::Future;
    use std::rc::Rc;
    use std::task::{Context as TaskContext, Poll, Waker};

    #[test]
    fn composer_font_shortcuts_accept_plus_equals_and_minus_with_platform_modifier() {
        for key in ["+", "=", "plus"] {
            assert_eq!(composer_font_shortcut_delta(key, true), Some(1.0));
        }
        for key in ["-", "minus"] {
            assert_eq!(composer_font_shortcut_delta(key, true), Some(-1.0));
        }
        assert_eq!(composer_font_shortcut_delta("+", false), None);
        assert_eq!(composer_font_shortcut_delta("-", false), None);
        assert_eq!(composer_font_shortcut_delta("0", true), None);
    }

    #[test]
    fn phase_4_status_requires_the_exact_isolated_test_case() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let support = root.join("support");
        let home = root.join("home");
        std::fs::create_dir_all(&support).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        assert!(
            phase_4_status_path(false, Some("phase-4"), &support, Some(&support), &home).is_none()
        );
        assert!(
            phase_4_status_path(true, Some("other"), &support, Some(&support), &home).is_none()
        );
        assert!(phase_4_status_path(true, Some("phase-4"), &support, Some(&home), &home).is_none());
        assert_eq!(
            phase_4_status_path(true, Some("phase-4"), &support, Some(&support), &home),
            Some(support.join(".muxy-p7-panel-status.json"))
        );
    }

    #[test]
    fn submission_targets_visible_live_terminals_in_stable_order() {
        let visible = vec![
            ("pane-b".to_owned(), true, true),
            ("browser".to_owned(), false, true),
            ("missing".to_owned(), true, false),
            ("pane-a".to_owned(), true, true),
            ("pane-b".to_owned(), true, true),
        ];
        assert_eq!(
            submission_target_ids("active", true, visible.clone()),
            ["pane-b", "pane-a"]
        );
        assert_eq!(submission_target_ids("active", false, visible), ["active"]);
    }

    #[test]
    fn broadcast_continues_after_an_earlier_pane_fails() {
        let visited = Rc::new(RefCell::new(Vec::new()));
        let captured = visited.clone();
        let targets = vec!["pane-a".to_owned(), "pane-b".to_owned()];
        let mut future = Box::pin(submit_to_targets(&targets, move |pane_id| {
            captured.borrow_mut().push(pane_id.to_owned());
            let (sender, receiver) = async_channel::bounded(1);
            let result: TerminalInputResult = if pane_id == "pane-a" {
                Err(TerminalInputError::SendFailed)
            } else {
                Ok(())
            };
            sender.try_send(result).unwrap();
            Some(receiver)
        }));
        let waker = Waker::noop();
        let mut context = TaskContext::from_waker(waker);
        let Poll::Ready(result) = Future::poll(future.as_mut(), &mut context) else {
            panic!("pre-filled submission completions must be ready");
        };
        assert_eq!(*visited.borrow(), ["pane-a", "pane-b"]);
        assert_eq!(result.unwrap(), ["pane-a: SendFailed"]);
    }

    #[test]
    fn edit_during_submission_prevents_revision_safe_clear() {
        let profile = tempfile::tempdir().unwrap();
        let mut store = ComposerStore::load_from(profile.path());
        let id = DraftId::new(
            "AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE",
            "11111111-2222-4333-8444-555555555555",
        )
        .unwrap();
        let submitted_revision = store
            .edit_content(id.clone(), "submitted".to_owned(), Vec::new())
            .unwrap();
        store
            .edit_content(id.clone(), "edited while sending".to_owned(), Vec::new())
            .unwrap();
        assert!(
            !clear_completed_submission(&mut store, &id, submitted_revision, true, true).unwrap()
        );
        assert_eq!(store.draft(&id).unwrap().text, "edited while sending");
    }

    #[test]
    fn failed_submission_clear_publication_restores_retained_draft() {
        let profile = tempfile::tempdir().unwrap();
        let mut store = ComposerStore::load_from(profile.path());
        let id = DraftId::new(
            "AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE",
            "11111111-2222-4333-8444-555555555555",
        )
        .unwrap();
        let revision = store
            .edit_content(
                id.clone(),
                "retained editor".to_owned(),
                vec!["/tmp/retained".to_owned()],
            )
            .unwrap();
        store.flush().unwrap();
        std::fs::remove_file(store.path()).unwrap();
        std::fs::create_dir(store.path()).unwrap();
        assert!(clear_completed_submission(&mut store, &id, revision, true, true).is_err());
        let retained = store.draft(&id).unwrap();
        assert_eq!(retained.text, "retained editor");
        assert_eq!(retained.file_attachments, ["/tmp/retained"]);
    }

    #[test]
    fn release_publication_clears_only_when_requested() {
        let profile = tempfile::tempdir().unwrap();
        let mut store = ComposerStore::load_from(profile.path());
        let id = DraftId::new(
            "AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE",
            "11111111-2222-4333-8444-555555555555",
        )
        .unwrap();
        publish_composer_release(
            &mut store,
            id.clone(),
            "keep".to_owned(),
            vec!["/tmp/keep".to_owned()],
            false,
        )
        .unwrap();
        assert_eq!(store.draft(&id).unwrap().text, "keep");
        publish_composer_release(
            &mut store,
            id.clone(),
            "clear".to_owned(),
            vec!["/tmp/clear".to_owned()],
            true,
        )
        .unwrap();
        assert!(store.draft(&id).is_none());
        assert!(
            ComposerStore::load_from(profile.path())
                .draft(&id)
                .is_none()
        );
    }

    #[test]
    fn failed_clear_publication_retains_the_live_editor_in_store() {
        let profile = tempfile::tempdir().unwrap();
        let mut store = ComposerStore::load_from(profile.path());
        let id = DraftId::new(
            "AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE",
            "11111111-2222-4333-8444-555555555555",
        )
        .unwrap();
        store
            .edit_content(id.clone(), "before".to_owned(), Vec::new())
            .unwrap();
        store.flush().unwrap();
        std::fs::remove_file(store.path()).unwrap();
        std::fs::create_dir(store.path()).unwrap();
        assert!(
            publish_composer_release(
                &mut store,
                id.clone(),
                "live".to_owned(),
                vec!["/tmp/live".to_owned()],
                true,
            )
            .is_err()
        );
        let retained = store.draft(&id).unwrap();
        assert_eq!(retained.text, "live");
        assert_eq!(retained.file_attachments, ["/tmp/live"]);
    }

    #[test]
    fn file_attachment_deduplication_preserves_first_seen_order() {
        let mut attachments = vec!["/tmp/one".to_owned()];
        let mut seen = attachments.iter().cloned().collect::<HashSet<_>>();
        attachments.extend(
            ["/tmp/two".to_owned(), "/tmp/one".to_owned()]
                .into_iter()
                .filter(|path| seen.insert(path.clone())),
        );
        assert_eq!(attachments, ["/tmp/one", "/tmp/two"]);
    }
}
