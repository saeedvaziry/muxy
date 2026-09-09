use std::error::Error;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};
use std::{env, fs, process, thread};

use muxy_protocol::{
    AttachSnapshot, ChannelId, ErrorCode, ExitReason, Row, ScreenFrame, SessionInfo, Size,
};
use muxy_server_core::{
    AttachmentEvent, AttachmentId, Registry, ServerEvent, ServerSettings, SessionCommand,
    SessionHandle,
};

type TestResult = Result<(), Box<dyn Error>>;

const TIMEOUT: Duration = Duration::from_secs(10);
const QUIET: Duration = Duration::from_millis(500);
const SIZE: Size = Size { cols: 80, rows: 24 };

struct Fixture {
    registry: Registry,
    events: Receiver<ServerEvent>,
    directory: PathBuf,
}

#[test]
fn create_lists_the_session_and_attach_delivers_a_snapshot() -> TestResult {
    let fixture = fixture("snapshot")?;
    let (info, handle) = session(&fixture)?;

    assert_eq!(
        info.directory.0,
        fixture.directory.to_string_lossy().as_bytes()
    );
    assert_eq!(fixture.registry.list(), vec![info.clone()]);
    assert_eq!(handle.id(), info.id);

    let (_events, snapshot) = attach(&handle, 1, SIZE)?;

    assert_eq!(snapshot.channel, ChannelId(1));
    assert_eq!(snapshot.size, SIZE);
    assert_eq!(snapshot.rows.len(), usize::from(SIZE.rows));
    assert_eq!(
        snapshot.directory.0,
        fixture.directory.canonicalize()?.as_os_str().as_bytes()
    );
    fixture.finish()
}

#[test]
fn echoed_input_arrives_in_a_frame() -> TestResult {
    let fixture = fixture("echo")?;
    let (_, handle) = session(&fixture)?;
    let (events, _) = attach(&handle, 1, SIZE)?;

    handle.send(SessionCommand::Input(b"echo muxy-ok\n".to_vec()))?;
    let frame = wait_for_text(&events, "muxy-ok")?;

    assert!(!frame.reset);
    assert!(frame.seq >= 1);
    fixture.finish()
}

#[test]
fn a_second_attachment_gets_its_own_snapshot_and_the_same_frames() -> TestResult {
    let fixture = fixture("second")?;
    let (_, handle) = session(&fixture)?;
    let (first, _) = attach(&handle, 1, SIZE)?;
    let (second, snapshot) = attach(&handle, 2, SIZE)?;

    assert_eq!(snapshot.channel, ChannelId(2));
    assert_eq!(snapshot.size, SIZE);

    handle.send(SessionCommand::Input(b"echo muxy-ok\n".to_vec()))?;
    let on_first = wait_for_text(&first, "muxy-ok")?;
    let on_second = wait_for_text(&second, "muxy-ok")?;

    assert_eq!(on_first.rows, on_second.rows);
    assert_eq!(on_second.seq, 1);

    handle.send(SessionCommand::Detach(AttachmentId(2)))?;
    handle.send(SessionCommand::Input(b"echo muxy-again\n".to_vec()))?;
    wait_for_text(&first, "muxy-again")?;

    assert!(no_frame_containing(&second, "muxy-again"));
    fixture.finish()
}

#[test]
fn resize_sends_a_full_reset_frame_of_the_new_size() -> TestResult {
    let fixture = fixture("resize")?;
    let (_, handle) = session(&fixture)?;
    let (events, _) = attach(&handle, 1, SIZE)?;
    let smaller = Size { cols: 60, rows: 20 };

    handle.send(SessionCommand::Resize(smaller))?;
    let frame = wait_for_frame(&events, |frame| frame.reset)?;

    assert_eq!(frame.rows.len(), usize::from(smaller.rows));
    assert!(frame.rows.iter().all(|row| row.index < smaller.rows));
    fixture.finish()
}

