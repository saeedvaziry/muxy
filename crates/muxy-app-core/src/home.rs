use std::collections::HashMap;
use std::path::PathBuf;

use crate::{AppError, AppState, Color, Project, ProjectId, ProjectStatus, ServerId, WindowState};

impl AppState {
    pub fn bootstrap() -> Result<Self, AppError> {
        let home = new_home(home_directory()?);
        let window = WindowState {
            active_pane: None,
            focus_history: Vec::new(),
            current_project: home.id,
            selected_tab: HashMap::new(),
            bounds: None,
        };
        Ok(Self {
            version: 1,
            projects: vec![home],
            window,
            pending_discards: Vec::new(),
        })
    }

    pub(crate) fn ensure_home(&mut self) -> Result<(), AppError> {
        let directory = home_directory()?;
        let index = self
            .projects
            .iter()
            .position(|project| project.home)
            .or_else(|| {
                self.projects.iter().position(|project| {
                    project.name == "Home"
                        && project.server_id == ServerId::local()
                        && project.kind.is_none()
                        && project.parent_id.is_none()
                })
            });
        if let Some(index) = index {
            let mut home = self.projects.remove(index);
            home.home = true;
            home.directory = directory;
            self.projects.insert(0, home);
        } else {
            self.projects.insert(0, new_home(directory));
        }
        self.refresh_project_statuses();
        if self.project(self.window.current_project).is_none() {
            self.window.current_project = self.home().id;
        }
        self.window.selected_tab.retain(|project, tab| {
            self.projects
                .iter()
                .find(|candidate| candidate.id == *project)
                .is_some_and(|project| project.tabs.iter().any(|candidate| candidate.id == *tab))
        });
        for project in &self.projects {
            if let Some(tab) = project.tabs.first() {
                self.window.selected_tab.entry(project.id).or_insert(tab.id);
            }
        }
        Ok(())
    }
}

fn home_directory() -> Result<PathBuf, AppError> {
    std::env::home_dir()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or(AppError::HomeDirectoryUnavailable)
}

fn new_home(directory: PathBuf) -> Project {
    Project {
        id: ProjectId::new(),
        home: true,
        name: "Home".into(),
        icon: None,
        color: Color::default(),
        server_id: ServerId::local(),
        directory,
        kind: None,
        parent_id: None,
        tabs: Vec::new(),
        status: ProjectStatus::Available,
    }
}
