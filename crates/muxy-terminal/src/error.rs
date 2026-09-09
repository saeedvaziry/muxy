use std::error::Error;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalStep {
    Create,
    Resize,
    Render,
    Cursor,
    Mode,
    Compress,
    History,
    Input,
    Colors,
}

impl fmt::Display for TerminalStep {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Create => "create",
            Self::Resize => "resize",
            Self::Render => "render",
            Self::Cursor => "cursor",
            Self::Mode => "mode",
            Self::Compress => "compress",
            Self::History => "history",
            Self::Input => "input",
            Self::Colors => "colors",
        };
        formatter.write_str(name)
    }
}

#[derive(Debug)]
pub struct TerminalError {
    step: TerminalStep,
    source: Box<dyn Error + Send + Sync>,
}

impl TerminalError {
    pub(crate) fn wrap(
        step: TerminalStep,
        source: impl Into<Box<dyn Error + Send + Sync>>,
    ) -> Self {
        Self {
            step,
            source: source.into(),
        }
    }

    pub fn step(&self) -> TerminalStep {
        self.step
    }
}

impl fmt::Display for TerminalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "terminal {} failed: {}", self.step, self.source)
    }
}

impl Error for TerminalError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}
