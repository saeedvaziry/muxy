use super::path_service::{PARENT_ROW, PathState};

pub(crate) struct Navigator<'a> {
    pub(crate) path_state: &'a PathState,
}

impl<'a> Navigator<'a> {
    pub(crate) fn new(path_state: &'a PathState) -> Self {
        Self { path_state }
    }

    pub(crate) fn completed_path(&self, highlighted_row: &str) -> String {
        format!(
            "{}{highlighted_row}/",
            self.path_state.completion_display_prefix
        )
    }

    pub(crate) fn ghost_text(&self, highlighted_row: Option<&str>) -> String {
        let Some(row) = highlighted_row.filter(|row| *row != PARENT_ROW) else {
            return String::new();
        };
        let input = &self.path_state.input;
        let completed = self.completed_path(row);
        if let Some(rest) = completed.strip_prefix(input.as_str()) {
            return rest.to_owned();
        }

        let trimmed = input.trim();
        if trimmed.contains('/') || trimmed.starts_with('~') {
            return String::new();
        }
        if row.eq_ignore_ascii_case(trimmed) {
            return "/".to_owned();
        }
        if !row.to_lowercase().starts_with(&trimmed.to_lowercase()) {
            return String::new();
        }
        format!("{}/", &row[trimmed.len()..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::picker::path_service::PathService;

    fn state(input: &str) -> PathState {
        PathService {
            home_directory: "/Users/alice".to_owned(),
        }
        .state(input)
    }

    #[test]
    fn tab_completion_replaces_typed_leaf() {
        let path_state = state("~/Projects/mu");
        assert_eq!(
            Navigator::new(&path_state).completed_path("muxy"),
            "~/Projects/muxy/"
        );
    }

    #[test]
    fn completion_from_empty_and_bare_tilde_stays_absolute() {
        let empty = state("");
        assert_eq!(Navigator::new(&empty).completed_path("Users"), "/Users/");
        let tilde = state("~");
        assert_eq!(
            Navigator::new(&tilde).completed_path("Projects"),
            "~/Projects/"
        );
    }

    #[test]
    fn ghost_text_completes_and_ignores_parent_rows() {
        let typed = state("~/Projects/mu");
        assert_eq!(Navigator::new(&typed).ghost_text(Some("muxy")), "xy/");

        let bare = state("mu");
        assert_eq!(Navigator::new(&bare).ghost_text(Some("muxy")), "xy/");

        let exact = state("muxy");
        assert_eq!(Navigator::new(&exact).ghost_text(Some("muxy")), "/");

        let parent = state("~/Projects/");
        assert_eq!(Navigator::new(&parent).ghost_text(Some("..")), "");
    }
}
