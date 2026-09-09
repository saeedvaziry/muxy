mod storage;

use storage::StoredRecord;

use std::collections::BTreeMap;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, BufWriter, Read, Write};
use std::ops::Range;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use muxy_protocol::{
    ErrorCode, ExitReason, HistoryCursor, HistoryPage, Message, ReplyBody, RequestId, Row, Run,
    SavedScreen, SearchPage, SessionId,
};
use muxy_terminal::TerminalArchive;
use serde::{Deserialize, Serialize};

use crate::ServerError;
use crate::search::Search;
use crate::session::frames;

const VERSION: u32 = 1;
const SCREEN_LIMIT: u64 = 16 * 1024 * 1024 - 1024;
type Completion = Sender<Result<(), String>>;
type Queue = Arc<Mutex<BTreeMap<SessionId, Job>>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Record {
    version: u32,
    screen: SavedScreen,
    history: Vec<Vec<Run>>,
}

impl Record {
    fn new(terminal: TerminalArchive, reason: Option<ExitReason>) -> Self {
        Self {
            version: VERSION,
            screen: SavedScreen {
                size: muxy_protocol::Size {
                    cols: terminal.size.cols,
                    rows: terminal.size.rows,
                },
                rows: frames::rows(terminal.rows),
                cursor: frames::cursor(terminal.cursor),
                reason,
            },
            history: frames::rows(
                terminal
                    .history
                    .into_iter()
                    .map(|runs| muxy_terminal::Row { index: 0, runs })
                    .collect(),
            )
            .into_iter()
            .map(|row| row.runs)
            .collect(),
        }
    }

    fn validate(&self) -> io::Result<()> {
        if self.version != VERSION {
            return Err(io::Error::other(format!(
                "unsupported terminal record version {}",
                self.version
            )));
        }
        validate_screen(&self.screen)
    }
}

fn validate_screen(screen: &SavedScreen) -> io::Result<()> {
    if storage::serialized_size(screen)? > SCREEN_LIMIT {
        return Err(io::Error::other("saved screen exceeds the size limit"));
    }
    Message::Reply {
        id: RequestId(0),
        body: ReplyBody::SavedScreen(screen.clone()),
    }
    .validate()
    .map_err(|error| io::Error::other(format!("invalid saved screen: {error:?}")))
}

#[derive(Debug)]
enum Operation {
    Save(Record),
    Discard,
}

#[derive(Debug)]
struct Job {
    operation: Operation,
    completions: Vec<Completion>,
}

#[derive(Debug)]
struct Disk {
    directory: PathBuf,
    budget: u64,
    queue: Queue,
    wake: Option<SyncSender<()>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    _lock: File,
}

impl Drop for Disk {
    fn drop(&mut self) {
        self.wake.take();
        if let Some(worker) = lock(&self.worker).take() {
            let _ = worker.join();
        }
    }
}

#[derive(Debug)]
enum Backend {
    Memory {
        records: Mutex<BTreeMap<SessionId, Record>>,
        revision: AtomicU64,
    },
    Disk(Disk),
}

#[derive(Clone, Debug)]
pub(crate) struct Archive(Arc<Backend>);

#[derive(Debug, Default)]
pub(crate) struct SearchCache {
    record: Option<(SessionId, [u64; 3], StoredRecord, u64)>,
}

impl Archive {
    pub(crate) fn memory(_budget: u64) -> Self {
        Self(Arc::new(Backend::Memory {
            records: Mutex::default(),
            revision: AtomicU64::new(0),
        }))
    }

    pub(crate) fn open(directory: &Path, budget: u64) -> io::Result<Self> {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        let directory_lock = File::open(directory)?;
        lock_directory(&directory_lock)?;
        let queue = Queue::default();
        let jobs = Arc::clone(&queue);
        let (wake, receiver) = mpsc::sync_channel(1);
        let root = directory.to_owned();
        let worker = thread::Builder::new()
            .name("muxy-terminal-save".into())
            .spawn(move || {
                write_jobs(&root, &jobs, &receiver);
            })?;
        Ok(Self(Arc::new(Backend::Disk(Disk {
            directory: directory.to_owned(),
            budget,
            queue,
            wake: Some(wake),
            worker: Mutex::new(Some(worker)),
            _lock: directory_lock,
        }))))
    }

