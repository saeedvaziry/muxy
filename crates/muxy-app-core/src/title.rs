use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use muxy_protocol::{ForegroundProcess, ServerPath};

pub fn derive(title: &str, process: Option<&ForegroundProcess>, directory: &ServerPath) -> String {
    if !title.trim().is_empty() {
        return title.to_owned();
    }
    if let Some(process) = process
        && !process.is_shell
        && !process.name.trim().is_empty()
    {
        return process.name.clone();
    }
    let directory = Path::new(OsStr::from_bytes(&directory.0));
    let basename = directory.file_name().unwrap_or(directory.as_os_str());
    if basename.is_empty() {
        "Terminal".into()
    } else {
        basename.to_string_lossy().into_owned()
    }
}
