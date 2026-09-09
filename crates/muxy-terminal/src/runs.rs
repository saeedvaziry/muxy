use crate::screen::{Run, Style};

#[derive(Debug, Default)]
pub(crate) struct RunBuilder {
    runs: Vec<Run>,
    last_is_ascii: bool,
}

impl RunBuilder {
    pub(crate) fn push(&mut self, text: &str, width: u16, style: Style) {
        let is_ascii = text.is_ascii() && text.len() == usize::from(width);
        match self.runs.last_mut() {
            Some(run) if self.last_is_ascii && is_ascii && run.style == style => {
                run.text.push_str(text);
                run.width = run.width.saturating_add(width);
            }
            _ => self.runs.push(Run {
                text: text.to_owned(),
                width,
                style,
            }),
        }
        self.last_is_ascii = is_ascii;
    }

    pub(crate) fn finish(mut self) -> Vec<Run> {
        while let Some(run) = self.runs.last_mut() {
            if !run.style.is_default() {
                break;
            }
            let trimmed = run.text.trim_end_matches(' ');
            let blanks = run.text.len() - trimmed.len();
            if blanks == 0 {
                break;
            }
            let kept = trimmed.len();
            run.text.truncate(kept);
            run.width = run
                .width
                .saturating_sub(u16::try_from(blanks).unwrap_or(u16::MAX));
            if run.text.is_empty() {
                self.runs.pop();
            }
        }
        self.runs
    }
}
