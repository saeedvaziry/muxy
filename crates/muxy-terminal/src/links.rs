use libghostty_vt::terminal::{Point, PointCoordinate, Terminal};

pub const MAX_LINK_SPANS: usize = 1024;
pub const MAX_LINK_URI: usize = 2048;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkRow {
    pub row: u16,
    pub spans: Vec<LinkSpan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkSpan {
    pub start: u16,
    pub end: u16,
    pub uri: String,
}

#[derive(Debug, Default)]
pub(crate) struct CachedRow {
    pub(crate) spans: Vec<LinkSpan>,
    complete: bool,
}

impl CachedRow {
    pub(crate) fn refresh(
        &mut self,
        engine: &Terminal<'_, '_>,
        row: u16,
        cols: u16,
        dirty: bool,
        budget: usize,
    ) -> Result<bool, libghostty_vt::Error> {
        if !dirty && self.complete && self.spans.len() <= budget {
            return Ok(false);
        }
        let mut spans: Vec<LinkSpan> = Vec::new();
        let mut complete = true;
        let first = engine.grid_ref(Point::Viewport(PointCoordinate {
            x: 0,
            y: u32::from(row),
        }))?;
        if first.row()?.has_hyperlink()? {
            let mut buffer = [0; MAX_LINK_URI];
            for column in 0..cols {
                let cell = engine.grid_ref(Point::Viewport(PointCoordinate {
                    x: column,
                    y: u32::from(row),
                }))?;
                if !cell.cell()?.has_hyperlink()? {
                    continue;
                }
                let length = match cell.hyperlink_uri(&mut buffer) {
                    Ok(length) => length,
                    Err(libghostty_vt::Error::OutOfSpace { .. }) => continue,
                    Err(error) => return Err(error),
                };
                let Ok(uri) = std::str::from_utf8(&buffer[..length]) else {
                    continue;
                };
                if uri.is_empty() || uri.chars().any(char::is_control) {
                    continue;
                }
                if let Some(last) = spans.last_mut()
                    && last.end == column
                    && last.uri == uri
                {
                    last.end = column + 1;
                } else if spans.len() < budget {
                    spans.push(LinkSpan {
                        start: column,
                        end: column + 1,
                        uri: uri.into(),
                    });
                } else {
                    complete = false;
                    break;
                }
            }
        }
        let changed = self.spans != spans;
        self.spans = spans;
        self.complete = complete;
        Ok(changed)
    }
}