#[test]
fn exit_ends_the_session_and_removes_it_from_the_listing() -> TestResult {
    let fixture = fixture("exit")?;
    let (info, handle) = session(&fixture)?;
    let (events, _) = attach(&handle, 1, SIZE)?;

    handle.send(SessionCommand::Input(b"exit 0\n".to_vec()))?;

    assert_eq!(wait_for_ended(&events)?, ExitReason::Exited(0));
    assert_eq!(
        fixture.events.recv_timeout(TIMEOUT)?,
        ServerEvent::SessionEnded {
            id: info.id,
            reason: ExitReason::Exited(0)
        }
    );
    assert!(fixture.registry.list().is_empty());
    assert!(fixture.registry.handle(info.id).is_none());
    assert_eq!(
        fixture.registry.end(info.id).map_err(|error| error.code()),
        Err(ErrorCode::UnknownSession)
    );
    fixture.finish()
}

#[test]
fn end_kills_a_running_program() -> TestResult {
    let fixture = fixture("end")?;
    let (info, handle) = session(&fixture)?;
    let (events, _) = attach(&handle, 1, SIZE)?;

    handle.send(SessionCommand::Input(b"cat\n".to_vec()))?;
    handle.send(SessionCommand::Input(b"ping\n".to_vec()))?;
    wait_for_text(&events, "ping\nping")?;

    fixture.registry.end(info.id)?;

    assert_eq!(wait_for_ended(&events)?, ExitReason::Ended);
    assert_eq!(
        fixture.events.recv_timeout(TIMEOUT)?,
        ServerEvent::SessionEnded {
            id: info.id,
            reason: ExitReason::Ended
        }
    );
    assert!(fixture.registry.list().is_empty());
    fixture.finish()
}

#[test]
fn creating_in_a_missing_directory_is_a_bad_path_error() -> TestResult {
    let fixture = fixture("missing")?;
    let missing = fixture.directory.join("missing");

    let error = fixture
        .registry
        .create(&missing, SIZE)
        .err()
        .ok_or("create succeeded in a missing directory")?;

    assert_eq!(error.code(), ErrorCode::BadPath);
    assert!(fixture.registry.list().is_empty());
    fixture.finish()
}

#[test]
fn creating_with_a_zero_size_is_a_bad_size_error() -> TestResult {
    let fixture = fixture("size")?;

    let error = fixture
        .registry
        .create(&fixture.directory, Size { cols: 0, rows: 24 })
        .err()
        .ok_or("create succeeded with zero columns")?;

    assert_eq!(error.code(), ErrorCode::BadSize);
    fixture.finish()
}

#[test]
fn no_frame_arrives_while_idle() -> TestResult {
    let fixture = fixture("idle")?;
    let (_, handle) = session(&fixture)?;
    let (events, _) = attach(&handle, 1, SIZE)?;

    handle.send(SessionCommand::Input(b"echo muxy-ok\n".to_vec()))?;
    wait_for_text(&events, "muxy-ok")?;
    drain_until_quiet(&events)?;

    assert_eq!(
        events.recv_timeout(QUIET).err(),
        Some(RecvTimeoutError::Timeout)
    );
    fixture.finish()
}

#[test]
fn a_cursor_position_query_is_answered_through_the_pty() -> TestResult {
    let fixture = fixture("cursor")?;
    let (_, handle) = session(&fixture)?;
    let (events, _) = attach(&handle, 1, SIZE)?;

    handle.send(SessionCommand::Input(b"echo muxy-ok\n".to_vec()))?;
    wait_for_text(&events, "muxy-ok")?;
    handle.send(SessionCommand::Input(
        b"stty -icanon min 1 time 5; printf '\\033[6n'; dd bs=64 count=1 2>/dev/null | tr -d '\\033'; stty icanon\n".to_vec(),
    ))?;

    wait_for_text(&events, ";1R")?;
    fixture.finish()
}

