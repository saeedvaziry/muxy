use std::error::Error;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use muxy_protocol::{
    CONTROL, ChannelId, ExitReason, Message, ReplyBody, RequestBody, RequestId, SUPPORTED,
    ServerPath, SessionId, Size,
};
use muxy_transport::{StreamCancellation, connect};
use muxy_wire::{Decoder, Encoder, WireError};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;
const TIMEOUT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(10);
const SIZE: Size = Size { cols: 80, rows: 24 };
static NEXT: AtomicU64 = AtomicU64::new(1);

struct Fixture {
    directory: PathBuf,
    child: Option<Child>,
}

impl Fixture {
    fn new() -> TestResult<Self> {
        let directory = std::env::temp_dir().join(format!(
            "mx8-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory)?;
        Ok(Self {
            directory,
            child: None,
        })
    }

    fn socket(&self) -> PathBuf {
        self.directory.join("server.sock")
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_muxy-server"));
        command
            .env("MUXY_DIR", &self.directory)
            .env("SHELL", "/bin/sh")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn start(&mut self) -> TestResult {
        let command = self.command();
        self.start_command(command, &self.socket())
    }

    fn start_command(&mut self, mut command: Command, socket: &Path) -> TestResult {
        self.child = Some(command.spawn()?);
        wait_until(|| {
            if !fs::symlink_metadata(socket).is_ok_and(|metadata| metadata.file_type().is_socket())
            {
                return Ok(false);
            }
            match UnixStream::connect(socket) {
                Ok(_) => Ok(true),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                    ) =>
                {
                    Ok(false)
                }
                Err(error) => Err(error.into()),
            }
        })
    }

    fn signal(&self, signal: &str) -> TestResult {
        let child = self.child.as_ref().ok_or("server is not running")?;
        let output = Command::new("kill")
            .args([signal, &child.id().to_string()])
            .output()?;
        assert!(output.status.success(), "{output:?}");
        Ok(())
    }

    fn finish(&mut self) -> TestResult<Output> {
        wait_until(|| {
            Ok(self
                .child
                .as_mut()
                .ok_or("server is not running")?
                .try_wait()?
                .is_some())
        })?;
        Ok(self
            .child
            .take()
            .ok_or("server is not running")?
            .wait_with_output()?)
    }

    fn stop(&mut self, signal: &str) -> TestResult {
        self.signal(signal)?;
        let output = self.finish()?;
        assert!(output.status.success(), "{output:?}");
        Ok(())
    }

    fn output(&mut self, mut command: Command) -> TestResult<Output> {
        self.child = Some(command.spawn()?);
        self.finish()
    }

    fn log(&self) -> TestResult<String> {
        Ok(fs::read_to_string(self.directory.join("server.log"))?)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.directory);
    }
}

struct Client {
    encoder: Encoder<Box<dyn Write + Send>>,
    incoming: Receiver<Result<(ChannelId, Message), WireError>>,
    cancellation: Box<dyn StreamCancellation>,
    next_request: u32,
}

impl Client {
    fn new(path: &Path) -> TestResult<Self> {
        let stream = connect(path)?;
        let cancellation = stream.cancellation()?;
        let (read, write) = stream.split()?;
        let mut decoder = Decoder::new(read);
        let (sender, incoming) = mpsc::channel();
        thread::spawn(move || {
            loop {
                let received = decoder.next();
                let closed = received.is_err();
                if sender.send(received).is_err() || closed {
                    break;
                }
            }
        });
        let mut client = Self {
            encoder: Encoder::new(write),
            incoming,
            cancellation,
            next_request: 1,
        };
        client.encoder.send(
            CONTROL,
            &Message::Hello {
                versions: SUPPORTED.to_vec(),
            },
        )?;
        assert!(matches!(
            client.receive()?,
            (CONTROL, Message::HelloReply { .. })
        ));
        Ok(client)
    }

    fn receive(&self) -> TestResult<(ChannelId, Message)> {
        Ok(self.incoming.recv_timeout(TIMEOUT)??)
    }

