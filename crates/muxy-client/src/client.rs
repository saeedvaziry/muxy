use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use muxy_protocol::{
    CONTROL, ChannelId, ErrorCode, ErrorReply, ForegroundProcess, HistoryCursor, HistoryPage,
    Message, MouseEvent, ReplyBody, RequestBody, SavedScreen, SearchPage, SearchSource, ServerPath,
    SessionId, SessionInfo, Size, TerminalColors, Version,
};
use muxy_transport::{ByteStream, StreamCancellation};
use muxy_wire::{Decoder, Encoder, message_version};

use crate::events::{self, ClientEvent};
use crate::handshake;
use crate::requests::Pending;
use crate::{ClientError, RunGrid};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attachment {
    pub channel: ChannelId,
    pub grid: RunGrid,
    pub title: String,
    pub directory: ServerPath,
    pub process: Option<ForegroundProcess>,
}

struct Shared {
    writer: Mutex<Encoder<Box<dyn Write + Send>>>,
    pending: Arc<Pending>,
    events: Mutex<Option<Receiver<ClientEvent>>>,
    cancellation: Box<dyn StreamCancellation>,
    version: Version,
}

impl Drop for Shared {
    fn drop(&mut self) {
        self.pending.close();
        self.cancellation.cancel();
    }
}

#[derive(Clone)]
pub struct Client {
    shared: Arc<Shared>,
    timeout: Duration,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Client")
            .field("timeout", &self.timeout)
            .field("connected", &!self.shared.pending.is_closed())
            .finish_non_exhaustive()
    }
}

impl Client {
    pub fn connect(socket: &Path) -> Result<Self, ClientError> {
        Self::from_stream(muxy_transport::connect(socket)?)
    }