fn fixture(name: &str) -> Result<Fixture, Box<dyn Error>> {
    let directory = env::temp_dir().join(format!("muxy-server-core-{}-{name}", process::id()));
    fs::create_dir_all(&directory)?;
    let (sender, events) = channel();
    let settings = ServerSettings {
        default_shell: Some(PathBuf::from("/bin/sh")),
        ..ServerSettings::default()
    };
    Ok(Fixture {
        registry: Registry::new(settings, sender),
        events,
        directory,
    })
}

impl Fixture {
    fn finish(self) -> TestResult {
        for session in self.registry.list() {
            self.registry.end(session.id)?;
        }
        let deadline = Instant::now() + TIMEOUT;
        while !self.registry.list().is_empty() {
            if Instant::now() > deadline {
                return Err("sessions did not end".into());
            }
            thread::sleep(Duration::from_millis(20));
        }
        fs::remove_dir(&self.directory)?;
        Ok(())
    }
}

fn session(fixture: &Fixture) -> Result<(SessionInfo, SessionHandle), Box<dyn Error>> {
    let info = fixture.registry.create(&fixture.directory, SIZE)?;
    let handle = fixture
        .registry
        .handle(info.id)
        .ok_or("created session has no handle")?;
    Ok((info, handle))
}

fn attach(
    handle: &SessionHandle,
    id: u64,
    size: Size,
) -> Result<(Receiver<AttachmentEvent>, AttachSnapshot), Box<dyn Error>> {
    let (sink, events) = channel();
    handle.send(SessionCommand::Attach {
        id: AttachmentId(id),
        channel: ChannelId(u32::try_from(id)?),
        size,
        sink,
    })?;
    match events.recv_timeout(TIMEOUT)? {
        AttachmentEvent::Snapshot { snapshot, .. } => Ok((events, snapshot)),
        other => Err(format!("expected a snapshot, got {other:?}").into()),
    }
}

fn wait_for_text(
    events: &Receiver<AttachmentEvent>,
    needle: &str,
) -> Result<ScreenFrame, Box<dyn Error>> {
    wait_for_frame(events, |frame| text(&frame.rows).contains(needle))
}

fn wait_for_frame(
    events: &Receiver<AttachmentEvent>,
    accept: impl Fn(&ScreenFrame) -> bool,
) -> Result<ScreenFrame, Box<dyn Error>> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(remaining)? {
            AttachmentEvent::Frame(frame) if accept(&frame) => return Ok(frame),
            AttachmentEvent::Frame(_) | AttachmentEvent::Metadata(_) => {}
            other => return Err(format!("expected a frame, got {other:?}").into()),
        }
    }
}

fn wait_for_ended(events: &Receiver<AttachmentEvent>) -> Result<ExitReason, Box<dyn Error>> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(remaining)? {
            AttachmentEvent::Ended(reason) => return Ok(reason),
            AttachmentEvent::Frame(_)
            | AttachmentEvent::Resized(_)
            | AttachmentEvent::Metadata(_) => {}
            AttachmentEvent::Snapshot { .. } => {
                return Err("expected a frame or ended, got a snapshot".into());
            }
        }
    }
}

fn drain_until_quiet(events: &Receiver<AttachmentEvent>) -> TestResult {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match events.recv_timeout(QUIET) {
            Ok(AttachmentEvent::Frame(_) | AttachmentEvent::Metadata(_))
                if Instant::now() < deadline => {}
            Ok(_) => return Err("session never became idle".into()),
            Err(RecvTimeoutError::Timeout) => return Ok(()),
            Err(RecvTimeoutError::Disconnected) => return Err("session went away".into()),
        }
    }
}

fn no_frame_containing(events: &Receiver<AttachmentEvent>, needle: &str) -> bool {
    loop {
        match events.recv_timeout(QUIET) {
            Ok(AttachmentEvent::Frame(frame)) if text(&frame.rows).contains(needle) => {
                return false;
            }
            Ok(_) => {}
            Err(_) => return true,
        }
    }
}

fn text(rows: &[Row]) -> String {
    let lines: Vec<String> = rows
        .iter()
        .map(|row| row.runs.iter().map(|run| run.text.as_str()).collect())
        .collect();
    lines.join("\n")
}