    fn request(&mut self, body: RequestBody) -> TestResult<ReplyBody> {
        let id = RequestId(self.next_request);
        self.next_request += 1;
        self.encoder.send(CONTROL, &Message::Request { id, body })?;
        loop {
            match self.receive()? {
                (CONTROL, Message::Reply { id: received, body }) if id == received => {
                    return Ok(body);
                }
                (channel, Message::Frame(frame)) => self.encoder.send(
                    CONTROL,
                    &Message::FrameAck {
                        channel,
                        seq: frame.seq,
                    },
                )?,
                (_, Message::Metadata(_)) => {}
                other => return Err(format!("unexpected reply: {other:?}").into()),
            }
        }
    }

    fn create(&mut self, directory: &Path) -> TestResult<SessionId> {
        match self.request(RequestBody::CreateSession {
            directory: ServerPath(directory.as_os_str().as_encoded_bytes().to_vec()),
            size: SIZE,
        })? {
            ReplyBody::SessionCreated(info) => Ok(info.id),
            other => Err(format!("expected session, got {other:?}").into()),
        }
    }

    fn attach(&mut self, session: SessionId) -> TestResult<ChannelId> {
        match self.request(RequestBody::Attach {
            session,
            size: SIZE,
        })? {
            ReplyBody::Attached { snapshot, .. } => Ok(snapshot.channel),
            other => Err(format!("expected snapshot, got {other:?}").into()),
        }
    }

    fn ended(&self, expected: &[SessionId], reason: ExitReason) -> TestResult {
        let mut remaining = expected.to_vec();
        while !remaining.is_empty() {
            match self.receive()? {
                (
                    CONTROL,
                    Message::SessionEnded {
                        session,
                        reason: received,
                    },
                ) => {
                    assert_eq!(received, reason);
                    assert!(remaining.contains(&session));
                    remaining.retain(|id| *id != session);
                }
                (_, Message::Frame(_) | Message::Metadata(_)) => {}
                other => return Err(format!("expected session end, got {other:?}").into()),
            }
        }
        Ok(())
    }