    pub(crate) fn contains(&self, session: SessionId) -> bool {
        match self.0.as_ref() {
            Backend::Memory { records, .. } => lock(records).contains_key(&session),
            Backend::Disk(disk) => record_path(&disk.directory, session).exists(),
        }
    }

    pub(crate) fn read(&self, session: SessionId) -> io::Result<SavedScreen> {
        self.load(session).map(|record| record.screen().clone())
    }

    pub(crate) fn history_page(
        &self,
        session: SessionId,
        before: HistoryCursor,
        max_rows: u16,
    ) -> Result<HistoryPage, ServerError> {
        let failed = |error| {
            ServerError::new(
                ErrorCode::SavedContentUnavailable,
                format!("saved history: {error}"),
            )
        };
        let mut record = self.load(session).map_err(failed)?;
        let generation = record.generation();
        let total = record.total();
        let (range, _) = history_range(session, generation, before, max_rows, total)?;
        let mut rows = Vec::new();
        let mut used = 128_u64;
        for index in range.rev() {
            let runs = record.row(index).map_err(failed)?;
            used = used.saturating_add(storage::serialized_size(&runs).map_err(failed)? + 10);
            if used > SCREEN_LIMIT {
                break;
            }
            rows.push(Row { index: 0, runs });
        }
        if rows.is_empty() && total > 0 {
            return Err(ServerError::new(
                ErrorCode::HistoryUnavailable,
                "history row exceeds the page size limit",
            ));
        }
        rows.reverse();
        for (index, row) in rows.iter_mut().enumerate() {
            row.index = u16::try_from(index).unwrap_or(u16::MAX);
        }
        let next = if rows.is_empty() {
            None
        } else {
            history_range(
                session,
                generation,
                before,
                u16::try_from(rows.len()).unwrap_or(max_rows),
                total,
            )?
            .1
        };
        Ok(HistoryPage {
            rows,
            next,
            total_rows: total as u64,
            screen: None,
        })
    }

    pub(crate) fn search(
        &self,
        session: SessionId,
        query: &str,
        ignore_case: bool,
        before: HistoryCursor,
        max_results: u16,
        cache: &mut SearchCache,
    ) -> Result<SearchPage, ServerError> {
        let failed = |error| {
            ServerError::new(
                ErrorCode::SavedContentUnavailable,
                format!("saved search: {error}"),
            )
        };
        let stamp = match self.0.as_ref() {
            Backend::Memory { revision, .. } => [revision.load(Ordering::Acquire), 0, 0],
            Backend::Disk(disk) => {
                let metadata =
                    fs::metadata(record_path(&disk.directory, session)).map_err(failed)?;
                [metadata.dev(), metadata.ino(), metadata.len()]
            }
        };
        if cache
            .record
            .as_ref()
            .is_none_or(|(id, previous, _, _)| *id != session || *previous != stamp)
        {
            let record = self.load(session).map_err(failed)?;
            let mut hasher = DefaultHasher::new();
            record.generation().hash(&mut hasher);
            for row in &record.screen().rows {
                row.runs.hash(&mut hasher);
            }
            cache.record = Some((session, stamp, record, hasher.finish()));
        }
        let (_, _, record, generation) = cache
            .record
            .as_mut()
            .ok_or_else(|| failed(io::Error::other("missing search record")))?;
        let search = Search {
            session,
            generation: *generation,
            query,
            ignore_case,
            before,
            max_results,
            history_rows: record.total(),
            screen_rows: record.screen().rows.len(),
        };
        search.scan(|index| {
            if index < record.total() {
                record.row(index).map_err(failed)
            } else {
                Ok(record.screen().rows[index - record.total()].runs.clone())
            }
        })
    }

