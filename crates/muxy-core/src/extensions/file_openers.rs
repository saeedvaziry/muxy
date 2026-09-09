use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;

use super::manifest::{ExtensionFileOpener, ExtensionTabType};
use super::runtime::ExtensionRuntimeCatalog;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileLocation {
    pub absolute_path: PathBuf,
    pub relative_path: String,
    pub line: Option<u64>,
    pub column: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExtensionFileOpenerBinding {
    pub extension_id: String,
    pub opener: ExtensionFileOpener,
    pub tab_type: ExtensionTabType,
}

pub fn resolve_file_location(value: &str, project_root: &Path) -> Option<FileLocation> {
    let project_root = std::fs::canonicalize(project_root).ok()?;
    let cleaned = value.trim_matches(|character: char| {
        matches!(
            character,
            '"' | '\'' | ' ' | '\t' | '\n' | '\r' | '(' | ')' | '[' | ']' | '<' | '>'
        )
    });
    if cleaned.is_empty() {
        return None;
    }
    if let Ok(url) = url::Url::parse(cleaned)
        && url.scheme() == "file"
    {
        return location_for_path(url.to_file_path().ok()?, &project_root, None, None);
    }
    let decoded = percent_decode_str(cleaned).decode_utf8().ok()?;
    if let Some(location) = location_for_path(&*decoded, &project_root, None, None) {
        return Some(location);
    }
    let (path, line, column) = strip_line_column(&decoded)?;
    location_for_path(path, &project_root, line, column)
}

pub fn resolve_file_opener(
    catalog: &ExtensionRuntimeCatalog,
    selection: &str,
    relative_path: &str,
) -> Option<ExtensionFileOpenerBinding> {
    let (extension_id, opener_id) = selection.split_once(':')?;
    if extension_id.is_empty() || opener_id.is_empty() {
        return None;
    }
    let record = catalog
        .records()
        .get(extension_id)
        .filter(|record| record.enabled)?;
    let opener = record
        .extension
        .manifest
        .file_opener(opener_id)
        .filter(|opener| opener.matches(relative_path))?
        .clone();
    let tab_type = record
        .extension
        .manifest
        .tab_type(&opener.tab_type)?
        .clone();
    Some(ExtensionFileOpenerBinding {
        extension_id: extension_id.to_owned(),
        opener,
        tab_type,
    })
}

fn location_for_path(
    value: impl AsRef<Path>,
    project_root: &Path,
    line: Option<u64>,
    column: Option<u64>,
) -> Option<FileLocation> {
    let path = value.as_ref();
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else if let Ok(rest) = path.strip_prefix("~") {
        crate::prefs::home_dir().join(rest)
    } else {
        project_root.join(path)
    };
    let absolute_path = std::fs::canonicalize(candidate).ok()?;
    if !absolute_path.is_file() || !absolute_path.starts_with(project_root) {
        return None;
    }
    let relative_path = absolute_path
        .strip_prefix(project_root)
        .ok()?
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
    (!relative_path.is_empty()).then_some(FileLocation {
        absolute_path,
        relative_path,
        line,
        column,
    })
}

fn strip_line_column(value: &str) -> Option<(&str, Option<u64>, Option<u64>)> {
    let (prefix, last) = value.rsplit_once(':')?;
    let last = numeric_component(last)?;
    if let Some((path, line)) = prefix.rsplit_once(':')
        && let Some(line) = numeric_component(line)
        && !path.is_empty()
    {
        return Some((path, Some(line), Some(last)));
    }
    (!prefix.is_empty()).then_some((prefix, Some(last), None))
}

fn numeric_component(value: &str) -> Option<u64> {
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::{BuildMode, RuntimePathPolicy};
    use crate::extensions::paths::ExtensionPaths;
    use crate::extensions::state::ExtensionStateStore;

    #[test]
    fn file_locations_are_canonical_project_relative_and_preserve_coordinates() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        std::fs::write(root.path().join("src/main.rs"), "fn main() {}\n").unwrap();

        let location = resolve_file_location("src/main.rs:12:4", root.path()).unwrap();
        assert_eq!(location.relative_path, "src/main.rs");
        assert_eq!((location.line, location.column), (Some(12), Some(4)));
        assert!(resolve_file_location("../outside.rs", root.path()).is_none());
    }

    #[test]
    fn opener_resolution_requires_enabled_matching_declaration() {
        let root = tempfile::tempdir().unwrap();
        let paths = ExtensionPaths::new(RuntimePathPolicy::new(BuildMode::Production), root.path());
        let package = paths.packages.join("editor");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("editor.html"), "editor").unwrap();
        std::fs::write(
            package.join("package.json"),
            r#"{"name":"editor","version":"1","muxy":{"tabTypes":[{"id":"editor","title":"Editor","entry":"editor.html"}],"fileOpeners":[{"id":"rust","tabType":"editor","patterns":["*.rs"]}]}}"#,
        )
        .unwrap();
        let mut state = ExtensionStateStore::open(paths.state_file()).unwrap();
        state.set_enabled("editor", Some(true)).unwrap();
        let catalog = ExtensionRuntimeCatalog::load(paths).unwrap();

        let binding = resolve_file_opener(&catalog, "editor:rust", "src/main.rs").unwrap();
        assert_eq!(binding.tab_type.id, "editor");
        assert!(resolve_file_opener(&catalog, "editor:rust", "README.md").is_none());
    }
}
