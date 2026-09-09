use std::error::Error;
use std::fmt;
use std::io;
use std::path::PathBuf;

use crate::{PaneId, ProjectId, TabId};

#[derive(Debug)]
pub enum AppError {
    HomeDirectoryUnavailable,
    UnknownProject(ProjectId),
    UnknownTab {
        project: ProjectId,
        tab: TabId,
    },
    UnknownPane(PaneId),
    NotTerminal(PaneId),
    InvalidTabIndex {
        index: usize,
        len: usize,
    },
    UnsupportedVersion(u32),
    InvalidState(String),
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeDirectoryUnavailable => {
                formatter.write_str("OS home directory is unavailable")
            }
            Self::UnknownProject(id) => write!(formatter, "project {id} does not exist"),
            Self::UnknownTab { project, tab } => {
                write!(formatter, "tab {tab} does not belong to project {project}")
            }
            Self::UnknownPane(id) => write!(formatter, "pane {id} does not exist"),
            Self::NotTerminal(id) => write!(formatter, "pane {id} is not a terminal"),
            Self::InvalidTabIndex { index, len } => {
                write!(
                    formatter,
                    "tab index {index} is out of range for {len} tabs"
                )
            }
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported app state version {version}")
            }
            Self::InvalidState(message) => write!(formatter, "invalid app state: {message}"),
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::Json { path, source } => write!(formatter, "{}: {source}", path.display()),
        }
    }
}

impl Error for AppError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            _ => None,
        }
    }
}
