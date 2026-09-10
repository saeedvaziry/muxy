use std::error::Error;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;

#[path = "client/colors.rs"]
mod colors;
#[path = "client/cursor.rs"]
mod cursor;
#[path = "client/links.rs"]
mod links;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use muxy_client::{Attachment, Client, ClientError, ClientEvent, RunGrid};
use muxy_protocol::{CONTROL, ChannelId, ErrorCode, ExitReason, Message, ScreenFrame, Size};
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

struct Connection {
    client: Client,
    events: Receiver<ClientEvent>,
    server: Box<dyn StreamCancellation>,
    finished: Receiver<Result<(), WireError>>,
}

impl Fixture {
    fn new() -> TestResult<Self> {
        Self::with_startup("")
    }

    fn with_startup(script: &str) -> TestResult<Self> {
        let directory = std::env::temp_dir().join(format!(
            "muxy-client-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory)?;
        let shell = if script.is_empty() {
            PathBuf::from("/bin/sh")
        } else {
            let shell = directory.join("shell");
            fs::write(
                &shell,
                format!(
                    "#!/bin/sh
{script}
exec /bin/sh -l
"
                ),
            )?;
            fs::set_permissions(&shell, fs::Permissions::from_mode(0o700))?;
            shell
        };
        let (sender, events) = mpsc::channel();
        let registry = Arc::new(Registry::new(
            ServerSettings {
                default_shell: Some(shell),
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

    fn connect(&self) -> TestResult<Connection> {
        let (socket, server) = UnixStream::pair()?;
        let server: Box<dyn ByteStream> = Box::new(server);
        let cancellation = server.cancellation()?;
        let (sender, events) = mpsc::channel();
        self.subscribers
            .lock()
            .map_err(|_| "subscriber lock poisoned")?
            .push(sender);
        let registry = Arc::clone(&self.registry);
        let (done, finished) = mpsc::channel();
        thread::spawn(move || {
            let _ = done.send(serve(server, registry, events));
        });
        let client = Client::from_stream(Box::new(socket))?;
        let events = client.events().ok_or("events already taken")?;
        Ok(Connection {
            client,
            events,
            server: cancellation,
            finished,
        })
    }

    fn create(&self, client: &Client) -> TestResult<muxy_protocol::SessionInfo> {
        Ok(client.create_session(&self.directory, SIZE)?)
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

impl Connection {
    fn next_frame(&self, channel: ChannelId) -> TestResult<ScreenFrame> {
        loop {
            match self.events.recv_timeout(TIMEOUT)? {
                ClientEvent::Frame {
                    channel: received,
                    frame,
                } if received == channel => return Ok(frame),
                ClientEvent::Metadata { .. } => {}
                other => return Err(format!("expected frame, got {other:?}").into()),
            }
        }
    }

    fn frame_containing(
        &self,
        attachment: &mut Attachment,
        needle: &str,
    ) -> TestResult<ScreenFrame> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if Instant::now() > deadline {
                return Err("frame never contained expected text".into());
            }
            let frame = self.next_frame(attachment.channel)?;
            attachment.grid.apply(&frame);
            if text(&attachment.grid).contains(needle) {
                return Ok(frame);
            }
            self.client.ack(attachment.channel, frame.seq)?;
        }
    }

    fn quiet(&self, attachment: &mut Attachment) -> TestResult {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match self.events.recv_timeout(QUIET) {
                Ok(ClientEvent::Frame { channel, frame })
                    if channel == attachment.channel && Instant::now() < deadline =>
                {
                    attachment.grid.apply(&frame);
                    self.client.ack(channel, frame.seq)?;
                }
                Ok(ClientEvent::Metadata { .. }) => {}
                Err(RecvTimeoutError::Timeout) => return Ok(()),
                other => return Err(format!("connection did not become quiet: {other:?}").into()),
            }
        }
    }

    fn expect_ended(&self, session: muxy_protocol::SessionId) -> TestResult<ExitReason> {
        loop {
            match self.events.recv_timeout(TIMEOUT)? {
                ClientEvent::SessionEnded {
                    session: ended,
                    reason,
                } if ended == session => return Ok(reason),
                ClientEvent::Frame { .. } | ClientEvent::Metadata { .. } => {}
                other => return Err(format!("expected session ended, got {other:?}").into()),
            }
        }
    }
}

#[test]
fn metadata_crosses_the_connection_and_is_included_in_the_next_attachment() -> TestResult {
    use muxy_protocol::{ForegroundProcess, MetadataEvent};
    let fixture = Fixture::new()?;
    let connection = fixture.connect()?;
    let client = &connection.client;
    let session = fixture.create(client)?;
    let attached = client.attach(session.id, SIZE)?;
    client.send_input(
        attached.channel,
        b"cd /tmp; printf '\\033]0;hello\\007'; sleep 30\n",
    )?;
    let deadline = Instant::now() + TIMEOUT;
    let (mut title, mut directory, mut process) = (false, false, false);
    while !(title && directory && process) {
        match connection
            .events
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))?
        {
            ClientEvent::Metadata { channel, event } => {
                assert_eq!(channel, attached.channel);
                match event {
                    MetadataEvent::Title(value) => title |= value == "hello",
                    MetadataEvent::Directory(path) => directory |= path.0.ends_with(b"/tmp"),
                    MetadataEvent::ForegroundProcess { name, is_shell } => {
                        process |= name == "sleep" && !is_shell;
                    }
                    MetadataEvent::Bell
                    | MetadataEvent::History { .. }
                    | MetadataEvent::InputModes(_)
                    | MetadataEvent::CursorBlinking(_)
                    | MetadataEvent::Links { .. }
                    | MetadataEvent::ScreenPrompts { .. } => {}
                }
            }
            ClientEvent::Frame { .. } => {}
            other => return Err(format!("unexpected event: {other:?}").into()),
        }
    }
    let current = client.attach(session.id, SIZE)?;
    assert_eq!(current.title, "hello");
    assert!(current.directory.0.ends_with(b"/tmp"));
    assert_eq!(
        current.process,
        Some(ForegroundProcess {
            name: "sleep".into(),
            is_shell: false
        })
    );
    client.detach(current.channel)?;
    client.end_session(session.id)?;
    Ok(())
}

fn text(grid: &RunGrid) -> String {
    (0..grid.rows.len())
        .map(|index| grid.row_text(index))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn connect_list_create_attach_input_and_end() -> TestResult {
    let fixture = Fixture::new()?;
    let connection = fixture.connect()?;
    let client = &connection.client;
    assert!(client.is_connected());
    assert_eq!(client.list_sessions()?, vec![]);
    let info = fixture.create(client)?;
    assert_eq!(client.list_sessions()?, vec![info.clone()]);
    let mut attachment = client.attach(info.id, SIZE)?;
    assert_ne!(attachment.channel, CONTROL);
    assert_eq!(attachment.grid.size, SIZE);
    assert_eq!(attachment.grid.rows.len(), usize::from(SIZE.rows));
    assert_eq!(
        attachment.directory.0,
        fixture.directory.canonicalize()?.as_os_str().as_bytes()
    );
    client.send_input(attachment.channel, b"echo hel\"\"lo\n")?;
    connection.frame_containing(&mut attachment, "hello")?;
    client.end_session(info.id)?;
    assert_eq!(connection.expect_ended(info.id)?, ExitReason::Ended);
    assert_eq!(client.list_sessions()?, vec![]);
    Ok(())
}

#[test]
fn a_frame_arrives_only_after_the_previous_one_is_acked() -> TestResult {
    let fixture = Fixture::new()?;
    let connection = fixture.connect()?;
    let client = &connection.client;
    let info = fixture.create(client)?;
    let mut attachment = client.attach(info.id, SIZE)?;
    client.send_input(
        attachment.channel,
        b"stty -echo; PS1=''; printf '\\033[2J\\033[Hready'\n",
    )?;
    let ready = connection.frame_containing(&mut attachment, "ready")?;
    client.ack(attachment.channel, ready.seq)?;
    connection.quiet(&mut attachment)?;
    client.send_input(attachment.channel, b"printf '\\033[1;1Hfirst'\n")?;
    let first = connection.frame_containing(&mut attachment, "first")?;
    client.send_input(attachment.channel, b"printf '\\033[3;1Hsecond'\n")?;
    thread::sleep(QUIET);
    assert!(matches!(
        connection.events.recv_timeout(QUIET),
        Err(RecvTimeoutError::Timeout)
    ));
    client.ping()?;
    client.ack(attachment.channel, first.seq)?;
    let second = connection.frame_containing(&mut attachment, "second")?;
    assert!(second.seq > first.seq);
    assert_eq!(attachment.grid.row_text(0), "first");
    assert_eq!(attachment.grid.row_text(2), "second");
    Ok(())
}

#[test]
fn detach_and_resize_reply_and_a_resize_resets_the_grid() -> TestResult {
    let fixture = Fixture::new()?;
    let connection = fixture.connect()?;
    let client = &connection.client;
    let info = fixture.create(client)?;
    let mut attachment = client.attach(info.id, SIZE)?;
    connection.quiet(&mut attachment)?;
    let size = Size { cols: 60, rows: 20 };
    client.resize(attachment.channel, size)?;
    attachment.grid.resize(size);
    let frame = connection.next_frame(attachment.channel)?;
    assert!(frame.reset);
    attachment.grid.apply(&frame);
    assert_eq!(attachment.grid.rows.len(), 20);
    client.ack(attachment.channel, frame.seq)?;
    client.detach(attachment.channel)?;
    assert!(matches!(
        client.detach(attachment.channel),
        Err(ClientError::Server(error)) if error.code == ErrorCode::UnknownChannel
    ));
    assert_eq!(client.list_sessions()?, vec![info]);
    Ok(())
}

#[test]
fn session_ended_reaches_an_unattached_client() -> TestResult {
    let fixture = Fixture::new()?;
    let owner = fixture.connect()?;
    let observer = fixture.connect()?;
    let info = fixture.create(&owner.client)?;
    let attachment = owner.client.attach(info.id, SIZE)?;
    owner.client.send_input(attachment.channel, b"exit 3\n")?;
    assert_eq!(observer.expect_ended(info.id)?, ExitReason::Exited(3));
    assert_eq!(owner.expect_ended(info.id)?, ExitReason::Exited(3));
    owner.client.ack(attachment.channel, 1)?;
    owner.client.ping()?;
    Ok(())
}

#[test]
fn request_errors_are_correlated_and_invalid_requests_never_leave_the_client() -> TestResult {
    let fixture = Fixture::new()?;
    let connection = fixture.connect()?;
    let client = &connection.client;
    let unknown = muxy_protocol::SessionId::new(1).ok_or("zero session")?;
    assert!(matches!(
        client.attach(unknown, SIZE),
        Err(ClientError::Server(error)) if error.code == ErrorCode::UnknownSession
    ));
    assert!(matches!(
        client.create_session(&fixture.directory, Size { cols: 0, rows: 1 }),
        Err(ClientError::Invalid(ErrorCode::BadSize))
    ));
    assert!(matches!(
        client.send_input(ChannelId(1), &vec![0; muxy_protocol::MAX_INPUT + 1]),
        Err(ClientError::Invalid(ErrorCode::BadRequest))
    ));
    assert!(matches!(
        client.send_input(CONTROL, b"x"),
        Err(ClientError::Invalid(ErrorCode::UnknownChannel))
    ));
    assert!(matches!(
        client.ack(CONTROL, 1),
        Err(ClientError::Invalid(ErrorCode::UnknownChannel))
    ));
    client.ping()?;
    Ok(())
}

#[test]
fn server_exit_disconnects_the_client() -> TestResult {
    let fixture = Fixture::new()?;
    let connection = fixture.connect()?;
    let client = connection.client.clone();
    let info = fixture.create(&client)?;
    let mut attachment = client.attach(info.id, SIZE)?;
    connection.quiet(&mut attachment)?;
    connection.server.cancel();
    connection.finished.recv_timeout(TIMEOUT)??;
    loop {
        match connection.events.recv_timeout(TIMEOUT)? {
            ClientEvent::Disconnected => break,
            ClientEvent::Frame { .. } | ClientEvent::Metadata { .. } => {}
            other @ ClientEvent::SessionEnded { .. } => {
                return Err(format!("expected disconnect, got {other:?}").into());
            }
        }
    }
    assert!(!client.is_connected());
    assert!(matches!(client.ping(), Err(ClientError::Disconnected)));
    assert!(matches!(
        connection.client.ack(attachment.channel, 1),
        Err(ClientError::Disconnected)
    ));
    Ok(())
}

#[test]
fn dropping_the_last_client_closes_the_connection() -> TestResult {
    let fixture = Fixture::new()?;
    let connection = fixture.connect()?;
    let clone = connection.client.clone();
    let Connection {
        client,
        events,
        finished,
        ..
    } = connection;
    drop(client);
    assert!(matches!(
        finished.recv_timeout(QUIET),
        Err(RecvTimeoutError::Timeout)
    ));
    drop(clone);
    finished.recv_timeout(TIMEOUT)??;
    assert_eq!(events.recv_timeout(TIMEOUT)?, ClientEvent::Disconnected);
    Ok(())
}

#[test]
fn a_late_handshake_reply_after_the_connect_timeout_is_closed() -> TestResult {
    let (socket, server) = UnixStream::pair()?;
    let mut decoder = Decoder::new(server.try_clone()?);
    let mut encoder = Encoder::new(server);
    let (release, released) = mpsc::channel();
    let fake = thread::spawn(move || -> Result<(), Box<dyn Error + Send + Sync>> {
        decoder.next()?;
        released.recv_timeout(TIMEOUT)?;
        let sent = encoder.send(
            CONTROL,
            &Message::HelloReply {
                versions: muxy_protocol::SUPPORTED.to_vec(),
            },
        );
        match (sent, decoder.next()) {
            (Err(_), _) | (Ok(()), Err(WireError::Closed)) => Ok(()),
            (Ok(()), other) => Err(format!("client did not close: {other:?}").into()),
        }
    });
    let started = Instant::now();
    assert!(matches!(
        Client::from_stream(Box::new(socket)),
        Err(ClientError::Timeout)
    ));
    assert!(started.elapsed() >= Duration::from_secs(5));
    release.send(())?;
    fake.join()
        .map_err(|_| "fake server panicked")?
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn handshake_rejections_fail_connect() -> TestResult {
    let (socket, server) = UnixStream::pair()?;
    let mut decoder = Decoder::new(server.try_clone()?);
    let mut encoder = Encoder::new(server);
    let fake = thread::spawn(move || -> Result<(), WireError> {
        assert!(matches!(decoder.next()?, (CONTROL, Message::Hello { .. })));
        encoder.send(CONTROL, &Message::VersionUnsupported)
    });
    assert!(matches!(
        Client::from_stream(Box::new(socket)),
        Err(ClientError::VersionUnsupported)
    ));
    fake.join().map_err(|_| "fake server panicked")??;

    let (socket, server) = UnixStream::pair()?;
    drop(server);
    assert!(matches!(
        Client::from_stream(Box::new(socket)),
        Err(ClientError::Disconnected)
    ));
    Ok(())
}

#[test]
fn a_misplaced_server_message_closes_the_client() -> TestResult {
    let (socket, server) = UnixStream::pair()?;
    let mut decoder = Decoder::new(server.try_clone()?);
    let mut encoder = Encoder::new(server.try_clone()?);
    let fake = thread::spawn(move || -> Result<(), WireError> {
        decoder.next()?;
        encoder.send(
            CONTROL,
            &Message::HelloReply {
                versions: muxy_protocol::SUPPORTED.to_vec(),
            },
        )?;
        encoder.send(CONTROL, &Message::Input(b"x".to_vec()))?;
        match decoder.next() {
            Err(WireError::Closed) => Ok(()),
            other => panic!("client did not close: {other:?}"),
        }
    });
    let client = Client::from_stream(Box::new(socket))?;
    let events = client.events().ok_or("events already taken")?;
    assert_eq!(events.recv_timeout(TIMEOUT)?, ClientEvent::Disconnected);
    assert!(matches!(client.ping(), Err(ClientError::Disconnected)));
    fake.join().map_err(|_| "fake server panicked")??;
    drop(server);
    Ok(())
}

#[test]
fn session_directory_round_trips_as_bytes() -> TestResult {
    let fixture = Fixture::new()?;
    let connection = fixture.connect()?;
    let info = fixture.create(&connection.client)?;
    assert_eq!(
        info.directory.0,
        fixture.directory.as_os_str().as_bytes().to_vec()
    );
    Ok(())
}

#[test]
fn history_reads_refresh_a_coherent_boundary_and_merge_all_older_pages() -> TestResult {
    use muxy_protocol::HistoryCursor;
    let fixture = Fixture::new()?;
    let connection = fixture.connect()?;
    let session = fixture.create(&connection.client)?;
    let mut attachment = connection.client.attach(session.id, SIZE)?;
    connection.quiet(&mut attachment)?;
    connection.client.send_input(
        attachment.channel,
        b"stty -echo; PS1=''; printf '\\033[2J\\033[H\\033[3J'; seq 1 5000; printf HISTORY_READY\n",
    )?;
    let deadline = Instant::now() + TIMEOUT;
    let page = loop {
        let page = connection
            .client
            .history_page(attachment.channel, HistoryCursor(0), 200)?;
        if page.screen.as_ref().is_some_and(|screen| {
            screen.rows.iter().any(|row| {
                row.runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
                    .trim_end()
                    == "HISTORY_READY"
            })
        }) {
            break page;
        }
        if Instant::now() >= deadline {
            return Err("history output never completed".into());
        }
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(page.rows.len(), 200);
    attachment.grid.replace_history(page);
    assert_eq!(attachment.grid.row_text(0).trim_end(), "4978");
    while let Some(before) = attachment.grid.history_cursor {
        let page = connection
            .client
            .history_page(attachment.channel, before, 500)?;
        attachment.grid.fetch_older(page);
    }
    let history = attachment
        .grid
        .history
        .iter()
        .map(|row| {
            row.runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        history,
        (1..=4977)
            .map(|number| number.to_string())
            .collect::<Vec<_>>()
    );
    assert!(matches!(
        connection
            .client
            .history_page(CONTROL, HistoryCursor(0), 200),
        Err(ClientError::Invalid(ErrorCode::UnknownChannel))
    ));
    assert!(matches!(
        connection
            .client
            .history_page(attachment.channel, HistoryCursor(0), 501),
        Err(ClientError::Invalid(ErrorCode::BadRequest))
    ));
    connection.client.ping()?;
    Ok(())
}

#[test]
fn mouse_events_reach_the_pty_and_input_modes_survive_reattachment() -> TestResult {
    use muxy_protocol::{
        InputModes, MetadataEvent, Modifiers, MouseAction, MouseButton, MouseEvent, ScrollDirection,
    };
    let fixture = Fixture::new()?;
    let connection = fixture.connect()?;
    let session = fixture.create(&connection.client)?;
    let attachment = connection.client.attach(session.id, SIZE)?;
    assert_eq!(
        wait_input_modes(&connection, attachment.channel)?,
        InputModes::default()
    );
    let expected = b"\x1b[<0;3;4M\x1b[<32;6;4M\x1b[<0;80;24m\x1b[<64;3;4M";
    let command = format!(
        "stty -echo -icanon min 1 time 0; printf '\\033[?1002h\\033[?1006h\\033[?1004h'; dd bs=1 count={} of=mouse.bin 2>/dev/null; printf '\\033[?1002l\\033[?1004l'; stty sane\n",
        expected.len()
    );
    connection
        .client
        .send_input(attachment.channel, command.as_bytes())?;
    let modes = InputModes {
        mouse_tracking: true,
        alternate_scroll: false,
        focus_events: true,
    };
    assert_eq!(wait_input_modes(&connection, attachment.channel)?, modes);
    connection.client.detach(attachment.channel)?;
    let attachment = connection.client.attach(session.id, SIZE)?;
    assert_eq!(wait_input_modes(&connection, attachment.channel)?, modes);
    let press = MouseEvent {
        action: MouseAction::Press,
        button: Some(MouseButton::Left),
        column: 2,
        row: 3,
        scroll: None,
        modifiers: Modifiers::default(),
    };
    for event in [
        press,
        MouseEvent {
            action: MouseAction::Motion,
            column: 5,
            ..press
        },
        MouseEvent {
            action: MouseAction::Release,
            column: u16::MAX,
            row: u16::MAX,
            ..press
        },
        MouseEvent {
            action: MouseAction::Scroll,
            button: None,
            scroll: Some(ScrollDirection::Up),
            ..press
        },
    ] {
        connection.client.send_mouse(attachment.channel, event)?;
    }
    assert_eq!(
        wait_input_modes(&connection, attachment.channel)?,
        InputModes::default()
    );
    let path = fixture.directory.join("mouse.bin");
    assert_eq!(fs::read(&path)?, expected);
    fs::remove_file(path)?;
    connection.client.detach(attachment.channel)?;
    connection.client.send_mouse(attachment.channel, press)?;
    connection.client.ping()?;
    assert!(!connection.events.try_iter().any(|event| matches!(event, ClientEvent::Metadata { event: MetadataEvent::InputModes(modes), .. } if modes.mouse_tracking)));
    Ok(())
}

fn wait_input_modes(
    connection: &Connection,
    expected: ChannelId,
) -> TestResult<muxy_protocol::InputModes> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match connection
            .events
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))?
        {
            ClientEvent::Metadata {
                channel,
                event: muxy_protocol::MetadataEvent::InputModes(modes),
            } if channel == expected => return Ok(modes),
            ClientEvent::Metadata { .. } => {}
            ClientEvent::Frame { channel, frame } => connection.client.ack(channel, frame.seq)?,
            event => return Err(format!("expected input modes, got {event:?}").into()),
        }
    }
}

#[test]
fn explicit_disconnect_closes_all_clones_without_ending_sessions() -> TestResult {
    let fixture = Fixture::new()?;
    let connection = fixture.connect()?;
    let clone = connection.client.clone();
    let session = fixture.create(&clone)?;
    connection.client.disconnect();
    connection.finished.recv_timeout(TIMEOUT)??;
    assert_eq!(
        connection.events.recv_timeout(TIMEOUT)?,
        ClientEvent::Disconnected
    );
    assert!(matches!(clone.ping(), Err(ClientError::Disconnected)));
    assert!(fixture.registry.handle(session.id).is_some());
    Ok(())
}
