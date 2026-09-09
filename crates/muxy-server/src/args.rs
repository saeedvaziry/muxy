use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

#[derive(Debug)]
pub(crate) struct Args {
    pub(crate) socket: PathBuf,
    pub(crate) settings: PathBuf,
    pub(crate) log: PathBuf,
}

impl Args {
    pub(crate) fn parse(arguments: impl IntoIterator<Item = OsString>) -> io::Result<Self> {
        let mut socket = None;
        let mut settings = None;
        let mut log = None;
        let mut arguments = arguments.into_iter();
        while let Some(flag) = arguments.next() {
            let destination = match flag.to_str() {
                Some("--socket") => &mut socket,
                Some("--settings") => &mut settings,
                Some("--log") => &mut log,
                _ => return Err(invalid(format!("unknown argument: {}", flag.display()))),
            };
            if destination.is_some() {
                return Err(invalid(format!("duplicate argument: {}", flag.display())));
            }
            let value = arguments
                .next()
                .filter(|value| !value.is_empty() && !value.as_encoded_bytes().starts_with(b"--"))
                .ok_or_else(|| invalid(format!("{} requires a path", flag.display())))?;
            *destination = Some(PathBuf::from(value));
        }
        let directory = if socket.is_none() || settings.is_none() || log.is_none() {
            muxy_core::dirs::muxy_dir()?
        } else {
            PathBuf::new()
        };
        Ok(Self {
            socket: socket.unwrap_or_else(|| directory.join("server.sock")),
            settings: settings.unwrap_or_else(|| directory.join("server.toml")),
            log: log.unwrap_or_else(|| directory.join("server.log")),
        })
    }
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