    fn load(&self, session: SessionId) -> io::Result<StoredRecord> {
        let record = match self.0.as_ref() {
            Backend::Memory { records, .. } => {
                let record = lock(records).get(&session).cloned().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "no saved terminal content")
                })?;
                record.validate()?;
                StoredRecord::Legacy(record)
            }
            Backend::Disk(disk) => {
                StoredRecord::open(&record_path(&disk.directory, session), disk.budget)?
            }
        };
        Ok(record)
    }

    pub(crate) fn save(
        &self,
        session: SessionId,
        terminal: TerminalArchive,
        reason: Option<ExitReason>,
    ) -> io::Result<()> {
        self.submit(
            session,
            Operation::Save(Record::new(terminal, reason)),
            reason.is_some(),
        )
    }

    pub(crate) fn discard(&self, session: SessionId) -> io::Result<()> {
        self.submit(session, Operation::Discard, true)
    }

    fn submit(&self, session: SessionId, operation: Operation, wait: bool) -> io::Result<()> {
        match self.0.as_ref() {
            Backend::Memory { records, revision } => {
                match operation {
                    Operation::Save(record) => {
                        record.validate()?;
                        lock(records).insert(session, record);
                    }
                    Operation::Discard => {
                        lock(records).remove(&session);
                    }
                }
                revision.fetch_add(1, Ordering::Release);
                Ok(())
            }
            Backend::Disk(disk) => {
                let (done, completed) = mpsc::channel();
                {
                    let mut queue = lock(&disk.queue);
                    let job = queue.entry(session).or_insert_with(|| Job {
                        operation: Operation::Discard,
                        completions: Vec::new(),
                    });
                    job.operation = operation;
                    if wait {
                        job.completions.push(done);
                    }
                }
                if let Some(wake) = &disk.wake {
                    match wake.try_send(()) {
                        Ok(()) | Err(mpsc::TrySendError::Full(())) => {}
                        Err(mpsc::TrySendError::Disconnected(())) => {
                            return Err(io::Error::other("terminal save worker stopped"));
                        }
                    }
                }
                if wait {
                    completed
                        .recv()
                        .map_err(io::Error::other)?
                        .map_err(io::Error::other)?;
                }
                Ok(())
            }
        }
    }
}

pub(crate) fn history_range(
    session: SessionId,
    generation: u64,
    before: HistoryCursor,
    max_rows: u16,
    total: usize,
) -> Result<(Range<usize>, Option<HistoryCursor>), ServerError> {
    if !(1..=500).contains(&max_rows) {
        return Err(ServerError::new(
            ErrorCode::BadRequest,
            "history pages contain 1 through 500 rows",
        ));
    }
    let total = u32::try_from(total).map_err(|_| {
        ServerError::new(
            ErrorCode::HistoryUnavailable,
            "history exceeds the cursor range",
        )
    })?;
    let mut hasher = DefaultHasher::new();
    session.hash(&mut hasher);
    generation.hash(&mut hasher);
    let tag = hasher.finish() & !u64::from(u32::MAX);
    let end = if before.0 == 0 {
        total
    } else {
        let end = u32::try_from(before.0 & u64::from(u32::MAX)).unwrap_or(u32::MAX);
        if end == 0 || end > total || before.0 != tag | u64::from(end) {
            return Err(ServerError::new(
                ErrorCode::StaleHistoryCursor,
                "history changed; start from a fresh window",
            ));
        }
        end
    };
    let start = end.saturating_sub(u32::from(max_rows));
    let next = (start > 0).then_some(HistoryCursor(tag | u64::from(start)));
    Ok((start as usize..end as usize, next))
}

pub(crate) fn bound_history_page(
    session: SessionId,
    generation: u64,
    before: HistoryCursor,
    mut page: HistoryPage,
) -> Result<HistoryPage, ServerError> {
    let size_error = |error| {
        ServerError::new(
            ErrorCode::HistoryUnavailable,
            format!("history page: {error}"),
        )
    };
    let screen_size = postcard::experimental::serialized_size(&page.screen).map_err(size_error)?;
    let mut used = u64::try_from(screen_size)
        .unwrap_or(u64::MAX)
        .saturating_add(128);
    let mut keep = 0_u16;
    for row in page.rows.iter().rev() {
        let size = postcard::experimental::serialized_size(row).map_err(size_error)?;
        used = used.saturating_add(u64::try_from(size).unwrap_or(u64::MAX));
        if used > SCREEN_LIMIT {
            break;
        }
        keep += 1;
    }
    if usize::from(keep) == page.rows.len() {
        return Ok(page);
    }
    if keep == 0 {
        return Err(ServerError::new(
            ErrorCode::HistoryUnavailable,
            "history row exceeds the page size limit",
        ));
    }
    page.rows.drain(..page.rows.len() - usize::from(keep));
    for (index, row) in page.rows.iter_mut().enumerate() {
        row.index = u16::try_from(index).unwrap_or(u16::MAX);
    }
    let total = usize::try_from(page.total_rows).unwrap_or(usize::MAX);
    page.next = history_range(session, generation, before, keep, total)?.1;
    Ok(page)
}

