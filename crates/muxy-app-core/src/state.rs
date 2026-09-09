use std::collections::HashSet;
use std::path::PathBuf;

use muxy_protocol::SessionId;
use serde::{Deserialize, Serialize};

use crate::{
    AppError, Branch, Color, Direction, PROJECT_COLORS, Pane, PaneContent, PaneId, Project,
    ProjectId, ProjectStatus, ServerId, Tab, TabId, WindowBounds, WindowState,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "StoredState")]
pub struct AppState {
    pub(crate) version: u32,
    pub(crate) projects: Vec<Project>,
    pub(crate) window: WindowState,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) pending_discards: Vec<SessionId>,
}

#[derive(Deserialize)]
struct StoredState {
    version: u32,
    projects: Vec<Project>,
    window: WindowState,
    #[serde(default)]
    pending_discards: Vec<SessionId>,
}

impl TryFrom<StoredState> for AppState {
    type Error = AppError;

    fn try_from(stored: StoredState) -> Result<Self, Self::Error> {
        if stored.version != 1 {
            return Err(AppError::UnsupportedVersion(stored.version));
        }
        let mut state = Self {
            version: stored.version,
            projects: stored.projects,
            window: stored.window,
            pending_discards: stored.pending_discards,
        };
        let previous_project = state.window.current_project;
        let previous_tab = state.window.selected_tab.get(&previous_project).copied();
        state.ensure_home()?;
        if state.window.current_project != previous_project
            || state
                .window
                .selected_tab
                .get(&state.window.current_project)
                .copied()
                != previous_tab
        {
            state.window.active_pane = None;
        }
        let selected = state
            .window
            .selected_tab
            .get(&state.window.current_project)
            .copied();
        for tab in state
            .projects
            .iter_mut()
            .flat_map(|project| &mut project.tabs)
        {
            let legacy = tab.legacy_active_pane.take();
            if let Some(pane) = legacy
                && !state.window.focus_history.contains(&pane)
            {
                state.window.focus_history.push(pane);
            }
            if state.window.active_pane.is_none() && Some(tab.id) == selected {
                state.window.active_pane = legacy.or_else(|| tab.layout.leaves().first().copied());
            }
        }
        let panes: HashSet<_> = state
            .projects
            .iter()
            .flat_map(|project| &project.tabs)
            .flat_map(|tab| &tab.panes)
            .map(|pane| pane.id)
            .collect();
        state
            .window
            .focus_history
            .retain(|pane| panes.contains(pane));
        state.validate()?;
        Ok(state)
    }
}

impl AppState {
    pub fn pending_discards(&self) -> &[SessionId] {
        &self.pending_discards
    }

    pub fn queue_discard(&mut self, session: SessionId) {
        if !self.pending_discards.contains(&session) {
            self.pending_discards.push(session);
        }
    }

