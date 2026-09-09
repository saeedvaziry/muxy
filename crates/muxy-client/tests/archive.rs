use std::error::Error;
use std::fs;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use muxy_client::{Client, ClientError, ClientEvent};
use muxy_protocol::{ErrorCode, ExitReason, Size};
use muxy_server_core::{Registry, ServerSettings, connection};

type TestResult = Result<(), Box<dyn Error>>;
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    registry: Arc<Registry>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.registry.shutdown();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.registry.is_stopped() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn exited_content_is_read_without_attachment_and_discard_is_idempotent() -> TestResult {
    let root = std::env::temp_dir().join(format!(
        "muxy-client-archive-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root)?;
    let (send, events) = mpsc::channel();
    let registry = Arc::new(Registry::persistent(
        ServerSettings {
            default_shell: Some(PathBuf::from("/bin/sh")),
            ..ServerSettings::default()
        },
        send,
        &root.join("sessions"),
    )?);
    let fixture = Fixture {
        root,
        registry: Arc::clone(&registry),
    };
    let (client, server) = UnixStream::pair()?;
    let worker = thread::spawn(move || connection::serve(Box::new(server), registry, events));
    let client = Client::from_stream(Box::new(client))?;
    let events = client.events().ok_or("events already taken")?;
    let size = Size { cols: 80, rows: 8 };
    let session = client.create_session(&fixture.root, size)?;
    let attachment = client.attach(session.id, size)?;
    client.send_input(
        attachment.channel,
        b"stty -echo; printf '\\033[2J\\033[Hsaved-final'; exit 9\n",
    )?;
    loop {
        match events.recv_timeout(Duration::from_secs(5))? {
            ClientEvent::SessionEnded {
                session: ended,
                reason,
            } => {
                assert_eq!(ended, session.id);
                assert_eq!(reason, ExitReason::Exited(9));
                break;
            }
            ClientEvent::Frame { channel, frame } => client.ack(channel, frame.seq)?,
            ClientEvent::Metadata { .. } => {}
            ClientEvent::Disconnected => return Err("client disconnected".into()),
        }
    }
    assert!(client.list_sessions()?.is_empty());
    let saved = client.read_saved_screen(session.id)?;
    assert_eq!(saved.reason, Some(ExitReason::Exited(9)));
    assert!(
        saved
            .rows
            .iter()
            .flat_map(|row| &row.runs)
            .any(|run| run.text.contains("saved-final"))
    );
    assert!(
        matches!(client.attach(session.id, size), Err(ClientError::Server(error)) if error.code == ErrorCode::UnknownSession)
    );
    client.discard_session(session.id)?;
    client.discard_session(session.id)?;
    assert!(
        matches!(client.read_saved_screen(session.id), Err(ClientError::Server(error)) if error.code == ErrorCode::SavedContentUnavailable)
    );
    let live = client.create_session(&fixture.root, size)?;
    client.discard_session(live.id)?;
    assert!(client.list_sessions()?.is_empty());
    assert!(client.read_saved_screen(live.id).is_err());
    client.ping()?;
    drop(client);
    worker.join().map_err(|_| "connection panicked")??;
    Ok(())
}

#[test]
fn saved_history_pages_remain_readable_after_the_server_reopens_its_archive() -> TestResult {
    use muxy_protocol::{HistoryCursor, SessionId};
    let root = std::env::temp_dir().join(format!(
        "muxy-history-restart-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root)?;
    let mut saved: Option<SessionId> = None;
    let mut expected = Vec::new();
    for restart in [false, true] {
        let (send, events) = mpsc::channel();
        let registry = Arc::new(Registry::persistent(
            ServerSettings {
                default_shell: Some(PathBuf::from("/bin/sh")),
                ..ServerSettings::default()
            },
            send,
            &root.join("sessions"),
        )?);
        let owner = Arc::clone(&registry);
        let (client, server) = UnixStream::pair()?;
        let worker = thread::spawn(move || connection::serve(Box::new(server), owner, events));
        let client = Client::from_stream(Box::new(client))?;
        if !restart {
            let size = Size { cols: 80, rows: 24 };
            let session = client.create_session(&root, size)?;
            saved = Some(session.id);
            let attachment = client.attach(session.id, size)?;
            let events = client.events().ok_or("events already taken")?;
            client.send_input(attachment.channel, b"stty -echo; PS1=''; printf '\\033[2J\\033[H\\033[3J'; seq 1 5000; printf ARCHIVE_READY; exit 7\n")?;
            loop {
                match events.recv_timeout(Duration::from_secs(5))? {
                    ClientEvent::Frame { channel, frame } => client.ack(channel, frame.seq)?,
                    ClientEvent::Metadata { .. } => {}
                    ClientEvent::SessionEnded {
                        session: id,
                        reason,
                    } => {
                        assert_eq!(id, session.id);
                        assert_eq!(reason, ExitReason::Exited(7));
                        break;
                    }
                    ClientEvent::Disconnected => return Err("unexpected disconnect".into()),
                }
            }
        }
        let session = saved.ok_or("missing saved ID")?;
        assert!(client.list_sessions()?.is_empty());
        let screen = client.read_saved_screen(session)?;
        assert_eq!(screen.size, Size { cols: 80, rows: 24 });
        assert_eq!(screen.reason, Some(ExitReason::Exited(7)));
        verify_saved_search(&client, session)?;
        let mut before = HistoryCursor(0);
        let mut history = Vec::new();
        loop {
            let page = client.saved_history_page(session, before, 500)?;
            assert!(page.screen.is_none());
            let mut older = page
                .rows
                .into_iter()
                .map(|row| {
                    row.runs
                        .into_iter()
                        .map(|run| run.text)
                        .collect::<String>()
                        .trim_end()
                        .to_owned()
                })
                .collect::<Vec<_>>();
            older.extend(history);
            history = older;
            let Some(next) = page.next else { break };
            before = next;
        }
        if restart {
            assert_eq!(history, expected);
        } else {
            verify_numbered_history(&history, &screen);
            expected = history;
        }
        drop(client);
        worker.join().map_err(|_| "server panicked")??;
        registry.shutdown();
        assert!(registry.is_stopped());
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

fn verify_numbered_history(history: &[String], screen: &muxy_protocol::SavedScreen) {
    for (index, row) in history.iter().enumerate() {
        assert_eq!(row, &(index + 1).to_string());
    }
    let screen_numbers = screen
        .rows
        .iter()
        .filter_map(|row| {
            row.runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
                .trim()
                .parse::<usize>()
                .ok()
        })
        .collect::<Vec<_>>();
    assert_eq!(screen_numbers.first(), Some(&(history.len() + 1)));
    assert_eq!(screen_numbers.last(), Some(&5000));
    assert_eq!(history.len() + screen_numbers.len(), 5000);
}

fn verify_saved_search(client: &Client, session: muxy_protocol::SessionId) -> TestResult {
    let mut cursor = muxy_protocol::HistoryCursor(0);
    let mut matches = Vec::new();
    loop {
        let page = client.search(
            muxy_protocol::SearchSource::Saved(session),
            "1",
            false,
            cursor,
            500,
        )?;
        matches.extend(page.matches);
        let Some(next) = page.next else { break };
        cursor = next;
    }
    assert_eq!(
        matches
            .last()
            .map(|found| (found.row, found.start, found.end)),
        Some((0, 0, 1))
    );
    assert!(matches.iter().any(|found| found.row == 4990));
    Ok(())
}
