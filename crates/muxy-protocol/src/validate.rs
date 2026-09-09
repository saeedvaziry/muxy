use crate::{
    ErrorCode, HistoryCursor, Message, MetadataEvent, MouseAction, MouseEvent, ReplyBody,
    RequestBody, Row, SavedScreen, SearchSource, ServerPath, Size, Version,
};

pub const MAX_COLS: u16 = 4096;
pub const MAX_ROWS: u16 = 1024;
pub const MAX_INPUT: usize = 1024 * 1024;

pub fn validate_search(query: &str, max_results: u16) -> Result<(), ErrorCode> {
    if !(1..=256).contains(&query.len()) {
        return Err(ErrorCode::BadRequest);
    }
    validate_page_size(max_results)
}

pub fn validate_size(size: Size) -> Result<(), ErrorCode> {
    if (1..=MAX_COLS).contains(&size.cols) && (1..=MAX_ROWS).contains(&size.rows) {
        Ok(())
    } else {
        Err(ErrorCode::BadSize)
    }
}

pub fn validate_input(input: &[u8]) -> Result<(), ErrorCode> {
    if input.len() <= MAX_INPUT {
        Ok(())
    } else {
        Err(ErrorCode::BadRequest)
    }
}

pub fn validate_path(path: &ServerPath) -> Result<(), ErrorCode> {
    if path.0.is_empty() {
        Err(ErrorCode::BadPath)
    } else {
        Ok(())
    }
}

pub fn validate_versions(versions: &[Version]) -> Result<(), ErrorCode> {
    if versions.is_empty() {
        Err(ErrorCode::BadRequest)
    } else {
        Ok(())
    }
}

impl Message {
    pub fn validate(&self) -> Result<(), ErrorCode> {
        match self {
            Self::Hello { versions } | Self::HelloReply { versions } => validate_versions(versions),
            Self::Request { body, .. } => validate_request(body),
            Self::Reply { body, .. } => validate_reply(body),
            Self::Input(input) => validate_input(input),
            Self::Mouse(event) => validate_mouse(event),
            Self::Metadata(MetadataEvent::Directory(path)) => validate_path(path),
            Self::FrameAck { .. }
            | Self::VersionUnsupported
            | Self::SessionEnded { .. }
            | Self::Fatal(_)
            | Self::Frame(_)
            | Self::Metadata(
                MetadataEvent::Title(_)
                | MetadataEvent::ForegroundProcess { .. }
                | MetadataEvent::Bell
                | MetadataEvent::History { .. }
                | MetadataEvent::InputModes(_)
                | MetadataEvent::CursorBlinking(_),
            ) => Ok(()),
        }
    }
}

fn validate_mouse(event: &MouseEvent) -> Result<(), ErrorCode> {
    let valid = match event.action {
        MouseAction::Press | MouseAction::Release => {
            event.button.is_some() && event.scroll.is_none()
        }
        MouseAction::Motion => event.scroll.is_none(),
        MouseAction::Scroll => event.button.is_none() && event.scroll.is_some(),
    };
    if valid {
        Ok(())
    } else {
        Err(ErrorCode::BadRequest)
    }
}

fn validate_request(body: &RequestBody) -> Result<(), ErrorCode> {
    match body {
        RequestBody::Search {
            source,
            query,
            max_results,
            ..
        } => {
            if *source == SearchSource::Live(crate::CONTROL) {
                return Err(ErrorCode::UnknownChannel);
            }
            validate_search(query, *max_results)
        }
        RequestBody::CreateSession { directory, size } => {
            validate_path(directory)?;
            validate_size(*size)
        }
        RequestBody::Attach { size, .. } | RequestBody::Resize { size, .. } => validate_size(*size),
        RequestBody::HistoryPage {
            channel, max_rows, ..
        } => {
            if *channel == crate::CONTROL {
                return Err(ErrorCode::UnknownChannel);
            }
            validate_page_size(*max_rows)
        }
        RequestBody::SavedHistoryPage { max_rows, .. } => validate_page_size(*max_rows),
        RequestBody::ListSessions
        | RequestBody::EndSession(_)
        | RequestBody::Detach(_)
        | RequestBody::Ping
        | RequestBody::ReadSavedScreen(_)
        | RequestBody::DiscardSession(_)
        | RequestBody::SetTerminalColors(_) => Ok(()),
    }
}

