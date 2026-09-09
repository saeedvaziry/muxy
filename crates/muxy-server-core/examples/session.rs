use std::error::Error;
use std::io::{self, BufRead, Write};
use std::iter;
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc::channel;
use std::{env, thread};

use muxy_protocol::{ChannelId, Cursor, Row, Run, Size};
use muxy_server_core::{
    AttachmentEvent, AttachmentId, Registry, ServerSettings, SessionCommand, SessionHandle,
};

const CURSOR: &str = "▮";
const CLEAR_SCREEN: &str = "\x1b[2J\x1b[H";
const DEFAULT_SIZE: Size = Size { cols: 80, rows: 24 };

struct Screen {
    rows: Vec<Vec<Run>>,
    cursor: Cursor,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr(), "session: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let size = terminal_size();
    let (events, _server_events) = channel();
    let registry = Registry::new(ServerSettings::default(), events);
    let info = registry.create(&env::current_dir()?, size)?;
    let handle = registry
        .handle(info.id)
        .ok_or("created session has no handle")?;

    let (sink, attachment) = channel();
    handle.send(SessionCommand::Attach {
        id: AttachmentId(1),
        channel: ChannelId(1),
        size,
        sink,
    })?;
    let input = handle.clone();
    thread::spawn(move || forward_stdin(&input));

    let mut screen = Screen::new(size);
    let mut stdout = io::stdout().lock();
    for event in attachment {
        match event {
            AttachmentEvent::Snapshot { snapshot, .. } => {
                screen = Screen::new(snapshot.size);
                screen.apply(snapshot.rows, snapshot.cursor);
            }
            AttachmentEvent::Frame(frame) | AttachmentEvent::Resized(frame) => {
                if frame.reset {
                    screen.clear();
                }
                screen.apply(frame.rows, frame.cursor);
            }
            AttachmentEvent::Metadata(event) => {
                writeln!(stdout, "\r\nmetadata: {event:?}")?;
                continue;
            }
            AttachmentEvent::Ended(reason) => {
                writeln!(stdout, "\r\nsession ended ({reason:?})")?;
                return Ok(());
            }
        }
        screen.draw(&mut stdout)?;
    }
    Err("session thread went away".into())
}

fn forward_stdin(handle: &SessionHandle) {
    let mut stdin = io::stdin().lock();
    let mut line = Vec::new();
    loop {
        line.clear();
        match stdin.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if handle.send(SessionCommand::Input(line.clone())).is_err() {
                    return;
                }
            }
        }
    }
    let _ = handle.send(SessionCommand::End);
}

fn terminal_size() -> Size {
    let output = Command::new("stty")
        .arg("size")
        .stdin(Stdio::inherit())
        .stderr(Stdio::null())
        .output();
    let Ok(output) = output else {
        return DEFAULT_SIZE;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut parts = text.split_whitespace().map(|part| part.parse::<u16>().ok());
    match (parts.next().flatten(), parts.next().flatten()) {
        (Some(rows), Some(cols)) if rows > 0 && cols > 0 => Size { cols, rows },
        _ => DEFAULT_SIZE,
    }
}

impl Screen {
    fn new(size: Size) -> Self {
        Self {
            rows: vec![Vec::new(); usize::from(size.rows)],
            cursor: Cursor {
                row: 0,
                col: 0,
                visible: true,
            },
        }
    }

    fn clear(&mut self) {
        for row in &mut self.rows {
            row.clear();
        }
    }

    fn apply(&mut self, rows: Vec<Row>, cursor: Cursor) {
        for row in rows {
            let index = usize::from(row.index);
            if index >= self.rows.len() {
                self.rows.resize(index + 1, Vec::new());
            }
            self.rows[index] = row.runs;
        }
        self.cursor = cursor;
    }

    fn draw(&self, stdout: &mut impl Write) -> io::Result<()> {
        let lines: Vec<String> = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, runs)| {
                let cursor_col = (self.cursor.visible && usize::from(self.cursor.row) == index)
                    .then_some(usize::from(self.cursor.col));
                row_text(runs, cursor_col)
            })
            .collect();
        write!(stdout, "{CLEAR_SCREEN}{}", lines.join("\r\n"))?;
        stdout.flush()
    }
}

fn row_text(runs: &[Run], cursor_col: Option<usize>) -> String {
    let mut cells: Vec<String> = Vec::new();
    for run in runs {
        let start = cells.len();
        let width = usize::from(run.width);
        let all_narrow = run.text.chars().count() == width;
        for ch in run.text.chars() {
            let cell_width = if all_narrow { 1 } else { cell_width(ch) };
            if cell_width == 0 && cells.len() > start {
                if let Some(cell) = cells.last_mut() {
                    cell.push(ch);
                }
                continue;
            }
            cells.push(ch.to_string());
            cells.extend(iter::repeat_n(String::new(), cell_width.saturating_sub(1)));
        }
        cells.truncate(start + width);
        cells.resize(start + width, String::new());
    }
    if let Some(col) = cursor_col {
        if cells.len() <= col {
            cells.resize(col + 1, " ".to_owned());
        }
        CURSOR.clone_into(&mut cells[col]);
    }
    cells.concat()
}

fn cell_width(ch: char) -> usize {
    match u32::from(ch) {
        0x0300..=0x036F | 0x200B..=0x200F | 0xFE00..=0xFE0F => 0,
        0..=0x10FF => 1,
        _ => 2,
    }
}
