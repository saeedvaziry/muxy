use std::error::Error;
use std::fmt;
use std::io;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PtyStep {
    Open,
    Spawn,
    CloneReader,
    TakeWriter,
    Write,
    Resize,
    Wait,
    Kill,
}

impl fmt::Display for PtyStep {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Open => "open",
            Self::Spawn => "spawn",
            Self::CloneReader => "clone reader",
            Self::TakeWriter => "take writer",
            Self::Write => "write",
            Self::Resize => "resize",
            Self::Wait => "wait",
            Self::Kill => "kill",
        };
        formatter.write_str(name)
    }
}

#[derive(Debug)]
pub struct PtyError {
    step: PtyStep,
    source: io::Error,
}

impl PtyError {
    pub(crate) fn new(step: PtyStep, source: io::Error) -> Self {
        Self { step, source }
    }

    pub(crate) fn wrap(step: PtyStep, source: impl Into<Box<dyn Error + Send + Sync>>) -> Self {
        Self::new(step, io::Error::other(source))
    }

    pub fn step(&self) -> PtyStep {
        self.step
    }
}

impl fmt::Display for PtyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "pty {} failed: {}", self.step, self.source)
    }
}

impl Error for PtyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}
