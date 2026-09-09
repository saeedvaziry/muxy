use muxy_protocol::SessionId;
use serde::{Deserialize, Serialize};

use crate::PaneId;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Pane {
    pub id: PaneId,
    pub title: String,
    pub content: PaneContent,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaneContent {
    Terminal { session: Option<SessionId> },
    Settings,
}
