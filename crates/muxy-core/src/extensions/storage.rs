use super::paths::ExtensionPaths;
use crate::store::write_private;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use thiserror::Error;
use unicode_segmentation::UnicodeSegmentation;

pub const MAX_STORAGE_KEY_CHARACTERS: usize = 256;
pub const MAX_STORAGE_VALUE_BYTES: usize = 1_048_576;
pub const MAX_STORAGE_BYTES: usize = 5_242_880;

#[derive(Debug, Error)]
pub enum ExtensionStorageError {
    #[error("storage key must not be empty")]
    EmptyKey,
    #[error("storage key exceeds {MAX_STORAGE_KEY_CHARACTERS} characters")]
    KeyTooLong,
    #[error("storage value exceeds {MAX_STORAGE_VALUE_BYTES} bytes")]
    ValueTooLarge,
    #[error("storage for this extension exceeds {MAX_STORAGE_BYTES} bytes")]
    StoreTooLarge,
    #[error("failed to read extension storage: {0}")]
    Read(#[source] std::io::Error),
    #[error("failed to encode extension storage: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("failed to write extension storage: {0}")]
    Write(#[source] std::io::Error),
}

#[derive(Clone, Debug)]
pub struct ExtensionStorage {
    directory: PathBuf,
}

impl ExtensionStorage {
    pub fn new(paths: &ExtensionPaths) -> Self {
        Self {
            directory: paths.storage_directory(),
        }
    }

    pub fn at(directory: impl AsRef<Path>) -> Self {
        Self {
            directory: directory.as_ref().to_path_buf(),
        }
    }

    pub fn get(&self, extension_id: &str, key: &str) -> Result<Value, ExtensionStorageError> {
        validate_key(key)?;
        Ok(self.load(extension_id)?.remove(key).unwrap_or(Value::Null))
    }

    pub fn set(
        &self,
        extension_id: &str,
        key: &str,
        value: Value,
    ) -> Result<(), ExtensionStorageError> {
        validate_key(key)?;
        let value_size = serde_json::to_vec(&value)
            .map_err(ExtensionStorageError::Encode)?
            .len();
        if value_size > MAX_STORAGE_VALUE_BYTES {
            return Err(ExtensionStorageError::ValueTooLarge);
        }
        let mut store = self.load(extension_id)?;
        store.insert(key.to_owned(), value);
        self.save(extension_id, &store)
    }

    pub fn delete(&self, extension_id: &str, key: &str) -> Result<(), ExtensionStorageError> {
        validate_key(key)?;
        let mut store = self.load(extension_id)?;
        if store.remove(key).is_some() {
            self.save(extension_id, &store)?;
        }
        Ok(())
    }

    pub fn keys(&self, extension_id: &str) -> Result<Vec<String>, ExtensionStorageError> {
        Ok(self.load(extension_id)?.into_keys().collect())
    }

    pub fn path(&self, extension_id: &str) -> PathBuf {
        self.directory.join(format!(
            "{}.json",
            super::paths::safe_extension_filename(extension_id)
        ))
    }

    fn load(&self, extension_id: &str) -> Result<BTreeMap<String, Value>, ExtensionStorageError> {
        let contents = match std::fs::read(self.path(extension_id)) {
            Ok(contents) if !contents.is_empty() => contents,
            Ok(_) => return Ok(BTreeMap::new()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BTreeMap::new());
            }
            Err(error) => return Err(ExtensionStorageError::Read(error)),
        };
        Ok(serde_json::from_slice::<BTreeMap<String, Value>>(&contents).unwrap_or_default())
    }

    fn save(
        &self,
        extension_id: &str,
        store: &BTreeMap<String, Value>,
    ) -> Result<(), ExtensionStorageError> {
        let contents = serde_json::to_vec(store).map_err(ExtensionStorageError::Encode)?;
        if contents.len() > MAX_STORAGE_BYTES {
            return Err(ExtensionStorageError::StoreTooLarge);
        }
        write_private(&self.path(extension_id), &contents).map_err(ExtensionStorageError::Write)
    }
}

fn validate_key(key: &str) -> Result<(), ExtensionStorageError> {
    if key.is_empty() {
        return Err(ExtensionStorageError::EmptyKey);
    }
    if key.graphemes(true).count() > MAX_STORAGE_KEY_CHARACTERS {
        return Err(ExtensionStorageError::KeyTooLong);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn storage_round_trips_values_and_sorts_keys() {
        let directory = tempfile::tempdir().unwrap();
        let storage = ExtensionStorage::at(directory.path());
        storage.set("git", "zeta", json!([1, true])).unwrap();
        storage
            .set("git", "alpha", json!({ "branch": "main" }))
            .unwrap();
        assert_eq!(
            storage.keys("git").unwrap(),
            ["alpha".to_owned(), "zeta".to_owned()]
        );
        assert_eq!(storage.get("git", "missing").unwrap(), Value::Null);
        storage.delete("git", "zeta").unwrap();
        assert_eq!(storage.keys("git").unwrap(), ["alpha".to_owned()]);
    }

    #[test]
    fn storage_limits_match_swift_and_do_not_mutate_on_failure() {
        let directory = tempfile::tempdir().unwrap();
        let storage = ExtensionStorage::at(directory.path());
        assert!(matches!(
            storage.set("git", "", Value::Null),
            Err(ExtensionStorageError::EmptyKey)
        ));
        let grapheme = "e\u{301}";
        storage
            .set("git", &grapheme.repeat(256), json!("ok"))
            .unwrap();
        assert!(matches!(
            storage.set("git", &grapheme.repeat(257), Value::Null),
            Err(ExtensionStorageError::KeyTooLong)
        ));
        let oversized = Value::String("x".repeat(MAX_STORAGE_VALUE_BYTES));
        assert!(matches!(
            storage.set("git", "large", oversized),
            Err(ExtensionStorageError::ValueTooLarge)
        ));
        assert_eq!(storage.get("git", "large").unwrap(), Value::Null);
    }

    #[test]
    fn malformed_legacy_storage_matches_swift_empty_store_behavior() {
        let directory = tempfile::tempdir().unwrap();
        let storage = ExtensionStorage::at(directory.path());
        std::fs::write(storage.path("git"), b"not json").unwrap();
        assert!(storage.keys("git").unwrap().is_empty());
    }
}
