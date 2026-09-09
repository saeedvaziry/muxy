use super::loader::{ExtensionLoadError, LoadedExtension, load_extension};
use super::paths::ExtensionPaths;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExtensionPackageError {
    #[error("extension package is invalid: {0}")]
    Invalid(#[from] ExtensionLoadError),
    #[error("extension '{0}' is already installed")]
    AlreadyInstalled(String),
    #[error("extension package contains an unsafe link at {0}")]
    UnsafeLink(PathBuf),
    #[error("extension package contains a recursive directory link at {0}")]
    RecursiveLink(PathBuf),
    #[error("extension package contains an unsupported entry at {0}")]
    UnsupportedEntry(PathBuf),
    #[error("failed to stage extension package: {0}")]
    Stage(#[source] std::io::Error),
    #[error("failed to publish extension package: {0}")]
    Publish(#[source] std::io::Error),
}

#[derive(Clone, Debug)]
pub struct ExtensionPackagePublisher {
    paths: ExtensionPaths,
}

impl ExtensionPackagePublisher {
    pub fn new(paths: ExtensionPaths) -> Self {
        Self { paths }
    }

    pub fn publish(
        &self,
        source: impl AsRef<Path>,
        replace: bool,
    ) -> Result<LoadedExtension, ExtensionPackageError> {
        let source = source.as_ref();
        let source_extension = load_extension(source)?;
        let destination = self.paths.packages.join(&source_extension.id);
        if destination.exists() && !replace {
            return Err(ExtensionPackageError::AlreadyInstalled(source_extension.id));
        }
        std::fs::create_dir_all(&self.paths.staging).map_err(ExtensionPackageError::Stage)?;
        std::fs::create_dir_all(&self.paths.packages).map_err(ExtensionPackageError::Publish)?;
        let stage = self.paths.staging.join(format!(
            ".{}.{}.stage",
            source_extension.id,
            crate::store::new_uuid()
        ));
        let result = self.stage_and_publish(source, &source_extension.id, &stage, &destination);
        if stage.exists() {
            let _ = std::fs::remove_dir_all(&stage);
        }
        result
    }

    fn stage_and_publish(
        &self,
        source: &Path,
        expected_id: &str,
        stage: &Path,
        destination: &Path,
    ) -> Result<LoadedExtension, ExtensionPackageError> {
        copy_package(source, stage)?;
        let staged = load_extension(stage)?;
        if staged.id != expected_id {
            return Err(ExtensionPackageError::Invalid(
                ExtensionLoadError::InvalidName(staged.id),
            ));
        }
        if destination.exists() {
            exchange_paths(stage, destination).map_err(ExtensionPackageError::Publish)?;
            if let Err(error) = std::fs::remove_dir_all(stage) {
                log::warn!("failed to remove replaced extension package: {error}");
            }
        } else {
            std::fs::rename(stage, destination).map_err(ExtensionPackageError::Publish)?;
        }
        sync_parent(destination).map_err(ExtensionPackageError::Publish)?;
        load_extension(destination).map_err(ExtensionPackageError::Invalid)
    }
}

fn copy_package(source: &Path, destination: &Path) -> Result<(), ExtensionPackageError> {
    let root = std::fs::canonicalize(source).map_err(ExtensionPackageError::Stage)?;
    let metadata = std::fs::metadata(&root).map_err(ExtensionPackageError::Stage)?;
    if !metadata.is_dir() {
        return Err(ExtensionPackageError::Stage(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "extension package root is not a directory",
        )));
    }
    let mut stack = BTreeSet::new();
    copy_directory(&root, &root, destination, &mut stack)
}

fn copy_directory(
    root: &Path,
    source: &Path,
    destination: &Path,
    stack: &mut BTreeSet<PathBuf>,
) -> Result<(), ExtensionPackageError> {
    let canonical = std::fs::canonicalize(source).map_err(ExtensionPackageError::Stage)?;
    if !canonical.starts_with(root) {
        return Err(ExtensionPackageError::UnsafeLink(source.to_path_buf()));
    }
    if !stack.insert(canonical.clone()) {
        return Err(ExtensionPackageError::RecursiveLink(source.to_path_buf()));
    }
    std::fs::create_dir(destination).map_err(ExtensionPackageError::Stage)?;
    let entries = std::fs::read_dir(&canonical).map_err(ExtensionPackageError::Stage)?;
    for entry in entries {
        let entry = entry.map_err(ExtensionPackageError::Stage)?;
        let source_entry = entry.path();
        let destination_entry = destination.join(entry.file_name());
        copy_entry(root, &source_entry, &destination_entry, stack)?;
    }
    stack.remove(&canonical);
    Ok(())
}

fn copy_entry(
    root: &Path,
    source: &Path,
    destination: &Path,
    stack: &mut BTreeSet<PathBuf>,
) -> Result<(), ExtensionPackageError> {
    let metadata = std::fs::symlink_metadata(source).map_err(ExtensionPackageError::Stage)?;
    let resolved = if metadata.file_type().is_symlink() {
        let resolved = std::fs::canonicalize(source).map_err(ExtensionPackageError::Stage)?;
        if !resolved.starts_with(root) {
            return Err(ExtensionPackageError::UnsafeLink(source.to_path_buf()));
        }
        resolved
    } else {
        source.to_path_buf()
    };
    let resolved_metadata = std::fs::metadata(&resolved).map_err(ExtensionPackageError::Stage)?;
    if resolved_metadata.is_dir() {
        copy_directory(root, &resolved, destination, stack)
    } else if resolved_metadata.is_file() {
        std::fs::copy(&resolved, destination)
            .map(|_| ())
            .map_err(ExtensionPackageError::Stage)
    } else {
        Err(ExtensionPackageError::UnsupportedEntry(
            source.to_path_buf(),
        ))
    }
}

#[cfg(target_os = "macos")]
fn exchange_paths(left: &Path, right: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let left = std::ffi::CString::new(left.as_os_str().as_bytes())?;
    let right = std::ffi::CString::new(right.as_os_str().as_bytes())?;
    let result = unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            left.as_ptr(),
            libc::AT_FDCWD,
            right.as_ptr(),
            libc::RENAME_SWAP,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn exchange_paths(left: &Path, right: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let left = std::ffi::CString::new(left.as_os_str().as_bytes())?;
    let right = std::ffi::CString::new(right.as_os_str().as_bytes())?;
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            left.as_ptr(),
            libc::AT_FDCWD,
            right.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn exchange_paths(left: &Path, right: &Path) -> std::io::Result<()> {
    let backup = right.with_extension(format!("backup-{}", crate::store::new_uuid()));
    std::fs::rename(right, &backup)?;
    if let Err(error) = std::fs::rename(left, right) {
        let _ = std::fs::rename(&backup, right);
        return Err(error);
    }
    std::fs::rename(&backup, left)
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent(_: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::{BuildMode, RuntimePathPolicy};

    fn paths(root: &Path) -> ExtensionPaths {
        ExtensionPaths::new(RuntimePathPolicy::new(BuildMode::Production), root)
    }

    fn package(root: &Path, version: &str) {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(
            root.join("package.json"),
            format!(r#"{{"name":"git","version":"{version}","muxy":{{}}}}"#),
        )
        .unwrap();
    }

    #[test]
    fn invalid_updates_never_mutate_the_active_package() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let first = directory.path().join("first");
        let invalid = directory.path().join("invalid");
        package(&first, "1.0.0");
        std::fs::create_dir(&invalid).unwrap();
        std::fs::write(invalid.join("package.json"), b"not json").unwrap();
        let publisher = ExtensionPackagePublisher::new(paths.clone());
        publisher.publish(&first, false).unwrap();
        assert!(publisher.publish(&invalid, true).is_err());
        let active = load_extension(paths.packages.join("git")).unwrap();
        assert_eq!(active.manifest.version, "1.0.0");
    }

    #[test]
    fn valid_updates_replace_the_package_through_staging() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let first = directory.path().join("first");
        let second = directory.path().join("second");
        package(&first, "1.0.0");
        package(&second, "2.0.0");
        let publisher = ExtensionPackagePublisher::new(paths.clone());
        publisher.publish(&first, false).unwrap();
        publisher.publish(&second, true).unwrap();
        let active = load_extension(paths.packages.join("git")).unwrap();
        assert_eq!(active.manifest.version, "2.0.0");
        assert!(std::fs::read_dir(paths.staging).unwrap().next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn escaping_symlinks_are_rejected_before_publication() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let source = directory.path().join("source");
        package(&source, "1.0.0");
        let outside = directory.path().join("outside.js");
        std::fs::write(&outside, b"outside").unwrap();
        symlink(&outside, source.join("linked.js")).unwrap();
        let error = ExtensionPackagePublisher::new(paths)
            .publish(&source, false)
            .unwrap_err();
        assert!(matches!(error, ExtensionPackageError::UnsafeLink(_)));
    }
}