    fn closed(&self) -> TestResult {
        assert!(matches!(
            self.incoming.recv_timeout(TIMEOUT)?,
            Err(WireError::Closed)
        ));
        Ok(())
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

fn wait_until(mut condition: impl FnMut() -> TestResult<bool>) -> TestResult {
    let deadline = Instant::now() + TIMEOUT;
    while !condition()? {
        if Instant::now() >= deadline {
            return Err("timed out".into());
        }
        thread::sleep(POLL);
    }
    Ok(())
}

#[test]
fn lifecycle_serves_shell_logs_and_stops_every_session() -> TestResult {
    let mut fixture = Fixture::new()?;
    fixture.start()?;
    let mut creator = Client::new(&fixture.socket())?;
    assert_eq!(creator.request(RequestBody::Ping)?, ReplyBody::Pong);
    let mut observer = Client::new(&fixture.socket())?;
    let first = creator.create(&fixture.directory)?;
    let second = creator.create(&fixture.directory)?;
    let channel = creator.attach(first)?;
    creator.encoder.send(
        channel,
        &Message::Input(b"printf 'phase-eight-ready\\n'\n".to_vec()),
    )?;
    loop {
        let (channel, message) = creator.receive()?;
        if let Message::Frame(frame) = message {
            creator.encoder.send(
                CONTROL,
                &Message::FrameAck {
                    channel,
                    seq: frame.seq,
                },
            )?;
            if frame.rows.iter().any(|row| {
                row.runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
                    .trim()
                    == "phase-eight-ready"
            }) {
                break;
            }
        }
    }
    assert!(
        matches!(observer.request(RequestBody::ListSessions)?, ReplyBody::Sessions(sessions) if sessions.len() == 2)
    );
    let defaults = fs::read_to_string(fixture.directory.join("server.toml"))?;
    assert_eq!(defaults.trim(), "history_budget_bytes = 16777216");
    fixture.signal("-TERM")?;
    creator.ended(&[first, second], ExitReason::ServerStopped)?;
    observer.ended(&[first, second], ExitReason::ServerStopped)?;
    creator.closed()?;
    observer.closed()?;
    assert!(fixture.finish()?.status.success());
    assert!(!fixture.socket().exists());
    let log = fixture.log()?;
    for event in [
        "server started: socket=",
        "client connected:",
        "client disconnected:",
        "session created:",
        "session ended:",
        "ServerStopped",
        "server stopped",
    ] {
        assert!(log.contains(event), "missing {event}: {log}");
    }
    Ok(())
}

#[test]
fn second_instance_exits_successfully_and_preserves_the_first() -> TestResult {
    let mut fixture = Fixture::new()?;
    fixture.start()?;
    let mut client = Client::new(&fixture.socket())?;
    let mut second = Fixture::new()?;
    let output = second.output(fixture.command())?;
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout)?.trim(),
        format!(
            "muxy-server already running at {}",
            fixture.socket().display()
        )
    );
    assert_eq!(client.request(RequestBody::Ping)?, ReplyBody::Pong);
    fixture.stop("-INT")?;
    client.closed()?;
    assert!(!fixture.socket().exists());
    Ok(())
}

#[test]
fn shutdown_cancels_clients_that_never_finish_hello() -> TestResult {
    let mut fixture = Fixture::new()?;
    fixture.start()?;
    let mut stalled = UnixStream::connect(fixture.socket())?;
    stalled.set_read_timeout(Some(TIMEOUT))?;
    let _ready = Client::new(&fixture.socket())?;
    fixture.stop("-TERM")?;
    assert_eq!(stalled.read(&mut [0])?, 0);
    assert!(!fixture.socket().exists());
    Ok(())
}

#[test]
fn shutdown_interrupts_a_blocked_pty_write() -> TestResult {
    assert_shutdown_interrupts_input(
        b"stty -icanon -echo; printf 'nonreading-ready\\n'; exec sleep 3\n",
    )
}

#[test]
fn shutdown_does_not_wait_for_a_descendant_holding_the_pty() -> TestResult {
    assert_shutdown_interrupts_input(
        b"stty -icanon -echo; trap '' HUP; sleep 3 & printf 'nonreading-ready\\n'; wait\n",
    )
}

fn assert_shutdown_interrupts_input(command: &[u8]) -> TestResult {
    let mut fixture = Fixture::new()?;
    fixture.start()?;
    let mut client = Client::new(&fixture.socket())?;
    let session = client.create(&fixture.directory)?;
    let channel = client.attach(session)?;
    client
        .encoder
        .send(channel, &Message::Input(command.to_vec()))?;
    loop {
        let (received, message) = client.receive()?;
        if let Message::Frame(frame) = message {
            client.encoder.send(
                CONTROL,
                &Message::FrameAck {
                    channel: received,
                    seq: frame.seq,
                },
            )?;
            if frame.rows.iter().any(|row| {
                row.runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
                    .trim()
                    == "nonreading-ready"
            }) {
                break;
            }
        }
    }
    client
        .encoder
        .send(channel, &Message::Input(vec![b'x'; 65_536]))?;
    assert_eq!(client.request(RequestBody::Ping)?, ReplyBody::Pong);
    thread::sleep(Duration::from_millis(50));
    let started = Instant::now();
    fixture.signal("-TERM")?;
    client.ended(&[session], ExitReason::ServerStopped)?;
    client.closed()?;
    assert!(fixture.finish()?.status.success());
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(!fixture.socket().exists());
    Ok(())
}

#[test]
fn shutdown_stops_a_session_after_its_output_closes() -> TestResult {
    let mut fixture = Fixture::new()?;
    fixture.start()?;
    let mut client = Client::new(&fixture.socket())?;
    let session = client.create(&fixture.directory)?;
    let channel = client.attach(session)?;
    client.encoder.send(
        channel,
        &Message::Input(b"exec </dev/null >/dev/null 2>&1; exec sleep 3\n".to_vec()),
    )?;
    assert_eq!(client.request(RequestBody::Ping)?, ReplyBody::Pong);
    thread::sleep(Duration::from_millis(50));
    let started = Instant::now();
    fixture.signal("-TERM")?;
    client.ended(&[session], ExitReason::ServerStopped)?;
    client.closed()?;
    assert!(fixture.finish()?.status.success());
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(!fixture.socket().exists());
    Ok(())
}

#[test]
fn malformed_settings_name_the_key_and_cleanup_the_socket() -> TestResult {
    let mut fixture = Fixture::new()?;
    let path = fixture.directory.join("server.toml");
    fs::write(&path, "unexpected_key = true\n")?;
    let output = fixture.output(fixture.command())?;
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)?.contains("unexpected_key"));
    assert_eq!(fs::read_to_string(path)?, "unexpected_key = true\n");
    assert!(!fixture.socket().exists());
    Ok(())
}

