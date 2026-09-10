use std::ops::Range;

use gpui::{Context, Task};
use muxy_app_core::opener::Target;
use muxy_protocol::{LinkSpan, Run};

use super::{
    pane::TerminalPane,
    selection::{self, Point},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Candidate {
    pub(crate) row: isize,
    pub(crate) columns: Range<u16>,
    pub(crate) text: String,
}

#[derive(Default)]
pub(crate) struct Hover {
    pub(crate) candidate: Option<Candidate>,
    pub(crate) target: Option<Target>,
    task: Option<Task<()>>,
}

pub(crate) fn detect(runs: &[Run], point: Point, links: &[LinkSpan]) -> Option<Candidate> {
    if let Some(span) = links
        .iter()
        .find(|span| (span.start..span.end).contains(&point.column))
    {
        return Some(Candidate {
            row: point.row,
            columns: span.start..span.end,
            text: span.uri.clone(),
        });
    }
    let mut text = String::new();
    let mut mapping = Vec::new();
    for (columns, value) in selection::cells(runs) {
        let start = text.len();
        text.push_str(value);
        if text.len() > 65536 {
            return None;
        }
        mapping.push((start..text.len(), columns));
    }
    let byte = mapping
        .iter()
        .find(|(_, columns)| columns.contains(&point.column))?
        .0
        .start;
    let mut start = 0;
    while start < text.len() {
        let character = text[start..].chars().next()?;
        if character.is_whitespace() {
            start += character.len_utf8();
            continue;
        }
        let quoted = matches!(character, '\'' | '"');
        let body = start + if quoted { character.len_utf8() } else { 0 };
        let end = text[body..]
            .char_indices()
            .find(|(_, c)| {
                if quoted {
                    *c == character
                } else {
                    c.is_whitespace()
                }
            })
            .map_or(text.len(), |(offset, _)| body + offset);
        let value = trim(&text[body..end]);
        let offset = body + text[body..end].len()
            - text[body..end]
                .trim_start_matches(['(', '[', '<', '"', '\''])
                .len();
        let link_start = ["https://", "http://", "file://"]
            .into_iter()
            .filter_map(|prefix| value.find(prefix))
            .min()
            .unwrap_or(0);
        let value = &value[link_start..];
        let begin = offset + link_start;
        let finish = begin + value.len();
        if begin <= byte
            && byte < finish
            && !value.is_empty()
            && value.len() <= muxy_protocol::MAX_LINK_URI
        {
            let first = mapping
                .iter()
                .find(|(bytes, _)| bytes.contains(&begin))?
                .1
                .start;
            let last = mapping
                .iter()
                .rev()
                .find(|(bytes, _)| bytes.start < finish)?
                .1
                .end;
            return Some(Candidate {
                row: point.row,
                columns: first..last,
                text: value.into(),
            });
        }
        start = end
            + if quoted && end < text.len() {
                character.len_utf8()
            } else {
                0
            };
    }
    None
}

fn trim(text: &str) -> &str {
    let mut text = text
        .trim_start_matches(['(', '[', '<', '"', '\''])
        .trim_end_matches(['.', ',', ';', '!', '"', '\'', '>']);
    loop {
        let pair = match text.chars().last() {
            Some(')') => ('(', ')'),
            Some(']') => ('[', ']'),
            Some('}') => ('{', '}'),
            _ => break,
        };
        if text.matches(pair.1).count() <= text.matches(pair.0).count() {
            break;
        }
        text = &text[..text.len() - 1];
    }
    text
}

impl TerminalPane {
    pub(crate) fn link_at(&self, position: gpui::Point<gpui::Pixels>) -> Option<Candidate> {
        if !self.native_visible || !self.geometry?.0.contains(&position) {
            return None;
        }
        let point = self.point_at(position, false)?;
        let grid = self.displayed_grid()?;
        let runs = grid.content_row(grid.history.len().checked_add_signed(point.row)?)?;
        let links = u16::try_from(point.row)
            .ok()
            .map_or(&[][..], |row| grid.links.row(row, grid.size));
        detect(runs, point, links)
    }

    pub(crate) fn hover_link(
        &mut self,
        position: gpui::Point<gpui::Pixels>,
        command: bool,
        cx: &mut Context<Self>,
    ) {
        let candidate = command.then(|| self.link_at(position)).flatten();
        if self.link_hover.candidate == candidate {
            return;
        }
        self.link_hover = Hover {
            candidate: candidate.clone(),
            ..Hover::default()
        };
        cx.notify();
        let Some(candidate) = candidate else { return };
        if let Some(target) = Target::web_url(&candidate.text) {
            self.link_hover.target = Some(target);
            return;
        }
        let Some(context) = self.link_context() else {
            return;
        };
        let text = candidate.text.clone();
        let Ok(result) = crate::opener::submit(move || crate::opener::resolve(&text, &context))
        else {
            return;
        };
        self.link_hover.task = Some(cx.spawn(async move |pane, cx| {
            if let Ok(target) = result.recv().await {
                let _ = pane.update(cx, |pane, cx| {
                    if pane.link_hover.candidate.as_ref() == Some(&candidate) {
                        pane.link_hover.target = target;
                        cx.notify();
                    }
                });
            }
        }));
    }

    pub(crate) fn link_context(&self) -> Option<muxy_app_core::opener::OpenContext> {
        let mut context = self.open_context.clone()?;
        if let Some(directory) = self.directory() {
            context.directory = directory;
        }
        Some(context)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn run(text: &str, width: u16) -> Run {
        Run {
            text: text.into(),
            width,
            style: muxy_protocol::Style::default(),
        }
    }

    #[test]
    fn detection_trims_punctuation_but_keeps_balanced_urls_and_file_locations() {
        for (text, expected) in [
            ("https://example.com/a?b=1.", "https://example.com/a?b=1"),
            ("(https://example.com/a_(b)).", "https://example.com/a_(b)"),
            ("'https://example.com',", "https://example.com"),
            ("url=https://example.com/a,", "https://example.com/a"),
            ("/etc/hosts", "/etc/hosts"),
            ("src/main.rs:12:3", "src/main.rs:12:3"),
            ("\"/tmp/my file.rs\"", "/tmp/my file.rs"),
            ("file:///tmp/my%20file.rs", "file:///tmp/my%20file.rs"),
        ] {
            let byte = text.find(expected).unwrap();
            let candidate = detect(
                &[run(text, u16::try_from(text.len()).unwrap())],
                Point {
                    row: -1,
                    column: u16::try_from(byte + 2).unwrap(),
                },
                &[],
            )
            .unwrap();
            assert_eq!(candidate.text, expected, "{text}");
            assert_eq!(
                candidate.columns,
                u16::try_from(byte).unwrap()..u16::try_from(byte + expected.len()).unwrap()
            );
            assert_eq!(candidate.row, -1);
        }
    }

    #[test]
    fn detection_respects_server_cell_widths_and_prefers_explicit_links() {
        let runs = [
            run("界", 2),
            run("e\u{301}", 1),
            run(" https://example.com", 20),
        ];
        let point = Point { row: 0, column: 5 };
        assert_eq!(detect(&runs, point, &[]).unwrap().columns, 4..23);
        let links = [LinkSpan {
            start: 0,
            end: 10,
            uri: "https://osc.test".into(),
        }];
        assert_eq!(
            detect(&runs, point, &links).unwrap().text,
            "https://osc.test"
        );
        assert!(detect(&runs, Point { row: 0, column: 3 }, &[]).is_none());
    }
}
