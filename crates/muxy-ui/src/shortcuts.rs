//! The single GPUI shortcut registration interface used by app and UI modules.

use gpui::{Action, KeyBinding};
use muxy_core::shortcuts::{ShortcutId, ShortcutSettings};

pub struct Registry<'a> {
    settings: &'a dyn ShortcutSettings,
    bindings: Vec<KeyBinding>,
}

impl std::fmt::Debug for Registry<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("bindings", &self.bindings)
            .finish_non_exhaustive()
    }
}

impl<'a> Registry<'a> {
    pub fn new(settings: &'a dyn ShortcutSettings) -> Self {
        Self {
            settings,
            bindings: Vec::new(),
        }
    }

    pub fn register(&mut self, id: ShortcutId, action: &(impl Action + Clone)) {
        let shortcut = id.definition();
        for context in shortcut.contexts {
            for key in self.settings.keys(shortcut.id, *context) {
                self.bindings
                    .push(KeyBinding::new(&key, action.clone(), *context));
            }
        }
    }

    pub fn into_bindings(self) -> Vec<KeyBinding> {
        self.bindings
    }
}
