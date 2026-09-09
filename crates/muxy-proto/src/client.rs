use crate::extension::{
    ExtensionBroadcast, ExtensionLocalEvent, InvokeRequest, ModalQuery, ModalResult,
};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use thiserror::Error;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::framing::{InputAccumulator, MAX_READ_BYTES, frame_persistent_line};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::io::{Read, Write};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::net::Shutdown;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::os::unix::net::UnixStream;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::thread;

pub const HOST_CONNECT_ATTEMPTS: usize = 15;
pub const HOST_CONNECT_RETRY_DELAY: Duration = Duration::from_millis(100);
pub const HOST_IDENTIFY_ATTEMPTS: usize = 15;
pub const HOST_IDENTIFY_RETRY_DELAY: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostIncoming {
    Broadcast(ExtensionBroadcast),
    ExtensionEvent(ExtensionLocalEvent),
    Invoke(InvokeRequest),
    ModalResult(ModalResult),
    ModalQuery(ModalQuery),
}

#[derive(Debug, Error)]
pub enum ExtensionHostClientError {
    #[error("extension host sockets are unsupported on this platform")]
    UnsupportedPlatform,
    #[error("could not connect to {path}: {source}")]
    Connect {
        path: String,
        source: std::io::Error,
    },
    #[error("could not clone the extension host socket: {0}")]
    Clone(#[source] std::io::Error),
    #[error("extension host socket is closed")]
    Closed,
    #[error("could not write to the extension host socket: {0}")]
    Write(#[source] std::io::Error),
    #[error("extension host request coordination failed")]
    RequestCoordination,
    #[error("extension identification was rejected: {0}")]
    IdentifyRejected(String),
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub struct ExtensionHostClient {
    writer: Mutex<UnixStream>,
    request: Mutex<()>,
    replies: Mutex<mpsc::Receiver<String>>,
    incoming: Mutex<mpsc::Receiver<HostIncoming>>,
    closed: Arc<AtomicBool>,
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub struct ExtensionHostClient;

impl ExtensionHostClient {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub fn connect_and_identify(
        socket_path: &Path,
        extension_id: &str,
        token: &str,
    ) -> Result<Self, ExtensionHostClientError> {
        let stream = connect_with_retries(socket_path)?;
        let reader = stream
            .try_clone()
            .map_err(ExtensionHostClientError::Clone)?;
        let (reply_sender, reply_receiver) = mpsc::channel();
        let (incoming_sender, incoming_receiver) = mpsc::channel();
        let closed = Arc::new(AtomicBool::new(false));
        spawn_reader(reader, reply_sender, incoming_sender, Arc::clone(&closed));
        let client = Self {
            writer: Mutex::new(stream),
            request: Mutex::new(()),
            replies: Mutex::new(reply_receiver),
            incoming: Mutex::new(incoming_receiver),
            closed,
        };
        let identify = format!("identify|{extension_id}|{token}");
        for attempt in 0..HOST_IDENTIFY_ATTEMPTS {
            let reply = client.send_and_wait_reply(&identify)?;
            if reply == "ok" {
                return Ok(client);
            }
            if !reply.starts_with("error:unknown extension")
                || attempt + 1 == HOST_IDENTIFY_ATTEMPTS
            {
                return Err(ExtensionHostClientError::IdentifyRejected(reply));
            }
            thread::sleep(HOST_IDENTIFY_RETRY_DELAY);
        }
        Err(ExtensionHostClientError::IdentifyRejected(
            "unknown extension".to_owned(),
        ))
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn connect_and_identify(
        _socket_path: &Path,
        _extension_id: &str,
        _token: &str,
    ) -> Result<Self, ExtensionHostClientError> {
        Err(ExtensionHostClientError::UnsupportedPlatform)
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub fn send(&self, line: &str) -> Result<(), ExtensionHostClientError> {
        if self.is_closed() {
            return Err(ExtensionHostClientError::Closed);
        }
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| ExtensionHostClientError::RequestCoordination)?;
        writer
            .write_all(&frame_persistent_line(line))
            .map_err(ExtensionHostClientError::Write)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn send(&self, _line: &str) -> Result<(), ExtensionHostClientError> {
        Err(ExtensionHostClientError::UnsupportedPlatform)
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub fn send_and_wait_reply(&self, line: &str) -> Result<String, ExtensionHostClientError> {
        let _request = self
            .request
            .lock()
            .map_err(|_| ExtensionHostClientError::RequestCoordination)?;
        self.send(line)?;
        self.replies
            .lock()
            .map_err(|_| ExtensionHostClientError::RequestCoordination)?
            .recv()
            .map_err(|_| ExtensionHostClientError::Closed)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn send_and_wait_reply(&self, _line: &str) -> Result<String, ExtensionHostClientError> {
        Err(ExtensionHostClientError::UnsupportedPlatform)
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub fn try_recv(&self) -> Result<Option<HostIncoming>, ExtensionHostClientError> {
        match self
            .incoming
            .lock()
            .map_err(|_| ExtensionHostClientError::RequestCoordination)?
            .try_recv()
        {
            Ok(incoming) => Ok(Some(incoming)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => {
                if self.is_closed() {
                    Err(ExtensionHostClientError::Closed)
                } else {
                    Ok(None)
                }
            }
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn try_recv(&self) -> Result<Option<HostIncoming>, ExtensionHostClientError> {
        Err(ExtensionHostClientError::UnsupportedPlatform)
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub const fn is_closed(&self) -> bool {
        true
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl Drop for ExtensionHostClient {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        if let Ok(writer) = self.writer.lock() {
            let _ = writer.shutdown(Shutdown::Both);
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn connect_with_retries(socket_path: &Path) -> Result<UnixStream, ExtensionHostClientError> {
    let mut last_error = None;
    for attempt in 0..HOST_CONNECT_ATTEMPTS {
        match UnixStream::connect(socket_path) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
        if attempt + 1 < HOST_CONNECT_ATTEMPTS {
            thread::sleep(HOST_CONNECT_RETRY_DELAY);
        }
    }
    Err(ExtensionHostClientError::Connect {
        path: socket_path.display().to_string(),
        source: last_error.unwrap_or_else(|| std::io::Error::other("connection failed")),
    })
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn spawn_reader(
    mut reader: UnixStream,
    replies: mpsc::Sender<String>,
    incoming: mpsc::Sender<HostIncoming>,
    closed: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        let mut accumulator = InputAccumulator::default();
        let mut bytes = [0_u8; MAX_READ_BYTES];
        loop {
            match reader.read(&mut bytes) {
                Ok(0) => break,
                Ok(count) => {
                    let Ok(records) = accumulator.push(&bytes[..count]) else {
                        break;
                    };
                    for record in records {
                        deliver_line(record.trimmed(), &replies, &incoming);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        closed.store(true, Ordering::Release);
    });
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn deliver_line(line: &str, replies: &mpsc::Sender<String>, incoming: &mpsc::Sender<HostIncoming>) {
    let routed = if line.starts_with("event|") {
        ExtensionBroadcast::parse(line).map(HostIncoming::Broadcast)
    } else if line.starts_with("extension-event|") {
        ExtensionLocalEvent::parse(line).map(HostIncoming::ExtensionEvent)
    } else if line.starts_with("invoke|") {
        InvokeRequest::parse(line).map(HostIncoming::Invoke)
    } else if line.starts_with("modal-result|") {
        ModalResult::parse(line).map(HostIncoming::ModalResult)
    } else if line.starts_with("modal-query|") {
        ModalQuery::parse(line).map(HostIncoming::ModalQuery)
    } else {
        let _ = replies.send(line.to_owned());
        return;
    };
    if let Some(routed) = routed {
        let _ = incoming.send(routed);
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn reader_routes_host_messages_without_consuming_replies() {
        let (reply_sender, reply_receiver) = mpsc::channel();
        let (incoming_sender, incoming_receiver) = mpsc::channel();
        deliver_line(
            "event|project.switched|projectID=sample",
            &reply_sender,
            &incoming_sender,
        );
        deliver_line("ok", &reply_sender, &incoming_sender);
        assert_eq!(
            incoming_receiver.recv().unwrap(),
            HostIncoming::Broadcast(ExtensionBroadcast {
                name: "project.switched".to_owned(),
                payload: BTreeMap::from([("projectID".to_owned(), "sample".to_owned())]),
            })
        );
        assert_eq!(reply_receiver.recv().unwrap(), "ok");
    }

    #[test]
    fn malformed_routed_messages_are_dropped_instead_of_becoming_replies() {
        let (reply_sender, reply_receiver) = mpsc::channel();
        let (incoming_sender, incoming_receiver) = mpsc::channel();
        deliver_line("invoke|||", &reply_sender, &incoming_sender);
        assert!(reply_receiver.try_recv().is_err());
        assert!(incoming_receiver.try_recv().is_err());
    }
}
