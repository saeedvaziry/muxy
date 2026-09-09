use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use muxy_protocol::{ExitReason, SavedScreen, SessionId, Size};
use muxy_server_core::{Registry, ServerEvent, ServerSettings, SessionCommand};

type TestResult = Result<(), Box<dyn Error>>;
const TIMEOUT: Duration = Duration::from_secs(10);
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    registry: Arc<Registry>,
    events: Receiver<ServerEvent>,
}

impl Fixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        let root = std::env::temp_dir().join(format!(
            "muxy-session-archive-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root)?;
        let (send, events) = mpsc::channel();
        let registry = Arc::new(Registry::persistent(
            settings(),
            send,
            &root.join("sessions"),
        )?);
        Ok(Self {
            root,
            registry,
            events,
        })
    }

    fn start(&self, input: &[u8]) -> Result<SessionId, Box<dyn Error>> {
        let info = self
            .registry
            .create(&self.root, Size { cols: 80, rows: 8 })?;
        self.registry
            .handle(info.id)
            .ok_or("missing session")?
            .send(SessionCommand::Input(input.to_vec()))?;
        Ok(info.id)
    }

    fn ended(&self, id: SessionId) -> Result<ExitReason, Box<dyn Error>> {
        let ServerEvent::SessionEnded { id: ended, reason } = self.events.recv_timeout(TIMEOUT)?;
        assert_eq!(ended, id);
        Ok(reason)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.registry.shutdown();
        let deadline = Instant::now() + TIMEOUT;
        while !self.registry.is_stopped() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn settings() -> ServerSettings {
    ServerSettings {
        default_shell: Some(PathBuf::from("/bin/sh")),
        ..ServerSettings::default()
    }
}

fn text(screen: &SavedScreen) -> String {
    screen
        .rows
        .iter()
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
fn normal_exit_saves_final_output_before_the_event_without_an_attachment() -> TestResult {
    let fixture = Fixture::new()?;
    let id = fixture.start(b"stty -echo; printf '\\033[2J\\033[Hfinal-output'; exit 7\n")?;
    assert_eq!(fixture.ended(id)?, ExitReason::Exited(7));
    let saved = fixture.registry.read_saved_screen(id)?;
    assert!(text(&saved).contains("final-output"));
    assert_eq!(saved.reason, Some(ExitReason::Exited(7)));
    assert!(fixture.registry.list().is_empty());
    fixture.registry.discard(id)?;
    fixture.registry.discard(id)?;
    assert!(fixture.registry.read_saved_screen(id).is_err());
    Ok(())
}

#[test]
fn checkpoint_exists_while_the_session_is_live_without_clients() -> TestResult {
    let fixture = Fixture::new()?;
    let id = fixture.start(b"stty -echo; printf '\\033[2J\\033[Hcheckpoint-live'\n")?;
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Ok(saved) = fixture.registry.read_saved_screen(id)
            && text(&saved).contains("checkpoint-live")
        {
            assert_eq!(saved.reason, None);
            break;
        }
        if Instant::now() >= deadline {
            return Err("checkpoint did not arrive".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fixture.registry.list().len(), 1);
    fixture.registry.discard(id)?;
    assert!(fixture.registry.list().is_empty());
    assert!(fixture.registry.read_saved_screen(id).is_err());
    Ok(())
}

#[test]
fn graceful_restart_recovers_saved_output_without_recreating_processes() -> TestResult {
    let mut fixture = Fixture::new()?;
    let id = fixture.start(b"stty -echo; printf '\\033[2J\\033[Hbefore-shutdown'\n")?;
    thread::sleep(Duration::from_millis(100));
    fixture.registry.shutdown();
    assert_eq!(fixture.ended(id)?, ExitReason::ServerStopped);
    let saved = fixture.registry.read_saved_screen(id)?;
    assert!(text(&saved).contains("before-shutdown"));
    let (sender, _) = mpsc::channel();
    let old = std::mem::replace(
        &mut fixture.registry,
        Arc::new(Registry::new(settings(), sender)),
    );
    drop(old);
    let (sender, events) = mpsc::channel();
    fixture.registry = Arc::new(Registry::persistent(
        settings(),
        sender,
        &fixture.root.join("sessions"),
    )?);
    fixture.events = events;
    assert!(fixture.registry.list().is_empty());
    assert_eq!(fixture.registry.read_saved_screen(id)?, saved);
    Ok(())
}

#[test]
fn discard_racing_checkpoints_and_exit_cannot_recreate_an_archive() -> TestResult {
    let fixture = Fixture::new()?;
    let id = fixture.start(
        b"stty -echo; i=0; while [ $i -lt 1000 ]; do echo tick-$i; i=$((i+1)); done; exit\n",
    )?;
    let first = Arc::clone(&fixture.registry);
    let second = Arc::clone(&fixture.registry);
    let a = thread::spawn(move || first.discard(id));
    let b = thread::spawn(move || second.discard(id));
    a.join().map_err(|_| "discard panicked")??;
    b.join().map_err(|_| "discard panicked")??;
    assert!(fixture.registry.list().is_empty());
    assert!(fixture.registry.read_saved_screen(id).is_err());
    fixture.registry.shutdown();
    assert!(fixture.registry.is_stopped());
    assert!(
        fs::read_dir(fixture.root.join("sessions"))?
            .next()
            .is_none()
    );
    Ok(())
}
