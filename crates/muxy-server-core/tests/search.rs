use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use muxy_protocol::{ErrorCode, HistoryCursor, HistoryPage, Row, SearchPage, Size};
use muxy_server_core::{Registry, ServerSettings, SessionCommand, SessionHandle};

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

    fn search(
        &self,
        query: &str,
        before: HistoryCursor,
        max_results: u16,
    ) -> TestResult<SearchPage> {
        let (reply, response) = mpsc::channel();
        self.handle.send(SessionCommand::Search {
            query: query.into(),
            ignore_case: false,
            before,
            max_results,
            reply,
        })?;
        Ok(response.recv_timeout(TIMEOUT)??)
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
fn search_walks_history_and_screen_newest_first_without_losing_matches() -> TestResult {
    let fixture = Fixture::new(16 * 1024 * 1024)?;
    fixture.produce()?;
    let mut before = HistoryCursor(0);
    let mut found = Vec::new();
    loop {
        let page = fixture.search("1", before, 500)?;
        assert_eq!(page.total_rows, 4977);
        assert!(page.scanned_rows <= 2000);
        assert!(page.matches.len() <= 500);
        found.extend(page.matches);
        let Some(next) = page.next else { break };
        before = next;
    }
    let mut expected = Vec::new();
    for number in (1..=5000_u64).rev() {
        let text = number.to_string();
        let starts: Vec<_> = text.match_indices('1').map(|(index, _)| index).collect();
        for start in starts.into_iter().rev() {
            expected.push(muxy_protocol::SearchMatch {
                row: number - 1,
                start: u16::try_from(start)?,
                end: u16::try_from(start + 1)?,
            });
        }
    }
    assert_eq!(found, expected);
    assert!(found.iter().any(|found| found.row < 4977));
    assert!(found.iter().any(|found| found.row >= 4977));
    Ok(())
}

#[test]
fn empty_search_pages_continue_and_scan_at_most_two_thousand_rows() -> TestResult {
    let fixture = Fixture::new(16 * 1024 * 1024)?;
    fixture.produce()?;
    let mut before = HistoryCursor(0);
    let mut scanned = Vec::new();
    loop {
        let page = fixture.search("absent", before, 500)?;
        assert!(page.matches.is_empty());
        scanned.push(page.scanned_rows);
        let Some(next) = page.next else { break };
        before = next;
    }
    assert_eq!(scanned, [2000, 2000, 1001]);
    Ok(())
}

#[test]
fn eviction_and_reflow_invalidate_search_cursors() -> TestResult {
    let fixture = Fixture::new(256 * 1024)?;
    fixture.produce()?;
    for resize in [false, true] {
        let before = fixture
            .search("1", HistoryCursor(0), 1)?
            .next
            .ok_or("missing cursor")?;
        if resize {
            fixture
                .handle
                .send(SessionCommand::Resize(Size { cols: 40, rows: 24 }))?;
        } else {
            fixture.input(b"seq 1 20000; printf EVICTED_READY\n")?;
            fixture.wait_for("EVICTED_READY")?;
        }
        let error = fixture
            .search("1", before, 1)
            .err()
            .ok_or("expected stale cursor")?;
        assert_eq!(
            error
                .downcast_ref::<muxy_server_core::ServerError>()
                .map(muxy_server_core::ServerError::code),
            Some(ErrorCode::StaleHistoryCursor)
        );
    }
    Ok(())
}
