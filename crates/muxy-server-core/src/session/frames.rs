use muxy_protocol::{Color, Cursor, Modes, Row, Run, Size, Style};
use muxy_pty::PtySize;

pub(crate) fn terminal_size(size: Size) -> muxy_terminal::Size {
    muxy_terminal::Size {
        cols: size.cols,
        rows: size.rows,
    }
}

pub(crate) fn pty_size(size: Size) -> PtySize {
    PtySize {
        cols: size.cols,
        rows: size.rows,
    }
}

pub(crate) fn rows(rows: Vec<muxy_terminal::Row>) -> Vec<Row> {
    rows.into_iter()
        .map(|row| Row {
            index: row.index,
            runs: row.runs.into_iter().map(run).collect(),
        })
        .collect()
}

pub(crate) fn cursor(cursor: muxy_terminal::Cursor) -> Cursor {
    Cursor {
        row: cursor.row,
        col: cursor.col,
        visible: cursor.visible,
    }
}

pub(crate) fn modes(modes: muxy_terminal::Modes) -> Modes {
    Modes {
        application_cursor_keys: modes.application_cursor_keys,
        bracketed_paste: modes.bracketed_paste,
    }
}

pub(crate) fn input_modes(modes: muxy_terminal::InputModes) -> muxy_protocol::InputModes {
    muxy_protocol::InputModes {
        mouse_tracking: modes.mouse_tracking,
        alternate_scroll: modes.alternate_scroll,
        focus_events: modes.focus_events,
    }
}

pub(crate) fn mouse(event: muxy_protocol::MouseEvent) -> muxy_terminal::MouseEvent {
    use muxy_protocol::{MouseAction, MouseButton, ScrollDirection};
    muxy_terminal::MouseEvent {
        action: match event.action {
            MouseAction::Press => muxy_terminal::MouseAction::Press,
            MouseAction::Release => muxy_terminal::MouseAction::Release,
            MouseAction::Motion => muxy_terminal::MouseAction::Motion,
            MouseAction::Scroll => muxy_terminal::MouseAction::Scroll,
        },
        button: event.button.map(|button| match button {
            MouseButton::Left => muxy_terminal::MouseButton::Left,
            MouseButton::Middle => muxy_terminal::MouseButton::Middle,
            MouseButton::Right => muxy_terminal::MouseButton::Right,
            MouseButton::Back => muxy_terminal::MouseButton::Back,
            MouseButton::Forward => muxy_terminal::MouseButton::Forward,
        }),
        column: event.column,
        row: event.row,
        scroll: event.scroll.map(|direction| match direction {
            ScrollDirection::Up => muxy_terminal::ScrollDirection::Up,
            ScrollDirection::Down => muxy_terminal::ScrollDirection::Down,
        }),
        modifiers: muxy_terminal::Modifiers {
            shift: event.modifiers.shift,
            alt: event.modifiers.alt,
            ctrl: event.modifiers.ctrl,
        },
    }
}

fn run(run: muxy_terminal::Run) -> Run {
    Run {
        text: run.text,
        width: run.width,
        style: style(run.style),
    }
}

fn style(style: muxy_terminal::Style) -> Style {
    Style {
        fg: color(style.fg),
        bg: color(style.bg),
        bold: style.bold,
        italic: style.italic,
        underline: style.underline,
        inverse: style.inverse,
        strikethrough: style.strikethrough,
        faint: style.faint,
    }
}

fn color(color: muxy_terminal::Color) -> Color {
    match color {
        muxy_terminal::Color::Default => Color::Default,
        muxy_terminal::Color::Indexed(index) => Color::Indexed(index),
        muxy_terminal::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}
