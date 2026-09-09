use std::fs;
use std::io::{self, IoSlice, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use crate::{BindError, ByteStream, Listener, StreamCancellation};

#[derive(Debug)]
pub struct UnixSocketListener {
    socket: Mutex<Option<UnixListener>>,
    closed: Condvar,
}

impl UnixSocketListener {
    pub fn bind(path: impl AsRef<Path>) -> Result<Self, BindError> {
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let directory = fs::File::open(parent)?;
        directory.lock()?;

        let socket = match UnixListener::bind(path) {
            Ok(socket) => socket,
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                match UnixStream::connect(path) {
                    Ok(_) => return Err(BindError::InUse),
                    Err(probe) if probe.kind() == io::ErrorKind::ConnectionRefused => {
                        if !fs::symlink_metadata(path)?.file_type().is_socket() {
                            return Err(error.into());
                        }
                        fs::remove_file(path)?;
                        UnixListener::bind(path)?
                    }
                    Err(probe) => return Err(probe.into()),
                }
            }
            Err(error) => return Err(error.into()),
        };
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket: Mutex::new(Some(socket)),
            closed: Condvar::new(),
        })
    }
}

impl Listener for UnixSocketListener {
    fn accept(&self) -> io::Result<Box<dyn ByteStream>> {
        let mut guard = self
            .socket
            .lock()
            .map_err(|_| io::Error::other("listener lock poisoned"))?;
        loop {
            let socket = guard
                .as_ref()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "listener is closed"))?;
            match socket.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false)?;
                    return Ok(Box::new(stream));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    (guard, _) = self
                        .closed
                        .wait_timeout(guard, Duration::from_millis(10))
                        .map_err(|_| io::Error::other("listener lock poisoned"))?;
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn close(&self) {
        let mut guard = self
            .socket
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.take();
        self.closed.notify_all();
    }
}

pub fn connect(path: impl AsRef<Path>) -> io::Result<Box<dyn ByteStream>> {
    let stream = UnixStream::connect(path)?;
    stream.set_nonblocking(false)?;
    Ok(Box::new(stream))
}

impl ByteStream for UnixStream {
    fn cancellation(&self) -> io::Result<Box<dyn StreamCancellation>> {
        Ok(Box::new(SocketCancellation {
            stream: self.try_clone()?,
        }))
    }

    #[allow(clippy::type_complexity)]
    fn split(self: Box<Self>) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
        let writer = SocketWriter {
            stream: self.try_clone()?,
        };
        Ok((self, Box::new(writer)))
    }
}

#[derive(Debug)]
struct SocketCancellation {
    stream: UnixStream,
}

impl StreamCancellation for SocketCancellation {
    fn cancel(&self) {
        let _ = self.stream.shutdown(Shutdown::Write);
        let _ = self.stream.shutdown(Shutdown::Read);
    }
}

#[derive(Debug)]
struct SocketWriter {
    stream: UnixStream,
}

impl Write for SocketWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.stream.write(bytes)
    }

    fn write_vectored(&mut self, buffers: &[IoSlice<'_>]) -> io::Result<usize> {
        self.stream.write_vectored(buffers)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

impl Drop for SocketWriter {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Write);
    }
}
