use super::localization::{incompatible_key, parse_strings_catalog};
use super::manifest::{ExtensionCommandAction, ExtensionIcon, ExtensionManifest, PackageManifest};
use plist::Value as PlistValue;
use serde_json::Value as JsonValue;
use std::collections::BTreeSet;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const MANIFEST_FILE_NAME: &str = "package.json";
pub const BUILD_OUTPUT_DIRECTORY_NAME: &str = "dist";
pub const MAX_ICON_SVG_BYTES: u64 = 256 * 1024;
pub const MAX_LOCALIZATION_CATALOG_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct LoadedExtension {
    pub id: String,
    pub package_root: PathBuf,
    pub resource_root: PathBuf,
    pub manifest_path: PathBuf,
    pub manifest: ExtensionManifest,
    pub legacy_enabled: Option<bool>,
}

impl LoadedExtension {
    pub fn display_name(&self) -> &str {
        &self.manifest.name
    }

    pub fn background_script_path(&self) -> Option<PathBuf> {
        self.manifest
            .background
            .as_deref()
            .and_then(|path| self.resolve_resource(path))
    }

    pub fn resolve_resource(&self, relative_path: &str) -> Option<PathBuf> {
        resolve_confined_path(&self.resource_root, relative_path)
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ExtensionLoadError {
    #[error("Manifest not found at {0}")]
    ManifestMissing(PathBuf),
    #[error("Invalid manifest at {path}: {reason}")]
    ManifestInvalid { path: PathBuf, reason: String },
    #[error("Background script not found at {0}")]
    BackgroundScriptMissing(PathBuf),
    #[error("Background script at {0} escapes the extension directory")]
    BackgroundScriptOutsideDirectory(PathBuf),
    #[error(
        "Extension name '{0}' contains invalid characters (use letters, digits, dash, underscore, dot)"
    )]
    InvalidName(String),
    #[error("Tab type '{id}' entry not found at {path}")]
    TabTypeEntryMissing { id: String, path: PathBuf },
    #[error("Tab type '{id}' entry at {path} escapes the extension directory")]
    TabTypeEntryOutsideDirectory { id: String, path: PathBuf },
    #[error("Duplicate tab type '{0}'")]
    DuplicateTabType(String),
    #[error("Home view id must not be empty")]
    HomeViewEmptyId,
    #[error("Home view '{0}' title must not be empty")]
    HomeViewEmptyTitle(String),
    #[error("Home view '{0}' entry must not be empty")]
    HomeViewEntryEmpty(String),
    #[error("Home view '{id}' entry not found at {path}")]
    HomeViewEntryMissing { id: String, path: PathBuf },
    #[error("Home view '{id}' entry at {path} escapes the extension directory")]
    HomeViewEntryOutsideDirectory { id: String, path: PathBuf },
    #[error("Home view '{id}' icon SVG not found at {path}")]
    HomeViewSvgMissing { id: String, path: PathBuf },
    #[error("Home view '{id}' icon SVG at {path} escapes the extension directory")]
    HomeViewSvgOutsideDirectory { id: String, path: PathBuf },
    #[error("Duplicate home view '{0}'")]
    DuplicateHomeView(String),
    #[error("Panel '{id}' entry not found at {path}")]
    PanelEntryMissing { id: String, path: PathBuf },
    #[error("Panel '{id}' entry at {path} escapes the extension directory")]
    PanelEntryOutsideDirectory { id: String, path: PathBuf },
    #[error("Duplicate panel '{0}'")]
    DuplicatePanel(String),
    #[error("Panel '{id}' icon SVG not found at {path}")]
    PanelSvgMissing { id: String, path: PathBuf },
    #[error("Panel '{id}' icon SVG at {path} escapes the extension directory")]
    PanelSvgOutsideDirectory { id: String, path: PathBuf },
    #[error("Panel '{0}' has a header button with an empty id")]
    PanelHeaderButtonEmptyId(String),
    #[error("Panel '{panel_id}' has a duplicate header button '{button_id}'")]
    DuplicatePanelHeaderButton { panel_id: String, button_id: String },
    #[error(
        "Panel '{panel_id}' header button '{button_id}' references unknown command '{command}'"
    )]
    PanelHeaderButtonReferencesUnknownCommand {
        panel_id: String,
        button_id: String,
        command: String,
    },
    #[error("Panel '{panel_id}' header button '{button_id}' icon SVG not found at {path}")]
    PanelHeaderButtonSvgMissing {
        panel_id: String,
        button_id: String,
        path: PathBuf,
    },
    #[error(
        "Panel '{panel_id}' header button '{button_id}' icon SVG at {path} escapes the extension directory"
    )]
    PanelHeaderButtonSvgOutsideDirectory {
        panel_id: String,
        button_id: String,
        path: PathBuf,
    },
    #[error("Popover '{id}' entry not found at {path}")]
    PopoverEntryMissing { id: String, path: PathBuf },
    #[error("Popover '{id}' entry at {path} escapes the extension directory")]
    PopoverEntryOutsideDirectory { id: String, path: PathBuf },
    #[error("Duplicate popover '{0}'")]
    DuplicatePopover(String),
    #[error("Sidebar id must not be empty")]
    SidebarEmptyId,
    #[error("Sidebar '{0}' entry must not be empty")]
    SidebarEntryEmpty(String),
    #[error("Sidebar '{id}' entry not found at {path}")]
    SidebarEntryMissing { id: String, path: PathBuf },
    #[error("Sidebar '{id}' entry at {path} escapes the extension directory")]
    SidebarEntryOutsideDirectory { id: String, path: PathBuf },
    #[error("Sidebar '{id}' icon SVG not found at {path}")]
    SidebarSvgMissing { id: String, path: PathBuf },
    #[error("Sidebar '{id}' icon SVG at {path} escapes the extension directory")]
    SidebarSvgOutsideDirectory { id: String, path: PathBuf },
    #[error("Command '{command_id}' references unknown tab type '{tab_type}'")]
    CommandReferencesUnknownTabType {
        command_id: String,
        tab_type: String,
    },
    #[error("Command '{command_id}' references unknown panel '{panel}'")]
    CommandReferencesUnknownPanel { command_id: String, panel: String },
    #[error("Command '{command_id}' references unknown popover '{popover}'")]
    CommandReferencesUnknownPopover { command_id: String, popover: String },
    #[error("Command '{command_id}' modal entry not found at {path}")]
    CommandModalEntryMissing { command_id: String, path: PathBuf },
    #[error("Command '{command_id}' modal entry at {path} escapes the extension directory")]
    CommandModalEntryOutsideDirectory { command_id: String, path: PathBuf },
    #[error("Command '{command_id}' script not found at {path}")]
    ScriptMissing { command_id: String, path: PathBuf },
    #[error("Command '{command_id}' script at {path} escapes the extension directory")]
    ScriptOutsideDirectory { command_id: String, path: PathBuf },
    #[error("Topbar item id must not be empty")]
    TopbarItemEmptyId,
    #[error("Duplicate topbar item '{0}'")]
    DuplicateTopbarItem(String),
    #[error("Topbar item '{item_id}' references unknown command '{command}'")]
    TopbarItemReferencesUnknownCommand { item_id: String, command: String },
    #[error("Topbar item '{item_id}' icon SVG not found at {path}")]
    TopbarItemSvgMissing { item_id: String, path: PathBuf },
    #[error("Topbar item '{item_id}' icon SVG at {path} escapes the extension directory")]
    TopbarItemSvgOutsideDirectory { item_id: String, path: PathBuf },
    #[error("Status bar item id must not be empty")]
    StatusBarItemEmptyId,
    #[error("Duplicate status bar item '{0}'")]
    DuplicateStatusBarItem(String),
    #[error("Status bar item '{item_id}' references unknown command '{command}'")]
    StatusBarItemReferencesUnknownCommand { item_id: String, command: String },
    #[error("Status bar item '{item_id}' icon SVG not found at {path}")]
    StatusBarItemSvgMissing { item_id: String, path: PathBuf },
    #[error("Status bar item '{item_id}' icon SVG at {path} escapes the extension directory")]
    StatusBarItemSvgOutsideDirectory { item_id: String, path: PathBuf },
    #[error("Setting key must not be empty")]
    SettingEmptyKey,
    #[error("Duplicate setting key '{0}'")]
    DuplicateSettingKey(String),
    #[error("File opener id must not be empty")]
    FileOpenerEmptyId,
    #[error("Duplicate file opener '{0}'")]
    DuplicateFileOpener(String),
    #[error("File opener '{opener_id}' references unknown tab type '{tab_type}'")]
    FileOpenerReferencesUnknownTabType { opener_id: String, tab_type: String },
    #[error("File opener '{0}' has an empty pattern")]
    FileOpenerEmptyPattern(String),
    #[error("Localization id must not be empty")]
    LocalizationEmptyId,
    #[error(
        "Localization id '{0}' contains invalid characters (use letters, digits, dash, underscore, dot)"
    )]
    LocalizationInvalidId(String),
    #[error("Duplicate localization '{0}'")]
    DuplicateLocalization(String),
    #[error("Localization '{localization_id}' has invalid language identifier '{language}'")]
    LocalizationInvalidLanguage {
        localization_id: String,
        language: String,
    },
    #[error("Localization '{0}' title must not be empty")]
    LocalizationEmptyTitle(String),
    #[error("Localization '{localization_id}' bundle at {path} escapes the extension directory")]
    LocalizationBundleOutsideDirectory {
        localization_id: String,
        path: PathBuf,
    },
    #[error("Localization '{localization_id}' bundle not found at {path}")]
    LocalizationBundleMissing {
        localization_id: String,
        path: PathBuf,
    },
    #[error("Localization '{localization_id}' resource bundle is invalid at {path}")]
    LocalizationBundleInvalid {
        localization_id: String,
        path: PathBuf,
    },
    #[error("Localization '{localization_id}' bundle at {path} must not declare executable code")]
    LocalizationBundleExecutable {
        localization_id: String,
        path: PathBuf,
    },
    #[error(
        "Localization '{localization_id}' has no Localizable.strings or Localizable.stringsdict for '{language}' at {path}"
    )]
    LocalizationCatalogMissing {
        localization_id: String,
        language: String,
        path: PathBuf,
    },
    #[error("Localization '{localization_id}' catalog is invalid at {path}")]
    LocalizationCatalogInvalid {
        localization_id: String,
        path: PathBuf,
    },
    #[error("Localization '{localization_id}' catalog at {path} exceeds the size limit")]
    LocalizationCatalogTooLarge {
        localization_id: String,
        path: PathBuf,
    },
    #[error(
        "Localization '{localization_id}' catalog at {path} changes the format placeholders of '{key}'"
    )]
    LocalizationCatalogFormatMismatch {
        localization_id: String,
        path: PathBuf,
        key: String,
    },
    #[error("Remote method id must not be empty")]
    RemoteMethodEmptyId,
    #[error("Remote method id '{0}' must not contain control characters or '|'")]
    RemoteMethodInvalidId(String),
    #[error("Duplicate remote method '{0}'")]
    DuplicateRemoteMethod(String),
}

