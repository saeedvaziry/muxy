use gpui::{Entity, Subscription};
use muxy_core::composer::DraftId;
use muxy_core::prefs::{ComposerPanelMode, ComposerPanelPosition};
use muxy_ui::text_input::TextInput;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposerTarget {
    pub project_id: String,
    pub worktree_id: String,
    pub pane_id: String,
}

impl ComposerTarget {
    pub fn new(project_id: String, worktree_id: String, pane_id: String) -> Option<Self> {
        DraftId::new(&project_id, &worktree_id)?;
        Some(Self {
            project_id,
            worktree_id,
            pane_id,
        })
    }

    pub fn draft_id(&self) -> DraftId {
        DraftId::new(&self.project_id, &self.worktree_id).expect("validated Composer target")
    }

    pub fn same_worktree(&self, other: &Self) -> bool {
        self.project_id.eq_ignore_ascii_case(&other.project_id)
            && self.worktree_id.eq_ignore_ascii_case(&other.worktree_id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetTransition {
    Unchanged,
    RebindPane,
    TransferWorktree,
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StagedComposerRestore {
    pub text: String,
    pub position: ComposerPanelPosition,
    pub mode: ComposerPanelMode,
    pub dimension: f64,
    pub broadcast: bool,
    pub font_size: f64,
}

pub fn target_transition(
    current: Option<&ComposerTarget>,
    next: Option<&ComposerTarget>,
) -> TargetTransition {
    match (current, next) {
        (Some(current), Some(next)) if current == next => TargetTransition::Unchanged,
        (Some(current), Some(next)) if current.same_worktree(next) => TargetTransition::RebindPane,
        (Some(_), Some(_)) => TargetTransition::TransferWorktree,
        (Some(_), None) => TargetTransition::Close,
        (None, _) => TargetTransition::Unchanged,
    }
}

pub fn clear_on_target_transition(transition: TargetTransition, clear_on_close: bool) -> bool {
    clear_on_close
        && matches!(
            transition,
            TargetTransition::TransferWorktree | TargetTransition::Close
        )
}

pub fn picker_target_matches(expected: &DraftId, current: Option<&ComposerTarget>) -> bool {
    current.is_some_and(|target| target.draft_id() == *expected)
}

#[derive(Default)]
pub struct ComposerController {
    target: Option<ComposerTarget>,
    input: Option<Entity<TextInput>>,
    file_attachments: Vec<String>,
    input_subscription: Option<Subscription>,
    release_blocked: bool,
    staged_restore: Option<StagedComposerRestore>,
    staged_prepared: bool,
    staged_submission_started: bool,
    menu_open: bool,
}

impl ComposerController {
    pub fn is_open(&self) -> bool {
        self.target.is_some() && self.input.is_some()
    }

    pub fn target(&self) -> Option<&ComposerTarget> {
        self.target.as_ref()
    }

    pub fn input(&self) -> Option<&Entity<TextInput>> {
        self.input.as_ref()
    }

    pub fn file_attachments(&self) -> &[String] {
        &self.file_attachments
    }

    pub fn open(
        &mut self,
        target: ComposerTarget,
        input: Entity<TextInput>,
        file_attachments: Vec<String>,
        subscription: Subscription,
    ) {
        self.target = Some(target);
        self.input = Some(input);
        self.file_attachments = file_attachments;
        self.input_subscription = Some(subscription);
        self.release_blocked = false;
    }

    pub fn rebind_pane(&mut self, target: ComposerTarget) {
        if self
            .target
            .as_ref()
            .is_some_and(|current| current.same_worktree(&target))
        {
            self.target = Some(target);
            self.release_blocked = false;
        }
    }

    pub fn release_blocked(&self) -> bool {
        self.release_blocked
    }

    pub fn block_release(&mut self) {
        self.release_blocked = true;
    }

    pub fn allow_release(&mut self) {
        self.release_blocked = false;
    }

    pub fn close(&mut self) {
        self.target = None;
        self.input = None;
        self.file_attachments.clear();
        self.input_subscription = None;
        self.release_blocked = false;
        self.menu_open = false;
    }

    pub fn menu_open(&self) -> bool {
        self.menu_open
    }

    pub fn set_menu_open(&mut self, open: bool) {
        self.menu_open = open;
    }

    pub fn replace_file_attachments(&mut self, paths: Vec<String>) {
        self.file_attachments = paths;
    }

    pub fn staged_restore(&self) -> Option<&StagedComposerRestore> {
        self.staged_restore.as_ref()
    }

    pub fn set_staged_restore(&mut self, restore: StagedComposerRestore) {
        self.staged_restore = Some(restore);
    }

    pub fn staged_prepared(&self) -> bool {
        self.staged_prepared
    }

    pub fn mark_staged_prepared(&mut self) {
        self.staged_prepared = true;
    }

    pub fn staged_submission_started(&self) -> bool {
        self.staged_submission_started
    }

    pub fn mark_staged_submission_started(&mut self) {
        self.staged_submission_started = true;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ComposerTarget, TargetTransition, clear_on_target_transition, picker_target_matches,
        target_transition,
    };

    fn target(project: &str, worktree: &str, pane: &str) -> ComposerTarget {
        ComposerTarget::new(project.to_owned(), worktree.to_owned(), pane.to_owned()).unwrap()
    }

    #[test]
    fn target_changes_distinguish_pane_rebind_transfer_and_loss() {
        let current = target(
            "AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE",
            "11111111-2222-4333-8444-555555555555",
            "pane-a",
        );
        let same = current.clone();
        let pane = target(
            "AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE",
            "11111111-2222-4333-8444-555555555555",
            "pane-b",
        );
        let worktree = target(
            "AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE",
            "66666666-7777-4888-8999-AAAAAAAAAAAA",
            "pane-c",
        );
        assert_eq!(
            target_transition(Some(&current), Some(&same)),
            TargetTransition::Unchanged
        );
        assert_eq!(
            target_transition(Some(&current), Some(&pane)),
            TargetTransition::RebindPane
        );
        assert_eq!(
            target_transition(Some(&current), Some(&worktree)),
            TargetTransition::TransferWorktree
        );
        assert_eq!(
            target_transition(Some(&current), None),
            TargetTransition::Close
        );
        assert_eq!(
            target_transition(None, Some(&current)),
            TargetTransition::Unchanged
        );
    }

    #[test]
    fn clear_on_close_applies_only_when_releasing_a_worktree_target() {
        for transition in [TargetTransition::Unchanged, TargetTransition::RebindPane] {
            assert!(!clear_on_target_transition(transition, true));
        }
        for transition in [TargetTransition::TransferWorktree, TargetTransition::Close] {
            assert!(clear_on_target_transition(transition, true));
            assert!(!clear_on_target_transition(transition, false));
        }
    }

    #[test]
    fn file_picker_result_accepts_same_worktree_rebind_only() {
        let original = target(
            "AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE",
            "11111111-2222-4333-8444-555555555555",
            "pane-a",
        );
        let rebound = target(
            "AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE",
            "11111111-2222-4333-8444-555555555555",
            "pane-b",
        );
        let transferred = target(
            "AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE",
            "66666666-7777-4888-8999-AAAAAAAAAAAA",
            "pane-c",
        );
        let expected = original.draft_id();
        assert!(picker_target_matches(&expected, Some(&original)));
        assert!(picker_target_matches(&expected, Some(&rebound)));
        assert!(!picker_target_matches(&expected, Some(&transferred)));
        assert!(!picker_target_matches(&expected, None));
    }
}
