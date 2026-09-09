use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use muxy_protocol::{AttachSnapshot, ChannelId, ErrorCode, HistoryCursor, HistoryPage, Row, Size};
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
    fn new(budget: u64) -> TestResult<Self> {
        let (events, _) = mpsc::channel();
        let registry = Registry::new(
            ServerSettings {
                default_shell: Some(PathBuf::from("/bin/sh")),
                history_budget_bytes: budget,
            },
            events,
        );
        let session = registry.create(Path::new("/tmp"), SIZE)?;
        let handle = registry.handle(session.id).ok_or("missing session")?;
        Ok(Self { registry, handle })
    }

    fn input(&self, input: &[u8]) -> TestResult {
        self.handle.send(SessionCommand::Input(input.to_vec()))?;
        Ok(())
    }

    fn attach(&self, id: u32) -> TestResult<(AttachSnapshot, Receiver<AttachmentEvent>)> {
        let (sink, events) = mpsc::channel();
        self.handle.send(SessionCommand::Attach {
            id: AttachmentId(u64::from(id)),
            channel: ChannelId(id),
            size: SIZE,
            sink,
        })?;
        match events.recv_timeout(TIMEOUT)? {
            AttachmentEvent::Snapshot { snapshot, .. } => Ok((snapshot, events)),
            other => Err(format!("expected snapshot, got {other:?}").into()),
        }
    }

    fn page(&self, before: HistoryCursor) -> TestResult<HistoryPage> {
        let (reply, response) = mpsc::channel();
        self.handle.send(SessionCommand::HistoryPage {
            before,
            max_rows: 500,
            reply,
        })?;
        Ok(response.recv_timeout(TIMEOUT)??)
    }

    fn wait_for(&self, marker: &str) -> TestResult<HistoryPage> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let page = self.page(HistoryCursor(0))?;
            if page
                .screen
                .as_ref()
                .is_some_and(|screen| texts(&screen.rows).iter().any(|row| row == marker))
            {
                return Ok(page);
            }
            if Instant::now() >= deadline {
                return Err(format!("missing marker {marker}").into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn produce(&self) -> TestResult {
        self.input(b"stty -echo; PS1=''; printf '\\033[2J\\033[H\\033[3J'; seq 1 5000; printf HISTORY_READY\n")?;
        self.wait_for("HISTORY_READY")?;
        Ok(())
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

fn texts(rows: &[Row]) -> Vec<String> {
    rows.iter()
        .map(|row| {
            row.runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

#[test]
fn attach_brings_two_hundred_rows_and_pages_walk_to_the_exact_oldest_row() -> TestResult {
    let fixture = Fixture::new(16 * 1024 * 1024)?;
    fixture.produce()?;
    let (snapshot, _events) = fixture.attach(1)?;
    assert_eq!(snapshot.history.len(), 200);
    assert_eq!(snapshot.history_total, 4977);
    let mut lines = texts(&snapshot.history);
    let mut before = snapshot.history_cursor;
    let mut pages = 0;
    while let Some(cursor) = before {
        let page = fixture.page(cursor)?;
        assert!(page.screen.is_none());
        assert!(!page.rows.is_empty());
        assert!(page.rows.len() <= 500);
        if page.next.is_some() {
            assert_eq!(page.rows.len(), 500);
        }
        let mut older = texts(&page.rows);
        older.extend(lines);
        lines = older;
        before = page.next;
        pages += 1;
    }
    assert_eq!(pages, 10);
    assert_eq!(
        lines,
        (1..=4977)
            .map(|number| number.to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(texts(&snapshot.rows)[0], "4978");
    Ok(())
}

#[test]
fn output_preserves_page_boundaries_and_resize_returns_a_stale_cursor_error() -> TestResult {
    let fixture = Fixture::new(16 * 1024 * 1024)?;
    fixture.produce()?;
    let (snapshot, _events) = fixture.attach(1)?;
    let cursor = snapshot.history_cursor.ok_or("missing older cursor")?;
    let expected = fixture.page(cursor)?.rows;
    fixture.input(b"printf '\\n'; seq 1 20; printf TAIL_READY\n")?;
    fixture.wait_for("TAIL_READY")?;
    assert_eq!(fixture.page(cursor)?.rows, expected);
    fixture
        .handle
        .send(SessionCommand::Resize(Size { cols: 40, rows: 24 }))?;
    let error = fixture.page(cursor).err().ok_or("expected stale cursor")?;
    assert_eq!(
        error
            .downcast_ref::<muxy_server_core::ServerError>()
            .map(muxy_server_core::ServerError::code),
        Some(ErrorCode::StaleHistoryCursor)
    );
    let refreshed = fixture.page(HistoryCursor(0))?;
    assert_eq!(refreshed.screen.ok_or("no refreshed screen")?.size.cols, 40);
    Ok(())
}

#[test]
fn eviction_rejects_old_cursors_instead_of_reinterpreting_their_indexes() -> TestResult {
    let fixture = Fixture::new(256 * 1024)?;
    fixture.produce()?;
    let (snapshot, _events) = fixture.attach(1)?;
    let cursor = snapshot.history_cursor.ok_or("missing older cursor")?;
    fixture.input(b"seq 1 20000; printf EVICTED_READY\n")?;
    fixture.wait_for("EVICTED_READY")?;
    let error = fixture.page(cursor).err().ok_or("expected stale cursor")?;
    assert_eq!(
        error
            .downcast_ref::<muxy_server_core::ServerError>()
            .map(muxy_server_core::ServerError::code),
        Some(ErrorCode::StaleHistoryCursor)
    );
    Ok(())
}
