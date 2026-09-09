use std::env::home_dir;

pub(crate) const PARENT_ROW: &str = "..";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DirectoryItem {
    Parent,
    Directory(String),
    DirectorySymlink(String),
}

impl DirectoryItem {
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Parent => PARENT_ROW,
            Self::Directory(name) | Self::DirectorySymlink(name) => name,
        }
    }

    pub(crate) fn is_parent(&self) -> bool {
        matches!(self, Self::Parent)
    }

    pub(crate) fn is_symlink(&self) -> bool {
        matches!(self, Self::DirectorySymlink(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypedPathState {
    Missing,
    Directory,
    NotDirectory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathState {
    pub(crate) input: String,
    pub(crate) directory_path: String,
    pub(crate) leaf_filter: String,
    pub(crate) confirm_path: String,
    pub(crate) standardized_confirm_path: String,
    pub(crate) parent_display_path: String,
    pub(crate) completion_display_prefix: String,
}

impl PathState {
    pub(crate) fn directory_read_failure_items(&self) -> Vec<DirectoryItem> {
        if self.directory_path == "/" {
            Vec::new()
        } else {
            vec![DirectoryItem::Parent]
        }
    }

    pub(crate) fn directory_items(&self, items: Vec<DirectoryItem>) -> Vec<DirectoryItem> {
        let shows_dotfiles = self.leaf_filter.starts_with('.');
        let filter = self.leaf_filter.to_lowercase();
        let mut rows: Vec<DirectoryItem> = items
            .into_iter()
            .filter(|item| shows_dotfiles || !item.name().starts_with('.'))
            .filter(|item| filter.is_empty() || item.name().to_lowercase().contains(&filter))
            .collect();
        rows.sort_by(|left, right| natural_compare(left.name(), right.name()));
        if self.directory_path == "/" {
            return rows;
        }
        let mut all = vec![DirectoryItem::Parent];
        all.extend(rows);
        all
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PathService {
    pub(crate) home_directory: String,
}

impl Default for PathService {
    fn default() -> Self {
        Self {
            home_directory: standardize(
                &home_dir().unwrap_or_else(|| "/".into()).to_string_lossy(),
            ),
        }
    }
}

impl PathService {
    pub(crate) fn state(&self, input: &str) -> PathState {
        let trimmed = input.trim();
        let confirm_path = self.confirm_path(trimmed);
        let directory_path = self.directory_path(trimmed, &confirm_path);
        PathState {
            input: input.to_owned(),
            leaf_filter: leaf_filter(trimmed),
            standardized_confirm_path: standardize(&confirm_path),
            parent_display_path: self.parent_display_path(&directory_path),
            completion_display_prefix: self.completion_display_prefix(trimmed, &directory_path),
            directory_path,
            confirm_path,
        }
    }

    pub(crate) fn expanded_path(&self, path: &str) -> String {
        let trimmed = path.trim();
        if trimmed == "~" {
            return self.home_directory.clone();
        }
        if let Some(rest) = trimmed.strip_prefix("~/") {
            return format!("{}/{rest}", self.home_directory);
        }
        trimmed.to_owned()
    }

    pub(crate) fn typed_path_state(path: &str) -> TypedPathState {
        match directory_state(&standardize(path)) {
            DirectoryState::Missing => TypedPathState::Missing,
            DirectoryState::Directory => TypedPathState::Directory,
            DirectoryState::NotDirectory => TypedPathState::NotDirectory,
        }
    }

    pub(crate) fn abbreviated_display_path(&self, path: &str) -> String {
        let standardized = standardize(path);
        let display = if standardized == self.home_directory {
            "~".to_owned()
        } else if let Some(rest) = standardized.strip_prefix(&format!("{}/", self.home_directory)) {
            format!("~/{rest}")
        } else {
            standardized
        };
        if display.ends_with('/') {
            display
        } else {
            format!("{display}/")
        }
    }

    fn confirm_path(&self, trimmed: &str) -> String {
        if trimmed.is_empty() {
            return "/".to_owned();
        }
        let expanded = self.expanded_path(trimmed);
        if expanded.starts_with('/') {
            expanded
        } else {
            format!("/{expanded}")
        }
    }

    fn directory_path(&self, trimmed: &str, expanded_input: &str) -> String {
        if trimmed.is_empty() {
            return "/".to_owned();
        }
        if trimmed == "~" {
            return standardize(&self.home_directory);
        }
        if expanded_input.ends_with('/') {
            return standardize(expanded_input);
        }
        standardize(&parent_path(expanded_input))
    }

    fn parent_display_path(&self, directory_path: &str) -> String {
        if directory_path == "/" {
            return "/".to_owned();
        }
        let parent = standardize(&parent_path(directory_path));
        if parent == self.home_directory {
            return "~/".to_owned();
        }
        match parent.strip_prefix(&format!("{}/", self.home_directory)) {
            Some(rest) => format!("~/{rest}/"),
            None => {
                if parent == "/" {
                    "/".to_owned()
                } else {
                    format!("{parent}/")
                }
            }
        }
    }

    fn completion_display_prefix(&self, trimmed: &str, directory_path: &str) -> String {
        if trimmed.starts_with('~') && directory_path == self.home_directory {
            return "~/".to_owned();
        }
        if trimmed.starts_with('~')
            && let Some(rest) = directory_path.strip_prefix(&format!("{}/", self.home_directory))
        {
            return format!("~/{rest}/");
        }
        if directory_path == "/" {
            "/".to_owned()
        } else {
            format!("{directory_path}/")
        }
    }
}

pub(crate) enum DirectoryState {
    Missing,
    Directory,
    NotDirectory,
}

pub(crate) fn directory_state(path: &str) -> DirectoryState {
    match std::fs::metadata(path) {
        Err(_) => DirectoryState::Missing,
        Ok(metadata) if metadata.is_dir() => DirectoryState::Directory,
        Ok(_) => DirectoryState::NotDirectory,
    }
}

pub(crate) fn directory_contents(path: &str) -> Result<Vec<DirectoryItem>, std::io::Error> {
    let mut items = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let symlink = entry
            .file_type()
            .map(|file_type| file_type.is_symlink())
            .unwrap_or(false);
        if symlink {
            if std::fs::metadata(entry.path())
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false)
            {
                items.push(DirectoryItem::DirectorySymlink(name));
            }
            continue;
        }
        if entry
            .file_type()
            .map(|file_type| file_type.is_dir())
            .unwrap_or(false)
        {
            items.push(DirectoryItem::Directory(name));
        }
    }
    Ok(items)
}

pub(crate) fn standardize(path: &str) -> String {
    let trimmed = path.trim();
    let is_absolute = trimmed.starts_with('/');
    let mut components: Vec<&str> = Vec::new();
    for segment in trimmed.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if matches!(components.last(), Some(last) if *last != "..") {
                    components.pop();
                } else if !is_absolute {
                    components.push("..");
                }
            }
            segment => components.push(segment),
        }
    }
    let joined = components.join("/");
    if is_absolute {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_owned()
    } else {
        joined
    }
}

pub(crate) fn parent_path(path: &str) -> String {
    let standardized = standardize(path);
    match standardized.rfind('/') {
        Some(0) | None => "/".to_owned(),
        Some(index) => standardized[..index].to_owned(),
    }
}

pub(crate) fn last_component(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(index) => trimmed[index + 1..].to_owned(),
        None => trimmed.to_owned(),
    }
}

