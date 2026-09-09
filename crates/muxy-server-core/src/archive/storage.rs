use std::fs::File;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

use muxy_protocol::{Run, SavedScreen};

use super::{Record, SCREEN_LIMIT};

const MAGIC: &[u8; 8] = b"MUXYSAV2";
const HEADER_LEN: u64 = 32;

#[derive(Debug)]
pub(super) enum StoredRecord {
    Indexed(IndexedRecord),
    Legacy(Record),
}

#[derive(Debug)]
pub(super) struct IndexedRecord {
    file: BufReader<File>,
    screen: SavedScreen,
    total: usize,
    generation: u64,
    index_start: u64,
    data_start: u64,
    file_len: u64,
}

impl StoredRecord {
    pub(super) fn open(path: &Path, legacy_budget: u64) -> io::Result<Self> {
        let mut file = BufReader::new(File::open(path)?);
        let mut magic = [0; 8];
        file.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return super::read_record(path, legacy_budget).map(Self::Legacy);
        }
        let screen_len = read_u64(&mut file)?;
        let total = read_u64(&mut file)?;
        let generation = read_u64(&mut file)?;
        let file_len = file.get_ref().metadata()?.len();
        if screen_len > SCREEN_LIMIT || total > u64::from(u32::MAX) {
            return Err(invalid());
        }
        let index_start = HEADER_LEN + screen_len;
        let data_start = index_start + (total + 1) * 8;
        if data_start > file_len {
            return Err(invalid());
        }
        let mut bytes = vec![0; usize::try_from(screen_len).map_err(io::Error::other)?];
        file.read_exact(&mut bytes)?;
        let screen = postcard::from_bytes::<SavedScreen>(&bytes).map_err(io::Error::other)?;
        super::validate_screen(&screen)?;
        let mut record = IndexedRecord {
            file,
            screen,
            total: usize::try_from(total).map_err(io::Error::other)?,
            generation,
            index_start,
            data_start,
            file_len,
        };
        if record.offset(0)? != data_start || record.offset(record.total)? != file_len {
            return Err(invalid());
        }
        Ok(Self::Indexed(record))
    }

    pub(super) fn screen(&self) -> &SavedScreen {
        match self {
            Self::Indexed(record) => &record.screen,
            Self::Legacy(record) => &record.screen,
        }
    }

    pub(super) fn total(&self) -> usize {
        match self {
            Self::Indexed(record) => record.total,
            Self::Legacy(record) => record.history.len(),
        }
    }

    pub(super) fn generation(&self) -> u64 {
        match self {
            Self::Indexed(record) => record.generation,
            Self::Legacy(record) => generation(record),
        }
    }

    pub(super) fn row(&mut self, index: usize) -> io::Result<Vec<Run>> {
        match self {
            Self::Legacy(record) => record.history.get(index).cloned().ok_or_else(invalid),
            Self::Indexed(record) => {
                if index >= record.total {
                    return Err(invalid());
                }
                let start = record.offset(index)?;
                let end = record.offset(index + 1)?;
                let len = end
                    .checked_sub(start)
                    .filter(|len| *len <= SCREEN_LIMIT)
                    .ok_or_else(invalid)?;
                record.file.seek(SeekFrom::Start(start))?;
                let mut bytes = vec![0; usize::try_from(len).map_err(io::Error::other)?];
                record.file.read_exact(&mut bytes)?;
                postcard::from_bytes(&bytes).map_err(io::Error::other)
            }
        }
    }
}

impl IndexedRecord {
    fn offset(&mut self, index: usize) -> io::Result<u64> {
        self.file
            .seek(SeekFrom::Start(self.index_start + index as u64 * 8))?;
        let offset = read_u64(&mut self.file)?;
        if offset < self.data_start || offset > self.file_len {
            return Err(invalid());
        }
        Ok(offset)
    }
}

pub(super) fn write(record: &Record, writer: impl Write) -> io::Result<()> {
    let mut writer = DeferredFlush(writer);
    record.validate()?;
    let screen_len = serialized_size(&record.screen)?;
    if screen_len > SCREEN_LIMIT || record.history.len() > u32::MAX as usize {
        return Err(invalid());
    }
    writer.write_all(MAGIC)?;
    writer.write_all(&screen_len.to_le_bytes())?;
    writer.write_all(&(record.history.len() as u64).to_le_bytes())?;
    writer.write_all(&generation(record).to_le_bytes())?;
    postcard::to_io(&record.screen, &mut writer).map_err(io::Error::other)?;
    let mut offset = HEADER_LEN + screen_len + (record.history.len() as u64 + 1) * 8;
    for row in &record.history {
        writer.write_all(&offset.to_le_bytes())?;
        let size = serialized_size(row)?;
        if size > SCREEN_LIMIT {
            return Err(invalid());
        }
        offset = offset.checked_add(size).ok_or_else(invalid)?;
    }
    writer.write_all(&offset.to_le_bytes())?;
    for row in &record.history {
        postcard::to_io(row, &mut writer).map_err(io::Error::other)?;
    }
    Ok(())
}

fn generation(record: &Record) -> u64 {
    let mut hash = DefaultHasher::new();
    record.history.hash(&mut hash);
    record.screen.size.cols.hash(&mut hash);
    hash.finish()
}

pub(super) fn serialized_size(value: &impl serde::Serialize) -> io::Result<u64> {
    postcard::experimental::serialized_size(value)
        .map_err(io::Error::other)
        .and_then(|size| u64::try_from(size).map_err(io::Error::other))
}

fn read_u64(reader: &mut impl Read) -> io::Result<u64> {
    let mut bytes = [0; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn invalid() -> io::Error {
    io::Error::other("invalid saved terminal record")
}

// Postcard flushes each value. A checkpoint flushes once after the whole batch.
struct DeferredFlush<W>(W);

impl<W: Write> Write for DeferredFlush<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
