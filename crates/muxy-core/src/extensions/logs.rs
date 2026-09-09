use super::paths::ExtensionPaths;
use crate::store::write_private;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const MAX_EXTENSION_LOG_BYTES: usize = 5 * 1024 * 1024;
pub const TRIM_EXTENSION_LOG_TO_BYTES: usize = 1_250_000;
pub const MAX_EXTENSION_AUDIT_BYTES: usize = 1024 * 1024;
pub const TRIM_EXTENSION_AUDIT_TO_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionAuditEntry {
    pub timestamp: String,
    #[serde(rename = "extensionID")]
    pub extension_id: String,
    pub verb: String,
    pub payload_summary: String,
    pub decision: String,
    pub rule_id: Option<String>,
    pub source: String,
}

#[derive(Debug, Error)]
pub enum ExtensionLogError {
    #[error("failed to encode extension audit entry: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("failed to access extension log: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug)]
pub struct ExtensionLogStore {
    directory: PathBuf,
}

impl ExtensionLogStore {
    pub fn new(paths: &ExtensionPaths) -> Self {
        Self {
            directory: paths.logs_directory(),
        }
    }

    pub fn at(directory: impl AsRef<Path>) -> Self {
        Self {
            directory: directory.as_ref().to_path_buf(),
        }
    }

    pub fn path(&self, extension_id: &str) -> PathBuf {
        self.directory.join(format!(
            "{}.log",
            super::paths::safe_extension_filename(extension_id)
        ))
    }

    pub fn append(&self, extension_id: &str, line: &str) -> Result<(), ExtensionLogError> {
        let mut payload = line.as_bytes().to_vec();
        if !payload.ends_with(b"\n") {
            payload.push(b'\n');
        }
        append_bounded(
            &self.path(extension_id),
            &payload,
            MAX_EXTENSION_LOG_BYTES,
            TRIM_EXTENSION_LOG_TO_BYTES,
        )?;
        Ok(())
    }

    pub fn clear(&self, extension_id: &str) -> Result<(), ExtensionLogError> {
        write_private(&self.path(extension_id), b"")?;
        Ok(())
    }

    pub fn read(&self, extension_id: &str) -> Result<Vec<u8>, ExtensionLogError> {
        match std::fs::read(self.path(extension_id)) {
            Ok(contents) => Ok(contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExtensionAuditLog {
    path: PathBuf,
}

impl ExtensionAuditLog {
    pub fn new(paths: &ExtensionPaths) -> Self {
        Self {
            path: paths.audit_file(),
        }
    }

    pub fn at(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn append(&self, entry: &ExtensionAuditEntry) -> Result<(), ExtensionLogError> {
        let mut payload = serde_json::to_vec(entry).map_err(ExtensionLogError::Encode)?;
        payload.push(b'\n');
        append_bounded(
            &self.path,
            &payload,
            MAX_EXTENSION_AUDIT_BYTES,
            TRIM_EXTENSION_AUDIT_TO_BYTES,
        )?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn append_bounded(
    path: &Path,
    payload: &[u8],
    maximum: usize,
    trim_to: usize,
) -> std::io::Result<()> {
    let current_size = std::fs::metadata(path)
        .map(|metadata| metadata.len() as usize)
        .unwrap_or_default();
    if current_size.saturating_add(payload.len()) <= maximum {
        ensure_private_file(path)?;
        let mut file = append_file(path)?;
        file.write_all(payload)?;
        file.sync_data()?;
        return Ok(());
    }
    let mut contents = std::fs::read(path).unwrap_or_default();
    contents.extend_from_slice(payload);
    let start = contents.len().saturating_sub(trim_to);
    let start = contents[start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|offset| start + offset + 1)
        .unwrap_or(contents.len());
    write_private(path, &contents[start..])
}

fn ensure_private_file(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        write_private(path, b"")?;
    }
    Ok(())
}

#[cfg(unix)]
fn append_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    std::fs::OpenOptions::new()
        .append(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn append_file(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().append(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_logs_append_newlines_and_clear() {
        let directory = tempfile::tempdir().unwrap();
        let logs = ExtensionLogStore::at(directory.path());
        logs.append("git", "first").unwrap();
        logs.append("git", "second\n").unwrap();
        assert_eq!(logs.read("git").unwrap(), b"first\nsecond\n");
        logs.clear("git").unwrap();
        assert!(logs.read("git").unwrap().is_empty());
    }

    #[test]
    fn bounded_logs_drop_partial_old_lines() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bounded.log");
        write_private(&path, b"old-a\nold-b\n").unwrap();
        append_bounded(&path, b"new-line\n", 16, 10).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"new-line\n");
    }

    #[test]
    fn audit_lines_keep_the_swift_json_shape() {
        let directory = tempfile::tempdir().unwrap();
        let audit = ExtensionAuditLog::at(directory.path().join("audit.log"));
        let entry = ExtensionAuditEntry {
            timestamp: "2026-09-02T12:00:00Z".to_owned(),
            extension_id: "git".to_owned(),
            verb: "git.write".to_owned(),
            payload_summary: "commit".to_owned(),
            decision: "allow".to_owned(),
            rule_id: None,
            source: "webview".to_owned(),
        };
        audit.append(&entry).unwrap();
        let contents = std::fs::read_to_string(audit.path()).unwrap();
        assert!(contents.ends_with('\n'));
        assert_eq!(
            serde_json::from_str::<ExtensionAuditEntry>(contents.trim()).unwrap(),
            entry
        );
    }
}