pub fn load_extension(
    package_root: impl AsRef<Path>,
) -> Result<LoadedExtension, ExtensionLoadError> {
    let package_root = package_root.as_ref().to_path_buf();
    let manifest_path = resolve_manifest_path(&package_root);
    if !manifest_path.exists() {
        return Err(ExtensionLoadError::ManifestMissing(manifest_path));
    }
    let data = fs::read(&manifest_path).map_err(|error| ExtensionLoadError::ManifestInvalid {
        path: manifest_path.clone(),
        reason: error.to_string(),
    })?;
    let package = serde_json::from_slice::<PackageManifest>(&data).map_err(|error| {
        ExtensionLoadError::ManifestInvalid {
            path: manifest_path.clone(),
            reason: error.to_string(),
        }
    })?;
    let manifest = ExtensionManifest::from(package);
    validate_name(&manifest.name)?;
    let resource_root = resolve_resource_root(&package_root);
    let loaded = LoadedExtension {
        id: manifest.name.clone(),
        package_root,
        resource_root,
        manifest_path,
        manifest,
        legacy_enabled: None,
    };
    validate_background(&loaded)?;
    validate_tab_types(&loaded)?;
    validate_home_views(&loaded)?;
    validate_file_openers(&loaded)?;
    validate_localizations(&loaded)?;
    validate_panels(&loaded)?;
    validate_popovers(&loaded)?;
    validate_sidebar(&loaded)?;
    validate_commands(&loaded)?;
    validate_topbar_items(&loaded)?;
    validate_status_bar_items(&loaded)?;
    validate_settings(&loaded)?;
    validate_remote_methods(&loaded)?;
    let legacy_enabled = serde_json::from_slice::<JsonValue>(&data)
        .ok()
        .and_then(|value| value.get("enabled").and_then(JsonValue::as_bool));
    Ok(LoadedExtension {
        legacy_enabled,
        ..loaded
    })
}