#[test]
fn custom_paths_and_settings_are_used_without_default_directory() -> TestResult {
    let mut fixture = Fixture::new()?;
    let socket = fixture.directory.join("custom.sock");
    let settings = fixture.directory.join("custom.toml");
    let log = fixture.directory.join("custom.log");
    fs::write(
        &settings,
        "default_shell = '/missing/muxy-shell'\nhistory_budget_bytes = 4096\n",
    )?;
    let mut command = fixture.command();
    command
        .env_remove("MUXY_DIR")
        .env_remove("HOME")
        .arg("--socket")
        .arg(&socket)
        .arg("--settings")
        .arg(&settings)
        .arg("--log")
        .arg(&log);
    fixture.start_command(command, &socket)?;
    let mut client = Client::new(&socket)?;
    assert!(matches!(client.request(RequestBody::CreateSession {
        directory: ServerPath(fixture.directory.as_os_str().as_encoded_bytes().to_vec()),
        size: SIZE,
    })?, ReplyBody::Error(error) if error.code == muxy_protocol::ErrorCode::SpawnFailed));
    fixture.stop("-INT")?;
    assert!(fs::read_to_string(log)?.contains("server started:"));
    assert!(!socket.exists());
    assert!(!fixture.directory.join("server.toml").exists());
    Ok(())
}

#[test]
fn default_alpha_directory_does_not_touch_stable_storage() -> TestResult {
    let mut fixture = Fixture::new()?;
    let stable = fixture.directory.join("Library/Application Support/Muxy");
    fs::create_dir_all(&stable)?;
    fs::write(
        stable.join("server.toml"),
        "stable settings must not be read",
    )?;
    let directory = fixture
        .directory
        .join("Library/Application Support/Muxy Alpha");
    let socket = fixture.socket();
    let mut command = fixture.command();
    command
        .env_remove("MUXY_DIR")
        .env("HOME", &fixture.directory)
        .arg("--socket")
        .arg(&socket);
    fixture.start_command(command, &socket)?;
    let mut client = Client::new(&socket)?;
    assert_eq!(client.request(RequestBody::Ping)?, ReplyBody::Pong);
    assert!(directory.join("server.toml").exists());
    assert!(directory.join("server.log").exists());
    assert_eq!(
        fs::read_to_string(stable.join("server.toml"))?,
        "stable settings must not be read"
    );
    assert!(!stable.join("server.log").exists());
    assert!(!stable.join("server.sock").exists());
    fixture.stop("-TERM")?;
    Ok(())
}

#[test]
fn socket_length_and_invalid_arguments_fail_clearly() -> TestResult {
    let mut fixture = Fixture::new()?;
    for (arguments, expected) in [
        (vec!["--socket".to_owned(), "a".repeat(104)], "104 bytes"),
        (vec!["--socket".to_owned()], "requires a path"),
        (vec!["--stdio".to_owned()], "unknown argument: --stdio"),
        (
            vec!["--log".to_owned(), "--settings".to_owned()],
            "requires a path",
        ),
        (
            vec![
                "--log".to_owned(),
                "a".to_owned(),
                "--log".to_owned(),
                "b".to_owned(),
            ],
            "duplicate argument",
        ),
    ] {
        let mut command = fixture.command();
        command.args(arguments);
        let output = fixture.output(command)?;
        assert!(!output.status.success());
        assert!(String::from_utf8(output.stderr)?.contains(expected));
    }
    Ok(())
}

#[test]
fn fatal_protocol_errors_are_logged_before_and_after_hello() -> TestResult {
    let mut fixture = Fixture::new()?;
    fixture.start()?;
    let socket = UnixStream::connect(fixture.socket())?;
    socket.set_read_timeout(Some(TIMEOUT))?;
    Encoder::new(socket.try_clone()?).send(CONTROL, &Message::VersionUnsupported)?;
    assert!(matches!(
        Decoder::new(socket).next()?,
        (CONTROL, Message::Fatal(_))
    ));
    let mut client = Client::new(&fixture.socket())?;
    client.encoder.send(CONTROL, &Message::VersionUnsupported)?;
    assert!(matches!(client.receive()?, (CONTROL, Message::Fatal(_))));
    client.closed()?;
    fixture.stop("-TERM")?;
    let log = fixture.log()?;
    assert!(log.contains("fatal protocol error: expected Hello on control"));
    assert!(log.contains("fatal protocol error: misplaced client message"));
    Ok(())
}
