use std::error::Error;
use std::fmt;
use std::io;

#[derive(Debug)]
pub enum WireError {
    Io(io::Error),
    Closed,
    FrameTooLarge,
    UnknownKind(u8),
    FlagsSet(u8),
    Decode(postcard::Error),
    UnsupportedVersion(u16),
}

impl fmt::Display for WireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "wire I/O failed: {error}"),
            Self::Closed => formatter.write_str("Closed"),
            Self::FrameTooLarge => formatter.write_str("frame exceeds 16 MiB"),
            Self::UnknownKind(kind) => write!(formatter, "unknown message kind: {kind}"),
            Self::FlagsSet(kind) => write!(formatter, "reserved flag bits set: {kind:#04x}"),
            Self::Decode(error) => write!(formatter, "invalid wire payload: {error}"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported wire version: {version}")
            }
        }
    }
}

impl Error for WireError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Decode(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for WireError {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            Self::Closed
        } else {
            Self::Io(error)
        }
    }
}

impl From<postcard::Error> for WireError {
    fn from(error: postcard::Error) -> Self {
        Self::Decode(error)
    }
}