fn write_jobs(directory: &Path, queue: &Queue, wake: &Receiver<()>) {
    while wake.recv().is_ok() {
        loop {
            let job = lock(queue).pop_first();
            let Some((session, job)) = job else { break };
            let result = match job.operation {
                Operation::Save(record) => write_record(directory, session, &record),
                Operation::Discard => discard_record(directory, session),
            }
            .map_err(|error| error.to_string());
            if let Err(error) = &result {
                log::error!("session {} saved content: {error}", session.get());
            }
            for completion in job.completions {
                let _ = completion.send(result.clone());
            }
        }
    }
}

fn lock_directory(directory: &File) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match directory.try_lock().map_err(io::Error::from) {
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(10)),
                );
            }
            result => return result,
        }
    }
}

fn record_path(directory: &Path, session: SessionId) -> PathBuf {
    directory.join(format!("{:016x}.postcard", session.get()))
}

fn read_record(path: &Path, budget: u64) -> io::Result<Record> {
    let limit = budget.saturating_add(SCREEN_LIMIT).saturating_add(1024);
    let mut bytes = Vec::new();
    File::open(path)?
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(io::Error::other(
            "saved terminal record exceeds the size limit",
        ));
    }
    let record: Record = postcard::from_bytes(&bytes).map_err(io::Error::other)?;
    record.validate()?;
    Ok(record)
}

