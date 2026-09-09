use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use libghostty_vt::render::{
    CellIteration, CellIterator, Dirty, RenderState, RowIteration, RowIterator,
};
use libghostty_vt::screen::{CellContentTag, CellWide, Screen, TrackedGridRef};
use libghostty_vt::style::{PaletteIndex, RgbColor, StyleColor, Underline};
use libghostty_vt::terminal::{
    CompressionMode, Mode, Options, Point, PointCoordinate, PointSpace, ScrollViewport,
    Terminal as Engine,
};
use libghostty_vt::{key, mouse};

use crate::error::{TerminalError, TerminalStep};
use crate::events::{Events, TerminalEvent};
use crate::runs::RunBuilder;
use crate::screen::{
    Color, Cursor, InputModes, Modes, Modifiers, MouseAction, MouseButton, MouseEvent, Row, Run,
    ScrollDirection, Size, Style, hash_runs,
};

type EngineResult<T> = Result<T, libghostty_vt::Error>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalArchive {
    pub size: Size,
    pub rows: Vec<Row>,
    pub cursor: Cursor,
    pub history: Vec<Vec<Run>>,
}

#[derive(Debug)]
pub struct Terminal {
    engine: Engine<'static, 'static>,
    render: RenderState<'static>,
    rows: RowIterator<'static>,
    cells: CellIterator<'static>,
    pty_output: Rc<RefCell<Vec<u8>>>,
    events: Events,
    row_hashes: Vec<Option<u64>>,
    text: String,
    redraw_all: bool,
    size: Size,
    screen_switch: ScreenSwitch,
    primary_history: Result<Vec<Vec<Run>>, String>,
    history_generation: u64,
    history_anchor: Option<(TrackedGridRef, PointCoordinate)>,
    history_screen: Screen,
    history_count: usize,
    mouse_encoder: mouse::Encoder<'static>,
    mouse_event: mouse::Event<'static>,
    held_buttons: Vec<MouseButton>,
}

