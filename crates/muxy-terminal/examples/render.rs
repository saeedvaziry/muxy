use std::env;
use std::error::Error;
use std::fmt::Write as _;
use std::io::{self, Read, Write};
use std::iter;
use std::process::ExitCode;

use muxy_terminal::{Color, Cursor, Row, Size, Style, Terminal};

const CURSOR: &str = "▮";

struct Options {
    size: Size,
    runs: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr(), "render: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let options = parse_args()?;
    let mut input = Vec::new();
    io::stdin().lock().read_to_end(&mut input)?;

    let mut terminal = Terminal::new(options.size, 1 << 20)?;
    terminal.feed(&carriage_returns_before_line_feeds(&input));
    let screen = terminal.screen()?;
    let cursor = terminal.cursor()?;

    let mut stdout = io::stdout().lock();
    for row in &screen {
        writeln!(stdout, "{}", row_text(row, cursor))?;
    }
    if options.runs {
        writeln!(stdout)?;
        for row in &screen {
            let mut col = 0;
            for run in &row.runs {
                writeln!(
                    stdout,
                    "{} {col} {} {} {:?}",
                    row.index,
                    run.width,
                    style_label(run.style),
                    run.text
                )?;
                col += run.width;
            }
        }
    }
    Ok(())
}

fn carriage_returns_before_line_feeds(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut previous = 0;
    for &byte in input {
        if byte == b'\n' && previous != b'\r' {
            output.push(b'\r');
        }
        output.push(byte);
        previous = byte;
    }
    output
}

fn parse_args() -> Result<Options, Box<dyn Error>> {
    let mut options = Options {
        size: Size { cols: 80, rows: 24 },
        runs: false,
    };
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--cols" => options.size.cols = dimension(args.next(), "--cols")?,
            "--rows" => options.size.rows = dimension(args.next(), "--rows")?,
            "--runs" => options.runs = true,
            _ => return Err("usage: render [--cols N] [--rows N] [--runs] < input".into()),
        }
    }
    Ok(options)
}

fn dimension(value: Option<String>, flag: &str) -> Result<u16, Box<dyn Error>> {
    let value = value.ok_or_else(|| format!("{flag} needs a value"))?;
    let parsed: u16 = value.parse()?;
    if parsed == 0 {
        return Err(format!("{flag} must be greater than zero").into());
    }
    Ok(parsed)
}

fn row_text(row: &Row, cursor: Cursor) -> String {
    let mut cells: Vec<String> = Vec::new();
    for run in &row.runs {
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
    if cursor.visible && cursor.row == row.index {
        let col = usize::from(cursor.col);
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

fn style_label(style: Style) -> String {
    let mut label = format!("{}/{}", color_label(style.fg), color_label(style.bg));
    let flags = [
        (style.bold, "bold"),
        (style.italic, "italic"),
        (style.underline, "underline"),
        (style.inverse, "inverse"),
        (style.strikethrough, "strikethrough"),
        (style.faint, "faint"),
    ];
    for (set, name) in flags {
        if set {
            let _ = write!(label, "+{name}");
        }
    }
    label
}

fn color_label(color: Color) -> String {
    match color {
        Color::Default => "default".to_owned(),
        Color::Indexed(index) => format!("i{index}"),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
    }
}
