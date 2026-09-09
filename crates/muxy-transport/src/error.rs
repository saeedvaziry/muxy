use std::error::Error;
use std::fmt;
use std::io;

#[derive(Debug)]
pub enum BindError {
    InUse,
    Io(io::Error),
}

impl fmt::Display for BindError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InUse => formatter.write_str("already in use"),
            Self::Io(error) => write!(formatter, "socket bind failed: {error}"),
        }
    }
}

impl Error for BindError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InUse => None,
            Self::Io(error) => Some(error),
        }
    }
}

impl From<io::Error> for BindError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
