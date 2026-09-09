use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

struct FileLogger(Mutex<File>);

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let message = record
                .args()
                .to_string()
                .replace('\r', "\\r")
                .replace('\n', "\\n");
            let mut file = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            if let Err(error) = writeln!(file, "{timestamp} {} {message}", record.level()) {
                let _ = writeln!(io::stderr(), "muxy-server: logging failed: {error}");
            }
        }
    }

    fn flush(&self) {
        let _ = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .flush();
    }
}

pub(crate) fn init(path: &Path) -> io::Result<()> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    log::set_boxed_logger(Box::new(FileLogger(Mutex::new(file)))).map_err(io::Error::other)?;
    log::set_max_level(log::LevelFilter::Info);
    Ok(())
}
