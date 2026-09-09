use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use muxy_protocol::{
    AttachSnapshot, ChannelId, ForegroundProcess, MetadataEvent, ServerPath, Size,
};
use muxy_server_core::{
    AttachmentEvent, AttachmentId, Registry, ServerSettings, SessionCommand, SessionHandle,
};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;
const SIZE: Size = Size { cols: 80, rows: 24 };
const TIMEOUT: Duration = Duration::from_secs(5);

struct Fixture {
    registry: Registry,
    handle: SessionHandle,
}

impl Fixture {
    fn new() -> TestResult<Self> {
        Self::with_shell("/bin/sh")
    }

    fn with_shell(shell: &str) -> TestResult<Self> {
        let (events, _) = mpsc::channel();
        let registry = Registry::new(
            ServerSettings {
                default_shell: Some(PathBuf::from(shell)),
                ..ServerSettings::default()
            },
            events,
        );
        let session = registry.create(Path::new("/"), SIZE)?;
        let handle = registry.handle(session.id).ok_or("missing session")?;
        Ok(Self { registry, handle })
    }

    fn input(&self, bytes: &[u8]) -> TestResult {
        self.handle.send(SessionCommand::Input(bytes.to_vec()))?;
        Ok(())
    }

    fn attach(
        &self,
        id: u32,
    ) -> TestResult<(
        Receiver<AttachmentEvent>,
        AttachSnapshot,
        Option<ForegroundProcess>,
    )> {
        let (sink, events) = mpsc::channel();
        self.handle.send(SessionCommand::Attach {
            id: AttachmentId(u64::from(id)),
            channel: ChannelId(id),
            size: SIZE,
            sink,
        })?;
        match events.recv_timeout(TIMEOUT)? {
            AttachmentEvent::Snapshot { snapshot, process } => Ok((events, snapshot, process)),
            other => Err(format!("expected snapshot, got {other:?}").into()),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.registry.shutdown();
        let deadline = Instant::now() + TIMEOUT;
        while !self.registry.is_stopped() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn metadata(
    events: &Receiver<AttachmentEvent>,
    accept: impl Fn(&MetadataEvent) -> bool,
) -> TestResult<MetadataEvent> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now()))? {
            AttachmentEvent::Metadata(event) if accept(&event) => return Ok(event),
            AttachmentEvent::Frame(_) | AttachmentEvent::Metadata(_) => {}
            other => return Err(format!("unexpected session event: {other:?}").into()),
        }
    }
}

#[test]
fn foreground_process_changes_while_silent_and_returns_to_the_spawned_shell() -> TestResult {
    let fixture = Fixture::new()?;
    let (events, _, _) = fixture.attach(1)?;
    fixture.input(b"stty -echo; PS1=''; sleep 30\n")?;
    metadata(
        &events,
        |event| matches!(event, MetadataEvent::ForegroundProcess { name, is_shell: false } if name == "sleep"),
    )?;
    let (_, _, process) = fixture.attach(2)?;
    assert_eq!(
        process,
        Some(ForegroundProcess {
            name: "sleep".into(),
            is_shell: false
        })
    );
    fixture.input(b"\x03")?;
    metadata(&events, |event| {
        matches!(
            event,
            MetadataEvent::ForegroundProcess { is_shell: true, .. }
        )
    })?;
    let (_, _, process) = fixture.attach(3)?;
    assert!(process.is_some_and(|process| process.is_shell));
    Ok(())
}

#[test]
fn a_pipeline_keeps_non_shell_metadata_after_its_group_leader_exits() -> TestResult {
    for shell in ["/bin/sh", "/bin/zsh"] {
        let fixture = Fixture::with_shell(shell)?;
        let (events, _, initial) = fixture.attach(1)?;
        assert!(initial.is_some_and(|process| process.is_shell), "{shell}");
        fixture.input(b"stty -echo; PS1=''; echo x | (cd /tmp; exec sleep 30)\n")?;
        metadata(
            &events,
            |event| matches!(event, MetadataEvent::ForegroundProcess { name, is_shell: false } if name == "sleep"),
        )?;
        let (_, snapshot, process) = fixture.attach(2)?;
        assert_eq!(
            process,
            Some(ForegroundProcess {
                name: "sleep".into(),
                is_shell: false
            }),
            "{shell}"
        );
        assert!(snapshot.directory.0.ends_with(b"/tmp"), "{shell}");
        fixture.input(b"\x03")?;
        metadata(&events, |event| {
            matches!(
                event,
                MetadataEvent::ForegroundProcess { is_shell: true, .. }
            )
        })?;
    }
    Ok(())
}

#[test]
fn idle_polling_detects_a_foreground_change_without_output() -> TestResult {
    let fixture = Fixture::new()?;
    let (events, _, _) = fixture.attach(1)?;
    fixture.input(b"stty -echo; PS1=''; sleep 0.3; tail -f /dev/null\n")?;
    metadata(
        &events,
        |event| matches!(event, MetadataEvent::ForegroundProcess { name, .. } if name == "sleep"),
    )?;
    let deadline = Instant::now() + Duration::from_millis(1500);
    metadata(
        &events,
        |event| matches!(event, MetadataEvent::ForegroundProcess { name, is_shell: false } if name == "tail"),
    )?;
    assert!(Instant::now() < deadline);
    Ok(())
}

#[test]
fn directory_changes_without_shell_integration_and_metadata_is_current_on_attach() -> TestResult {
    let fixture = Fixture::new()?;
    let (events, _, _) = fixture.attach(1)?;
    fixture.input(b"cd /tmp\n")?;
    metadata(
        &events,
        |event| matches!(event, MetadataEvent::Directory(path) if path.0.ends_with(b"/tmp")),
    )?;
    fixture.input(b"printf '\\033]0;hello\\007'\n")?;
    metadata(
        &events,
        |event| matches!(event, MetadataEvent::Title(title) if title == "hello"),
    )?;
    let (_, snapshot, _) = fixture.attach(2)?;
    assert_eq!(snapshot.title, "hello");
    assert!(snapshot.directory.0.ends_with(b"/tmp"));
    fixture.input(b"printf '\\033]0;\\007'\n")?;
    metadata(
        &events,
        |event| matches!(event, MetadataEvent::Title(title) if title.is_empty()),
    )?;
    Ok(())
}

#[test]
fn terminal_directory_and_bell_events_are_delivered_without_replaying_bells_on_attach() -> TestResult
{
    let fixture = Fixture::new()?;
    let (events, _, _) = fixture.attach(1)?;
    fixture.input(b"printf '\\033]7;file:///tmp/a%%20b\\007'\n")?;
    assert_eq!(
        metadata(
            &events,
            |event| matches!(event, MetadataEvent::Directory(path) if path.0 == b"/tmp/a b")
        )?,
        MetadataEvent::Directory(ServerPath(b"/tmp/a b".to_vec()))
    );
    fixture.input(b"printf '\\007'\n")?;
    metadata(&events, |event| matches!(event, MetadataEvent::Bell))?;
    let (second, snapshot, _) = fixture.attach(2)?;
    assert_eq!(snapshot.directory.0, b"/tmp/a b");
    assert!(
        second
            .try_iter()
            .all(|event| !matches!(event, AttachmentEvent::Metadata(MetadataEvent::Bell)))
    );
    Ok(())
}