fn validate_reply(body: &ReplyBody) -> Result<(), ErrorCode> {
    match body {
        ReplyBody::SearchPage(page) => {
            if page.matches.len() > 500
                || page.scanned_rows > 2000
                || page.next == Some(HistoryCursor(0))
                || (page.scanned_rows == 0 && (!page.matches.is_empty() || page.next.is_some()))
                || page.matches.iter().any(|found| {
                    found.start >= found.end
                        || found.end > MAX_COLS
                        || found.row >= page.total_rows.saturating_add(u64::from(MAX_ROWS))
                })
                || page
                    .matches
                    .windows(2)
                    .any(|pair| (pair[0].row, pair[0].start) <= (pair[1].row, pair[1].start))
            {
                return Err(ErrorCode::BadRequest);
            }
            Ok(())
        }
        ReplyBody::Sessions(sessions) => {
            for session in sessions {
                validate_path(&session.directory)?;
            }
            Ok(())
        }
        ReplyBody::SessionCreated(session) => validate_path(&session.directory),
        ReplyBody::Attached { snapshot, .. } => {
            validate_size(snapshot.size)?;
            validate_path(&snapshot.directory)?;
            validate_history(
                &snapshot.history,
                snapshot.history_cursor,
                snapshot.history_total,
                200,
            )
        }
        ReplyBody::HistoryPage(page) => {
            validate_history(&page.rows, page.next, page.total_rows, 500)?;
            if let Some(screen) = &page.screen {
                validate_saved_screen(screen)?;
            }
            Ok(())
        }
        ReplyBody::SavedScreen(screen) => validate_saved_screen(screen),
        ReplyBody::SessionEnded
        | ReplyBody::Detached
        | ReplyBody::Resized
        | ReplyBody::TerminalColorsSet
        | ReplyBody::Pong
        | ReplyBody::Error(_)
        | ReplyBody::SessionDiscarded => Ok(()),
    }
}

fn validate_page_size(max_rows: u16) -> Result<(), ErrorCode> {
    if (1..=500).contains(&max_rows) {
        Ok(())
    } else {
        Err(ErrorCode::BadRequest)
    }
}

fn validate_history(
    rows: &[Row],
    next: Option<HistoryCursor>,
    total: u64,
    limit: usize,
) -> Result<(), ErrorCode> {
    if rows.len() > limit
        || rows.len() as u64 > total
        || next == Some(HistoryCursor(0))
        || (rows.is_empty() && next.is_some())
        || rows.iter().enumerate().any(|(index, row)| {
            usize::from(row.index) != index
                || row.runs.iter().any(|run| run.width == 0)
                || row.runs.iter().map(|run| u64::from(run.width)).sum::<u64>()
                    > u64::from(MAX_COLS)
        })
    {
        Err(ErrorCode::BadRequest)
    } else {
        Ok(())
    }
}

fn validate_saved_screen(screen: &SavedScreen) -> Result<(), ErrorCode> {
    validate_size(screen.size)?;
    if screen.rows.len() != usize::from(screen.size.rows)
        || screen.cursor.row >= screen.size.rows
        || screen.cursor.col >= screen.size.cols
        || screen.rows.iter().enumerate().any(|(index, row)| {
            usize::from(row.index) != index
                || row.runs.iter().any(|run| run.width == 0)
                || row.runs.iter().map(|run| u64::from(run.width)).sum::<u64>()
                    > u64::from(screen.size.cols)
        })
    {
        return Err(ErrorCode::BadRequest);
    }
    Ok(())
}
