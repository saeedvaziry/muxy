use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{AppError, PaneId, ProjectId, TabId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    #[serde(default)]
    pub active_pane: Option<PaneId>,
    #[serde(default)]
    pub(crate) focus_history: Vec<PaneId>,
    pub current_project: ProjectId,
    pub selected_tab: HashMap<ProjectId, TabId>,
    pub bounds: Option<WindowBounds>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl WindowBounds {
    pub(crate) fn validate(self) -> Result<(), AppError> {
        if self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width > 0.0
            && self.height > 0.0
        {
            Ok(())
        } else {
            Err(AppError::InvalidState(
                "window bounds must be finite with positive width and height".into(),
            ))
        }
    }
}

impl WindowState {
    pub(crate) fn activate(&mut self, pane: Option<PaneId>) {
        self.active_pane = pane;
        if let Some(pane) = pane {
            self.focus_history.retain(|previous| *previous != pane);
            self.focus_history.push(pane);
        }
    }
}