    pub fn complete_discard(&mut self, session: SessionId) {
        self.pending_discards.retain(|pending| *pending != session);
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn projects(&self) -> &[Project] {
        &self.projects
    }

    pub fn window(&self) -> &WindowState {
        &self.window
    }

    pub fn project(&self, id: ProjectId) -> Option<&Project> {
        self.projects.iter().find(|project| project.id == id)
    }

    pub fn home(&self) -> &Project {
        &self.projects[0]
    }

    pub fn current_project(&self) -> &Project {
        self.project(self.window.current_project)
            .unwrap_or_else(|| self.home())
    }

    pub fn refresh_project_statuses(&mut self) {
        for project in &mut self.projects {
            project.refresh_status();
        }
    }

    pub fn select_project(&mut self, id: ProjectId) -> Result<(), AppError> {
        self.project_mut(id)?.require_available()?;
        self.window.current_project = id;
        self.focus_selected_tab();
        Ok(())
    }

    pub fn add_project(&mut self, directory: PathBuf) -> Result<ProjectId, AppError> {
        if !directory.is_absolute() || !directory.is_dir() {
            return Err(AppError::InvalidState(
                "project path must be an existing absolute directory".into(),
            ));
        }
        let id = ProjectId::new();
        let name = directory.file_name().map_or_else(
            || directory.to_string_lossy().into_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        let color = PROJECT_COLORS[(self.projects.len() - 1) % PROJECT_COLORS.len()]
            .1
            .parse()?;
        self.projects.push(Project {
            id,
            home: false,
            name,
            icon: None,
            color,
            server_id: ServerId::local(),
            directory,
            kind: None,
            parent_id: None,
            tabs: Vec::new(),
            status: ProjectStatus::Available,
        });
        self.window.current_project = id;
        self.window.active_pane = None;
        Ok(id)
    }

    pub fn rename_project(&mut self, id: ProjectId, name: &str) -> Result<(), AppError> {
        let project = self.project_mut(id)?;
        project.require_available()?;
        if name.trim().is_empty() {
            return Err(AppError::InvalidState(
                "project name cannot be empty".into(),
            ));
        }
        name.trim().clone_into(&mut project.name);
        Ok(())
    }

    pub fn set_project_icon(
        &mut self,
        id: ProjectId,
        icon: Option<String>,
    ) -> Result<(), AppError> {
        let project = self.project_mut(id)?;
        project.require_available()?;
        crate::project::validate_icon(icon.as_deref())?;
        project.icon = icon;
        Ok(())
    }

    pub fn set_project_color(&mut self, id: ProjectId, color: Color) -> Result<(), AppError> {
        let project = self.project_mut(id)?;
        project.require_available()?;
        project.color = color;
        Ok(())
    }

    pub fn move_project(&mut self, id: ProjectId, to: usize) -> Result<(), AppError> {
        let project = self.project_mut(id)?;
        project.require_available()?;
        if project.home || to == 0 || to >= self.projects.len() {
            return Err(AppError::InvalidState(
                "Home stays first; project destination must be in range".into(),
            ));
        }
        let from = self
            .projects
            .iter()
            .position(|project| project.id == id)
            .ok_or(AppError::UnknownProject(id))?;
        let project = self.projects.remove(from);
        self.projects.insert(to, project);
        Ok(())
    }

    pub fn remove_project(&mut self, id: ProjectId) -> Result<Vec<SessionId>, AppError> {
        let index = self
            .projects
            .iter()
            .position(|project| project.id == id)
            .ok_or(AppError::UnknownProject(id))?;
        if self.projects[index].home {
            return Err(AppError::InvalidState("Home cannot be removed".into()));
        }
        let project = self.projects.remove(index);
        self.window.selected_tab.remove(&id);
        self.window
            .focus_history
            .retain(|pane| !project.tabs.iter().any(|tab| tab.layout.contains(*pane)));
        if self.window.current_project == id {
            self.window.current_project = self.home().id;
            self.focus_selected_tab();
        }
        let mut sessions = Vec::new();
        for pane in project.tabs.into_iter().flat_map(|tab| tab.panes) {
            if let PaneContent::Terminal {
                session: Some(session),
            } = pane.content
                && !sessions.contains(&session)
            {
                sessions.push(session);
            }
        }
        Ok(sessions)
    }

    pub fn open_terminal_tab(&mut self, project: ProjectId) -> Result<TabId, AppError> {
        self.project_mut(project)?.require_available()?;
        let tab = Tab::terminal();
        let id = tab.id;
        self.window.activate(tab.layout.leaves().first().copied());
        self.project_mut(project)?.tabs.push(tab);
        self.window.selected_tab.insert(project, id);
        self.window.current_project = project;
        Ok(id)
    }

    pub fn close_tab(&mut self, project: ProjectId, tab: TabId) -> Result<(), AppError> {
        let active = self.window.active_pane;
        let tabs = &mut self.project_mut(project)?.tabs;
        let index = tabs
            .iter()
            .position(|candidate| candidate.id == tab)
            .ok_or(AppError::UnknownTab { project, tab })?;
        let removed_active = active.is_some_and(|pane| tabs[index].layout.contains(pane));
        let removed = tabs.remove(index);
        let next_index = index.min(tabs.len().saturating_sub(1));
        let next = tabs.get_mut(next_index);
        let next_pane = next
            .as_ref()
            .and_then(|tab| tab.layout.leaves().first().copied());
        let next = next.map(|tab| {
            if removed_active {
                tab.zoomed = None;
            }
            tab.id
        });
        self.window
            .focus_history
            .retain(|pane| !removed.layout.contains(*pane));
        if self.window.selected_tab.get(&project) == Some(&tab) {
            if let Some(next) = next {
                self.window.selected_tab.insert(project, next);
            } else {
                self.window.selected_tab.remove(&project);
            }
        }
        if removed_active {
            self.window.activate(next_pane);
        }
        Ok(())
    }

    pub fn close_pane(&mut self, pane: PaneId) -> Result<(), AppError> {
        let (project, tab) = self
            .projects
            .iter()
            .find_map(|project| {
                project.tabs.iter().find_map(|tab| {
                    tab.panes
                        .iter()
                        .any(|candidate| candidate.id == pane)
                        .then_some((project.id, tab.id))
                })
            })
            .ok_or(AppError::UnknownPane(pane))?;
        let target = self.tab_mut(tab)?;
        if target.panes.len() == 1 {
            return self.close_tab(project, tab);
        }
        let neighbor = [
            Direction::Right,
            Direction::Left,
            Direction::Down,
            Direction::Up,
        ]
        .into_iter()
        .find_map(|direction| target.layout.neighbor(pane, direction));
        target.layout.remove(pane);
        target.panes.retain(|candidate| candidate.id != pane);
        if target.zoomed == Some(pane) {
            target.zoomed = None;
        }
        let fallback = neighbor.or_else(|| target.layout.leaves().first().copied());
        self.window
            .focus_history
            .retain(|previous| *previous != pane);
        if self.window.active_pane == Some(pane) {
            self.window.activate(fallback);
        }
        Ok(())
    }

    pub fn split_pane(&mut self, pane: PaneId, edge: Direction) -> Result<PaneId, AppError> {
        let tab = self.pane_tab_mut(pane)?;
        let new = Pane {
            id: PaneId::new(),
            title: "Terminal".into(),
            content: PaneContent::Terminal { session: None },
        };
        tab.layout.split(pane, new.id, edge);
        let id = new.id;
        tab.zoomed = None;
        tab.panes.push(new);
        self.focus_pane(id)?;
        Ok(id)
    }

    pub fn focus_pane(&mut self, pane: PaneId) -> Result<(), AppError> {
        let tab = self.pane_tab_mut(pane)?;
        if tab.zoomed.is_some() {
            tab.zoomed = Some(pane);
        }
        let tab = tab.id;
        let project = self
            .projects
            .iter()
            .find(|project| project.tabs.iter().any(|item| item.id == tab))
            .ok_or(AppError::UnknownPane(pane))?
            .id;
        self.window.current_project = project;
        self.window.selected_tab.insert(project, tab);
        self.window.activate(Some(pane));
        Ok(())
    }

    pub fn toggle_zoom(&mut self, pane: PaneId) -> Result<(), AppError> {
        self.focus_pane(pane)?;
        let tab = self.pane_tab_mut(pane)?;
        tab.zoomed = if tab.zoomed == Some(pane) {
            None
        } else {
            Some(pane)
        };
        Ok(())
    }

    pub fn neighbor(&self, pane: PaneId, direction: Direction) -> Option<PaneId> {
        self.projects
            .iter()
            .flat_map(|project| &project.tabs)
            .find(|tab| tab.layout.contains(pane))?
            .layout
            .neighbor(pane, direction)
    }

    pub fn set_ratio(&mut self, tab: TabId, path: &[Branch], ratio: f32) -> Result<(), AppError> {
        self.tab_mut(tab)?.layout.set_ratio(path, ratio)
    }

    fn tab_mut(&mut self, id: TabId) -> Result<&mut Tab, AppError> {
        let project = self
            .projects
            .iter_mut()
            .find(|project| project.tabs.iter().any(|tab| tab.id == id))
            .ok_or_else(|| AppError::InvalidState("unknown tab".into()))?;
        project.require_available()?;
        project
            .tabs
            .iter_mut()
            .find(|tab| tab.id == id)
            .ok_or_else(|| AppError::InvalidState("unknown tab".into()))
    }

    fn pane_tab_mut(&mut self, id: PaneId) -> Result<&mut Tab, AppError> {
        let project = self
            .projects
            .iter_mut()
            .find(|project| project.tabs.iter().any(|tab| tab.layout.contains(id)))
            .ok_or(AppError::UnknownPane(id))?;
        project.require_available()?;
        project
            .tabs
            .iter_mut()
            .find(|tab| tab.layout.contains(id))
            .ok_or(AppError::UnknownPane(id))
    }

    pub fn select_tab(&mut self, project: ProjectId, tab: TabId) -> Result<(), AppError> {
        self.project_mut(project)?.require_available()?;
        if !self
            .project_mut(project)?
            .tabs
            .iter()
            .any(|item| item.id == tab)
        {
            return Err(AppError::UnknownTab { project, tab });
        }
        self.window.selected_tab.insert(project, tab);
        self.window.current_project = project;
        self.focus_selected_tab();
        Ok(())
    }

    fn focus_selected_tab(&mut self) {
        let selected = self.window.selected_tab.get(&self.window.current_project);
        let tab = self
            .current_project()
            .tabs
            .iter()
            .find(|tab| Some(&tab.id) == selected);
        let pane = tab.and_then(|tab| {
            self.window
                .active_pane
                .filter(|pane| tab.layout.contains(*pane))
                .or(tab.zoomed)
                .or_else(|| {
                    self.window
                        .focus_history
                        .iter()
                        .rev()
                        .copied()
                        .find(|pane| tab.layout.contains(*pane))
                })
                .or_else(|| tab.layout.leaves().first().copied())
        });
        self.window.activate(pane);
    }

    pub fn move_tab(&mut self, project: ProjectId, from: usize, to: usize) -> Result<(), AppError> {
        self.project_mut(project)?.require_available()?;
        let tabs = &mut self.project_mut(project)?.tabs;
        for index in [from, to] {
            if index >= tabs.len() {
                return Err(AppError::InvalidTabIndex {
                    index,
                    len: tabs.len(),
                });
            }
        }
        let tab = tabs.remove(from);
        tabs.insert(to, tab);
        Ok(())
    }

    pub fn set_pane_session(
        &mut self,
        pane: PaneId,
        session: Option<SessionId>,
    ) -> Result<(), AppError> {
        match &mut self.pane_mut(pane)?.content {
            PaneContent::Terminal { session: current } => {
                *current = session;
                Ok(())
            }
            PaneContent::Settings => Err(AppError::NotTerminal(pane)),
        }
    }

    pub fn set_pane_title(
        &mut self,
        pane: PaneId,
        title: impl Into<String>,
    ) -> Result<(), AppError> {
        self.pane_mut(pane)?.title = title.into();
        Ok(())
    }

    pub fn set_window_bounds(&mut self, bounds: Option<WindowBounds>) -> Result<(), AppError> {
        if let Some(bounds) = bounds {
            bounds.validate()?;
        }
        self.window.bounds = bounds;
        Ok(())
    }

    fn project_mut(&mut self, id: ProjectId) -> Result<&mut Project, AppError> {
        self.projects
            .iter_mut()
            .find(|project| project.id == id)
            .ok_or(AppError::UnknownProject(id))
    }

    fn pane_mut(&mut self, id: PaneId) -> Result<&mut Pane, AppError> {
        self.projects
            .iter_mut()
            .flat_map(|project| &mut project.tabs)
            .flat_map(|tab| &mut tab.panes)
            .find(|pane| pane.id == id)
            .ok_or(AppError::UnknownPane(id))
    }

    pub(crate) fn validate(&self) -> Result<(), AppError> {
        if self.projects.first().is_none_or(|project| !project.home)
            || self.projects.iter().filter(|project| project.home).count() != 1
        {
            return Err(AppError::InvalidState(
                "exactly one Home project must be first".into(),
            ));
        }
        let mut projects = HashSet::new();
        for project in &self.projects {
            if !projects.insert(project.id) {
                return Err(AppError::InvalidState(format!(
                    "duplicate project ID {}",
                    project.id
                )));
            }
            if project.kind.is_some()
                || project.parent_id.is_some()
                || project.server_id != ServerId::local()
            {
                return Err(AppError::InvalidState(
                    "only top-level projects on the current device are supported".into(),
                ));
            }
            if !project.directory.is_absolute() || project.name.trim().is_empty() {
                return Err(AppError::InvalidState(
                    "project must have a name and an absolute directory".into(),
                ));
            }
            crate::project::validate_icon(project.icon.as_deref())?;
        }
        let mut tabs = HashSet::new();
        let mut panes = HashSet::new();
        for tab in self.projects.iter().flat_map(|project| &project.tabs) {
            if !tabs.insert(tab.id) {
                return Err(AppError::InvalidState(format!(
                    "duplicate tab ID {}",
                    tab.id
                )));
            }
            tab.validate()?;
            for pane in &tab.panes {
                if !panes.insert(pane.id) {
                    return Err(AppError::InvalidState(format!(
                        "duplicate pane ID {}",
                        pane.id
                    )));
                }
            }
        }

        if let Some(active) = self.window.active_pane {
            let selected = self.window.selected_tab.get(&self.window.current_project);
            if !self
                .current_project()
                .tabs
                .iter()
                .any(|tab| Some(&tab.id) == selected && tab.layout.contains(active))
            {
                return Err(AppError::InvalidState(
                    "window active pane must belong to the selected tab".into(),
                ));
            }
        }
        if let Some(bounds) = self.window.bounds {
            bounds.validate()?;
        }
        Ok(())
    }
}