pub fn resolve_resource_root(package_root: &Path) -> PathBuf {
    let dist = package_root.join(BUILD_OUTPUT_DIRECTORY_NAME);
    if dist.is_dir() {
        dist
    } else {
        package_root.to_path_buf()
    }
}

pub fn resolve_manifest_path(package_root: &Path) -> PathBuf {
    let dist_manifest = package_root
        .join(BUILD_OUTPUT_DIRECTORY_NAME)
        .join(MANIFEST_FILE_NAME);
    if dist_manifest.exists() {
        dist_manifest
    } else {
        package_root.join(MANIFEST_FILE_NAME)
    }
}

pub fn validate_name(name: &str) -> Result<(), ExtensionLoadError> {
    if name.is_empty()
        || name.starts_with('.')
        || !name
            .chars()
            .all(|character| character.is_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(ExtensionLoadError::InvalidName(name.to_owned()));
    }
    Ok(())
}

fn validate_background(extension: &LoadedExtension) -> Result<(), ExtensionLoadError> {
    let Some(background) = extension.manifest.background.as_deref() else {
        return Ok(());
    };
    let raw_path = extension.resource_root.join(background);
    let Some(path) = extension.resolve_resource(background) else {
        return Err(ExtensionLoadError::BackgroundScriptOutsideDirectory(
            raw_path,
        ));
    };
    if !path.exists() {
        return Err(ExtensionLoadError::BackgroundScriptMissing(path));
    }
    Ok(())
}

fn validate_tab_types(extension: &LoadedExtension) -> Result<(), ExtensionLoadError> {
    let mut seen = BTreeSet::new();
    for tab_type in &extension.manifest.tab_types {
        if !seen.insert(tab_type.id.as_str()) {
            return Err(ExtensionLoadError::DuplicateTabType(tab_type.id.clone()));
        }
        let raw_path = extension.resource_root.join(&tab_type.entry);
        let Some(path) = extension.resolve_resource(&tab_type.entry) else {
            return Err(ExtensionLoadError::TabTypeEntryOutsideDirectory {
                id: tab_type.id.clone(),
                path: raw_path,
            });
        };
        if !path.exists() {
            return Err(ExtensionLoadError::TabTypeEntryMissing {
                id: tab_type.id.clone(),
                path,
            });
        }
    }
    Ok(())
}

fn validate_home_views(extension: &LoadedExtension) -> Result<(), ExtensionLoadError> {
    let mut seen = BTreeSet::new();
    for home_view in &extension.manifest.home_views {
        if home_view.id.is_empty() {
            return Err(ExtensionLoadError::HomeViewEmptyId);
        }
        if !seen.insert(home_view.id.as_str()) {
            return Err(ExtensionLoadError::DuplicateHomeView(home_view.id.clone()));
        }
        if home_view.title.is_empty() {
            return Err(ExtensionLoadError::HomeViewEmptyTitle(home_view.id.clone()));
        }
        if home_view.entry.is_empty() {
            return Err(ExtensionLoadError::HomeViewEntryEmpty(home_view.id.clone()));
        }
        let raw_path = extension.resource_root.join(&home_view.entry);
        let Some(path) = extension.resolve_resource(&home_view.entry) else {
            return Err(ExtensionLoadError::HomeViewEntryOutsideDirectory {
                id: home_view.id.clone(),
                path: raw_path,
            });
        };
        if !path.exists() {
            return Err(ExtensionLoadError::HomeViewEntryMissing {
                id: home_view.id.clone(),
                path,
            });
        }
        if let Some(icon) = &home_view.icon {
            validate_icon(
                extension,
                icon,
                |path| ExtensionLoadError::HomeViewSvgMissing {
                    id: home_view.id.clone(),
                    path,
                },
                |path| ExtensionLoadError::HomeViewSvgOutsideDirectory {
                    id: home_view.id.clone(),
                    path,
                },
            )?;
        }
    }
    Ok(())
}

fn validate_file_openers(extension: &LoadedExtension) -> Result<(), ExtensionLoadError> {
    let tab_type_ids = extension
        .manifest
        .tab_types
        .iter()
        .map(|tab_type| tab_type.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for opener in &extension.manifest.file_openers {
        if opener.id.is_empty() {
            return Err(ExtensionLoadError::FileOpenerEmptyId);
        }
        if !seen.insert(opener.id.as_str()) {
            return Err(ExtensionLoadError::DuplicateFileOpener(opener.id.clone()));
        }
        if !tab_type_ids.contains(opener.tab_type.as_str()) {
            return Err(ExtensionLoadError::FileOpenerReferencesUnknownTabType {
                opener_id: opener.id.clone(),
                tab_type: opener.tab_type.clone(),
            });
        }
        if opener.patterns.iter().any(String::is_empty) {
            return Err(ExtensionLoadError::FileOpenerEmptyPattern(
                opener.id.clone(),
            ));
        }
    }
    Ok(())
}

fn validate_localizations(extension: &LoadedExtension) -> Result<(), ExtensionLoadError> {
    let mut seen = BTreeSet::new();
    for localization in &extension.manifest.localizations {
        if localization.id.is_empty() {
            return Err(ExtensionLoadError::LocalizationEmptyId);
        }
        if !localization
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(ExtensionLoadError::LocalizationInvalidId(
                localization.id.clone(),
            ));
        }
        if !seen.insert(localization.id.as_str()) {
            return Err(ExtensionLoadError::DuplicateLocalization(
                localization.id.clone(),
            ));
        }
        if localization.language.trim() != localization.language
            || !is_valid_language_identifier(&localization.language)
        {
            return Err(ExtensionLoadError::LocalizationInvalidLanguage {
                localization_id: localization.id.clone(),
                language: localization.language.clone(),
            });
        }
        if localization.title.trim().is_empty() {
            return Err(ExtensionLoadError::LocalizationEmptyTitle(
                localization.id.clone(),
            ));
        }
        let raw_bundle_path = extension.resource_root.join(&localization.bundle);
        if !localization.bundle.ends_with(".bundle") {
            return Err(ExtensionLoadError::LocalizationBundleOutsideDirectory {
                localization_id: localization.id.clone(),
                path: raw_bundle_path,
            });
        }
        let Some(bundle_path) = extension.resolve_resource(&localization.bundle) else {
            return Err(ExtensionLoadError::LocalizationBundleOutsideDirectory {
                localization_id: localization.id.clone(),
                path: raw_bundle_path,
            });
        };
        if !bundle_path.is_dir() {
            return Err(ExtensionLoadError::LocalizationBundleMissing {
                localization_id: localization.id.clone(),
                path: bundle_path,
            });
        }
        validate_localization_bundle(extension, localization, &bundle_path)?;
    }
    Ok(())
}