impl Terminal {
    pub fn new(size: Size, history_budget_bytes: usize) -> Result<Self, TerminalError> {
        let create = |error| TerminalError::wrap(TerminalStep::Create, error);
        let mut engine = Engine::new(Options {
            cols: size.cols,
            rows: size.rows,
            max_scrollback: history_budget_bytes,
        })
        .map_err(create)?;
        engine
            .set_default_cursor_blink(Some(true))
            .map_err(create)?;
        let pty_output = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&pty_output);
        engine
            .on_pty_write(move |_, bytes| sink.borrow_mut().extend_from_slice(bytes))
            .map_err(create)?;
        let events = Events::default();
        let title = Rc::clone(&events.title);
        let directory = Rc::clone(&events.directory);
        let bell = Rc::clone(&events.bell);
        engine
            .on_title_changed(move |_| title.set(true))
            .map_err(create)?;
        engine
            .on_pwd_changed(move |_| directory.set(true))
            .map_err(create)?;
        engine.on_bell(move |_| bell.set(true)).map_err(create)?;
        Ok(Self {
            engine,
            render: RenderState::new().map_err(create)?,
            rows: RowIterator::new().map_err(create)?,
            cells: CellIterator::new().map_err(create)?,
            pty_output,
            events,
            row_hashes: Vec::new(),
            text: String::new(),
            redraw_all: true,
            size,
            screen_switch: ScreenSwitch::default(),
            primary_history: Ok(Vec::new()),
            history_generation: 0,
            history_anchor: None,
            history_screen: Screen::Primary,
            history_count: 0,
            mouse_encoder: {
                let mut encoder = mouse::Encoder::new().map_err(create)?;
                encoder.set_size(mouse_size(size));
                encoder
            },
            mouse_event: mouse::Event::new().map_err(create)?,
            held_buttons: Vec::new(),
        })
    }

    pub fn set_colors(
        &mut self,
        foreground: [u8; 3],
        background: [u8; 3],
        cursor: [u8; 3],
        ansi: [[u8; 3]; 16],
    ) -> Result<(), TerminalError> {
        let colors = |error| TerminalError::wrap(TerminalStep::Colors, error);
        let rgb = |[r, g, b]: [u8; 3]| RgbColor { r, g, b };
        self.engine
            .set_default_fg_color(Some(rgb(foreground)))
            .map_err(colors)?
            .set_default_bg_color(Some(rgb(background)))
            .map_err(colors)?
            .set_default_cursor_color(Some(rgb(cursor)))
            .map_err(colors)?;
        let mut palette = self.engine.default_color_palette().map_err(colors)?;
        for (index, color) in (0_u8..16).zip(ansi) {
            palette.set(PaletteIndex(index), rgb(color));
        }
        self.engine
            .set_default_color_palette(Some(palette))
            .map_err(colors)?;
        Ok(())
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        let mut start = 0;
        for (index, byte) in bytes.iter().copied().enumerate() {
            if self.screen_switch.advance(byte) {
                self.engine.vt_write(&bytes[start..index]);
                start = index;
                if matches!(self.engine.active_screen(), Ok(Screen::Primary)) {
                    self.primary_history =
                        self.capture_history().map_err(|error| error.to_string());
                }
            }
        }
        self.engine.vt_write(&bytes[start..]);
    }

    pub fn take_events(&mut self) -> Vec<TerminalEvent> {
        let mut events = Vec::new();
        if self.events.title.replace(false)
            && let Ok(title) = self.engine.title()
        {
            events.push(TerminalEvent::Title(title.to_owned()));
        }
        if self.events.directory.replace(false)
            && let Ok(directory) = self.engine.pwd()
        {
            events.push(TerminalEvent::Directory(directory.to_owned()));
        }
        if self.events.bell.replace(false) {
            events.push(TerminalEvent::Bell);
        }
        events
    }

    pub fn resize(&mut self, size: Size) -> Result<(), TerminalError> {
        self.engine
            .resize(size.cols, size.rows, 0, 0)
            .map_err(|error| TerminalError::wrap(TerminalStep::Resize, error))?;
        self.redraw_all = true;
        self.size = size;
        self.mouse_encoder.set_size(mouse_size(size));
        self.mouse_encoder.reset();
        self.history_generation = self.history_generation.wrapping_add(1);
        self.history_anchor = None;
        Ok(())
    }

    pub fn cursor_blinking(&self) -> Result<bool, TerminalError> {
        self.engine
            .mode(Mode::CURSOR_BLINKING)
            .map_err(|error| TerminalError::wrap(TerminalStep::Cursor, error))
    }

    pub fn cursor(&mut self) -> Result<Cursor, TerminalError> {
        let query = |error| TerminalError::wrap(TerminalStep::Cursor, error);
        let row = self.engine.cursor_y().map_err(query)?;
        let col = self.engine.cursor_x().map_err(query)?;
        let snapshot = self.render.update(&self.engine).map_err(query)?;
        let visible = snapshot.cursor_visible().map_err(query)?;
        Ok(Cursor { row, col, visible })
    }

    pub fn modes(&self) -> Result<Modes, TerminalError> {
        let query = |error| TerminalError::wrap(TerminalStep::Mode, error);
        Ok(Modes {
            application_cursor_keys: self.engine.mode(Mode::DECCKM).map_err(query)?,
            bracketed_paste: self.engine.mode(Mode::BRACKETED_PASTE).map_err(query)?,
        })
    }

    pub fn input_modes(&self) -> Result<InputModes, TerminalError> {
        let query = |error| TerminalError::wrap(TerminalStep::Mode, error);
        Ok(InputModes {
            mouse_tracking: self.engine.is_mouse_tracking().map_err(query)?,
            alternate_scroll: self.engine.mode(Mode::ALT_SCROLL).map_err(query)?
                && self.engine.active_screen().map_err(query)? == Screen::Alternate,
            focus_events: self.engine.mode(Mode::FOCUS_EVENT).map_err(query)?,
        })
    }

    pub fn encode_mouse(&mut self, event: &MouseEvent) -> Result<Vec<u8>, TerminalError> {
        let modes = self.input_modes()?;
        if !modes.mouse_tracking {
            self.held_buttons.clear();
            self.mouse_encoder.reset();
            if modes.alternate_scroll
                && event.action == MouseAction::Scroll
                && let Some(direction) = event.scroll
            {
                let prefix = if self.modes()?.application_cursor_keys {
                    b'O'
                } else {
                    b'['
                };
                let suffix = match direction {
                    ScrollDirection::Up => b'A',
                    ScrollDirection::Down => b'B',
                };
                return Ok([0x1b, prefix, suffix].repeat(3));
            }
            return Ok(Vec::new());
        }
        if let Some(button) = event.button {
            match event.action {
                MouseAction::Press if !self.held_buttons.contains(&button) => {
                    self.held_buttons.push(button);
                }
                MouseAction::Release => self.held_buttons.retain(|held| *held != button),
                _ => {}
            }
        }
        let (action, button) = match event.action {
            MouseAction::Press => (mouse::Action::Press, event.button.map(mouse_button)),
            MouseAction::Release => (mouse::Action::Release, event.button.map(mouse_button)),
            MouseAction::Motion => (mouse::Action::Motion, event.button.map(mouse_button)),
            MouseAction::Scroll => {
                let Some(direction) = event.scroll else {
                    return Ok(Vec::new());
                };
                (
                    mouse::Action::Press,
                    Some(match direction {
                        ScrollDirection::Up => mouse::Button::Four,
                        ScrollDirection::Down => mouse::Button::Five,
                    }),
                )
            }
        };
        self.mouse_encoder
            .set_options_from_terminal(&self.engine)
            .set_any_button_pressed(!self.held_buttons.is_empty());
        self.mouse_event
            .set_action(action)
            .set_button(button)
            .set_mods(mouse_modifiers(event.modifiers))
            .set_position(mouse::Position {
                x: f32::from(event.column.min(self.size.cols.saturating_sub(1))) + 0.5,
                y: f32::from(event.row.min(self.size.rows.saturating_sub(1))) + 0.5,
            });
        let mut bytes = Vec::new();
        self.mouse_encoder
            .encode_to_vec(&self.mouse_event, &mut bytes)
            .map_err(|error| TerminalError::wrap(TerminalStep::Input, error))?;
        Ok(bytes)
    }

    pub fn screen(&mut self) -> Result<Vec<Row>, TerminalError> {
        let render = |error| TerminalError::wrap(TerminalStep::Render, error);
        let snapshot = self.render.update(&self.engine).map_err(render)?;
        let mut iteration = self.rows.update(&snapshot).map_err(render)?;
        let mut rows = Vec::with_capacity(usize::from(snapshot.rows().map_err(render)?));
        let mut index = 0;
        while let Some(row) = iteration.next() {
            let runs = row_runs(&mut self.cells, row, &mut self.text).map_err(render)?;
            rows.push(Row { index, runs });
            index += 1;
        }
        Ok(rows)
    }

    pub fn take_changed_rows(&mut self) -> Result<Vec<Row>, TerminalError> {
        let render = |error| TerminalError::wrap(TerminalStep::Render, error);
        let snapshot = self.render.update(&self.engine).map_err(render)?;
        let all_dirty = self.redraw_all || snapshot.dirty().map_err(render)? == Dirty::Full;
        self.row_hashes
            .resize(usize::from(snapshot.rows().map_err(render)?), None);
        let mut iteration = self.rows.update(&snapshot).map_err(render)?;
        let mut changed = Vec::new();
        let mut index = 0;
        while let Some(row) = iteration.next() {
            if all_dirty || row.dirty().map_err(render)? {
                let runs = row_runs(&mut self.cells, row, &mut self.text).map_err(render)?;
                let hash = hash_runs(&runs);
                let last = self.row_hashes.get_mut(usize::from(index));
                if self.redraw_all || last.as_deref() != Some(&Some(hash)) {
                    if let Some(last) = last {
                        *last = Some(hash);
                    }
                    changed.push(Row { index, runs });
                }
                row.set_dirty(false).map_err(render)?;
            }
            index += 1;
        }
        snapshot.set_dirty(Dirty::Clean).map_err(render)?;
        self.redraw_all = false;
        Ok(changed)
    }

    pub fn take_pty_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut *self.pty_output.borrow_mut())
    }

    pub fn compress_idle(&mut self) -> Result<(), TerminalError> {
        self.engine
            .compress(CompressionMode::Full)
            .map(drop)
            .map_err(|error| TerminalError::wrap(TerminalStep::Compress, error))
    }

    pub fn history_rows(&self) -> Result<usize, TerminalError> {
        self.engine
            .scrollback_rows()
            .map_err(|error| TerminalError::wrap(TerminalStep::History, error))
    }

    pub fn archive(&mut self) -> Result<TerminalArchive, TerminalError> {
        let history = match self
            .engine
            .active_screen()
            .map_err(|error| TerminalError::wrap(TerminalStep::History, error))?
        {
            Screen::Primary => self.capture_history()?,
            Screen::Alternate => self.primary_history.clone().map_err(|message| {
                TerminalError::wrap(TerminalStep::History, std::io::Error::other(message))
            })?,
        };
        Ok(TerminalArchive {
            size: self.size,
            rows: self.screen()?,
            cursor: self.cursor()?,
            history,
        })
    }

    pub fn history_generation(&mut self) -> Result<u64, TerminalError> {
        let query = |error| TerminalError::wrap(TerminalStep::History, error);
        let screen = self.engine.active_screen().map_err(query)?;
        let count = self.history_rows()?;
        let lost = match &self.history_anchor {
            Some((anchor, point)) => {
                anchor.point(PointSpace::Screen).map_err(query)? != Some(*point)
            }
            None => false,
        };
        if screen != self.history_screen || lost || count < self.history_count {
            self.history_generation = self.history_generation.wrapping_add(1);
            self.history_anchor = None;
            self.history_screen = screen;
        }
        self.history_count = count;
        if self.history_anchor.is_none() {
            let point = PointCoordinate {
                x: 0,
                y: u32::from(count + usize::from(self.size.rows) > 1),
            };
            self.history_anchor = Some((
                self.engine
                    .track_grid_ref(Point::Screen(point))
                    .map_err(query)?,
                point,
            ));
        }
        Ok(self.history_generation)
    }

    pub fn history(&mut self, range: Range<usize>) -> Result<Vec<Row>, TerminalError> {
        let end = range.end.min(self.history_rows()?);
        if range.start >= end {
            return Ok(Vec::new());
        }
        let query = |error| TerminalError::wrap(TerminalStep::History, error);
        let snapshot = self.render.update(&self.engine).map_err(query)?;
        let dirty = snapshot.dirty().map_err(query)?;
        let mut iteration = self.rows.update(&snapshot).map_err(query)?;
        let mut row_dirty = Vec::with_capacity(usize::from(self.size.rows));
        while let Some(row) = iteration.next() {
            row_dirty.push(row.dirty().map_err(query)?);
        }
        let captured = (|| {
            let mut history = Vec::with_capacity(end - range.start);
            for start in (range.start..end).step_by(usize::from(self.size.rows)) {
                self.engine.scroll_viewport(ScrollViewport::Row(start));
                for mut row in self.screen()?.into_iter().take(end - start) {
                    row.index = u16::try_from(history.len()).unwrap_or(u16::MAX);
                    history.push(row);
                }
            }
            Ok(history)
        })();
        self.engine.scroll_viewport(ScrollViewport::Bottom);
        let snapshot = self.render.update(&self.engine).map_err(query)?;
        snapshot.set_dirty(dirty).map_err(query)?;
        let mut iteration = self.rows.update(&snapshot).map_err(query)?;
        for dirty in row_dirty {
            if let Some(row) = iteration.next() {
                row.set_dirty(dirty).map_err(query)?;
            }
        }
        captured
    }

    fn capture_history(&mut self) -> Result<Vec<Vec<Run>>, TerminalError> {
        self.history(0..self.history_rows()?)
            .map(|rows| rows.into_iter().map(|row| row.runs).collect())
    }
}