fn leaf_filter(trimmed: &str) -> String {
    if trimmed.is_empty() || trimmed == "~" || trimmed.ends_with('/') {
        return String::new();
    }
    last_component(trimmed)
}

pub(crate) fn natural_compare(left: &str, right: &str) -> std::cmp::Ordering {
    let mut left_chars = left.chars().peekable();
    let mut right_chars = right.chars().peekable();

    loop {
        match (left_chars.peek().copied(), right_chars.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(left_char), Some(right_char)) => {
                if left_char.is_ascii_digit() && right_char.is_ascii_digit() {
                    let left_number = take_number(&mut left_chars);
                    let right_number = take_number(&mut right_chars);
                    match left_number.cmp(&right_number) {
                        std::cmp::Ordering::Equal => {}
                        order => return order,
                    }
                }
                left_chars.next();
                right_chars.next();
                let left_key = left_char.to_lowercase().next().unwrap_or(left_char);
                let right_key = right_char.to_lowercase().next().unwrap_or(right_char);
                match left_key.cmp(&right_key) {
                    std::cmp::Ordering::Equal => {}
                    order => return order,
                }
            }
        }
    }
}

fn take_number(chars: &mut std::iter::Peekable<std::str::Chars>) -> u128 {
    let mut value: u128 = 0;
    while let Some(character) = chars.peek().copied() {
        if !character.is_ascii_digit() {
            break;
        }
        value = value
            .saturating_mul(10)
            .saturating_add(character as u128 - '0' as u128);
        chars.next();
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> PathService {
        PathService {
            home_directory: "/Users/alice".to_owned(),
        }
    }

    #[test]
    fn expands_standardizes_and_abbreviates() {
        let service = service();
        assert_eq!(service.expanded_path("~/Projects"), "/Users/alice/Projects");
        assert_eq!(service.expanded_path("~"), "/Users/alice");
        assert_eq!(
            standardize("/Users/alice/Projects/../Code"),
            "/Users/alice/Code"
        );
        assert_eq!(
            service.abbreviated_display_path("/Users/alice/Projects"),
            "~/Projects/"
        );
        assert_eq!(service.abbreviated_display_path("/tmp/muxy"), "/tmp/muxy/");
    }

    #[test]
    fn separates_directory_leaf_and_confirm_paths() {
        let service = service();
        let tilde = service.state("~/Projects/mu");
        assert_eq!(tilde.directory_path, "/Users/alice/Projects");
        assert_eq!(tilde.leaf_filter, "mu");
        assert_eq!(tilde.confirm_path, "/Users/alice/Projects/mu");
        assert_eq!(tilde.completion_display_prefix, "~/Projects/");

        let bare = service.state("mu");
        assert_eq!(bare.directory_path, "/");
        assert_eq!(bare.leaf_filter, "mu");
        assert_eq!(bare.confirm_path, "/mu");

        let root = service.state("");
        assert_eq!(root.directory_path, "/");
        assert_eq!(root.leaf_filter, "");
        assert_eq!(root.confirm_path, "/");
    }

    #[test]
    fn parent_display_path_walks_to_root() {
        let service = service();
        assert_eq!(service.state("~/Projects/").parent_display_path, "~/");
        assert_eq!(service.state("~/").parent_display_path, "/Users/");
        assert_eq!(service.state("/Users/").parent_display_path, "/");
        assert_eq!(service.state("/").parent_display_path, "/");
    }

    #[test]
    fn directory_items_filter_sort_and_prepend_parent() {
        let service = service();
        let items = vec![
            DirectoryItem::Directory("beta".into()),
            DirectoryItem::Directory("alpha".into()),
            DirectoryItem::Directory(".hidden".into()),
        ];

        let unfiltered = service
            .state("/Users/alice/code/")
            .directory_items(items.clone());
        assert_eq!(unfiltered[0], DirectoryItem::Parent);
        assert_eq!(unfiltered[1].name(), "alpha");
        assert_eq!(unfiltered[2].name(), "beta");
        assert_eq!(unfiltered.len(), 3);

        let filtered = service
            .state("/Users/alice/code/al")
            .directory_items(items.clone());
        assert!(filtered.iter().any(|item| item.name() == "alpha"));
        assert!(!filtered.iter().any(|item| item.name() == "beta"));

        let dotted = service.state("/Users/alice/code/.").directory_items(items);
        assert!(dotted.iter().any(|item| item.name() == ".hidden"));
    }

    #[test]
    fn root_directory_has_no_parent_row() {
        let rows = service()
            .state("/")
            .directory_items(vec![DirectoryItem::Directory("Users".into())]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name(), "Users");
    }
}
