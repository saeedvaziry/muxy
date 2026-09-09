use std::error::Error;
use std::fs;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use muxy_protocol::{
    AttachSnapshot, CONTROL, ChannelId, ErrorCode, ExitReason, Message, ReplyBody, RequestBody,
    RequestId, Row, SUPPORTED, ScreenFrame, ServerPath, SessionId, SessionInfo, Size, Version,
};
use muxy_server_core::{Registry, ServerEvent, ServerSettings, connection::serve};
use muxy_transport::{ByteStream, StreamCancellation};
use muxy_wire::{Decoder, Encoder, WireError};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;
const TIMEOUT: Duration = Duration::from_secs(10);
const QUIET: Duration = Duration::from_millis(150);
const SIZE: Size = Size { cols: 80, rows: 24 };
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct Fixture {
    registry: Arc<Registry>,
    subscribers: Arc<Mutex<Vec<Sender<ServerEvent>>>>,
    running: Arc<AtomicBool>,
    directory: PathBuf,
}

struct Client {
    socket: UnixStream,
    encoder: Encoder<UnixStream>,
    incoming: Receiver<Result<(ChannelId, Message), WireError>>,
    finished: Receiver<Result<(), WireError>>,
    next_request: u32,
}

impl Fixture {
    fn new() -> TestResult<Self> {
        let directory = std::env::temp_dir().join(format!(
            "muxy-connection-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory)?;
        let (sender, events) = mpsc::channel();
        let registry = Arc::new(Registry::new(
            ServerSettings {
                default_shell: Some(PathBuf::from("/bin/sh")),
                ..ServerSettings::default()
            },
            sender,
        ));
        let subscribers = Arc::new(Mutex::new(Vec::<Sender<ServerEvent>>::new()));
        let sinks = Arc::clone(&subscribers);
        let running = Arc::new(AtomicBool::new(true));
        let active = Arc::clone(&running);
        thread::spawn(move || {
            while active.load(Ordering::Relaxed) {
                match events.recv_timeout(QUIET) {
                    Ok(event) => {
                        if let Ok(mut sinks) = sinks.lock() {
                            sinks.retain(|sink| sink.send(event.clone()).is_ok());
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        Ok(Self {
            registry,
            subscribers,
            running,
            directory,
        })
    }

    fn client(&self, hello: bool) -> TestResult<Client> {
        let (socket, server) = UnixStream::pair()?;
        let (sender, events) = mpsc::channel();
        self.subscribers
            .lock()
            .map_err(|_| "subscriber lock poisoned")?
            .push(sender);
        let registry = Arc::clone(&self.registry);
        let (done, finished) = mpsc::channel();
        thread::spawn(move || {
            let _ = done.send(serve(Box::new(server), registry, events));
        });
        let mut decoder = Decoder::new(socket.try_clone()?);
        let (sender, incoming) = mpsc::channel();
        thread::spawn(move || {
            loop {
                let result = decoder.next();
                let ended = result.is_err();
                if sender.send(result).is_err() || ended {
                    break;
                }
            }
        });
        let mut client = Client {
            encoder: Encoder::new(socket.try_clone()?),
            socket,
            incoming,
            finished,
            next_request: 1,
        };
        if hello {
            client.send(
                CONTROL,
                Message::Hello {
                    versions: SUPPORTED.to_vec(),
                },
            )?;
            assert_eq!(
                client.receive()?,
                (
                    CONTROL,
                    Message::HelloReply {
                        versions: SUPPORTED.to_vec()
                    }
                )
            );
        }
        Ok(client)
    }

    fn create(&self, client: &mut Client) -> TestResult<SessionInfo> {
        match client.request(RequestBody::CreateSession {
            directory: ServerPath(self.directory.as_os_str().as_bytes().to_vec()),
            size: SIZE,
        })? {
            ReplyBody::SessionCreated(info) => Ok(info),
            other => Err(format!("expected created, got {other:?}").into()),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for session in self.registry.list() {
            let _ = self.registry.end(session.id);
        }
        let deadline = Instant::now() + TIMEOUT;
        while !self.registry.list().is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        self.running.store(false, Ordering::Relaxed);
        let _ = fs::remove_dir(&self.directory);
    }
}

impl Client {
    #[allow(clippy::needless_pass_by_value)]
    fn send(&mut self, channel: ChannelId, message: Message) -> TestResult {
        self.encoder.send(channel, &message)?;
        Ok(())
    }

    fn receive(&self) -> TestResult<(ChannelId, Message)> {
        Ok(self.incoming.recv_timeout(TIMEOUT)??)
    }

    fn request(&mut self, body: RequestBody) -> TestResult<ReplyBody> {
        let id = RequestId(self.next_request);
        self.next_request += 1;
        self.send(CONTROL, Message::Request { id, body })?;
        loop {
            match self.receive()? {
                (CONTROL, Message::Reply { id: received, body }) if received == id => {
                    return Ok(body);
                }
                (channel, Message::Frame(frame)) => self.ack(channel, frame.seq)?,
                (_, Message::Metadata(_)) => {}
                other => return Err(format!("unexpected reply: {other:?}").into()),
            }
        }
    }

    fn attach(&mut self, session: SessionId) -> TestResult<AttachSnapshot> {
        match self.request(RequestBody::Attach {
            session,
            size: SIZE,
        })? {
            ReplyBody::Attached { snapshot, .. } => Ok(*snapshot),
            other => Err(format!("expected attached, got {other:?}").into()),
        }
    }

    fn input(&mut self, channel: ChannelId, bytes: &[u8]) -> TestResult {
        self.send(channel, Message::Input(bytes.to_vec()))
    }

    fn ack(&mut self, channel: ChannelId, seq: u64) -> TestResult {
        self.send(CONTROL, Message::FrameAck { channel, seq })
    }

    fn frame(&mut self, channel: ChannelId, needle: &str) -> TestResult<ScreenFrame> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if Instant::now() > deadline {
                return Err("frame never contained expected text".into());
            }
            match self.receive()? {
                (received, Message::Frame(frame)) if received == channel => {
                    if text(&frame.rows).contains(needle) {
                        return Ok(frame);
                    }
                    self.ack(channel, frame.seq)?;
                }
                (_, Message::Metadata(_)) => {}
                other => return Err(format!("expected frame, got {other:?}").into()),
            }
        }
    }

    fn quiet(&mut self) -> TestResult {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match self.incoming.recv_timeout(QUIET) {
                Ok(Ok((channel, Message::Frame(frame)))) if Instant::now() < deadline => {
                    self.ack(channel, frame.seq)?;
                }
                Ok(Ok((_, Message::Metadata(_)))) if Instant::now() < deadline => {}
                Err(RecvTimeoutError::Timeout) => return Ok(()),
                other => return Err(format!("connection did not become quiet: {other:?}").into()),
            }
        }
    }

    fn disconnect(self) -> TestResult {
        self.socket.shutdown(Shutdown::Both)?;
        self.finished.recv_timeout(TIMEOUT)??;
        Ok(())
    }

    fn closed(&self) -> TestResult {
        assert!(matches!(
            self.incoming.recv_timeout(TIMEOUT)?,
            Err(WireError::Closed)
        ));
        self.finished.recv_timeout(TIMEOUT)??;
        Ok(())
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.socket.shutdown(Shutdown::Both);
    }
}

fn text(rows: &[Row]) -> String {
    rows.iter()
        .map(|row| {
            row.runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn hello_list_create_attach_and_input() -> TestResult {
    let fixture = Fixture::new()?;
    let mut client = fixture.client(true)?;
    assert_eq!(
        client.request(RequestBody::ListSessions)?,
        ReplyBody::Sessions(vec![])
    );
    let info = fixture.create(&mut client)?;
    assert_eq!(
        client.request(RequestBody::ListSessions)?,
        ReplyBody::Sessions(vec![info.clone()])
    );
    let snapshot = client.attach(info.id)?;
    assert_ne!(snapshot.channel, CONTROL);
    assert_eq!(snapshot.size, SIZE);
    assert_eq!(
        snapshot.directory.0,
        fixture.directory.canonicalize()?.as_os_str().as_bytes()
    );
    assert_eq!(snapshot.rows.len(), usize::from(SIZE.rows));
    client.input(snapshot.channel, b"echo hi\n")?;
    client.frame(snapshot.channel, "hi")?;
    client.disconnect()
}

#[test]
fn a_request_before_hello_is_fatal() -> TestResult {
    let fixture = Fixture::new()?;
    let mut client = fixture.client(false)?;
    client.send(
        CONTROL,
        Message::Request {
            id: RequestId(7),
            body: RequestBody::Ping,
        },
    )?;
    assert!(matches!(client.receive()?, (CONTROL, Message::Fatal(_))));
    client.closed()
}

#[test]
fn incompatible_versions_are_rejected_and_closed() -> TestResult {
    let fixture = Fixture::new()?;
    let mut client = fixture.client(false)?;
    client.send(
        CONTROL,
        Message::Hello {
            versions: vec![Version(99)],
        },
    )?;
    assert_eq!(client.receive()?, (CONTROL, Message::VersionUnsupported));
    client.closed()
}

#[test]
fn invalid_or_misplaced_hellos_are_fatal() -> TestResult {
    let fixture = Fixture::new()?;
    for (channel, versions) in [(CONTROL, vec![]), (ChannelId(1), SUPPORTED.to_vec())] {
        let mut client = fixture.client(false)?;
        client.send(channel, Message::Hello { versions })?;
        assert!(matches!(client.receive()?, (CONTROL, Message::Fatal(_))));
        client.closed()?;
    }
    Ok(())
}

#[test]
fn malformed_headers_and_payloads_are_fatal() -> TestResult {
    let fixture = Fixture::new()?;
    for hello in [false, true] {
        for kind in [0x3f, muxy_wire::MessageKind::Hello as u8] {
            let mut client = fixture.client(hello)?;
            client.socket.write_all(
                &muxy_wire::Header {
                    length: 7,
                    version: 1,
                    channel: 0,
                    kind,
                }
                .to_bytes(),
            )?;
            assert!(matches!(client.receive()?, (CONTROL, Message::Fatal(_))));
            client.closed()?;
        }
    }
    Ok(())
}

#[test]
fn server_messages_repeated_hello_and_unknown_input_are_fatal() -> TestResult {
    let fixture = Fixture::new()?;
    for (channel, message) in [
        (
            CONTROL,
            Message::Hello {
                versions: SUPPORTED.to_vec(),
            },
        ),
        (
            CONTROL,
            Message::Reply {
                id: RequestId(1),
                body: ReplyBody::Pong,
            },
        ),
        (ChannelId(1), Message::Input(b"x".to_vec())),
        (CONTROL, Message::Input(b"x".to_vec())),
        (
            ChannelId(1),
            Message::Request {
                id: RequestId(2),
                body: RequestBody::Ping,
            },
        ),
    ] {
        let mut client = fixture.client(true)?;
        client.send(channel, message)?;
        assert!(matches!(client.receive()?, (CONTROL, Message::Fatal(_))));
        client.closed()?;
    }
    Ok(())
}

#[test]
fn request_errors_are_correlated_and_leave_connection_usable() -> TestResult {
    let fixture = Fixture::new()?;
    let mut client = fixture.client(true)?;
    let unknown = SessionId::new(1).ok_or("zero session")?;
    for (request, code) in [
        (
            RequestBody::CreateSession {
                directory: ServerPath(vec![]),
                size: SIZE,
            },
            ErrorCode::BadPath,
        ),
        (
            RequestBody::CreateSession {
                directory: ServerPath(b"/not/a/muxy/directory".to_vec()),
                size: SIZE,
            },
            ErrorCode::BadPath,
        ),
        (
            RequestBody::CreateSession {
                directory: ServerPath(b"/tmp".to_vec()),
                size: Size { cols: 0, rows: 1 },
            },
            ErrorCode::BadSize,
        ),
        (
            RequestBody::Attach {
                session: unknown,
                size: SIZE,
            },
            ErrorCode::UnknownSession,
        ),
        (RequestBody::EndSession(unknown), ErrorCode::UnknownSession),
        (RequestBody::Detach(ChannelId(1)), ErrorCode::UnknownChannel),
        (
            RequestBody::Resize {
                channel: ChannelId(1),
                size: SIZE,
            },
            ErrorCode::UnknownChannel,
        ),
    ] {
        assert!(matches!(client.request(request)?, ReplyBody::Error(error) if error.code == code));
        assert_eq!(client.request(RequestBody::Ping)?, ReplyBody::Pong);
    }
    client.disconnect()
}

#[test]
fn one_credit_merges_rows_and_control_remains_available() -> TestResult {
    let fixture = Fixture::new()?;
    let mut client = fixture.client(true)?;
    let info = fixture.create(&mut client)?;
    let channel = client.attach(info.id)?.channel;
    client.input(
        channel,
        b"stty -echo; PS1=''; printf '\\033[2J\\033[Hready'\n",
    )?;
    let ready = client.frame(channel, "ready")?;
    client.ack(channel, ready.seq)?;
    client.quiet()?;
    client.input(channel, b"printf '\\033[1;1Hfirst'\n")?;
    let first = client.frame(channel, "first")?;
    client.input(channel, b"printf '\\033[3;1Hsecond'\n")?;
    thread::sleep(QUIET);
    client.input(channel, b"printf '\\033[5;1Hthird'\n")?;
    thread::sleep(QUIET);
    assert!(matches!(
        client.incoming.recv_timeout(QUIET),
        Err(RecvTimeoutError::Timeout)
    ));
    assert_eq!(client.request(RequestBody::Ping)?, ReplyBody::Pong);
    client.ack(channel, first.seq)?;
    let merged = client.frame(channel, "third")?;
    assert!(text(&merged.rows).contains("second"));
    assert!(merged.seq > first.seq + 1);
    client.disconnect()
}

#[test]
fn detach_ignores_late_input_and_ack_and_does_not_reuse_channel() -> TestResult {
    let fixture = Fixture::new()?;
    let mut client = fixture.client(true)?;
    let info = fixture.create(&mut client)?;
    let channel = client.attach(info.id)?.channel;
    client.quiet()?;
    assert_eq!(
        client.request(RequestBody::Detach(channel))?,
        ReplyBody::Detached
    );
    client.input(channel, b"exit 37\n")?;
    client.ack(channel, 1)?;
    assert_eq!(client.request(RequestBody::Ping)?, ReplyBody::Pong);
    thread::sleep(QUIET);
    assert_eq!(
        client.request(RequestBody::ListSessions)?,
        ReplyBody::Sessions(vec![info.clone()])
    );
    let next = client.attach(info.id)?.channel;
    assert_ne!(next, channel);
    client.disconnect()
}

#[test]
fn resize_returns_reply_and_a_reset_frame() -> TestResult {
    let fixture = Fixture::new()?;
    let mut client = fixture.client(true)?;
    let info = fixture.create(&mut client)?;
    let channel = client.attach(info.id)?.channel;
    client.quiet()?;
    assert_eq!(
        client.request(RequestBody::Resize {
            channel,
            size: Size { cols: 60, rows: 20 }
        })?,
        ReplyBody::Resized
    );
    let frame = client.frame(channel, "")?;
    assert!(frame.reset);
    assert_eq!(frame.rows.len(), 20);
    client.disconnect()
}

#[test]
fn ending_a_session_notifies_attached_and_unattached_connections_once() -> TestResult {
    let fixture = Fixture::new()?;
    let mut attached = fixture.client(true)?;
    let observer = fixture.client(true)?;
    let info = fixture.create(&mut attached)?;
    let channel = attached.attach(info.id)?.channel;
    attached.quiet()?;
    attached.send(
        CONTROL,
        Message::Request {
            id: RequestId(99),
            body: RequestBody::EndSession(info.id),
        },
    )?;
    let ended = Message::SessionEnded {
        session: info.id,
        reason: ExitReason::Ended,
    };
    let mut messages = vec![attached.receive()?, attached.receive()?];
    assert!(messages.contains(&(CONTROL, ended.clone())));
    messages.retain(|(_, message)| message != &ended);
    assert_eq!(
        messages,
        vec![(
            CONTROL,
            Message::Reply {
                id: RequestId(99),
                body: ReplyBody::SessionEnded
            }
        )]
    );
    assert_eq!(observer.receive()?, (CONTROL, ended));
    assert!(fixture.registry.list().is_empty());
    attached.input(channel, b"ignored")?;
    attached.ack(channel, 1)?;
    assert_eq!(attached.request(RequestBody::Ping)?, ReplyBody::Pong);
    assert!(matches!(
        observer.incoming.recv_timeout(QUIET),
        Err(RecvTimeoutError::Timeout)
    ));
    observer.disconnect()?;
    attached.disconnect()
}

#[test]
fn disconnect_keeps_session_for_a_new_client() -> TestResult {
    let fixture = Fixture::new()?;
    let mut first = fixture.client(true)?;
    let info = fixture.create(&mut first)?;
    let channel = first.attach(info.id)?.channel;
    first.input(channel, b"echo retained\n")?;
    first.frame(channel, "retained")?;
    first.disconnect()?;
    let mut second = fixture.client(true)?;
    assert_eq!(
        second.request(RequestBody::ListSessions)?,
        ReplyBody::Sessions(vec![info.clone()])
    );
    let snapshot = second.attach(info.id)?;
    assert!(text(&snapshot.rows).contains("retained"));
    second.input(snapshot.channel, b"echo reattached\n")?;
    second.frame(snapshot.channel, "reattached")?;
    second.disconnect()
}

#[test]
fn attachments_from_two_connections_have_independent_identity_and_credit() -> TestResult {
    let fixture = Fixture::new()?;
    let mut first = fixture.client(true)?;
    let mut second = fixture.client(true)?;
    let info = fixture.create(&mut first)?;
    let first_channel = first.attach(info.id)?.channel;
    let second_channel = second.attach(info.id)?.channel;
    assert_eq!(first_channel, second_channel);
    first.input(first_channel, b"echo shared-one\n")?;
    first.frame(first_channel, "shared-one")?;
    let frame = second.frame(second_channel, "shared-one")?;
    second.ack(second_channel, frame.seq)?;
    first.disconnect()?;
    second.input(second_channel, b"echo shared-two\n")?;
    second.frame(second_channel, "shared-two")?;
    second.disconnect()
}

#[test]
fn fatal_is_the_last_message_even_with_an_attach_in_progress() -> TestResult {
    let fixture = Fixture::new()?;
    let info = fixture.registry.create(&fixture.directory, SIZE)?;
    for _ in 0..20 {
        let mut client = fixture.client(true)?;
        client.send(
            CONTROL,
            Message::Request {
                id: RequestId(1),
                body: RequestBody::Attach {
                    session: info.id,
                    size: SIZE,
                },
            },
        )?;
        client.send(
            CONTROL,
            Message::Hello {
                versions: SUPPORTED.to_vec(),
            },
        )?;
        loop {
            match client.receive()? {
                (CONTROL, Message::Fatal(_)) => break,
                (
                    CONTROL,
                    Message::Reply {
                        id: RequestId(1), ..
                    },
                )
                | (_, Message::Frame(_) | Message::Metadata(_)) => {}
                other => return Err(format!("unexpected message before fatal: {other:?}").into()),
            }
        }
        client.closed()?;
    }
    Ok(())
}

#[test]
fn oversized_input_on_an_active_channel_is_fatal() -> TestResult {
    let fixture = Fixture::new()?;
    let mut client = fixture.client(true)?;
    let info = fixture.create(&mut client)?;
    let channel = client.attach(info.id)?.channel;
    client.quiet()?;
    client.input(channel, &vec![b'x'; muxy_protocol::MAX_INPUT + 1])?;
    assert!(matches!(client.receive()?, (CONTROL, Message::Fatal(_))));
    client.closed()
}

#[test]
fn resize_replaces_old_pending_rows_and_correlates_pipelined_requests() -> TestResult {
    let fixture = Fixture::new()?;
    let mut client = fixture.client(true)?;
    let info = fixture.create(&mut client)?;
    let channel = client.attach(info.id)?.channel;
    client.input(
        channel,
        b"stty -echo; PS1=''; printf '\\033[2J\\033[Hready'\n",
    )?;
    let ready = client.frame(channel, "ready")?;
    client.ack(channel, ready.seq)?;
    client.quiet()?;
    client.input(channel, b"printf '\\033[1;1Hhold'\n")?;
    let hold = client.frame(channel, "hold")?;
    client.input(channel, b"printf '\\033[23;1Hold-geometry'\n")?;
    thread::sleep(QUIET);
    assert_eq!(
        client.request(RequestBody::Resize {
            channel,
            size: Size { cols: 60, rows: 20 },
        })?,
        ReplyBody::Resized
    );
    client.ack(channel, hold.seq)?;
    let reset = client.frame(channel, "")?;
    assert!(reset.reset);
    assert_eq!(reset.rows.len(), 20);
    assert!(reset.rows.iter().all(|row| row.index < 20));
    for (id, rows) in [(91, 18), (92, 12)] {
        client.send(
            CONTROL,
            Message::Request {
                id: RequestId(id),
                body: RequestBody::Resize {
                    channel,
                    size: Size { cols: 60, rows },
                },
            },
        )?;
    }
    let mut replies = vec![];
    while replies.len() < 2 {
        match client.receive()? {
            (
                CONTROL,
                Message::Reply {
                    id,
                    body: ReplyBody::Resized,
                },
            ) => replies.push(id.0),
            (_, Message::Metadata(muxy_protocol::MetadataEvent::History { .. })) => {}
            other => return Err(format!("expected resize reply, got {other:?}").into()),
        }
    }
    assert_eq!(replies, vec![91, 92]);
    loop {
        match client.incoming.recv_timeout(QUIET) {
            Ok(Ok((_, Message::Metadata(muxy_protocol::MetadataEvent::History { .. })))) => {}
            Err(RecvTimeoutError::Timeout) => break,
            other => return Err(format!("expected frames to remain blocked, got {other:?}").into()),
        }
    }
    client.ack(channel, reset.seq)?;
    let newest = client.frame(channel, "")?;
    assert!(newest.reset);
    assert_eq!(newest.rows.len(), 12);
    assert!(newest.rows.iter().all(|row| row.index < 12));
    client.disconnect()
}

#[test]
fn detaching_replies_once_to_each_pending_resize() -> TestResult {
    let fixture = Fixture::new()?;
    let mut client = fixture.client(true)?;
    let info = fixture.create(&mut client)?;
    let channel = client.attach(info.id)?.channel;
    client.quiet()?;
    for id in 90..100 {
        client.send(
            CONTROL,
            Message::Request {
                id: RequestId(id),
                body: RequestBody::Resize {
                    channel,
                    size: Size { cols: 60, rows: 20 },
                },
            },
        )?;
    }
    client.send(
        CONTROL,
        Message::Request {
            id: RequestId(100),
            body: RequestBody::Detach(channel),
        },
    )?;
    let mut replies = vec![];
    let mut detached = false;
    while replies.len() < 11 {
        match client.receive()? {
            (CONTROL, Message::Reply { id, body }) => {
                if id == RequestId(100) {
                    assert_eq!(body, ReplyBody::Detached);
                    detached = true;
                } else {
                    assert!((90..100).contains(&id.0));
                    match body {
                        ReplyBody::Resized => {}
                        ReplyBody::Error(error) => {
                            assert_eq!(error.code, ErrorCode::UnknownChannel);
                        }
                        other => return Err(format!("unexpected resize reply: {other:?}").into()),
                    }
                }
                replies.push(id.0);
            }
            (_, Message::Frame(_) | Message::Metadata(_)) => assert!(!detached),
            other => return Err(format!("unexpected message: {other:?}").into()),
        }
    }
    replies.sort_unstable();
    assert_eq!(replies, (90..=100).collect::<Vec<_>>());
    assert_eq!(client.request(RequestBody::Ping)?, ReplyBody::Pong);
    client.disconnect()
}

struct EofStream {
    stream: UnixStream,
    eof: Sender<()>,
}

impl ByteStream for EofStream {
    fn cancellation(&self) -> io::Result<Box<dyn StreamCancellation>> {
        self.stream.cancellation()
    }

    #[allow(clippy::type_complexity)]
    fn split(self: Box<Self>) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
        let (reader, writer) = Box::new(self.stream).split()?;
        Ok((
            Box::new(EofReader {
                reader,
                eof: self.eof,
            }),
            writer,
        ))
    }
}

struct EofReader {
    reader: Box<dyn Read + Send>,
    eof: Sender<()>,
}

impl Read for EofReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let count = self.reader.read(bytes)?;
        if count == 0 {
            let _ = self.eof.send(());
        }
        Ok(count)
    }
}

#[test]
fn reader_eof_and_fatal_cancel_a_blocked_writer() -> TestResult {
    let fixture = Fixture::new()?;
    for fatal in [false, true] {
        let (peer, stream) = UnixStream::pair()?;
        peer.set_read_timeout(Some(TIMEOUT))?;
        peer.set_write_timeout(Some(TIMEOUT))?;
        let (eof, observed) = mpsc::channel();
        let (_sender, events) = mpsc::channel();
        let (done, finished) = mpsc::channel();
        let registry = Arc::clone(&fixture.registry);
        let worker = thread::spawn(move || {
            let _ = done.send(serve(Box::new(EofStream { stream, eof }), registry, events));
        });
        let mut encoder = Encoder::new(peer.try_clone()?);
        let mut decoder = Decoder::new(peer.try_clone()?);
        let hello = Message::Hello {
            versions: SUPPORTED.to_vec(),
        };
        encoder.send(CONTROL, &hello)?;
        assert!(matches!(
            decoder.next()?,
            (CONTROL, Message::HelloReply { .. })
        ));
        for id in 1..=50_000 {
            encoder.send(
                CONTROL,
                &Message::Request {
                    id: RequestId(id),
                    body: RequestBody::Ping,
                },
            )?;
        }
        if fatal {
            encoder.send(CONTROL, &hello)?;
        } else {
            peer.shutdown(Shutdown::Write)?;
            observed.recv_timeout(TIMEOUT)?;
        }
        let result = finished.recv_timeout(Duration::from_secs(3));
        drop(encoder);
        drop(decoder);
        drop(peer);
        worker.join().map_err(|_| "connection panicked")?;
        result.map_err(|error| format!("shutdown timed out (fatal={fatal}): {error}"))??;
    }
    Ok(())
}

struct FailingStream {
    stream: UnixStream,
    ready: Sender<()>,
    fail: Receiver<()>,
}

impl ByteStream for FailingStream {
    fn cancellation(&self) -> io::Result<Box<dyn StreamCancellation>> {
        self.stream.cancellation()
    }

    #[allow(clippy::type_complexity)]
    fn split(self: Box<Self>) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
        let handshake = Arc::new(AtomicBool::new(false));
        let writer = FailingWriter {
            stream: self.stream.try_clone()?,
            handshake: Arc::clone(&handshake),
            fail: self.fail,
        };
        let reader = IdleReader {
            stream: self.stream,
            handshake,
            ready: Some(self.ready),
        };
        Ok((Box::new(reader), Box::new(writer)))
    }
}

struct IdleReader {
    stream: UnixStream,
    handshake: Arc<AtomicBool>,
    ready: Option<Sender<()>>,
}

impl Read for IdleReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if self.handshake.load(Ordering::SeqCst)
            && let Some(ready) = self.ready.take()
        {
            let _ = ready.send(());
        }
        self.stream.read(bytes)
    }
}

struct FailingWriter {
    stream: UnixStream,
    handshake: Arc<AtomicBool>,
    fail: Receiver<()>,
}

impl Write for FailingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.handshake.load(Ordering::SeqCst) {
            self.fail.recv_timeout(TIMEOUT).map_err(io::Error::other)?;
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "injected writer failure",
            ));
        }
        self.stream.write_all(bytes)?;
        self.handshake.store(true, Ordering::SeqCst);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

#[test]
fn writer_failure_cancels_an_idle_reader_and_returns_the_error() -> TestResult {
    let fixture = Fixture::new()?;
    let (peer, stream) = UnixStream::pair()?;
    peer.set_read_timeout(Some(TIMEOUT))?;
    let (ready, idle) = mpsc::channel();
    let (fail, failure) = mpsc::channel();
    let (sender, events) = mpsc::channel();
    let (done, finished) = mpsc::channel();
    let registry = Arc::clone(&fixture.registry);
    let worker = thread::spawn(move || {
        let _ = done.send(serve(
            Box::new(FailingStream {
                stream,
                ready,
                fail: failure,
            }),
            registry,
            events,
        ));
    });
    let mut encoder = Encoder::new(peer.try_clone()?);
    let mut decoder = Decoder::new(peer.try_clone()?);
    encoder.send(
        CONTROL,
        &Message::Hello {
            versions: SUPPORTED.to_vec(),
        },
    )?;
    assert!(matches!(
        decoder.next()?,
        (CONTROL, Message::HelloReply { .. })
    ));
    idle.recv_timeout(TIMEOUT)?;
    assert!(matches!(
        finished.recv_timeout(QUIET),
        Err(RecvTimeoutError::Timeout)
    ));
    sender.send(ServerEvent::SessionEnded {
        id: SessionId::new(1).ok_or("zero session")?,
        reason: ExitReason::Ended,
    })?;
    fail.send(())?;
    let result = finished.recv_timeout(Duration::from_secs(2));
    let _ = peer.shutdown(Shutdown::Both);
    worker.join().map_err(|_| "connection panicked")?;
    assert!(
        matches!(result?, Err(WireError::Io(error)) if error.kind() == io::ErrorKind::BrokenPipe)
    );
    Ok(())
}
