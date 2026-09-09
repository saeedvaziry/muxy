use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use muxy_client::{Client, ClientError};

pub(crate) fn ensure_server_running(socket: &Path) -> Result<Client, ClientError> {
    match Client::connect(socket) {
        Ok(client) => return Ok(client),
        Err(error) if unavailable(&error) => {}
        Err(error) => return Err(error),
    }

    let executable = server_executable()?;
    let mut child = Command::new(&executable)
        .arg("--socket")
        .arg(socket)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "could not launch {}: {error}; build muxy-server or set MUXY_SERVER_BIN",
                    executable.display()
                ),
            )
        })?;
    thread::spawn(move || {
        let _ = child.wait();
    });

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match connect_before(socket, deadline) {
            Ok(client) => return Ok(client),
            Err(error) if unavailable(&error) => {}
            Err(error) => return Err(error),
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "server did not start at {} within 3 seconds",
                    socket.display()
                ),
            )
            .into());
        }
        thread::sleep(remaining.min(Duration::from_millis(25)));
    }
}

fn connect_before(socket: &Path, deadline: Instant) -> Result<Client, ClientError> {
    let socket = socket.to_owned();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    thread::Builder::new()
        .name("muxy-server-connect".into())
        .spawn(move || {
            let _ = sender.send(Client::connect(&socket));
        })?;
    receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|error| match error {
            std::sync::mpsc::RecvTimeoutError::Timeout => ClientError::Timeout,
            std::sync::mpsc::RecvTimeoutError::Disconnected => ClientError::Disconnected,
        })?
}

fn server_executable() -> io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("MUXY_SERVER_BIN") {
        if path.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MUXY_SERVER_BIN must not be empty",
            ));
        }
        return Ok(path.into());
    }
    Ok(std::env::current_exe()?.with_file_name("muxy-server"))
}

fn unavailable(error: &ClientError) -> bool {
    matches!(error, ClientError::Io(error) if matches!(error.kind(), io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_missing_or_refused_socket_can_start_a_server() {
        for kind in [io::ErrorKind::NotFound, io::ErrorKind::ConnectionRefused] {
            assert!(unavailable(&ClientError::Io(kind.into())));
        }
        for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::TimedOut] {
            assert!(!unavailable(&ClientError::Io(kind.into())));
        }
        assert!(!unavailable(&ClientError::VersionUnsupported));
        assert!(!unavailable(&ClientError::Disconnected));
    }
}