    pub fn from_stream(stream: Box<dyn ByteStream>) -> Result<Self, ClientError> {
        let cancellation = stream.cancellation()?;
        let reader_cancellation = stream.cancellation()?;
        let (read, write) = stream.split()?;
        let mut decoder = Decoder::new(read);
        let mut encoder = Encoder::new(write);
        encoder.send(CONTROL, &handshake::hello())?;
        let (events, receiver) = mpsc::channel();
        let (connected, accepted) = mpsc::channel();
        let pending = Arc::new(Pending::default());
        let routed = Arc::clone(&pending);
        thread::Builder::new()
            .name("muxy-client-reader".into())
            .spawn(move || {
                events::route(
                    &mut decoder,
                    &routed,
                    &events,
                    &connected,
                    reader_cancellation.as_ref(),
                );
            })?;
        let negotiated = accepted
            .recv_timeout(DEFAULT_TIMEOUT)
            .map_err(|error| match error {
                RecvTimeoutError::Timeout => ClientError::Timeout,
                RecvTimeoutError::Disconnected => ClientError::Disconnected,
            })
            .and_then(std::convert::identity);
        let version = match negotiated {
            Ok(version) => version,
            Err(error) => {
                cancellation.cancel();
                return Err(error);
            }
        };
        let shared = Arc::new(Shared {
            writer: Mutex::new(encoder),
            pending,
            events: Mutex::new(Some(receiver)),
            cancellation,
            version,
        });
        Ok(Self {
            shared,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn events(&self) -> Option<Receiver<ClientEvent>> {
        lock(&self.shared.events).take()
    }

    pub fn is_connected(&self) -> bool {
        !self.shared.pending.is_closed()
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionInfo>, ClientError> {
        match self.request(RequestBody::ListSessions)? {
            ReplyBody::Sessions(sessions) => Ok(sessions),
            other => Err(ClientError::UnexpectedReply(other)),
        }
    }

    pub fn create_session(&self, directory: &Path, size: Size) -> Result<SessionInfo, ClientError> {
        let directory = ServerPath(directory.as_os_str().as_bytes().to_vec());
        match self.request(RequestBody::CreateSession { directory, size })? {
            ReplyBody::SessionCreated(info) => Ok(info),
            other => Err(ClientError::UnexpectedReply(other)),
        }
    }

    pub fn end_session(&self, id: SessionId) -> Result<(), ClientError> {
        match self.request(RequestBody::EndSession(id))? {
            ReplyBody::SessionEnded => Ok(()),
            other => Err(ClientError::UnexpectedReply(other)),
        }
    }

    pub fn read_saved_screen(&self, id: SessionId) -> Result<SavedScreen, ClientError> {
        match self.request(RequestBody::ReadSavedScreen(id))? {
            ReplyBody::SavedScreen(screen) => Ok(screen),
            other => Err(ClientError::UnexpectedReply(other)),
        }
    }

    pub fn discard_session(&self, id: SessionId) -> Result<(), ClientError> {
        match self.request(RequestBody::DiscardSession(id))? {
            ReplyBody::SessionDiscarded => Ok(()),
            other => Err(ClientError::UnexpectedReply(other)),
        }
    }

    pub fn attach(&self, id: SessionId, size: Size) -> Result<Attachment, ClientError> {
        let (snapshot, process) = match self.request(RequestBody::Attach { session: id, size })? {
            ReplyBody::Attached { snapshot, process } => (*snapshot, process),
            other => return Err(ClientError::UnexpectedReply(other)),
        };
        Ok(Attachment {
            channel: snapshot.channel,
            grid: RunGrid::from_snapshot(&snapshot),
            title: snapshot.title,
            directory: snapshot.directory,
            process,
        })
    }

    pub fn history_page(
        &self,
        channel: ChannelId,
        before: HistoryCursor,
        max_rows: u16,
    ) -> Result<HistoryPage, ClientError> {
        self.read_history(RequestBody::HistoryPage {
            channel,
            before,
            max_rows,
        })
    }

    pub fn saved_history_page(
        &self,
        session: SessionId,
        before: HistoryCursor,
        max_rows: u16,
    ) -> Result<HistoryPage, ClientError> {
        self.read_history(RequestBody::SavedHistoryPage {
            session,
            before,
            max_rows,
        })
    }

    fn read_history(&self, request: RequestBody) -> Result<HistoryPage, ClientError> {
        match self.request(request)? {
            ReplyBody::HistoryPage(page) => Ok(page),
            other => Err(ClientError::UnexpectedReply(other)),
        }
    }

    pub fn detach(&self, channel: ChannelId) -> Result<(), ClientError> {
        match self.request(RequestBody::Detach(channel))? {
            ReplyBody::Detached => Ok(()),
            other => Err(ClientError::UnexpectedReply(other)),
        }
    }

    pub fn search(
        &self,
        source: SearchSource,
        query: &str,
        ignore_case: bool,
        before: HistoryCursor,
        max_results: u16,
    ) -> Result<SearchPage, ClientError> {
        match self.request(RequestBody::Search {
            source,
            query: query.to_owned(),
            ignore_case,
            before,
            max_results,
        })? {
            ReplyBody::SearchPage(page) => Ok(page),
            other => Err(ClientError::UnexpectedReply(other)),
        }
    }

    pub fn resize(&self, channel: ChannelId, size: Size) -> Result<(), ClientError> {
        match self.request(RequestBody::Resize { channel, size })? {
            ReplyBody::Resized => Ok(()),
            other => Err(ClientError::UnexpectedReply(other)),
        }
    }

    /// Sets color defaults for this connection and its attached sessions.
    pub fn set_terminal_colors(&self, colors: TerminalColors) -> Result<(), ClientError> {
        match self.request(RequestBody::SetTerminalColors(colors))? {
            ReplyBody::TerminalColorsSet => Ok(()),
            other => Err(ClientError::UnexpectedReply(other)),
        }
    }

    pub fn ping(&self) -> Result<(), ClientError> {
        match self.request(RequestBody::Ping)? {
            ReplyBody::Pong => Ok(()),
            other => Err(ClientError::UnexpectedReply(other)),
        }
    }

    pub fn send_input(&self, channel: ChannelId, bytes: &[u8]) -> Result<(), ClientError> {
        session_channel(channel)?;
        self.send(channel, &Message::Input(bytes.to_vec()))
    }

    pub fn send_mouse(&self, channel: ChannelId, event: MouseEvent) -> Result<(), ClientError> {
        session_channel(channel)?;
        self.send(channel, &Message::Mouse(event))
    }

    /// Close this connection and cancel outstanding requests for every client clone.
    pub fn disconnect(&self) {
        self.shared.pending.close();
        self.shared.cancellation.cancel();
    }

    pub fn ack(&self, channel: ChannelId, seq: u64) -> Result<(), ClientError> {
        session_channel(channel)?;
        self.send(CONTROL, &Message::FrameAck { channel, seq })
    }

    fn request(&self, body: RequestBody) -> Result<ReplyBody, ClientError> {
        let (id, reply) = self.shared.pending.register()?;
        if let Err(error) = self.send(CONTROL, &Message::Request { id, body }) {
            self.shared.pending.forget(id);
            return Err(error);
        }
        match reply.recv_timeout(self.timeout) {
            Ok(ReplyBody::Error(error)) => Err(ClientError::Server(error)),
            Ok(body) => Ok(body),
            Err(RecvTimeoutError::Timeout) => {
                self.shared.pending.forget(id);
                Err(ClientError::Timeout)
            }
            Err(RecvTimeoutError::Disconnected) => Err(ClientError::Disconnected),
        }
    }

    fn send(&self, channel: ChannelId, message: &Message) -> Result<(), ClientError> {
        message.validate().map_err(ClientError::Invalid)?;
        if message_version(message) > self.shared.version {
            return Err(ClientError::Server(ErrorReply {
                code: ErrorCode::BadRequest,
                message: "The running server does not support this message".into(),
            }));
        }
        if self.shared.pending.is_closed() {
            return Err(ClientError::Disconnected);
        }
        lock(&self.shared.writer).send(channel, message)?;
        Ok(())
    }
}

fn session_channel(channel: ChannelId) -> Result<(), ClientError> {
    if channel == CONTROL {
        Err(ClientError::Invalid(ErrorCode::UnknownChannel))
    } else {
        Ok(())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
