use std::fmt;

#[derive(Debug)]
pub struct Error {
    key: String,
    message: String,
}

impl Error {
    pub(crate) fn new(key: impl Into<String>, message: impl fmt::Display) -> Self {
        Self {
            key: key.into(),
            message: message.to_string(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.key, self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
