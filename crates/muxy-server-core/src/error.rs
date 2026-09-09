use std::error::Error;
use std::fmt;

use muxy_protocol::{ErrorCode, ErrorReply, SessionId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerError {
    code: ErrorCode,
    message: String,
}

impl ServerError {
    pub(crate) fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn spawn_failed(error: impl fmt::Display) -> Self {
        Self::new(ErrorCode::SpawnFailed, error.to_string())
    }

    pub(crate) fn unknown_session(id: SessionId) -> Self {
        Self::new(
            ErrorCode::UnknownSession,
            format!("session {} does not exist", id.get()),
        )
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn to_reply(&self) -> ErrorReply {
        ErrorReply {
            code: self.code,
            message: self.message.clone(),
        }
    }
}

impl fmt::Display for ServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ServerError {}
