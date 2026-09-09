use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::{AppError, AppState};

pub fn default_path() -> Result<PathBuf, AppError> {
    muxy_core::dirs::muxy_dir()
        .map(|directory| directory.join("state.json"))
        .map_err(|source| AppError::Io {
            path: PathBuf::from("Muxy state directory"),
            source,
        })
}

pub fn load(path: impl AsRef<Path>) -> Result<AppState, AppError> {
    let path = path.as_ref();
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return AppState::bootstrap(),
        Err(source) => {
            return Err(AppError::Io {
                path: path.into(),
                source,
            });
        }
    };
    serde_json::from_slice(&bytes).map_err(|source| AppError::Json {
        path: path.into(),
        source,
    })
}

pub fn save(path: impl AsRef<Path>, state: &AppState) -> Result<(), AppError> {
    let path = path.as_ref();
    let mut bytes = serde_json::to_vec_pretty(state).map_err(|source| AppError::Json {
        path: path.into(),
        source,
    })?;
    bytes.push(b'\n');
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let mut file = open_temporary(&temporary).map_err(|source| AppError::Io {
        path: temporary.clone(),
        source,
    })?;
    let result = file
        .set_len(0)
        .and_then(|()| file.write_all(&bytes))
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::rename(&temporary, path));
    if let Err(source) = result {
        let _ = fs::remove_file(&temporary);
        return Err(AppError::Io {
            path: path.into(),
            source,
        });
    }
    Ok(())
}

fn open_temporary(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    file.try_lock()?;
    let opened = file.metadata()?;
    let current = fs::metadata(path)?;
    if (opened.dev(), opened.ino()) != (current.dev(), current.ino()) {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "temporary state file was replaced by another writer",
        ));
    }
    Ok(file)
}