fn mouse_size(size: Size) -> mouse::EncoderSize {
    mouse::EncoderSize {
        screen_width: u32::from(size.cols),
        screen_height: u32::from(size.rows),
        cell_width: 1,
        cell_height: 1,
        padding_top: 0,
        padding_bottom: 0,
        padding_left: 0,
        padding_right: 0,
    }
}

fn mouse_button(button: MouseButton) -> mouse::Button {
    match button {
        MouseButton::Left => mouse::Button::Left,
        MouseButton::Middle => mouse::Button::Middle,
        MouseButton::Right => mouse::Button::Right,
        MouseButton::Back => mouse::Button::Eight,
        MouseButton::Forward => mouse::Button::Nine,
    }
}

fn mouse_modifiers(modifiers: Modifiers) -> key::Mods {
    let mut mods = key::Mods::empty();
    mods.set(key::Mods::SHIFT, modifiers.shift);
    mods.set(key::Mods::ALT, modifiers.alt);
    mods.set(key::Mods::CTRL, modifiers.ctrl);
    mods
}

#[derive(Debug, Default)]
enum ScreenSwitch {
    #[default]
    Ground,
    Escape,
    Csi,
    Private {
        parameter: u32,
        alternate: bool,
    },
}

impl ScreenSwitch {
    fn advance(&mut self, byte: u8) -> bool {
        if byte == 0x1b {
            *self = Self::Escape;
            return false;
        }
        if byte == 0x9b {
            *self = Self::Csi;
            return false;
        }
        if matches!(byte, 0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f) {
            return false;
        }
        match self {
            Self::Escape if byte == b'[' => *self = Self::Csi,
            Self::Csi if byte == b'?' => {
                *self = Self::Private {
                    parameter: 0,
                    alternate: false,
                };
            }
            Self::Private { parameter, .. } if byte.is_ascii_digit() => {
                *parameter = parameter
                    .saturating_mul(10)
                    .saturating_add(u32::from(byte - b'0'));
            }
            Self::Private {
                parameter,
                alternate,
            } if byte == b';' => {
                *alternate |= matches!(*parameter, 47 | 1047 | 1049);
                *parameter = 0;
            }
            Self::Private {
                parameter,
                alternate,
            } if byte == b'h' => {
                let switch = *alternate || matches!(*parameter, 47 | 1047 | 1049);
                *self = Self::Ground;
                return switch;
            }
            _ => *self = Self::Ground,
        }
        false
    }
}

