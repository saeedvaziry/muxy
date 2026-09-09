use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;

use muxy_protocol::{
    ErrorCode, HistoryCursor, MAX_COLS, Run, SearchMatch, SearchPage, SessionId, validate_search,
};

use crate::ServerError;

const SCAN_LIMIT: usize = 2000;
// A position includes a column so even a single row can span result pages.
// The remaining bits identify the session, history generation and query.
const POSITION_BITS: u32 = 44;
const POSITION_MASK: u64 = (1 << POSITION_BITS) - 1;

pub(crate) struct Search<'a> {
    pub(crate) session: SessionId,
    pub(crate) generation: u64,
    pub(crate) query: &'a str,
    pub(crate) ignore_case: bool,
    pub(crate) before: HistoryCursor,
    pub(crate) max_results: u16,
    pub(crate) history_rows: usize,
    pub(crate) screen_rows: usize,
}

impl Search<'_> {
    fn position(&self) -> Result<u64, ServerError> {
        validate_search(self.query, self.max_results)
            .map_err(|code| ServerError::new(code, "invalid search query or result limit"))?;
        let total = self.history_rows.saturating_add(self.screen_rows);
        let total = u32::try_from(total).map_err(|_| {
            ServerError::new(
                ErrorCode::HistoryUnavailable,
                "history exceeds the cursor range",
            )
        })?;
        let end = u64::from(total) * u64::from(MAX_COLS);
        if self.before.0 == 0 {
            return Ok(end);
        }
        let position = self.before.0 & POSITION_MASK;
        if position == 0 || position > end || self.before.0 != self.tag() | position {
            return Err(ServerError::new(
                ErrorCode::StaleHistoryCursor,
                "history changed; start a fresh search",
            ));
        }
        Ok(position)
    }

    fn tag(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        (self.session, self.generation, self.query, self.ignore_case).hash(&mut hasher);
        hasher.finish() & !POSITION_MASK
    }

    pub(crate) fn range(&self) -> Result<Range<usize>, ServerError> {
        let end =
            usize::try_from(self.position()?.div_ceil(u64::from(MAX_COLS))).unwrap_or(usize::MAX);
        Ok(end.saturating_sub(SCAN_LIMIT)..end)
    }

    pub(crate) fn scan<R: AsRef<[Run]>>(
        &self,
        mut row: impl FnMut(usize) -> Result<R, ServerError>,
    ) -> Result<SearchPage, ServerError> {
        let mut position = self.position()?;
        let range = self.range()?;
        let mut page = SearchPage {
            matches: Vec::new(),
            next: None,
            total_rows: self.history_rows as u64,
            scanned_rows: 0,
        };
        for index in range.rev() {
            page.scanned_rows += 1;
            let base = index as u64 * u64::from(MAX_COLS);
            let before = position - base;
            for found in row_matches(row(index)?.as_ref(), self.query, self.ignore_case)
                .into_iter()
                .rev()
            {
                if u64::from(found.start) >= before {
                    continue;
                }
                position = base + u64::from(found.start);
                page.matches.push(SearchMatch {
                    row: index as u64,
                    ..found
                });
                if page.matches.len() == usize::from(self.max_results) {
                    page.next = (position > 0).then(|| HistoryCursor(self.tag() | position));
                    return Ok(page);
                }
            }
            position = base;
        }
        page.next = (position > 0).then(|| HistoryCursor(self.tag() | position));
        Ok(page)
    }
}

fn folded(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

fn row_matches(runs: &[Run], query: &str, ignore_case: bool) -> Vec<SearchMatch> {
    let query = if ignore_case {
        folded(query)
    } else {
        query.to_owned()
    };
    let mut text = String::new();
    let mut columns = Vec::new();
    let mut column = 0_u16;
    for run in runs {
        let ascii = run.text.is_ascii() && run.text.len() == usize::from(run.width);
        if ascii {
            if ignore_case {
                text.push_str(&run.text.to_ascii_lowercase());
            } else {
                text.push_str(&run.text);
            }
            for _ in 0..run.width {
                let end = column.saturating_add(1);
                columns.push(column..end);
                column = end;
            }
            continue;
        }
        for character in run.text.chars() {
            let end = column.saturating_add(run.width);
            let value = if ignore_case {
                character.to_lowercase().collect::<String>()
            } else {
                character.to_string()
            };
            columns.extend(std::iter::repeat_n(column..end, value.len()));
            text.push_str(&value);
        }
        column = column.saturating_add(run.width);
    }
    let mut matches = Vec::new();
    for (byte, _) in text.match_indices(&query) {
        let start = columns[byte].start;
        let end = columns[byte + query.len() - 1].end;
        if start < end
            && matches
                .last()
                .is_none_or(|found: &SearchMatch| found.start != start)
        {
            matches.push(SearchMatch { row: 0, start, end });
        }
    }
    matches
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxy_protocol::Style;

    fn run(text: &str, width: u16) -> Run {
        Run {
            text: text.into(),
            width,
            style: Style::default(),
        }
    }

    #[test]
    fn columns_cross_styles_wide_cells_and_lowercase_expansion() {
        let runs = [run("ab", 2), run("界", 2), run("İ", 1), run("CD", 2)];
        assert_eq!(
            row_matches(&runs, "b界İC", false),
            vec![SearchMatch {
                row: 0,
                start: 1,
                end: 6
            }]
        );
        assert_eq!(
            row_matches(&runs, "i\u{307}cd", true),
            vec![SearchMatch {
                row: 0,
                start: 4,
                end: 7
            }]
        );
        assert!(row_matches(&runs, "cd", false).is_empty());
    }

    #[test]
    fn dense_rows_resume_without_missing_or_repeating_matches() -> Result<(), ServerError> {
        let rows = [vec![run(&"a".repeat(4096), 4096)]];
        let mut search = Search {
            session: SessionId::from(std::num::NonZeroU64::MIN),
            generation: 1,
            query: "a",
            ignore_case: false,
            before: HistoryCursor(0),
            max_results: 500,
            history_rows: 0,
            screen_rows: 1,
        };
        let mut matches = Vec::new();
        loop {
            let page = search.scan(|index| Ok(&rows[index]))?;
            assert!(page.scanned_rows <= 1);
            matches.extend(page.matches);
            let Some(next) = page.next else { break };
            search.before = next;
        }
        assert_eq!(matches.len(), 4096);
        assert!(
            matches
                .iter()
                .enumerate()
                .all(|(index, found)| usize::from(found.start) == 4095 - index)
        );
        search.generation += 1;
        assert_eq!(
            search.range().err().map(|error| error.code()),
            Some(ErrorCode::StaleHistoryCursor)
        );
        Ok(())
    }
}
