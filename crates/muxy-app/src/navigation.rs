use muxy_app_core::{ProjectId, TabId};

type Location = (ProjectId, TabId);

#[derive(Default)]
pub(crate) struct Navigation {
    entries: Vec<Location>,
    cursor: usize,
}

impl Navigation {
    pub(crate) fn record(&mut self, tab: Location) {
        if self.entries.get(self.cursor) == Some(&tab) {
            return;
        }
        self.entries.truncate(self.cursor.saturating_add(1));
        self.entries.push(tab);
        if self.entries.len() > 100 {
            self.entries.remove(0);
        }
        self.cursor = self.entries.len() - 1;
    }

    pub(crate) fn target(
        &self,
        forward: bool,
        live: impl Fn(Location) -> bool,
    ) -> Option<(usize, Location)> {
        if forward {
            (self.cursor + 1..self.entries.len()).find_map(|index| {
                let tab = self.entries[index];
                live(tab).then_some((index, tab))
            })
        } else {
            (0..self.cursor).rev().find_map(|index| {
                let tab = self.entries[index];
                live(tab).then_some((index, tab))
            })
        }
    }

    pub(crate) fn commit(&mut self, index: usize) {
        if index < self.entries.len() {
            self.cursor = index;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_skips_closed_tabs_and_truncates_forward_navigation() {
        let mut history = Navigation::default();
        let first = (ProjectId::new(), TabId::new());
        let second = (ProjectId::new(), TabId::new());
        let third = (ProjectId::new(), TabId::new());
        for tab in [first, second, third] {
            history.record(tab);
        }
        let target = history.target(false, |tab| tab != second);
        assert_eq!(target, Some((0, first)));
        history.commit(0);
        history.record(second);
        assert!(history.target(true, |_| true).is_none());
        assert_eq!(history.target(false, |_| true), Some((0, first)));
    }
}
