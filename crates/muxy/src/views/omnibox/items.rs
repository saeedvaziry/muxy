use crate::state::AppState;
use muxy_core::fold::fold;

use crate::terminal::TerminalSurfaces;
use std::cmp::Reverse;

const CONTIGUOUS_BONUS: i32 = 8;
const BOUNDARY_BONUS: i32 = 6;
const HEAD_START_BONUS: i32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    OpenTabs,
    Projects,
    RecentlyRemovedProjects,
    Worktrees,
    Workspaces,
    CommandShortcuts,
}

impl Scope {
    pub fn placeholder(self) -> &'static str {
        match self {
            Self::Projects => "Search project...",
            Self::RecentlyRemovedProjects => "Search recently removed projects...",
            Self::Worktrees => "Search worktree...",
            Self::Workspaces => "Search workspace...",
            Self::OpenTabs => "Search open tabs...",
            Self::CommandShortcuts => "Search custom commands...",
        }
    }

    pub fn empty_state(self) -> &'static str {
        match self {
            Self::Projects => "No projects found",
            Self::RecentlyRemovedProjects => "No recently removed projects",
            Self::Worktrees => "No worktrees found",
            Self::Workspaces => "No workspaces found",
            Self::OpenTabs => "No open tabs found",
            Self::CommandShortcuts => "No custom commands found",
        }
    }

    pub fn return_label(self) -> &'static str {
        match self {
            Self::Projects | Self::Worktrees | Self::Workspaces => "Switch",
            Self::RecentlyRemovedProjects => "Restore",
            Self::OpenTabs | Self::CommandShortcuts => "Open",
        }
    }

    fn search_word(self) -> &'static str {
        match self {
            Self::Projects => "project",
            Self::RecentlyRemovedProjects => "recently removed project",
            Self::Worktrees => "worktree",
            Self::Workspaces => "workspace",
            Self::OpenTabs => "open tab",
            Self::CommandShortcuts => "custom command",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemAction {
    SelectProject(String),
    RestoreProject(String),
    SelectWorktree {
        project_id: String,
        worktree_id: String,
    },
    SelectGroup(Option<String>),
    SelectTab {
        project_id: String,
        worktree_path: String,
        tab_id: String,
    },
    RunCommand(String),
    RunExtensionCommand {
        extension_id: String,
        command_id: String,
    },
}

#[derive(Debug, Clone)]
pub struct Item {
    pub id: String,
    pub symbol: String,
    pub section: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub search_key: String,
    pub action: ItemAction,
    pub unread: bool,
}

impl Item {
    pub fn accessibility_label(&self) -> String {
        if self.unread {
            format!("{}, unread", self.title)
        } else {
            self.title.clone()
        }
    }
}

pub fn items(state: &AppState, terminals: &TerminalSurfaces, scope: Scope) -> Vec<Item> {
    match scope {
        Scope::Projects => project_items(state),
        Scope::RecentlyRemovedProjects => recently_removed_items(),
        Scope::Worktrees => worktree_items(state),
        Scope::Workspaces => workspace_items(state),
        Scope::OpenTabs => open_tab_items(state, terminals),
        Scope::CommandShortcuts => command_items(state),
    }
}

fn project_items(state: &AppState) -> Vec<Item> {
    state
        .workspace
        .projects
        .iter()
        .map(|project| Item {
            id: format!("project-{}", project.id),
            symbol: "folder".to_owned(),
            section: "Projects".to_owned(),
            title: project.name.clone(),
            subtitle: Some(project.path.clone()),
            search_key: [project.name.as_str(), project.path.as_str()].join(" "),
            action: ItemAction::SelectProject(project.id.clone()),
            unread: state.notification_store.unread_project(&project.id) > 0,
        })
        .collect()
}

fn recently_removed_items() -> Vec<Item> {
    muxy_core::store::load_recently_removed()
        .into_iter()
        .map(|entry| Item {
            id: format!("recent-project-{}", entry.project.id),
            symbol: entry
                .project
                .icon
                .clone()
                .unwrap_or_else(|| "folder".to_owned()),
            section: "Recently Removed".to_owned(),
            title: entry.project.name.clone(),
            subtitle: Some(entry.project.path.clone()),
            search_key: [entry.project.name.as_str(), entry.project.path.as_str()].join(" "),
            action: ItemAction::RestoreProject(entry.project.id.clone()),
            unread: false,
        })
        .collect()
}

fn worktree_items(state: &AppState) -> Vec<Item> {
    let Some(project) = state.active_project() else {
        return Vec::new();
    };
    let Some(worktrees) = state.worktrees.get(&project.id) else {
        return Vec::new();
    };
    worktrees
        .iter()
        .map(|worktree| Item {
            id: format!("worktree-{}", worktree.id),
            symbol: if worktree.is_primary {
                "folder.badge.gearshape".to_owned()
            } else {
                "arrow.triangle.branch".to_owned()
            },
            section: "Worktrees".to_owned(),
            title: worktree.name.clone(),
            subtitle: Some(match worktree.branch.as_deref() {
                Some(branch) => format!("({branch}) {}", worktree.path),
                None => worktree.path.clone(),
            }),
            search_key: [
                worktree.name.as_str(),
                worktree.path.as_str(),
                worktree.branch.as_deref().unwrap_or(""),
            ]
            .join(" "),
            action: ItemAction::SelectWorktree {
                project_id: project.id.clone(),
                worktree_id: worktree.id.clone(),
            },
            unread: state
                .notification_store
                .unread_worktree(&project.id, &worktree.id)
                > 0,
        })
        .collect()
}

fn workspace_items(state: &AppState) -> Vec<Item> {
    let stored: Vec<&muxy_core::store::Project> = state
        .workspace
        .projects
        .iter()
        .filter(|project| !project.is_home())
        .collect();
    let mut items = vec![Item {
        id: "workspace-all".to_owned(),
        symbol: "square.grid.2x2".to_owned(),
        section: "Workspaces".to_owned(),
        title: "All Projects".to_owned(),
        subtitle: Some("All projects".to_owned()),
        search_key: "All Projects".to_owned(),
        action: ItemAction::SelectGroup(None),
        unread: false,
    }];
    for group in state.workspace.groups.all() {
        let count = stored
            .iter()
            .filter(|project| {
                group
                    .project_ids
                    .iter()
                    .any(|member| member.eq_ignore_ascii_case(&project.id))
            })
            .count();
        items.push(Item {
            id: format!("workspace-{}", group.id),
            symbol: "square.stack.3d.up".to_owned(),
            section: "Workspaces".to_owned(),
            title: group.name.clone(),
            subtitle: Some(if count == 1 {
                "1 project".to_owned()
            } else {
                format!("{count} projects")
            }),
            search_key: group.name.clone(),
            action: ItemAction::SelectGroup(Some(group.id.clone())),
            unread: false,
        });
    }
    items
}

fn open_tab_items(state: &AppState, terminals: &TerminalSurfaces) -> Vec<Item> {
    let active_project = state.active_project().map(|project| project.id.clone());
    let active_worktree = active_project
        .as_ref()
        .and_then(|id| state.prefs.active_worktree_ids.get(id).cloned());
    let mut items: Vec<(bool, Item)> = Vec::new();

    for project in &state.workspace.projects {
        for workspace in state.tab_workspaces.states() {
            if !workspace.project_id.eq_ignore_ascii_case(&project.id) {
                continue;
            }
            let worktree = workspace.worktree_id.as_deref().and_then(|worktree_id| {
                state
                    .worktrees
                    .get(&project.id)?
                    .iter()
                    .find(|worktree| worktree.id.eq_ignore_ascii_case(worktree_id))
            });
            let context = worktree
                .and_then(|worktree| {
                    worktree
                        .branch
                        .clone()
                        .or_else(|| Some(worktree.name.clone()))
                })
                .filter(|value| !value.is_empty());
            let section = match &context {
                Some(context) => format!("Open Tabs — {} ({context})", project.name),
                None => format!("Open Tabs — {}", project.name),
            };
            let worktree_path = workspace
                .worktree_path
                .clone()
                .unwrap_or_else(|| project.path.clone());
            let is_active = active_project
                .as_deref()
                .is_some_and(|id| id.eq_ignore_ascii_case(&project.id))
                && workspace.worktree_id.as_deref() == active_worktree.as_deref();

            let Some(root) = workspace.root.as_ref() else {
                continue;
            };
            for area_id in root.area_ids() {
                let Some(area) = root.area_by_id(&area_id) else {
                    continue;
                };
                for tab in &area.tabs {
                    let working_directory = terminals
                        .handle(&tab.id)
                        .and_then(|handle| handle.metadata().working_directory.clone())
                        .or_else(|| tab.project_path.clone());
                    let title = tab.title().to_owned();
                    let search_key = [
                        title.as_str(),
                        working_directory.as_deref().unwrap_or(""),
                        project.name.as_str(),
                        worktree
                            .map(|worktree| worktree.name.as_str())
                            .unwrap_or(""),
                        worktree
                            .and_then(|worktree| worktree.branch.as_deref())
                            .unwrap_or(""),
                    ]
                    .join(" ");
                    items.push((
                        is_active,
                        Item {
                            id: format!("open-{area_id}-{}", tab.id),
                            symbol: "terminal".to_owned(),
                            section: section.clone(),
                            title,
                            subtitle: working_directory,
                            search_key,
                            action: ItemAction::SelectTab {
                                project_id: project.id.clone(),
                                worktree_path: worktree_path.clone(),
                                tab_id: tab.id.clone(),
                            },
                            unread: workspace.worktree_id.as_deref().is_some_and(|worktree_id| {
                                state.notification_store.unread_tab(
                                    &project.id,
                                    worktree_id,
                                    tab.root_id(),
                                ) > 0
                            }),
                        },
                    ));
                }
            }
        }
    }

    let (active, rest): (Vec<_>, Vec<_>) = items.into_iter().partition(|(active, _)| *active);
    active
        .into_iter()
        .chain(rest)
        .map(|(_, item)| item)
        .collect()
}

fn command_items(state: &AppState) -> Vec<Item> {
    if state.active_project().is_none() {
        return Vec::new();
    }
    state
        .command_shortcuts
        .shortcuts
        .iter()
        .filter(|shortcut| !shortcut.trimmed_command().is_empty())
        .map(|shortcut| Item {
            id: format!("shortcut-{}", shortcut.id),
            symbol: "command".to_owned(),
            section: "Custom Commands".to_owned(),
            title: shortcut.display_name(),
            subtitle: Some(shortcut.trimmed_command()),
            search_key: [shortcut.display_name(), shortcut.trimmed_command()].join(" "),
            action: ItemAction::RunCommand(shortcut.id.clone()),
            unread: false,
        })
        .collect()
}

pub(crate) fn extension_command_items(
    catalog: &muxy_core::extensions::runtime::ExtensionRuntimeCatalog,
) -> Vec<Item> {
    catalog
        .records()
        .iter()
        .filter(|(_, record)| record.enabled)
        .flat_map(|(extension_id, record)| {
            let display_name = record.extension.display_name().to_owned();
            record
                .extension
                .manifest
                .commands
                .iter()
                .filter(|command| !command.action.is_anchored())
                .map(move |command| Item {
                    id: format!("extension-command-{extension_id}-{}", command.id),
                    symbol: "puzzlepiece.extension".to_owned(),
                    section: "Extension Commands".to_owned(),
                    title: command.title.clone(),
                    subtitle: command
                        .subtitle
                        .clone()
                        .or_else(|| Some(display_name.clone())),
                    search_key: [
                        display_name.as_str(),
                        command.title.as_str(),
                        command.subtitle.as_deref().unwrap_or(""),
                    ]
                    .join(" "),
                    action: ItemAction::RunExtensionCommand {
                        extension_id: extension_id.clone(),
                        command_id: command.id.clone(),
                    },
                    unread: false,
                })
        })
        .collect()
}

fn haystack(item: &Item, scope: Scope) -> String {
    fold(
        &[
            item.search_key.as_str(),
            item.title.as_str(),
            item.subtitle.as_deref().unwrap_or(""),
            item.section.as_str(),
            scope.search_word(),
        ]
        .join(" "),
    )
}

pub fn score(item: &Item, scope: Scope, folded_query: &str) -> Option<i32> {
    let haystack = haystack(item, scope);
    let bytes = haystack.as_bytes();
    let mut total = 0;
    let mut cursor = 0usize;
    let mut previous: Option<usize> = None;
    let mut first_match: Option<usize> = None;

    for character in folded_query.chars() {
        let mut buffer = [0u8; 4];
        let needle = character.encode_utf8(&mut buffer);
        let index = haystack[cursor..].find(&*needle)? + cursor;
        if first_match.is_none() {
            first_match = Some(index);
        }
        if previous == Some(index) {
            total += CONTIGUOUS_BONUS;
        }
        if index == 0 || matches!(bytes[index - 1], b' ' | b'/' | b'-' | b'_' | b'.') {
            total += BOUNDARY_BONUS;
        }
        cursor = index + needle.len();
        previous = Some(cursor);
    }

    let head = first_match.unwrap_or(0).min(HEAD_START_BONUS as usize) as i32;
    Some(total + HEAD_START_BONUS - head)
}

pub fn ranked(items: Vec<Item>, scope: Scope, query: &str) -> Vec<Item> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return items;
    }
    let folded_query = fold(trimmed);
    let mut sections: Vec<String> = Vec::new();
    let mut scored: Vec<(usize, i32, usize, Item)> = Vec::new();
    for (index, item) in items.into_iter().enumerate() {
        let section_index = match sections.iter().position(|value| value == &item.section) {
            Some(position) => position,
            None => {
                sections.push(item.section.clone());
                sections.len() - 1
            }
        };
        let Some(score) = score(&item, scope, &folded_query) else {
            continue;
        };
        scored.push((section_index, score, index, item));
    }
    scored.sort_by_key(|(section, score, index, _)| (*section, Reverse(*score), *index));
    scored.into_iter().map(|(_, _, _, item)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str, section: &str) -> Item {
        Item {
            id: title.to_owned(),
            symbol: "folder".to_owned(),
            section: section.to_owned(),
            title: title.to_owned(),
            subtitle: None,
            search_key: title.to_owned(),
            action: ItemAction::SelectProject(title.to_owned()),
            unread: false,
        }
    }

    fn titles(items: &[Item]) -> Vec<&str> {
        items.iter().map(|item| item.title.as_str()).collect()
    }

    #[test]
    fn omnibox_unread_accessibility_output_is_explicit() {
        let mut value = item("Project", "Projects");
        assert_eq!(value.accessibility_label(), "Project");
        value.unread = true;
        assert_eq!(value.accessibility_label(), "Project, unread");
    }

    #[test]
    fn cargo_app_tests_cannot_resolve_production_app_support() {
        let production = muxy_core::prefs::home_dir().join("Library/Application Support/Muxy");
        assert_ne!(muxy_core::prefs::app_support_dir(), production);
    }

    fn worktree(id: &str, name: &str, path: &str, is_primary: bool) -> muxy_core::store::Worktree {
        muxy_core::store::Worktree {
            id: id.to_owned(),
            name: name.to_owned(),
            path: path.to_owned(),
            branch: None,
            source: Default::default(),
            is_primary,
            created_at: 0.0,
            last_active_at: None,
        }
    }

    fn state(directory: &str) -> AppState {
        let projects = vec![
            muxy_core::store::Project::new("Alpha".to_owned(), "/tmp/alpha".to_owned(), 0),
            muxy_core::store::Project::new("Beta".to_owned(), "/tmp/beta".to_owned(), 1),
        ];
        let alpha = projects[0].id.clone();
        let beta = projects[1].id.clone();
        let mut tab_workspaces = muxy_core::workspace_store::WorkspaceStore::load_from(
            std::path::Path::new(directory).join("workspaces.json"),
        );
        tab_workspaces.ensure_project(alpha.clone(), "/tmp/alpha".to_owned());
        tab_workspaces.ensure_project(beta.clone(), "/tmp/beta".to_owned());

        let mut worktrees = std::collections::HashMap::new();
        worktrees.insert(
            alpha.clone(),
            vec![
                worktree("wt-alpha", "alpha", "/tmp/alpha", true),
                worktree("wt-alpha-2", "feature", "/tmp/alpha-feature", false),
            ],
        );
        worktrees.insert(
            beta.clone(),
            vec![worktree("wt-beta", "beta", "/tmp/beta", true)],
        );

        let prefs = muxy_core::prefs::Prefs::default();
        AppState {
            metrics: muxy_ui::theme::Metrics::new(prefs.scale.multiplier()),
            theme: crate::themes::load("Muxy", "Muxy"),
            workspace: muxy_core::store::Workspace::for_tests(projects),
            tab_workspaces,
            shortcuts: muxy_core::shortcuts::ShortcutMap::load(),
            command_shortcuts: muxy_core::store::CommandShortcuts::default(),
            worktrees,
            project_operations: crate::project_operations::ProjectOperations::default(),
            navigation: muxy_core::navigation::NavigationHistory::default(),
            navigation_recording_suppressed: false,
            socket_ingress: crate::socket::ingress::IngressQueues::default(),
            notification_store: muxy_core::notifications::NotificationStore::empty_at(
                std::path::Path::new(directory).join("notifications.json"),
            ),
            active_project_id: Some(alpha),
            ide_name: None,
            appearance: muxy_ui::theme::Appearance::Dark,
            prefs,
        }
    }

    #[test]
    fn active_worktree_path_uses_the_persisted_workspace_before_truth_refreshes() {
        let mut state = state("/tmp/muxy-active-worktree-path");
        let project = state.workspace.projects[0].clone();
        state.worktrees.clear();
        state
            .tab_workspaces
            .ensure_worktree(&project.id, "wt-secondary", "/tmp/alpha-secondary");
        state
            .prefs
            .active_worktree_ids
            .insert(project.id.clone(), "wt-secondary".to_owned());

        assert_eq!(state.active_worktree_path(&project), "/tmp/alpha-secondary");

        state.worktrees.insert(
            project.id.clone(),
            vec![worktree("wt-primary", "alpha", "/tmp/alpha", true)],
        );
        assert_eq!(state.active_worktree_path(&project), "/tmp/alpha");
    }

    #[test]
    fn worktrees_are_narrowed_to_the_active_project() {
        let state = state("/tmp/muxy-omnibox-worktrees");
        let surfaces = TerminalSurfaces::new();
        let rows = items(&state, &surfaces, Scope::Worktrees);
        assert_eq!(titles(&rows), vec!["alpha", "feature"]);
        assert_eq!(rows[0].symbol, "folder.badge.gearshape");
        assert_eq!(rows[1].symbol, "arrow.triangle.branch");

        let mut state = state;
        state.active_project_id = None;
        assert!(items(&state, &surfaces, Scope::Worktrees).is_empty());
    }

    #[test]
    fn open_tabs_put_the_active_project_first_and_keep_source_order() {
        let mut state = state("/tmp/muxy-omnibox-tabs");
        let beta = state.workspace.projects[1].id.clone();
        state.active_project_id = Some(beta.clone());
        let surfaces = TerminalSurfaces::new();
        let rows = items(&state, &surfaces, Scope::OpenTabs);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].section.contains("Beta"));
        assert!(rows[1].section.contains("Alpha"));
    }

    #[test]
    fn command_shortcuts_are_empty_without_an_active_project() {
        let mut state = state("/tmp/muxy-omnibox-commands");
        state.active_project_id = None;
        let surfaces = TerminalSurfaces::new();
        assert!(items(&state, &surfaces, Scope::CommandShortcuts).is_empty());
    }

    #[test]
    fn scoring_is_case_and_accent_insensitive() {
        let entry = item("Café", "Projects");
        assert!(score(&entry, Scope::Projects, &fold("CAFE")).is_some());
        assert!(score(&entry, Scope::Projects, &fold("café")).is_some());
    }

    #[test]
    fn a_non_contiguous_subsequence_still_matches() {
        let entry = item("my new terminal", "Open Tabs");
        assert!(score(&entry, Scope::OpenTabs, &fold("mnt")).is_some());
    }

    #[test]
    fn a_character_absent_from_the_haystack_does_not_match() {
        let entry = item("alpha", "Projects");
        assert!(score(&entry, Scope::Projects, &fold("alphaz")).is_none());
    }

    #[test]
    fn a_contiguous_boundary_aligned_hit_ranks_first() {
        let items = vec![item("zzz apple", "Projects"), item("a p p l e", "Projects")];
        let ranked = ranked(items, Scope::Projects, "apple");
        assert_eq!(titles(&ranked)[0], "zzz apple");
    }

    #[test]
    fn equal_scores_keep_resolver_order() {
        let items = vec![item("alpha one", "Projects"), item("alpha two", "Projects")];
        let ranked = ranked(items, Scope::Projects, "alpha");
        assert_eq!(titles(&ranked), vec!["alpha one", "alpha two"]);
    }

    #[test]
    fn an_empty_query_returns_the_input_unchanged() {
        let items = vec![item("b", "Projects"), item("a", "Projects")];
        let ranked = ranked(items, Scope::Projects, "   ");
        assert_eq!(titles(&ranked), vec!["b", "a"]);
    }

    #[test]
    fn sections_never_interleave_regardless_of_score() {
        let items = vec![
            item("zzzz tab", "Open Tabs — A"),
            item("tab", "Open Tabs — B"),
            item("tab exact", "Open Tabs — A"),
        ];
        let ranked = ranked(items, Scope::OpenTabs, "tab");
        let sections: Vec<&str> = ranked.iter().map(|item| item.section.as_str()).collect();
        assert_eq!(
            sections,
            vec!["Open Tabs — A", "Open Tabs — A", "Open Tabs — B"]
        );
    }
}