fn validate_localization_bundle(
    extension: &LoadedExtension,
    localization: &super::manifest::ExtensionLocalization,
    bundle_path: &Path,
) -> Result<(), ExtensionLoadError> {
    let bundle_root = fs::canonicalize(bundle_path).map_err(|_| {
        ExtensionLoadError::LocalizationBundleInvalid {
            localization_id: localization.id.clone(),
            path: bundle_path.to_path_buf(),
        }
    })?;
    let info_path = bundle_path.join("Info.plist");
    let Some(resolved_info_path) = resolve_confined_path(bundle_path, "Info.plist") else {
        return Err(ExtensionLoadError::LocalizationBundleInvalid {
            localization_id: localization.id.clone(),
            path: bundle_path.to_path_buf(),
        });
    };
    let info_data = read_regular_bounded(&resolved_info_path, MAX_LOCALIZATION_CATALOG_BYTES)
        .map_err(|_| ExtensionLoadError::LocalizationBundleInvalid {
            localization_id: localization.id.clone(),
            path: bundle_path.to_path_buf(),
        })?;
    if !is_confined(&bundle_root, &resolved_info_path) || !info_path.exists() {
        return Err(ExtensionLoadError::LocalizationBundleInvalid {
            localization_id: localization.id.clone(),
            path: bundle_path.to_path_buf(),
        });
    }
    let info = PlistValue::from_reader(Cursor::new(info_data))
        .ok()
        .and_then(PlistValue::into_dictionary)
        .ok_or_else(|| ExtensionLoadError::LocalizationBundleInvalid {
            localization_id: localization.id.clone(),
            path: bundle_path.to_path_buf(),
        })?;
    if info.contains_key("CFBundleExecutable") {
        return Err(ExtensionLoadError::LocalizationBundleExecutable {
            localization_id: localization.id.clone(),
            path: bundle_path.to_path_buf(),
        });
    }
    let localization_directory = bundle_path.join(format!("{}.lproj", localization.language));
    let catalog_paths = [
        localization_directory.join("Localizable.strings"),
        localization_directory.join("Localizable.stringsdict"),
    ]
    .into_iter()
    .filter(|path| path.exists())
    .collect::<Vec<_>>();
    if catalog_paths.is_empty() {
        return Err(ExtensionLoadError::LocalizationCatalogMissing {
            localization_id: localization.id.clone(),
            language: localization.language.clone(),
            path: localization_directory,
        });
    }
    for catalog_path in catalog_paths {
        validate_localization_catalog(extension, localization, &bundle_root, &catalog_path)?;
    }
    Ok(())
}

fn validate_localization_catalog(
    _extension: &LoadedExtension,
    localization: &super::manifest::ExtensionLocalization,
    bundle_root: &Path,
    catalog_path: &Path,
) -> Result<(), ExtensionLoadError> {
    let resolved_path = fs::canonicalize(catalog_path).map_err(|_| {
        ExtensionLoadError::LocalizationCatalogInvalid {
            localization_id: localization.id.clone(),
            path: catalog_path.to_path_buf(),
        }
    })?;
    if !is_confined(bundle_root, &resolved_path) {
        return Err(ExtensionLoadError::LocalizationBundleOutsideDirectory {
            localization_id: localization.id.clone(),
            path: resolved_path,
        });
    }
    let metadata = fs::metadata(&resolved_path).map_err(|_| {
        ExtensionLoadError::LocalizationCatalogInvalid {
            localization_id: localization.id.clone(),
            path: resolved_path.clone(),
        }
    })?;
    if !metadata.is_file() {
        return Err(ExtensionLoadError::LocalizationCatalogInvalid {
            localization_id: localization.id.clone(),
            path: resolved_path,
        });
    }
    if metadata.len() > MAX_LOCALIZATION_CATALOG_BYTES {
        return Err(ExtensionLoadError::LocalizationCatalogTooLarge {
            localization_id: localization.id.clone(),
            path: resolved_path,
        });
    }
    let data =
        fs::read(&resolved_path).map_err(|_| ExtensionLoadError::LocalizationCatalogInvalid {
            localization_id: localization.id.clone(),
            path: resolved_path.clone(),
        })?;
    if data.len() as u64 > MAX_LOCALIZATION_CATALOG_BYTES {
        return Err(ExtensionLoadError::LocalizationCatalogTooLarge {
            localization_id: localization.id.clone(),
            path: resolved_path,
        });
    }
    let catalog =
        if resolved_path.extension().and_then(|value| value.to_str()) == Some("strings") {
            parse_strings_catalog(&data)
        } else {
            PlistValue::from_reader(Cursor::new(data))
                .ok()
                .and_then(PlistValue::into_dictionary)
        }
        .ok_or_else(|| ExtensionLoadError::LocalizationCatalogInvalid {
            localization_id: localization.id.clone(),
            path: resolved_path.clone(),
        })?;
    if let Some(key) = incompatible_key(&catalog) {
        return Err(ExtensionLoadError::LocalizationCatalogFormatMismatch {
            localization_id: localization.id.clone(),
            path: resolved_path,
            key,
        });
    }
    Ok(())
}

fn read_regular_bounded(path: &Path, maximum: u64) -> Result<Vec<u8>, ()> {
    let metadata = fs::metadata(path).map_err(|_| ())?;
    if !metadata.is_file() || metadata.len() > maximum {
        return Err(());
    }
    let data = fs::read(path).map_err(|_| ())?;
    if data.len() as u64 > maximum {
        return Err(());
    }
    Ok(data)
}

