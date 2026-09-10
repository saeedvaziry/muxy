//! Resource targets and opener selection shared by app surfaces.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::{PaneId, ProjectId, ServerId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenContext {
    pub project: ProjectId,
    pub pane: PaneId,
    pub server: ServerId,
    pub directory: PathBuf,
    pub project_directory: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenRequest {
    pub target: Target,
    pub context: OpenContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Target {
    Url(String),
    File(FileLocation),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileLocation {
    pub path: PathBuf,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

impl Target {
    pub fn web_url(text: &str) -> Option<Self> {
        if text.len() > 2048 || text.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return None;
        }
        let (scheme, rest) = text.split_once("://")?;
        if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
            return None;
        }
        let host = rest.split(['/', '?', '#']).next()?;
        if host.is_empty() || host.contains('\\') {
            return None;
        }
        Some(Self::Url(text.into()))
    }

    /// Resolve a local file candidate. The caller supplies filesystem lookup so
    /// UI code can run it on a bounded worker, never in rendering or hit testing.
    pub fn local_file(
        text: &str,
        directory: &Path,
        home: Option<&Path>,
        exists: impl Fn(&Path) -> bool,
    ) -> Option<Self> {
        if text.is_empty() || text.len() > 2048 || text.chars().any(char::is_control) {
            return None;
        }
        let path = if let Some(uri) = text.strip_prefix("file://") {
            let path = if uri.starts_with('/') {
                uri
            } else {
                let (host, path) = uri.split_once('/')?;
                if !host.eq_ignore_ascii_case("localhost") {
                    return None;
                }
                return Self::local_file(&format!("file:///{path}"), directory, home, exists);
            };
            if path.contains(['?', '#']) {
                return None;
            }
            decode_path(path)?
        } else {
            if text.contains("://") {
                return None;
            }
            PathBuf::from(text)
        };
        let path = if let Ok(relative) = path.strip_prefix("~") {
            home?.join(relative)
        } else if path.is_absolute() {
            path
        } else {
            directory.join(path)
        };
        if exists(&path) {
            return Some(Self::File(FileLocation {
                path,
                line: None,
                column: None,
            }));
        }
        let (base, last) = path.to_str()?.rsplit_once(':')?;
        let last = position(last)?;
        let (base, line, column) = base
            .rsplit_once(':')
            .and_then(|(path, line)| position(line).map(|line| (path, line, Some(last))))
            .unwrap_or((base, last, None));
        let path = PathBuf::from(base);
        exists(&path).then_some(Self::File(FileLocation {
            path,
            line: Some(line),
            column,
        }))
    }
}

fn position(text: &str) -> Option<u32> {
    (!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| text.parse::<u32>().ok().filter(|value| *value > 0))
        .flatten()
}

fn decode_path(text: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let mut bytes = Vec::with_capacity(text.len());
    let mut source = text.bytes();
    while let Some(byte) = source.next() {
        let byte = if byte == b'%' {
            let high = char::from(source.next()?).to_digit(16)?;
            let low = char::from(source.next()?).to_digit(16)?;
            u8::try_from(high * 16 + low).ok()?
        } else {
            byte
        };
        if byte.is_ascii_control() {
            return None;
        }
        bytes.push(byte);
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

type Supports = Box<dyn Fn(&OpenRequest) -> bool + Send + Sync>;

pub struct Opener<H> {
    pub id: String,
    pub title: String,
    pub handler: H,
    supports: Supports,
}

impl<H: fmt::Debug> fmt::Debug for Opener<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Opener")
            .field("id", &self.id)
            .field("title", &self.title)
            .field("handler", &self.handler)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct Registry<H> {
    entries: Vec<Opener<H>>,
}

impl<H> Default for Registry<H> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<H> Registry<H> {
    pub fn register(
        &mut self,
        id: impl Into<String>,
        title: impl Into<String>,
        supports: impl Fn(&OpenRequest) -> bool + Send + Sync + 'static,
        handler: H,
    ) -> bool {
        let id = id.into();
        if id.is_empty() || self.entries.iter().any(|entry| entry.id == id) {
            return false;
        }
        self.entries.push(Opener {
            id,
            title: title.into(),
            handler,
            supports: Box::new(supports),
        });
        true
    }

    pub fn available<'a>(
        &'a self,
        request: &'a OpenRequest,
    ) -> impl Iterator<Item = &'a Opener<H>> {
        self.entries
            .iter()
            .filter(|entry| (entry.supports)(request))
    }

    pub fn resolve(
        &self,
        request: &OpenRequest,
        preferred: &str,
        fallback: &str,
    ) -> Option<&Opener<H>> {
        [preferred, fallback].into_iter().find_map(|id| {
            self.entries
                .iter()
                .find(|entry| entry.id == id && (entry.supports)(request))
        })
    }
}
