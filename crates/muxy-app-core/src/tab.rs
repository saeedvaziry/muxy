use serde::{Deserialize, Serialize};

use crate::{AppError, Layout, Pane, PaneContent, PaneId, TabId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "StoredTab")]
pub struct Tab {
    pub id: TabId,
    pub panes: Vec<Pane>,
    #[serde(skip)]
    pub(crate) legacy_active_pane: Option<PaneId>,
    pub layout: Layout,
    pub zoomed: Option<PaneId>,
}

#[derive(Deserialize)]
struct StoredTab {
    id: TabId,
    panes: Vec<Pane>,
    #[serde(default)]
    active_pane: Option<PaneId>,
    #[serde(default)]
    layout: Option<Layout>,
    #[serde(default)]
    zoomed: Option<PaneId>,
}

impl TryFrom<StoredTab> for Tab {
    type Error = AppError;

    fn try_from(stored: StoredTab) -> Result<Self, Self::Error> {
        let first = stored
            .panes
            .first()
            .ok_or_else(|| AppError::InvalidState("tab must contain a pane".into()))?;
        let tab = Self {
            id: stored.id,
            layout: stored.layout.unwrap_or(Layout::Leaf(first.id)),
            panes: stored.panes,
            legacy_active_pane: stored.active_pane,
            zoomed: stored.zoomed,
        };
        tab.validate()?;
        Ok(tab)
    }
}

impl Tab {
    pub fn visible_panes(&self) -> Vec<PaneId> {
        self.zoomed
            .map_or_else(|| self.layout.leaves(), |pane| vec![pane])
    }

    pub(crate) fn validate(&self) -> Result<(), AppError> {
        self.layout.validate()?;
        let leaves = self.layout.leaves();
        let ids: std::collections::HashSet<_> = self.panes.iter().map(|pane| pane.id).collect();
        let layout_ids: std::collections::HashSet<_> = leaves.iter().copied().collect();
        if ids.len() != self.panes.len() || layout_ids.len() != leaves.len() || ids != layout_ids {
            return Err(AppError::InvalidState(
                "layout leaves must match the tab's panes exactly".into(),
            ));
        }
        if self
            .legacy_active_pane
            .is_some_and(|pane| !ids.contains(&pane))
            || self.zoomed.is_some_and(|pane| !ids.contains(&pane))
        {
            return Err(AppError::InvalidState(
                "active and zoomed panes must be layout leaves".into(),
            ));
        }
        Ok(())
    }

    pub fn title(&self, active: Option<PaneId>) -> &str {
        self.panes
            .iter()
            .find(|pane| Some(pane.id) == active)
            .or_else(|| {
                let first = self.layout.leaves().first().copied();
                self.panes.iter().find(|pane| Some(pane.id) == first)
            })
            .map_or("", |pane| &pane.title)
    }

    pub(crate) fn terminal() -> Self {
        let pane = Pane {
            id: PaneId::new(),
            title: "Terminal".into(),
            content: PaneContent::Terminal { session: None },
        };
        Self {
            id: TabId::new(),
            legacy_active_pane: None,
            layout: Layout::Leaf(pane.id),
            zoomed: None,
            panes: vec![pane],
        }
    }
}