fn is_valid_language_identifier(identifier: &str) -> bool {
    let mut subtags = identifier.split('-');
    let Some(language) = subtags.next() else {
        return false;
    };
    if !(2..=8).contains(&language.len())
        || !language.bytes().all(|byte| byte.is_ascii_alphabetic())
    {
        return false;
    }
    subtags.all(|subtag| {
        (1..=8).contains(&subtag.len()) && subtag.bytes().all(|byte| byte.is_ascii_alphanumeric())
    })
}

fn validate_panels(extension: &LoadedExtension) -> Result<(), ExtensionLoadError> {
    let command_ids = extension
        .manifest
        .commands
        .iter()
        .map(|command| command.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for panel in &extension.manifest.panels {
        if !seen.insert(panel.id.as_str()) {
            return Err(ExtensionLoadError::DuplicatePanel(panel.id.clone()));
        }
        let raw_path = extension.resource_root.join(&panel.entry);
        let Some(path) = extension.resolve_resource(&panel.entry) else {
            return Err(ExtensionLoadError::PanelEntryOutsideDirectory {
                id: panel.id.clone(),
                path: raw_path,
            });
        };
        if !path.exists() {
            return Err(ExtensionLoadError::PanelEntryMissing {
                id: panel.id.clone(),
                path,
            });
        }
        if let Some(icon) = &panel.icon {
            validate_icon(
                extension,
                icon,
                |path| ExtensionLoadError::PanelSvgMissing {
                    id: panel.id.clone(),
                    path,
                },
                |path| ExtensionLoadError::PanelSvgOutsideDirectory {
                    id: panel.id.clone(),
                    path,
                },
            )?;
        }
        let mut seen_buttons = BTreeSet::new();
        for button in &panel.header_buttons {
            if button.id.is_empty() {
                return Err(ExtensionLoadError::PanelHeaderButtonEmptyId(
                    panel.id.clone(),
                ));
            }
            if !seen_buttons.insert(button.id.as_str()) {
                return Err(ExtensionLoadError::DuplicatePanelHeaderButton {
                    panel_id: panel.id.clone(),
                    button_id: button.id.clone(),
                });
            }
            if !command_ids.contains(button.command.as_str()) {
                return Err(
                    ExtensionLoadError::PanelHeaderButtonReferencesUnknownCommand {
                        panel_id: panel.id.clone(),
                        button_id: button.id.clone(),
                        command: button.command.clone(),
                    },
                );
            }
            validate_icon(
                extension,
                &button.icon,
                |path| ExtensionLoadError::PanelHeaderButtonSvgMissing {
                    panel_id: panel.id.clone(),
                    button_id: button.id.clone(),
                    path,
                },
                |path| ExtensionLoadError::PanelHeaderButtonSvgOutsideDirectory {
                    panel_id: panel.id.clone(),
                    button_id: button.id.clone(),
                    path,
                },
            )?;
        }
    }
    Ok(())
}

fn validate_popovers(extension: &LoadedExtension) -> Result<(), ExtensionLoadError> {
    let mut seen = BTreeSet::new();
    for popover in &extension.manifest.popovers {
        if !seen.insert(popover.id.as_str()) {
            return Err(ExtensionLoadError::DuplicatePopover(popover.id.clone()));
        }
        let raw_path = extension.resource_root.join(&popover.entry);
        let Some(path) = extension.resolve_resource(&popover.entry) else {
            return Err(ExtensionLoadError::PopoverEntryOutsideDirectory {
                id: popover.id.clone(),
                path: raw_path,
            });
        };
        if !path.exists() {
            return Err(ExtensionLoadError::PopoverEntryMissing {
                id: popover.id.clone(),
                path,
            });
        }
    }
    Ok(())
}

fn validate_sidebar(extension: &LoadedExtension) -> Result<(), ExtensionLoadError> {
    let Some(sidebar) = &extension.manifest.sidebar else {
        return Ok(());
    };
    if sidebar.id.is_empty() {
        return Err(ExtensionLoadError::SidebarEmptyId);
    }
    if sidebar.entry.is_empty() {
        return Err(ExtensionLoadError::SidebarEntryEmpty(sidebar.id.clone()));
    }
    let raw_path = extension.resource_root.join(&sidebar.entry);
    let Some(path) = extension.resolve_resource(&sidebar.entry) else {
        return Err(ExtensionLoadError::SidebarEntryOutsideDirectory {
            id: sidebar.id.clone(),
            path: raw_path,
        });
    };
    if !path.exists() {
        return Err(ExtensionLoadError::SidebarEntryMissing {
            id: sidebar.id.clone(),
            path,
        });
    }
    if let Some(icon) = &sidebar.icon {
        validate_icon(
            extension,
            icon,
            |path| ExtensionLoadError::SidebarSvgMissing {
                id: sidebar.id.clone(),
                path,
            },
            |path| ExtensionLoadError::SidebarSvgOutsideDirectory {
                id: sidebar.id.clone(),
                path,
            },
        )?;
    }
    Ok(())
}

fn validate_commands(extension: &LoadedExtension) -> Result<(), ExtensionLoadError> {
    let tab_type_ids = extension
        .manifest
        .tab_types
        .iter()
        .map(|tab_type| tab_type.id.as_str())
        .collect::<BTreeSet<_>>();
    let panel_ids = extension
        .manifest
        .panels
        .iter()
        .map(|panel| panel.id.as_str())
        .collect::<BTreeSet<_>>();
    let popover_ids = extension
        .manifest
        .popovers
        .iter()
        .map(|popover| popover.id.as_str())
        .collect::<BTreeSet<_>>();
    for command in &extension.manifest.commands {
        match &command.action {
            ExtensionCommandAction::Event => {}
            ExtensionCommandAction::OpenTab { tab_type, .. } => {
                if !tab_type_ids.contains(tab_type.as_str()) {
                    return Err(ExtensionLoadError::CommandReferencesUnknownTabType {
                        command_id: command.id.clone(),
                        tab_type: tab_type.clone(),
                    });
                }
            }
            ExtensionCommandAction::TogglePanel { panel } => {
                if !panel_ids.contains(panel.as_str()) {
                    return Err(ExtensionLoadError::CommandReferencesUnknownPanel {
                        command_id: command.id.clone(),
                        panel: panel.clone(),
                    });
                }
            }
            ExtensionCommandAction::OpenPopover { popover } => {
                if !popover_ids.contains(popover.as_str()) {
                    return Err(ExtensionLoadError::CommandReferencesUnknownPopover {
                        command_id: command.id.clone(),
                        popover: popover.clone(),
                    });
                }
            }
            ExtensionCommandAction::OpenModal { entry, .. } => {
                let raw_path = extension.resource_root.join(entry);
                let Some(path) = extension.resolve_resource(entry) else {
                    return Err(ExtensionLoadError::CommandModalEntryOutsideDirectory {
                        command_id: command.id.clone(),
                        path: raw_path,
                    });
                };
                if !path.exists() {
                    return Err(ExtensionLoadError::CommandModalEntryMissing {
                        command_id: command.id.clone(),
                        path,
                    });
                }
            }
            ExtensionCommandAction::RunScript { script } => {
                let raw_path = extension.resource_root.join(script);
                let Some(path) = extension.resolve_resource(script) else {
                    return Err(ExtensionLoadError::ScriptOutsideDirectory {
                        command_id: command.id.clone(),
                        path: raw_path,
                    });
                };
                if !path.exists() {
                    return Err(ExtensionLoadError::ScriptMissing {
                        command_id: command.id.clone(),
                        path,
                    });
                }
            }
        }
    }
    Ok(())
}

fn validate_topbar_items(extension: &LoadedExtension) -> Result<(), ExtensionLoadError> {
    let command_ids = extension
        .manifest
        .commands
        .iter()
        .map(|command| command.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for item in &extension.manifest.topbar_items {
        if item.id.is_empty() {
            return Err(ExtensionLoadError::TopbarItemEmptyId);
        }
        if !seen.insert(item.id.as_str()) {
            return Err(ExtensionLoadError::DuplicateTopbarItem(item.id.clone()));
        }
        if !command_ids.contains(item.command.as_str()) {
            return Err(ExtensionLoadError::TopbarItemReferencesUnknownCommand {
                item_id: item.id.clone(),
                command: item.command.clone(),
            });
        }
        validate_icon(
            extension,
            &item.icon,
            |path| ExtensionLoadError::TopbarItemSvgMissing {
                item_id: item.id.clone(),
                path,
            },
            |path| ExtensionLoadError::TopbarItemSvgOutsideDirectory {
                item_id: item.id.clone(),
                path,
            },
        )?;
    }
    Ok(())
}

fn validate_status_bar_items(extension: &LoadedExtension) -> Result<(), ExtensionLoadError> {
    let command_ids = extension
        .manifest
        .commands
        .iter()
        .map(|command| command.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for item in &extension.manifest.status_bar_items {
        if item.id.is_empty() {
            return Err(ExtensionLoadError::StatusBarItemEmptyId);
        }
        if !seen.insert(item.id.as_str()) {
            return Err(ExtensionLoadError::DuplicateStatusBarItem(item.id.clone()));
        }
        if !command_ids.contains(item.command.as_str()) {
            return Err(ExtensionLoadError::StatusBarItemReferencesUnknownCommand {
                item_id: item.id.clone(),
                command: item.command.clone(),
            });
        }
        validate_icon(
            extension,
            &item.icon,
            |path| ExtensionLoadError::StatusBarItemSvgMissing {
                item_id: item.id.clone(),
                path,
            },
            |path| ExtensionLoadError::StatusBarItemSvgOutsideDirectory {
                item_id: item.id.clone(),
                path,
            },
        )?;
    }
    Ok(())
}

fn validate_icon<M, O>(
    extension: &LoadedExtension,
    icon: &ExtensionIcon,
    missing: M,
    outside: O,
) -> Result<(), ExtensionLoadError>
where
    M: Fn(PathBuf) -> ExtensionLoadError,
    O: Fn(PathBuf) -> ExtensionLoadError,
{
    let ExtensionIcon::Svg(path) = icon else {
        return Ok(());
    };
    let raw_path = extension.resource_root.join(path);
    if !path.to_lowercase().ends_with(".svg") {
        return Err(outside(raw_path));
    }
    let Some(resolved_path) = extension.resolve_resource(path) else {
        return Err(outside(raw_path));
    };
    if !resolved_path.exists() {
        return Err(missing(resolved_path));
    }
    let metadata = fs::metadata(&resolved_path).map_err(|_| missing(resolved_path.clone()))?;
    if metadata.len() > MAX_ICON_SVG_BYTES {
        return Err(missing(resolved_path));
    }
    Ok(())
}

fn validate_settings(extension: &LoadedExtension) -> Result<(), ExtensionLoadError> {
    let mut seen = BTreeSet::new();
    for setting in &extension.manifest.settings {
        if setting.key.is_empty() {
            return Err(ExtensionLoadError::SettingEmptyKey);
        }
        if !seen.insert(setting.key.as_str()) {
            return Err(ExtensionLoadError::DuplicateSettingKey(setting.key.clone()));
        }
    }
    Ok(())
}

fn validate_remote_methods(extension: &LoadedExtension) -> Result<(), ExtensionLoadError> {
    let mut seen = BTreeSet::new();
    for method in &extension.manifest.remote_methods {
        if method.id.is_empty() {
            return Err(ExtensionLoadError::RemoteMethodEmptyId);
        }
        if method
            .id
            .chars()
            .any(|character| character == '|' || character.is_control())
        {
            return Err(ExtensionLoadError::RemoteMethodInvalidId(method.id.clone()));
        }
        if !seen.insert(method.id.as_str()) {
            return Err(ExtensionLoadError::DuplicateRemoteMethod(method.id.clone()));
        }
    }
    Ok(())
}

fn resolve_confined_path(root: &Path, relative_path: &str) -> Option<PathBuf> {
    let base = fs::canonicalize(root).ok()?;
    let candidate = root.join(relative_path);
    let resolved = canonicalize_allow_missing(&candidate)?;
    is_confined(&base, &resolved).then_some(resolved)
}

fn canonicalize_allow_missing(path: &Path) -> Option<PathBuf> {
    if let Ok(path) = fs::canonicalize(path) {
        return Some(path);
    }
    let mut ancestor = path;
    let mut missing = Vec::new();
    while !ancestor.exists() {
        missing.push(ancestor.file_name()?.to_os_string());
        ancestor = ancestor.parent()?;
    }
    let mut resolved = fs::canonicalize(ancestor).ok()?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    Some(resolved)
}

fn is_confined(root: &Path, path: &Path) -> bool {
    path == root || path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn package(manifest: &str, files: &[(&str, &str)]) -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("package.json"), manifest).unwrap();
        for (path, contents) in files {
            let path = directory.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        directory
    }

    #[test]
    fn root_and_dist_selection_matches_swift() {
        let directory = package(
            r#"{"name":"root","version":"1.0.0","muxy":{"background":"root.js"}}"#,
            &[("root.js", "root"), ("dist/background.js", "dist")],
        );
        fs::write(
            directory.path().join("dist/package.json"),
            r#"{"name":"built","version":"2.0.0","muxy":{"background":"background.js"}}"#,
        )
        .unwrap();
        let loaded = load_extension(directory.path()).unwrap();
        assert_eq!(loaded.id, "built");
        assert_eq!(loaded.resource_root, directory.path().join("dist"));
        assert_eq!(
            loaded.background_script_path().unwrap(),
            fs::canonicalize(directory.path().join("dist/background.js")).unwrap()
        );
    }

    #[test]
    fn dist_resources_are_used_with_a_root_manifest() {
        let directory = package(
            r#"{"name":"root","version":"1.0.0","muxy":{"background":"background.js"}}"#,
            &[("dist/background.js", "dist")],
        );
        let loaded = load_extension(directory.path()).unwrap();
        assert_eq!(loaded.manifest_path, directory.path().join("package.json"));
        assert_eq!(loaded.resource_root, directory.path().join("dist"));
    }

    #[test]
    fn traversal_and_symlink_escapes_are_rejected() {
        let directory = package(
            r#"{"name":"escape","version":"1.0.0","muxy":{"background":"../outside.js"}}"#,
            &[],
        );
        assert!(matches!(
            load_extension(directory.path()),
            Err(ExtensionLoadError::BackgroundScriptOutsideDirectory(_))
        ));

        let directory = package(
            r#"{"name":"escape","version":"1.0.0","muxy":{"background":"link.js"}}"#,
            &[],
        );
        let outside = tempfile::NamedTempFile::new().unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), directory.path().join("link.js")).unwrap();
        assert!(matches!(
            load_extension(directory.path()),
            Err(ExtensionLoadError::BackgroundScriptOutsideDirectory(_))
        ));
    }

    #[test]
    fn svg_limit_is_inclusive() {
        let directory = package(
            r#"{
                "name":"icons","version":"1.0.0","muxy":{
                    "commands":[{"id":"run","title":"Run"}],
                    "topbarItems":[{"id":"item","icon":{"svg":"icon.svg"},"command":"run"}]
                }
            }"#,
            &[],
        );
        let mut icon = fs::File::create(directory.path().join("icon.svg")).unwrap();
        icon.write_all(&vec![b'x'; MAX_ICON_SVG_BYTES as usize])
            .unwrap();
        load_extension(directory.path()).unwrap();
        icon.write_all(b"x").unwrap();
        assert!(matches!(
            load_extension(directory.path()),
            Err(ExtensionLoadError::TopbarItemSvgMissing { .. })
        ));
    }

    #[test]
    fn duplicate_identifiers_are_rejected_for_every_collection() {
        let cases = [
            (
                r#"{"name":"duplicates","version":"1","muxy":{"homeViews":[{"id":"same","title":"A","entry":"index.html"},{"id":"same","title":"B","entry":"index.html"}]}}"#,
                ExtensionLoadError::DuplicateHomeView("same".to_owned()),
            ),
            (
                r#"{"name":"duplicates","version":"1","muxy":{"panels":[{"id":"same","entry":"index.html"},{"id":"same","entry":"index.html"}]}}"#,
                ExtensionLoadError::DuplicatePanel("same".to_owned()),
            ),
            (
                r#"{"name":"duplicates","version":"1","muxy":{"popovers":[{"id":"same","entry":"index.html"},{"id":"same","entry":"index.html"}]}}"#,
                ExtensionLoadError::DuplicatePopover("same".to_owned()),
            ),
            (
                r#"{"name":"duplicates","version":"1","muxy":{"tabTypes":[{"id":"editor","title":"Editor","entry":"index.html"}],"fileOpeners":[{"id":"same","tabType":"editor"},{"id":"same","tabType":"editor"}]}}"#,
                ExtensionLoadError::DuplicateFileOpener("same".to_owned()),
            ),
            (
                r#"{"name":"duplicates","version":"1","muxy":{"commands":[{"id":"run","title":"Run"}],"topbarItems":[{"id":"same","icon":"bolt","command":"run"},{"id":"same","icon":"bolt","command":"run"}]}}"#,
                ExtensionLoadError::DuplicateTopbarItem("same".to_owned()),
            ),
            (
                r#"{"name":"duplicates","version":"1","muxy":{"commands":[{"id":"run","title":"Run"}],"statusBarItems":[{"id":"same","icon":"bolt","side":"left","command":"run"},{"id":"same","icon":"bolt","side":"right","command":"run"}]}}"#,
                ExtensionLoadError::DuplicateStatusBarItem("same".to_owned()),
            ),
            (
                r#"{"name":"duplicates","version":"1","muxy":{"settings":[{"key":"same","title":"A","type":"string"},{"key":"same","title":"B","type":"bool"}]}}"#,
                ExtensionLoadError::DuplicateSettingKey("same".to_owned()),
            ),
            (
                r#"{"name":"duplicates","version":"1","muxy":{"remoteMethods":[{"id":"same"},{"id":"same"}]}}"#,
                ExtensionLoadError::DuplicateRemoteMethod("same".to_owned()),
            ),
        ];
        for (manifest, expected) in cases {
            let directory = package(manifest, &[("index.html", "index")]);
            assert_eq!(load_extension(directory.path()), Err(expected));
        }

        let localization =
            r#"{"id":"same","language":"de","title":"German","bundle":"German.bundle"}"#;
        let manifest = format!(
            r#"{{"name":"duplicates","version":"1","muxy":{{"localizations":[{localization},{localization}]}}}}"#
        );
        let directory = package(
            &manifest,
            &[
                (
                    "German.bundle/Info.plist",
                    r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>app.muxy.german</string></dict></plist>"#,
                ),
                (
                    "German.bundle/de.lproj/Localizable.strings",
                    r#""Settings" = "Einstellungen";"#,
                ),
            ],
        );
        assert_eq!(
            load_extension(directory.path()),
            Err(ExtensionLoadError::DuplicateLocalization("same".to_owned()))
        );
    }

    #[test]
    fn malformed_remote_method_ids_are_rejected() {
        for id in ["", "bad|id", "bad\nid"] {
            let manifest = serde_json::json!({
                "name": "remote",
                "version": "1",
                "muxy": {"remoteMethods": [{"id": id}]}
            });
            let directory = package(&manifest.to_string(), &[]);
            let error = load_extension(directory.path()).unwrap_err();
            if id.is_empty() {
                assert_eq!(error, ExtensionLoadError::RemoteMethodEmptyId);
            } else {
                assert_eq!(
                    error,
                    ExtensionLoadError::RemoteMethodInvalidId(id.to_owned())
                );
            }
        }
    }

    #[test]
    fn localization_bundles_enforce_structure_size_and_formats() {
        let manifest = r#"{"name":"locale","version":"1","muxy":{"localizations":[{"id":"de","language":"de","title":"German","bundle":"German.bundle"}]}}"#;
        let valid_info = r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>app.muxy.german</string></dict></plist>"#;
        let executable_info = r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>payload</string></dict></plist>"#;

        let executable = package(
            manifest,
            &[
                ("German.bundle/Info.plist", executable_info),
                (
                    "German.bundle/de.lproj/Localizable.strings",
                    r#""Settings" = "Einstellungen";"#,
                ),
            ],
        );
        assert!(matches!(
            load_extension(executable.path()),
            Err(ExtensionLoadError::LocalizationBundleExecutable { .. })
        ));

        let missing_catalog = package(manifest, &[("German.bundle/Info.plist", valid_info)]);
        assert!(matches!(
            load_extension(missing_catalog.path()),
            Err(ExtensionLoadError::LocalizationCatalogMissing { .. })
        ));

        let mismatch = package(
            manifest,
            &[
                ("German.bundle/Info.plist", valid_info),
                (
                    "German.bundle/de.lproj/Localizable.strings",
                    r#""Created branch %@" = "Zweig %d erstellt";"#,
                ),
            ],
        );
        assert!(matches!(
            load_extension(mismatch.path()),
            Err(ExtensionLoadError::LocalizationCatalogFormatMismatch { .. })
        ));

        let oversized = package(manifest, &[("German.bundle/Info.plist", valid_info)]);
        let catalog_path = oversized
            .path()
            .join("German.bundle/de.lproj/Localizable.strings");
        fs::create_dir_all(catalog_path.parent().unwrap()).unwrap();
        let catalog = fs::File::create(&catalog_path).unwrap();
        catalog.set_len(MAX_LOCALIZATION_CATALOG_BYTES + 1).unwrap();
        assert!(matches!(
            load_extension(oversized.path()),
            Err(ExtensionLoadError::LocalizationCatalogTooLarge { .. })
        ));

        let stringsdict = package(
            manifest,
            &[
                ("German.bundle/Info.plist", valid_info),
                (
                    "German.bundle/de.lproj/Localizable.stringsdict",
                    r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>%lld files</key><dict><key>NSStringLocalizedFormatKey</key><string>%#@files@</string><key>files</key><dict><key>NSStringFormatSpecTypeKey</key><string>NSStringPluralRuleType</string><key>NSStringFormatValueTypeKey</key><string>lld</string><key>one</key><string>%lld Datei</string><key>other</key><string>%lld Dateien</string></dict></dict></dict></plist>"#,
                ),
            ],
        );
        load_extension(stringsdict.path()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn localization_symlinks_cannot_escape_the_bundle() {
        let manifest = r#"{"name":"locale","version":"1","muxy":{"localizations":[{"id":"de","language":"de","title":"German","bundle":"German.bundle"}]}}"#;
        let valid_info = r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>app.muxy.german</string></dict></plist>"#;

        let info_escape = package(
            manifest,
            &[(
                "German.bundle/de.lproj/Localizable.strings",
                r#""Settings" = "Einstellungen";"#,
            )],
        );
        let outside_info = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside_info.path(), valid_info).unwrap();
        std::os::unix::fs::symlink(
            outside_info.path(),
            info_escape.path().join("German.bundle/Info.plist"),
        )
        .unwrap();
        assert!(matches!(
            load_extension(info_escape.path()),
            Err(ExtensionLoadError::LocalizationBundleInvalid { .. })
        ));

        let catalog_escape = package(manifest, &[("German.bundle/Info.plist", valid_info)]);
        let catalog_directory = catalog_escape.path().join("German.bundle/de.lproj");
        fs::create_dir_all(&catalog_directory).unwrap();
        let outside_catalog = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside_catalog.path(), r#""Settings" = "Einstellungen";"#).unwrap();
        std::os::unix::fs::symlink(
            outside_catalog.path(),
            catalog_directory.join("Localizable.strings"),
        )
        .unwrap();
        assert!(matches!(
            load_extension(catalog_escape.path()),
            Err(ExtensionLoadError::LocalizationBundleOutsideDirectory { .. })
        ));
    }

    #[test]
    fn missing_and_malformed_manifests_are_distinct() {
        let missing = tempfile::tempdir().unwrap();
        assert!(matches!(
            load_extension(missing.path()),
            Err(ExtensionLoadError::ManifestMissing(_))
        ));
        fs::write(missing.path().join("package.json"), b"{").unwrap();
        assert!(matches!(
            load_extension(missing.path()),
            Err(ExtensionLoadError::ManifestInvalid { .. })
        ));
    }

    #[test]
    fn legacy_enabled_is_exposed_only_after_validation() {
        let directory = package(
            r#"{"name":"legacy","version":"1.0.0","enabled":false,"muxy":{}}"#,
            &[],
        );
        assert_eq!(
            load_extension(directory.path()).unwrap().legacy_enabled,
            Some(false)
        );
    }

    #[test]
    fn valid_names_follow_swift_loader_behavior() {
        validate_name("my_ext.123").unwrap();
        validate_name("müxy").unwrap();
        for invalid in ["", ".hidden", "has space", "slash/in/name"] {
            assert_eq!(
                validate_name(invalid),
                Err(ExtensionLoadError::InvalidName(invalid.to_owned()))
            );
        }
    }
}
