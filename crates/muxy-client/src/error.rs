use std::error::Error;
use std::fmt;
use std::io;

use muxy_protocol::{ErrorCode, ErrorReply, ReplyBody};
use muxy_wire::WireError;

#[derive(Debug)]
pub enum ClientError {
    Io(io::Error),
    Wire(WireError),
    VersionUnsupported,
    Protocol(String),
    Invalid(ErrorCode),
    Server(ErrorReply),
    UnexpectedReply(ReplyBody),
    Timeout,
    Disconnected,
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "connection failed: {error}"),
            Self::Wire(error) => write!(formatter, "wire error: {error}"),
            Self::VersionUnsupported => formatter.write_str("server supports no common version"),
            Self::Protocol(message) => write!(formatter, "protocol violation: {message}"),
            Self::Invalid(code) => write!(formatter, "invalid request: {code:?}"),
            Self::Server(error) => write!(formatter, "{:?}: {}", error.code, error.message),
            Self::UnexpectedReply(body) => write!(formatter, "unexpected reply: {body:?}"),
            Self::Timeout => formatter.write_str("request timed out"),
            Self::Disconnected => formatter.write_str("disconnected from server"),
        }
    }
}

impl Error for ClientError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Wire(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ClientError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<WireError> for ClientError {
    fn from(error: WireError) -> Self {
        match error {
            WireError::Closed => Self::Disconnected,
            WireError::Io(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::BrokenPipe
                        | io::ErrorKind::ConnectionReset
                        | io::ErrorKind::ConnectionAborted
                        | io::ErrorKind::NotConnected
                ) =>
            {
                Self::Disconnected
            }
            other => Self::Wire(other),
        }
    }
}