fn write_record(directory: &Path, session: SessionId, record: &Record) -> io::Result<()> {
    let nonce = getrandom::u64().map_err(|error| io::Error::other(error.to_string()))?;
    let temporary = directory.join(format!(".{:016x}-{nonce:016x}.tmp", session.get()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        write_buffered(record, &mut file)?;
        file.sync_all()?;
        fs::rename(&temporary, record_path(directory, session))?;
        File::open(directory)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn write_buffered(record: &Record, writer: impl Write) -> io::Result<()> {
    let mut buffered = BufWriter::new(writer);
    storage::write(record, &mut buffered)?;
    buffered.flush()
}

fn discard_record(directory: &Path, session: SessionId) -> io::Result<()> {
    match fs::remove_file(record_path(directory, session)) {
        Ok(()) => File::open(directory)?.sync_all(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxy_terminal::{Size, Terminal};
    use std::error::Error;
    use std::io::Read;
    use std::sync::atomic::{AtomicU64, Ordering};

    type TestResult = Result<(), Box<dyn Error>>;
    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Directory(PathBuf);

    impl Directory {
        fn new() -> io::Result<Self> {
            let path = std::env::temp_dir().join(format!(
                "muxy-archive-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path)?;
            Ok(Self(path))
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn capture(marker: &str) -> Result<TerminalArchive, Box<dyn Error>> {
        let mut terminal = Terminal::new(Size { cols: 30, rows: 3 }, 1024 * 1024)?;
        for row in 0..100 {
            terminal.feed(format!("\x1b[31mhistory-{row:03}\r\n").as_bytes());
        }
        terminal.feed(marker.as_bytes());
        Ok(terminal.archive()?)
    }

    #[test]
    fn checkpoint_writes_are_buffered_and_flush_errors_are_reported() -> TestResult {
        #[derive(Default)]
        struct CountingWriter {
            bytes: Vec<u8>,
            writes: usize,
        }
        impl Write for CountingWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.writes += 1;
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        struct FailedWriter;
        impl Write for FailedWriter {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("disk full"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let record = Record::new(capture("buffered")?, None);
        let mut writer = CountingWriter::default();
        write_buffered(&record, &mut writer)?;
        let mut expected = Vec::new();
        storage::write(&record, &mut expected)?;
        assert_eq!(writer.bytes, expected);
        assert!(writer.writes <= writer.bytes.len() / 8192 + 1);
        assert!(write_buffered(&record, FailedWriter).is_err());
        Ok(())
    }

    #[test]
    fn saved_search_reuses_the_open_record_and_detects_replacement() -> TestResult {
        let directory = Directory::new()?;
        let session = SessionId::new(1).ok_or("zero ID")?;
        let archive = Archive::open(&directory.0, 1024 * 1024)?;
        archive.save(session, capture("done")?, Some(ExitReason::Exited(0)))?;
        let mut cache = SearchCache::default();
        let first = archive.search(session, "history", false, HistoryCursor(0), 1, &mut cache)?;
        let cursor = first.next.ok_or("missing cursor")?;
        let cached = cache.record.as_ref().ok_or("missing cached record")?;
        assert!(matches!(cached.2, StoredRecord::Indexed(_)));
        let next = archive.search(session, "history", false, cursor, 1, &mut cache)?;
        assert!(next.matches[0].row < first.matches[0].row);
        archive.save(
            session,
            capture("replacement")?,
            Some(ExitReason::Exited(0)),
        )?;
        assert_eq!(
            archive
                .search(session, "history", false, cursor, 1, &mut cache)
                .err()
                .map(|error| error.code()),
            Some(ErrorCode::StaleHistoryCursor)
        );
        archive.discard(session)?;
        assert!(
            archive
                .search(session, "history", false, cursor, 1, &mut cache)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn large_pages_fit_the_wire_limit_without_skipping_rows() -> TestResult {
        let session = SessionId::new(1).ok_or("zero ID")?;
        let row = Row {
            index: 0,
            runs: vec![
                Run {
                    text: "wide".into(),
                    width: 1,
                    style: muxy_protocol::Style::default()
                };
                4096
            ],
        };
        let page = bound_history_page(
            session,
            7,
            HistoryCursor(0),
            HistoryPage {
                rows: (0..500)
                    .map(|index| Row {
                        index,
                        ..row.clone()
                    })
                    .collect(),
                next: None,
                total_rows: 500,
                screen: None,
            },
        )?;
        assert!(page.rows.len() < 500);
        assert!(!page.rows.is_empty());
        assert!(postcard::experimental::serialized_size(&page)? < usize::try_from(SCREEN_LIMIT)?);
        let kept = page.rows.len();
        let (older, next) =
            history_range(session, 7, page.next.ok_or("missing cursor")?, 500, 500)?;
        assert_eq!(older, 0..500 - kept);
        assert_eq!(next, None);
        assert_eq!(page.rows.first().ok_or("empty page")?.index, 0);
        Ok(())
    }

    #[test]
    fn restart_waits_for_the_previous_archive_owner_to_release_its_lock() -> TestResult {
        let directory = Directory::new()?;
        let previous = File::open(&directory.0)?;
        previous.lock()?;
        let root = directory.0.clone();
        let (sender, completed) = mpsc::channel();
        let next = thread::spawn(move || {
            let result = Archive::open(&root, 1024);
            let _ = sender.send(result);
        });
        assert!(matches!(
            completed.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        drop(previous);
        let archive = completed.recv_timeout(Duration::from_secs(3))??;
        next.join().map_err(|_| "archive startup panicked")?;
        drop(archive);
        Ok(())
    }

    #[test]
    fn records_round_trip_atomically_with_private_permissions_and_all_retained_rows() -> TestResult
    {
        let directory = Directory::new()?;
        let archive = Archive::open(&directory.0, 512)?;
        let session = SessionId::new(1).ok_or("zero ID")?;
        archive.save(session, capture("first")?, Some(ExitReason::Exited(3)))?;
        let path = record_path(&directory.0, session);
        let old_bytes = fs::read(&path)?;
        let mut old_file = File::open(&path)?;
        archive.save(session, capture("last")?, Some(ExitReason::Exited(7)))?;
        let mut still_old = Vec::new();
        old_file.read_to_end(&mut still_old)?;
        assert_eq!(old_bytes, still_old);
        assert_ne!(fs::read(&path)?, old_bytes);
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
        assert_eq!(
            fs::metadata(&directory.0)?.permissions().mode() & 0o777,
            0o700
        );
        let record = StoredRecord::open(&path, 512)?;
        assert_eq!(record.total(), capture("last")?.history.len());
        assert_eq!(*record.screen(), archive.read(session)?);
        assert_eq!(record.screen().reason, Some(ExitReason::Exited(7)));
        drop(archive);
        let reopened = Archive::open(&directory.0, 512)?;
        assert_eq!(reopened.read(session)?, *record.screen());
        Ok(())
    }

    #[test]
    fn corrupt_and_unsupported_records_do_not_hide_other_records() -> TestResult {
        let directory = Directory::new()?;
        let archive = Archive::open(&directory.0, 1024)?;
        let first = SessionId::new(1).ok_or("zero ID")?;
        let second = SessionId::new(2).ok_or("zero ID")?;
        archive.save(first, capture("good")?, Some(ExitReason::Ended))?;
        let path = record_path(&directory.0, second);
        fs::write(&path, b"broken")?;
        assert!(archive.read(second).is_err());
        let mut unsupported = Record::new(capture("unsupported")?, None);
        unsupported.version = VERSION + 1;
        fs::write(&path, postcard::to_stdvec(&unsupported)?)?;
        assert!(archive.read(second).is_err());
        assert!(archive.read(first).is_ok());
        Ok(())
    }

    #[test]
    fn failed_write_preserves_previous_complete_record_and_discard_drains_pending_saves()
    -> TestResult {
        let directory = Directory::new()?;
        let root = directory.0.join("records");
        let moved = directory.0.join("moved");
        let archive = Archive::open(&root, 1024)?;
        let session = SessionId::new(1).ok_or("zero ID")?;
        archive.save(session, capture("complete")?, Some(ExitReason::Ended))?;
        let before = archive.read(session)?;
        fs::rename(&root, &moved)?;
        fs::write(&root, "not a directory")?;
        assert!(
            archive
                .save(session, capture("failed")?, Some(ExitReason::Ended))
                .is_err()
        );
        fs::remove_file(&root)?;
        fs::rename(&moved, &root)?;
        assert_eq!(archive.read(session)?, before);
        for _ in 0..20 {
            archive.save(session, capture("queued")?, None)?;
        }
        archive.discard(session)?;
        archive.discard(session)?;
        drop(archive);
        assert!(!record_path(&root, session).exists());
        assert!(fs::read_dir(&root)?.next().is_none());
        Ok(())
    }

    #[test]
    fn styled_history_keeps_the_live_retained_window_on_disk_and_after_restart() -> TestResult {
        let directory = Directory::new()?;
        let budget = 64 * 1024;
        let mut terminal = Terminal::new(Size { cols: 80, rows: 24 }, budget)?;
        for _ in 0..10 {
            for index in 0..500 {
                use std::fmt::Write as _;
                let mut line = String::new();
                for col in 0..78 {
                    write!(
                        line,
                        "\x1b[38;2;{};{};{}mX",
                        col,
                        index % 255,
                        (col * 3) % 255
                    )?;
                }
                line.push_str("\r\n");
                terminal.feed(line.as_bytes());
            }
            terminal.compress_idle()?;
        }
        let live_rows = terminal.history_rows()?;
        let expected = Record::new(terminal.archive()?, Some(ExitReason::Ended));
        assert!(storage::serialized_size(&expected.history)? > budget as u64);
        let session = SessionId::from(std::num::NonZeroU64::MIN);
        let archive = Archive::open(&directory.0, budget as u64)?;
        archive.save(session, terminal.archive()?, Some(ExitReason::Ended))?;
        drop(archive);
        let archive = Archive::open(&directory.0, budget as u64)?;
        assert_eq!(archive.read(session)?, expected.screen);
        let mut before = HistoryCursor(0);
        let mut end = live_rows;
        loop {
            let page = archive.history_page(session, before, 123)?;
            assert_eq!(page.total_rows, live_rows as u64);
            let start = end - page.rows.len();
            let actual: Vec<_> = page.rows.into_iter().map(|row| row.runs).collect();
            assert_eq!(actual, expected.history[start..end]);
            end = start;
            let Some(next) = page.next else { break };
            before = next;
        }
        assert_eq!(end, 0);
        Ok(())
    }

    #[test]
    fn indexed_screens_do_not_decode_history_and_legacy_saved_data_still_loads() -> TestResult {
        use std::io::{Seek, SeekFrom};
        let directory = Directory::new()?;
        let session = SessionId::from(std::num::NonZeroU64::MIN);
        let archive = Archive::open(&directory.0, 1024 * 1024)?;
        let record = Record::new(capture("preserved")?, Some(ExitReason::Ended));
        let path = record_path(&directory.0, session);
        fs::write(&path, postcard::to_stdvec(&record)?)?;
        assert_eq!(archive.read(session)?, record.screen);
        assert_eq!(
            archive
                .history_page(session, HistoryCursor(0), 500)?
                .total_rows,
            record.history.len() as u64
        );
        write_record(&directory.0, session, &record)?;
        let mut file = OpenOptions::new().read(true).write(true).open(&path)?;
        let data_start =
            32 + storage::serialized_size(&record.screen)? + (record.history.len() as u64 + 1) * 8;
        file.seek(SeekFrom::Start(data_start))?;
        file.write_all(&[255; 8])?;
        assert_eq!(archive.read(session)?, record.screen);
        assert!(
            archive
                .history_page(session, HistoryCursor(0), 500)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn slow_archive_read_does_not_block_ping_or_another_sessions_input() -> TestResult {
        use muxy_protocol::{CONTROL, RequestBody, SUPPORTED};
        use muxy_wire::{Decoder, Encoder};
        use std::os::unix::net::UnixStream;
        let (server_events, events) = mpsc::channel();
        let directory = Directory::new()?;
        let registry = Arc::new(crate::Registry::new(
            crate::ServerSettings {
                default_shell: Some(PathBuf::from("/bin/sh")),
                ..crate::ServerSettings::default()
            },
            server_events,
        ));
        let size = muxy_protocol::Size { cols: 80, rows: 24 };
        let live = registry.create(&directory.0, size)?;
        let archive = registry.archive();
        let (socket, server) = UnixStream::pair()?;
        let serving = Arc::clone(&registry);
        let worker =
            thread::spawn(move || crate::connection::serve(Box::new(server), serving, events));
        let mut encoder = Encoder::new(socket.try_clone()?);
        let (incoming_messages, messages) = mpsc::channel();
        let incoming = socket.try_clone()?;
        let reader = thread::spawn(move || {
            let mut decoder = Decoder::new(incoming);
            while let Ok(message) = decoder.next() {
                if incoming_messages.send(message).is_err() {
                    break;
                }
            }
        });
        encoder.send(
            CONTROL,
            &Message::Hello {
                versions: SUPPORTED.to_vec(),
            },
        )?;
        assert!(matches!(
            messages.recv_timeout(Duration::from_secs(2))?,
            (CONTROL, Message::HelloReply { .. })
        ));
        encoder.send(
            CONTROL,
            &Message::Request {
                id: RequestId(100),
                body: RequestBody::Attach {
                    session: live.id,
                    size,
                },
            },
        )?;
        let (_, attached) = next_reply(&messages, &mut encoder)?;
        let ReplyBody::Attached { snapshot, .. } = attached else {
            return Err("expected attachment".into());
        };
        let Backend::Memory { records, .. } = archive.0.as_ref() else {
            return Err("expected memory archive".into());
        };
        let blocked_archive = lock(records);
        let session = SessionId::from(std::num::NonZeroU64::MIN);
        encoder.send(
            CONTROL,
            &Message::Request {
                id: RequestId(101),
                body: RequestBody::ReadSavedScreen(session),
            },
        )?;
        encoder.send(
            snapshot.channel,
            &Message::Input(b"touch proof-input\n".to_vec()),
        )?;
        encoder.send(
            CONTROL,
            &Message::Request {
                id: RequestId(102),
                body: RequestBody::Ping,
            },
        )?;
        let pong = next_reply(&messages, &mut encoder);
        let marker = directory.0.join("proof-input");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !marker.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        let input_completed = marker.exists();
        drop(blocked_archive);
        let saved = next_reply(&messages, &mut encoder);
        registry.shutdown();
        socket.shutdown(std::net::Shutdown::Both)?;
        drop(encoder);
        worker.join().map_err(|_| "server thread panicked")??;
        reader.join().map_err(|_| "reader thread panicked")?;
        assert_eq!(pong?, (RequestId(102), ReplyBody::Pong));
        assert_eq!(saved?.0, RequestId(101));
        assert!(
            input_completed,
            "another session's input must run while storage is blocked"
        );
        Ok(())
    }

    fn next_reply(
        messages: &Receiver<(muxy_protocol::ChannelId, Message)>,
        encoder: &mut muxy_wire::Encoder<impl Write>,
    ) -> Result<(RequestId, ReplyBody), Box<dyn Error>> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match messages.recv_timeout(deadline.saturating_duration_since(Instant::now()))? {
                (_, Message::Reply { id, body }) => return Ok((id, body)),
                (channel, Message::Frame(frame)) => encoder.send(
                    muxy_protocol::CONTROL,
                    &Message::FrameAck {
                        channel,
                        seq: frame.seq,
                    },
                )?,
                (_, Message::Metadata(_)) => {}
                other => return Err(format!("unexpected message: {other:?}").into()),
            }
        }
    }
}
