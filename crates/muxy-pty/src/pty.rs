use std::ffi::OsString;
use std::fmt;
use std::io::Write;
use std::os::unix::io::RawFd;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::Child;
use std::sync::mpsc::Sender;

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtyPair, native_pty_system};

use crate::error::{PtyError, PtyStep};
use crate::reader::{PtyEvent, ReaderHandle, spawn_reader};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PtySize {
    pub cols: u16,
    pub rows: u16,
}

impl From<PtySize> for portable_pty::PtySize {
    fn from(size: PtySize) -> Self {
        Self {
            rows: size.rows,
            cols: size.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpawnRequest {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub size: PtySize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExitStatus {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

impl From<std::process::ExitStatus> for ExitStatus {
    fn from(status: std::process::ExitStatus) -> Self {
        Self {
            code: status.code(),
            signal: status.signal(),
        }
    }
}

pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    writer: Option<Box<dyn Write + Send>>,
    child: Child,
    child_pid: u32,
    master_fd: RawFd,
}

impl Pty {
    pub fn spawn(request: SpawnRequest) -> Result<Self, PtyError> {
        let PtyPair { master, slave } = native_pty_system()
            .openpty(request.size.into())
            .map_err(|error| PtyError::wrap(PtyStep::Open, error))?;
        let master_fd = master
            .as_raw_fd()
            .ok_or_else(|| PtyError::wrap(PtyStep::Open, "master side has no file descriptor"))?;

        let mut command = CommandBuilder::new(request.program);
        command.args(request.args);
        command.cwd(request.cwd);
        for (key, value) in request.env {
            command.env(key, value);
        }
        let child: Box<dyn portable_pty::Child> = slave
            .spawn_command(command)
            .map_err(|error| PtyError::wrap(PtyStep::Spawn, error))?;
        drop(slave);
        let child = child
            .downcast::<Child>()
            .map_err(|_| PtyError::wrap(PtyStep::Spawn, "child is not a process handle"))?;
        let child_pid = child.id();

        let writer = master
            .take_writer()
            .map_err(|error| PtyError::wrap(PtyStep::TakeWriter, error))?;

        Ok(Self {
            master,
            writer: Some(writer),
            child: *child,
            child_pid,
            master_fd,
        })
    }

    pub fn start_reader(&self, sink: Sender<PtyEvent>) -> Result<ReaderHandle, PtyError> {
        let reader = self
            .master
            .try_clone_reader()
            .map_err(|error| PtyError::wrap(PtyStep::CloneReader, error))?;
        Ok(spawn_reader(reader, sink))
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        self.writer
            .as_mut()
            .ok_or_else(|| PtyError::wrap(PtyStep::Write, "PTY writer has been taken"))?
            .write_all(bytes)
            .map_err(|error| PtyError::new(PtyStep::Write, error))
    }

    pub fn take_writer(&mut self) -> Result<Box<dyn Write + Send>, PtyError> {
        self.writer
            .take()
            .ok_or_else(|| PtyError::wrap(PtyStep::TakeWriter, "PTY writer has been taken"))
    }

    pub fn resize(&self, size: PtySize) -> Result<(), PtyError> {
        self.master
            .resize(size.into())
            .map_err(|error| PtyError::wrap(PtyStep::Resize, error))
    }

    pub fn try_wait(&mut self) -> Option<ExitStatus> {
        self.child.try_wait().ok().flatten().map(ExitStatus::from)
    }

    pub fn wait(&mut self) -> Result<ExitStatus, PtyError> {
        self.child
            .wait()
            .map(ExitStatus::from)
            .map_err(|error| PtyError::new(PtyStep::Wait, error))
    }

    pub fn kill(&mut self) -> Result<(), PtyError> {
        ChildKiller::kill(&mut self.child).map_err(|error| PtyError::new(PtyStep::Kill, error))
    }

    pub fn child_pid(&self) -> u32 {
        self.child_pid
    }

    pub fn foreground_pid(&self) -> Option<u32> {
        self.master
            .process_group_leader()
            .and_then(|pid| u32::try_from(pid).ok())
    }

    pub fn master_fd(&self) -> RawFd {
        self.master_fd
    }
}

impl fmt::Debug for Pty {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Pty")
            .field("child_pid", &self.child_pid)
            .field("master_fd", &self.master_fd)
            .finish_non_exhaustive()
    }
}
