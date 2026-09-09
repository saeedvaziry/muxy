use muxy_core::extensions::logs::ExtensionLogStore;
use muxy_core::extensions::runtime::ExtensionHostLaunch;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use thiserror::Error;

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(25);
const PROCESS_TERMINATION_GRACE_PERIOD: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HostProcessExit {
    pub extension_id: String,
    pub generation: u64,
    pub status: i32,
}

#[derive(Debug, Error)]
pub(crate) enum HostProcessError {
    #[cfg(not(target_os = "macos"))]
    #[error("extension background hosts are unsupported on this platform")]
    UnsupportedPlatform,
    #[error("extension host binary not found at {0}")]
    HostNotFound(PathBuf),
    #[error("failed to resolve the extension host binary: {0}")]
    ResolveHost(#[source] std::io::Error),
    #[error("failed to start the extension host: {0}")]
    Spawn(#[source] std::io::Error),
}

impl HostProcessError {
    #[cfg(target_os = "macos")]
    pub const fn is_unsupported(&self) -> bool {
        false
    }

    #[cfg(not(target_os = "macos"))]
    pub const fn is_unsupported(&self) -> bool {
        matches!(self, Self::UnsupportedPlatform)
    }
}

pub(crate) trait RunningHost {
    fn stop(&mut self);
}

pub(crate) trait HostLauncher {
    fn launch(
        &self,
        request: &ExtensionHostLaunch,
        socket_path: &Path,
        generation: u64,
        events: async_channel::Sender<HostProcessExit>,
        logs: ExtensionLogStore,
    ) -> Result<Box<dyn RunningHost>, HostProcessError>;
}

pub(crate) struct SystemHostLauncher {
    executable: Result<PathBuf, HostProcessError>,
}

impl SystemHostLauncher {
    pub fn resolve() -> Self {
        Self {
            executable: resolve_host_executable(),
        }
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn run_script(
        script_path: &Path,
        working_directory: &Path,
        socket_path: &Path,
        extension_id: &str,
        token: &str,
    ) -> Result<(), HostProcessError> {
        let executable = resolve_host_executable()?;
        if !executable.is_file() {
            return Err(HostProcessError::HostNotFound(executable));
        }
        Command::new(executable)
            .arg(script_path)
            .current_dir(working_directory)
            .env("MUXY_SOCKET_PATH", socket_path)
            .env("MUXY_EXTENSION_ID", extension_id)
            .env("MUXY_EXTENSION_TOKEN", token)
            .env("MUXY_EXTENSION_ONESHOT", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(HostProcessError::Spawn)
    }

    #[cfg(not(target_os = "macos"))]
    pub(crate) fn run_script(
        _script_path: &Path,
        _working_directory: &Path,
        _socket_path: &Path,
        _extension_id: &str,
        _token: &str,
    ) -> Result<(), HostProcessError> {
        Err(HostProcessError::UnsupportedPlatform)
    }
}

#[cfg(target_os = "macos")]
impl HostLauncher for SystemHostLauncher {
    fn launch(
        &self,
        request: &ExtensionHostLaunch,
        socket_path: &Path,
        generation: u64,
        events: async_channel::Sender<HostProcessExit>,
        logs: ExtensionLogStore,
    ) -> Result<Box<dyn RunningHost>, HostProcessError> {
        use std::os::unix::process::CommandExt;

        let executable = self.executable.as_ref().map_err(clone_host_error)?;
        if !executable.is_file() {
            return Err(HostProcessError::HostNotFound(executable.clone()));
        }
        let mut command = Command::new(executable);
        command
            .arg(&request.background_script)
            .current_dir(&request.working_directory)
            .env("MUXY_SOCKET_PATH", socket_path)
            .env("MUXY_EXTENSION_ID", &request.extension_id)
            .env("MUXY_EXTENSION_TOKEN", &request.token)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.process_group(0);
        let mut child = command.spawn().map_err(HostProcessError::Spawn)?;
        if let Some(stdout) = child.stdout.take() {
            forward_output(request.extension_id.clone(), stdout, logs.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            forward_output(request.extension_id.clone(), stderr, logs);
        }
        Ok(Box::new(SystemRunningHost::new(
            request.extension_id.clone(),
            generation,
            child,
            events,
        )))
    }
}

#[cfg(not(target_os = "macos"))]
impl HostLauncher for SystemHostLauncher {
    fn launch(
        &self,
        _request: &ExtensionHostLaunch,
        _socket_path: &Path,
        _generation: u64,
        _events: async_channel::Sender<HostProcessExit>,
        _logs: ExtensionLogStore,
    ) -> Result<Box<dyn RunningHost>, HostProcessError> {
        Err(HostProcessError::UnsupportedPlatform)
    }
}

fn resolve_host_executable() -> Result<PathBuf, HostProcessError> {
    std::env::current_exe()
        .map(|path| path.with_file_name("muxy-extension-host"))
        .map_err(HostProcessError::ResolveHost)
}

fn clone_host_error(error: &HostProcessError) -> HostProcessError {
    match error {
        #[cfg(not(target_os = "macos"))]
        HostProcessError::UnsupportedPlatform => HostProcessError::UnsupportedPlatform,
        HostProcessError::HostNotFound(path) => HostProcessError::HostNotFound(path.clone()),
        HostProcessError::ResolveHost(error) => {
            HostProcessError::ResolveHost(std::io::Error::new(error.kind(), error.to_string()))
        }
        HostProcessError::Spawn(error) => {
            HostProcessError::Spawn(std::io::Error::new(error.kind(), error.to_string()))
        }
    }
}

fn forward_output(
    extension_id: String,
    output: impl std::io::Read + Send + 'static,
    logs: ExtensionLogStore,
) {
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(output)
            .lines()
            .map_while(Result::ok)
        {
            let _ = logs.append(&extension_id, &line);
        }
    });
}

struct SystemRunningHost {
    stop: Option<mpsc::Sender<()>>,
}

impl SystemRunningHost {
    fn new(
        extension_id: String,
        generation: u64,
        mut child: Child,
        events: async_channel::Sender<HostProcessExit>,
    ) -> Self {
        let (stop, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let status = loop {
                if receiver.try_recv().is_ok() {
                    break terminate_process(&mut child);
                }
                match child.try_wait() {
                    Ok(Some(status)) => break status,
                    Ok(None) => std::thread::sleep(PROCESS_POLL_INTERVAL),
                    Err(_) => break child.wait().unwrap_or_else(failed_exit_status),
                }
            };
            let _ = events.send_blocking(HostProcessExit {
                extension_id,
                generation,
                status: exit_code(status),
            });
        });
        Self { stop: Some(stop) }
    }
}

impl RunningHost for SystemRunningHost {
    fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

impl Drop for SystemRunningHost {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(unix)]
fn terminate_process(child: &mut Child) -> ExitStatus {
    let process_group = -(child.id() as libc::pid_t);
    unsafe {
        libc::kill(process_group, libc::SIGTERM);
    }
    let deadline = Instant::now() + PROCESS_TERMINATION_GRACE_PERIOD;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(status)) => return status,
            Ok(None) => std::thread::sleep(PROCESS_POLL_INTERVAL),
            Err(_) => break,
        }
    }
    unsafe {
        libc::kill(process_group, libc::SIGKILL);
    }
    child.wait().unwrap_or_else(failed_exit_status)
}

#[cfg(not(unix))]
fn terminate_process(child: &mut Child) -> ExitStatus {
    let _ = child.kill();
    child.wait().unwrap_or_else(failed_exit_status)
}

#[cfg(unix)]
fn failed_exit_status(_: std::io::Error) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw(1 << 8)
}

#[cfg(windows)]
fn failed_exit_status(_: std::io::Error) -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    ExitStatus::from_raw(1)
}

fn exit_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}
