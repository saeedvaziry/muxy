use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

pub fn muxy_dir() -> io::Result<PathBuf> {
    let directory = match env::var_os("MUXY_DIR") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        Some(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MUXY_DIR must not be empty",
            ));
        }
        None => env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(|home| PathBuf::from(home).join("Library/Application Support/Muxy"))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?,
    };
    fs::create_dir_all(&directory)?;
    Ok(directory)
}