fn row_runs(
    cells: &mut CellIterator<'static>,
    row: &RowIteration<'static, '_>,
    text: &mut String,
) -> EngineResult<Vec<Run>> {
    let mut builder = RunBuilder::default();
    let mut iteration = cells.update(row)?;
    while let Some(cell) = iteration.next() {
        let raw = cell.raw_cell()?;
        let width = match raw.wide()? {
            CellWide::Narrow => 1,
            CellWide::Wide => 2,
            CellWide::SpacerTail | CellWide::SpacerHead => continue,
        };
        text.clear();
        cell.graphemes_utf8(text)?;
        if text.is_empty() {
            text.push(' ');
        }
        builder.push(text, width, cell_style(cell)?);
    }
    Ok(builder.finish())
}

fn cell_style(cell: &CellIteration<'static, '_>) -> EngineResult<Style> {
    let raw = cell.raw_cell()?;
    let style = cell.style()?;
    let bg = match raw.content_tag()? {
        CellContentTag::BgColorPalette => Color::Indexed(raw.bg_color_palette()?.0),
        CellContentTag::BgColorRgb => {
            let rgb = raw.bg_color_rgb()?;
            Color::Rgb(rgb.r, rgb.g, rgb.b)
        }
        CellContentTag::Codepoint | CellContentTag::CodepointGrapheme => color(style.bg_color),
    };
    Ok(Style {
        fg: color(style.fg_color),
        bg,
        bold: style.bold,
        italic: style.italic,
        underline: style.underline != Underline::None,
        inverse: style.inverse,
        strikethrough: style.strikethrough,
        faint: style.faint,
    })
}

fn color(color: StyleColor) -> Color {
    match color {
        StyleColor::None => Color::Default,
        StyleColor::Palette(index) => Color::Indexed(index.0),
        StyleColor::Rgb(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
    }
}
