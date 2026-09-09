use muxy_core::shortcuts::ShortcutId;

use crate::scrollbar::{MINIMUM_THUMB_LENGTH, TRACK_INSET, ThumbGeometry, WIDTH};
use crate::theme::{Metrics, Theme};
use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, DispatchPhase, Element,
    ElementId, ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, GlobalElementId, Hsla, InteractiveElement, IntoElement, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, ParentElement, Pixels, Point, Render,
    ScrollWheelEvent, ShapedLine, SharedString, Size, Style, Styled, Task, TextAlign, TextRun,
    UTF16Selection, UnderlineStyle, Window, WrapBoundary, WrappedLine, actions, div, fill, point,
    px, size,
};
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        DeleteWord,
        Left,
        Right,
        WordLeft,
        WordRight,
        SelectLeft,
        SelectRight,
        SelectWordLeft,
        SelectWordRight,
        SelectAll,
        SelectHome,
        SelectEnd,
        Home,
        End,
        ShowCharacterPalette,
        Paste,
        Cut,
        Copy,
        Submit,
        Cancel,
        InsertNewline,
        Up,
        Down,
        SelectUp,
        SelectDown,
        PageUp,
        PageDown,
        SelectPageUp,
        SelectPageDown,
        DocumentStart,
        DocumentEnd,
        SelectDocumentStart,
        SelectDocumentEnd,
        Undo,
        Redo,
    ]
);

pub const DEFAULT_CONTEXT: &str = "TextInput";
pub const BARE_CONTEXT: &str = "BareInput";
pub const MULTILINE_CONTEXT: &str = "MultilineInput";
pub const SEARCH_CONTEXT: &str = "TerminalSearchInput";

pub fn growing_input(input: &Entity<TextInput>) -> impl IntoElement {
    div().flex_grow().min_w(px(0.0)).child(input.clone())
}

pub fn register_shortcuts(registry: &mut crate::shortcuts::Registry<'_>) {
    registry.register(ShortcutId::TextInputBackspace, &Backspace);
    registry.register(ShortcutId::TextInputDelete, &Delete);
    registry.register(ShortcutId::TextInputLeft, &Left);
    registry.register(ShortcutId::TextInputRight, &Right);
    registry.register(ShortcutId::TextInputWordLeft, &WordLeft);
    registry.register(ShortcutId::TextInputWordRight, &WordRight);
    registry.register(ShortcutId::TextInputHome, &Home);
    registry.register(ShortcutId::TextInputEnd, &End);
    registry.register(ShortcutId::TextInputSelectLeft, &SelectLeft);
    registry.register(ShortcutId::TextInputSelectRight, &SelectRight);
    registry.register(ShortcutId::TextInputSelectWordLeft, &SelectWordLeft);
    registry.register(ShortcutId::TextInputSelectWordRight, &SelectWordRight);
    registry.register(ShortcutId::TextInputSelectHome, &SelectHome);
    registry.register(ShortcutId::TextInputSelectEnd, &SelectEnd);
    registry.register(ShortcutId::TextInputSelectAll, &SelectAll);
    registry.register(ShortcutId::TextInputCopy, &Copy);
    registry.register(ShortcutId::TextInputCut, &Cut);
    registry.register(ShortcutId::TextInputPaste, &Paste);
    registry.register(ShortcutId::TextInputUndo, &Undo);
    registry.register(ShortcutId::TextInputRedo, &Redo);
    registry.register(ShortcutId::TextInputDocumentStart, &DocumentStart);
    registry.register(ShortcutId::TextInputDocumentEnd, &DocumentEnd);
    registry.register(
        ShortcutId::TextInputShowCharacterPalette,
        &ShowCharacterPalette,
    );
    registry.register(ShortcutId::TextInputInsertNewline, &InsertNewline);
    registry.register(ShortcutId::TextInputUp, &Up);
    registry.register(ShortcutId::TextInputDown, &Down);
    registry.register(ShortcutId::TextInputSelectUp, &SelectUp);
    registry.register(ShortcutId::TextInputSelectDown, &SelectDown);
    registry.register(ShortcutId::TextInputDeleteWord, &DeleteWord);
    registry.register(ShortcutId::TextInputPageUp, &PageUp);
    registry.register(ShortcutId::TextInputPageDown, &PageDown);
    registry.register(ShortcutId::TextInputSelectPageUp, &SelectPageUp);
    registry.register(ShortcutId::TextInputSelectPageDown, &SelectPageDown);
    registry.register(
        ShortcutId::TextInputSelectDocumentStart,
        &SelectDocumentStart,
    );
    registry.register(ShortcutId::TextInputSelectDocumentEnd, &SelectDocumentEnd);
    registry.register(ShortcutId::TextInputSubmit, &Submit);
    registry.register(ShortcutId::TextInputCancel, &Cancel);
}

pub fn key_bindings() -> Vec<gpui::KeyBinding> {
    let mut registry = crate::shortcuts::Registry::new(&muxy_core::shortcuts::Defaults);
    register_shortcuts(&mut registry);
    registry.into_bindings()
}

pub fn line_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for (index, character) in text.char_indices() {
        if character == '\n' {
            ranges.push(start..index);
            start = index + 1;
        }
    }
    ranges.push(start..text.len());
    ranges
}

fn line_index_for(ranges: &[Range<usize>], offset: usize) -> usize {
    ranges
        .iter()
        .position(|range| offset >= range.start && offset <= range.end)
        .unwrap_or(ranges.len().saturating_sub(1))
}

fn first_row_of(rows: &[usize], line: usize) -> usize {
    rows.iter().take(line).sum()
}

fn line_at_row(rows: &[usize], row: usize) -> (usize, usize) {
    let mut remaining = row;
    for (index, count) in rows.iter().enumerate() {
        if remaining < *count {
            return (index, remaining);
        }
        remaining -= count;
    }
    let last = rows.len().saturating_sub(1);
    let height = rows.get(last).copied().unwrap_or(1);
    (last, height.saturating_sub(1))
}

fn clamp_scroll(
    scroll: Point<Pixels>,
    content: Size<Pixels>,
    viewport: Size<Pixels>,
) -> Point<Pixels> {
    let horizontal = (content.width - viewport.width).max(px(0.0));
    let vertical = (content.height - viewport.height).max(px(0.0));
    point(
        scroll.x.clamp(px(0.0), horizontal),
        scroll.y.clamp(px(0.0), vertical),
    )
}

fn word_range_at(text: &str, offset: usize) -> Range<usize> {
    let mut fallback = None;
    for (index, word) in text.split_word_bound_indices() {
        let end = index + word.len();
        if offset < index {
            break;
        }
        if offset <= end {
            if !word.trim().is_empty() {
                return index..end;
            }
            fallback.get_or_insert(index..end);
        }
    }
    fallback.unwrap_or(offset..offset)
}

fn hard_line_range_at(text: &str, offset: usize) -> Range<usize> {
    let ranges = line_ranges(text);
    ranges[line_index_for(&ranges, offset)].clone()
}

fn valid_offset(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn selection_cursor(range: &Range<usize>, reversed: bool) -> usize {
    if reversed { range.start } else { range.end }
}

fn replace_content(content: &str, range: Range<usize>, new_text: &str) -> SharedString {
    (content[0..range.start].to_owned() + new_text + &content[range.end..]).into()
}

fn normalized_paste_text(text: String, multiline: bool) -> String {
    if multiline {
        text
    } else {
        text.replace('\n', " ")
    }
}

fn utf8_offset_from_utf16(text: &str, offset: usize) -> usize {
    let mut utf8_offset = 0;
    let mut utf16_count = 0;
    for character in text.chars() {
        if utf16_count >= offset {
            break;
        }
        utf16_count += character.len_utf16();
        utf8_offset += character.len_utf8();
    }
    utf8_offset
}

fn utf16_offset_from_utf8(text: &str, offset: usize) -> usize {
    let mut utf16_offset = 0;
    let mut utf8_count = 0;
    for character in text.chars() {
        if utf8_count >= offset {
            break;
        }
        utf8_count += character.len_utf8();
        utf16_offset += character.len_utf16();
    }
    utf16_offset
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Granularity {
    Character,
    Word,
    Line,
}

const UNDO_LIMIT: usize = 256;
const BLINK_INTERVAL: Duration = Duration::from_millis(530);

#[derive(Clone)]
struct Edit {
    at: usize,
    removed: String,
    inserted: String,
    before: Range<usize>,
    reversed: bool,
}

#[derive(Default)]
struct History {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    coalesce: bool,
    composing: Option<Edit>,
}

impl History {
    fn record(&mut self, edit: Edit, coalescing: bool) {
        self.redo.clear();
        let breaks = edit.inserted.contains('\n') || edit.inserted.contains(' ');
        if coalescing
            && self.coalesce
            && !breaks
            && let Some(top) = self.undo.last_mut()
        {
            if edit.removed.is_empty()
                && top.removed.is_empty()
                && edit.at == top.at + top.inserted.len()
            {
                top.inserted.push_str(&edit.inserted);
                self.coalesce = true;
                return;
            }
            if edit.inserted.is_empty()
                && top.inserted.is_empty()
                && edit.at + edit.removed.len() == top.at
            {
                let mut removed = edit.removed.clone();
                removed.push_str(&top.removed);
                top.removed = removed;
                top.at = edit.at;
                self.coalesce = true;
                return;
            }
        }
        self.undo.push(edit);
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.coalesce = coalescing && !breaks;
    }

    fn begin_composition(
        &mut self,
        at: usize,
        removed: String,
        before: Range<usize>,
        reversed: bool,
    ) {
        if self.composing.is_none() {
            self.composing = Some(Edit {
                at,
                removed,
                inserted: String::new(),
                before,
                reversed,
            });
        }
    }

    fn end_composition(&mut self, inserted: String) {
        if let Some(mut edit) = self.composing.take() {
            edit.inserted = inserted;
            self.coalesce = false;
            if edit.removed != edit.inserted {
                self.record(edit, false);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputStyle {
    pub text: Hsla,
    pub placeholder: Hsla,
    pub ghost: Hsla,
    pub cursor: Hsla,
    pub selection: Hsla,
    pub scrollbar: Hsla,
    pub font_size: Pixels,
    pub line_height: Pixels,
}

impl InputStyle {
    pub fn field(theme: &Theme, metrics: &Metrics) -> Self {
        Self {
            text: theme.fg,
            placeholder: theme.fg_dim,
            ghost: theme.fg_alpha(0.35),
            cursor: theme.accent,
            selection: theme.accent_soft,
            scrollbar: theme.fg_alpha(0.35),
            font_size: metrics.font_emphasis(),
            line_height: metrics.line_height_field(),
        }
    }

    pub fn compact(theme: &Theme, metrics: &Metrics) -> Self {
        Self {
            text: theme.fg,
            placeholder: theme.fg_dim,
            ghost: theme.fg_alpha(0.35),
            cursor: theme.accent,
            selection: theme.accent_soft,
            scrollbar: theme.fg_alpha(0.35),
            font_size: metrics.font_body(),
            line_height: metrics.line_height_compact(),
        }
    }
}

struct Layout {
    lines: Vec<WrappedLine>,
    ranges: Vec<Range<usize>>,
    rows: Vec<usize>,
    line_height: Pixels,
    bounds: Bounds<Pixels>,
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Text rows are mapped to GPUI f32 coordinates and clamped before indexing."
)]
impl Layout {
    fn total_rows(&self) -> usize {
        self.rows.iter().sum()
    }

    fn content_size(&self) -> Size<Pixels> {
        let width = self
            .lines
            .iter()
            .map(|line| line.size(self.line_height).width)
            .max()
            .unwrap_or(px(0.0));
        size(width, self.line_height * self.total_rows() as f32)
    }

    fn position_for_offset(&self, offset: usize) -> Point<Pixels> {
        let line = line_index_for(&self.ranges, offset);
        let Some(range) = self.ranges.get(line) else {
            return point(px(0.0), px(0.0));
        };
        let local = offset.clamp(range.start, range.end) - range.start;
        let base = self.line_height * first_row_of(&self.rows, line) as f32;
        let Some(shaped) = self.lines.get(line) else {
            return point(px(0.0), base);
        };
        let position = shaped
            .position_for_index(local, self.line_height)
            .unwrap_or_else(|| point(shaped.size(self.line_height).width, px(0.0)));
        point(position.x, position.y + base)
    }

    fn offset_for_position(&self, position: Point<Pixels>) -> usize {
        let total = self.total_rows();
        if total == 0 || self.line_height <= px(0.0) {
            return 0;
        }
        let row = (f32::from(position.y) / f32::from(self.line_height)).floor();
        let row = (row.max(0.0) as usize).min(total - 1);
        let (line, local_row) = line_at_row(&self.rows, row);
        let Some(range) = self.ranges.get(line) else {
            return 0;
        };
        let Some(shaped) = self.lines.get(line) else {
            return range.start;
        };
        let local = point(position.x, self.line_height * local_row as f32);
        let index = shaped
            .closest_index_for_position(local, self.line_height)
            .unwrap_or_else(|index| index);
        (range.start + index).min(range.end)
    }

    fn row_of_offset(&self, offset: usize) -> usize {
        let position = self.position_for_offset(offset);
        if self.line_height <= px(0.0) {
            return 0;
        }
        (f32::from(position.y) / f32::from(self.line_height))
            .floor()
            .max(0.0) as usize
    }

    fn end_of_row(&self, row: usize) -> usize {
        let far = self.content_size().width + self.line_height;
        self.offset_for_position(point(far, self.line_height * row as f32))
    }

    fn start_of_row(&self, row: usize) -> usize {
        self.offset_for_position(point(px(0.0), self.line_height * row as f32))
    }

    fn row_span(&self, row: usize) -> Option<(usize, Range<usize>, Pixels)> {
        let (line, local) = line_at_row(&self.rows, row);
        let range = self.ranges.get(line)?;
        let shaped = self.lines.get(line)?;
        let boundaries = shaped.wrap_boundaries();
        let start = match local.checked_sub(1) {
            None => 0,
            Some(previous) => wrap_index(shaped, *boundaries.get(previous)?),
        };
        let end = boundaries
            .get(local)
            .map_or(shaped.unwrapped_layout.len, |boundary| {
                wrap_index(shaped, *boundary)
            });
        let base = shaped.unwrapped_layout.x_for_index(start);
        Some((line, range.start + start..range.start + end, base))
    }

    fn x_of_offset(&self, line: usize, offset: usize) -> Pixels {
        let (Some(range), Some(shaped)) = (self.ranges.get(line), self.lines.get(line)) else {
            return px(0.0);
        };
        shaped
            .unwrapped_layout
            .x_for_index(offset.clamp(range.start, range.end) - range.start)
    }
}

fn wrap_index(line: &WrappedLine, boundary: WrapBoundary) -> usize {
    line.unwrapped_layout
        .runs
        .get(boundary.run_ix)
        .and_then(|run| run.glyphs.get(boundary.glyph_ix))
        .map_or(line.unwrapped_layout.len, |glyph| glyph.index)
}

#[derive(Debug)]
pub enum InputEvent {
    Changed,
    Submitted,
    Cancelled,
}

type PasteDelegate = Rc<RefCell<Box<dyn FnMut(&mut Window, &mut App) -> bool + 'static>>>;

#[must_use]
#[allow(
    clippy::struct_excessive_bools,
    reason = "Focus, selection, wrapping, and cursor visibility are independent editing states."
)]
pub struct TextInput {
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    ghost: SharedString,
    style: InputStyle,
    font_family: Option<SharedString>,
    key_context: &'static str,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<Layout>,
    last_content: Size<Pixels>,
    is_selecting: bool,
    multiline: bool,
    scroll: Point<Pixels>,
    follow_caret: bool,
    granularity: Granularity,
    anchor: Range<usize>,
    drag_position: Option<Point<Pixels>>,
    autoscroll: Option<Task<()>>,
    focused: bool,
    cursor_visible: bool,
    blink: Option<Task<()>>,
    goal_x: Option<Pixels>,
    history: History,
    coalesce_next: bool,
    paste_delegate: Option<PasteDelegate>,
}

impl EventEmitter<InputEvent> for TextInput {}

impl TextInput {
    pub fn new(style: InputStyle, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: SharedString::default(),
            placeholder: SharedString::default(),
            ghost: SharedString::default(),
            style,
            font_family: None,
            key_context: DEFAULT_CONTEXT,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_content: size(px(0.0), px(0.0)),
            is_selecting: false,
            multiline: false,
            scroll: point(px(0.0), px(0.0)),
            follow_caret: false,
            granularity: Granularity::Character,
            anchor: 0..0,
            drag_position: None,
            autoscroll: None,
            focused: false,
            cursor_visible: true,
            blink: None,
            goal_x: None,
            history: History::default(),
            coalesce_next: true,
            paste_delegate: None,
        }
    }

    pub fn multiline(mut self) -> Self {
        self.multiline = true;
        self.key_context = MULTILINE_CONTEXT;
        self
    }

    pub fn with_placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn with_key_context(mut self, context: &'static str) -> Self {
        self.key_context = context;
        self
    }

    pub fn with_font_family(mut self, family: impl Into<SharedString>) -> Self {
        self.font_family = Some(family.into());
        self
    }

    pub fn with_paste_delegate(
        mut self,
        delegate: impl FnMut(&mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        self.paste_delegate = Some(Rc::new(RefCell::new(Box::new(delegate))));
        self
    }

    pub fn with_text(mut self, text: impl Into<SharedString>) -> Self {
        self.content = text.into();
        self.selected_range = self.content.len()..self.content.len();
        self
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    pub fn selected_range(&self) -> Range<usize> {
        self.selected_range.clone()
    }

    pub fn selected_text(&self) -> &str {
        &self.content[self.selected_range.clone()]
    }

    pub fn cursor_offset(&self) -> usize {
        selection_cursor(&self.selected_range, self.selection_reversed)
    }

    pub fn replace_selection(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(marked) = self.marked_range.take() {
            self.history
                .end_composition(self.content[marked].to_string());
        }
        let range = self.selected_range.clone();
        self.apply_edit(range, text, false, false, cx);
    }

    pub fn insert_at_selection(&mut self, text: &str, cx: &mut Context<Self>) {
        self.replace_selection(text, cx);
    }

    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        let text = text.into();
        if text != self.content {
            self.history.coalesce = false;
            self.history.record(
                Edit {
                    at: 0,
                    removed: self.content.to_string(),
                    inserted: text.to_string(),
                    before: self.selected_range.clone(),
                    reversed: self.selection_reversed,
                },
                false,
            );
        }
        self.content = text;
        let end = self.content.len();
        self.selected_range = end..end;
        self.selection_reversed = false;
        self.marked_range = None;
        self.history.composing = None;
        self.follow_caret = true;
        self.goal_x = None;
        cx.notify();
    }

    pub fn set_style(&mut self, style: InputStyle, cx: &mut Context<Self>) {
        if self.style != style {
            self.style = style;
            self.last_layout = None;
            cx.notify();
        }
    }

    pub fn set_font_family(&mut self, family: Option<SharedString>, cx: &mut Context<Self>) {
        if self.font_family != family {
            self.font_family = family;
            self.last_layout = None;
            cx.notify();
        }
    }

    pub fn set_paste_delegate(
        &mut self,
        delegate: impl FnMut(&mut Window, &mut App) -> bool + 'static,
    ) {
        self.paste_delegate = Some(Rc::new(RefCell::new(Box::new(delegate))));
    }

    pub fn clear_paste_delegate(&mut self) {
        self.paste_delegate = None;
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>) {
        self.placeholder = placeholder.into();
    }

    pub fn set_ghost(&mut self, ghost: impl Into<SharedString>, cx: &mut Context<Self>) {
        let ghost = ghost.into();
        if self.ghost != ghost {
            self.ghost = ghost;
            cx.notify();
        }
    }

    pub fn select_all_text(&mut self, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        self.follow_caret = true;
        self.goal_x = None;
        self.history.coalesce = false;
        cx.notify();
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.follow_caret = true;
        self.goal_x = None;
        self.history.coalesce = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.follow_caret = true;
        self.goal_x = None;
        self.history.coalesce = false;
        cx.notify();
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.previous_word_boundary(self.cursor_offset()), cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.next_word_boundary(self.cursor_offset()), cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_word_boundary(self.cursor_offset()), cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_word_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_all_text(cx);
    }

    fn valid(&self, offset: usize) -> usize {
        valid_offset(&self.content, offset)
    }

    fn row_start(&self) -> usize {
        let Some(layout) = self.last_layout.as_ref() else {
            return 0;
        };
        self.valid(layout.start_of_row(layout.row_of_offset(self.cursor_offset())))
    }

    fn row_end(&self) -> usize {
        let Some(layout) = self.last_layout.as_ref() else {
            return self.content.len();
        };
        self.valid(layout.end_of_row(layout.row_of_offset(self.cursor_offset())))
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.row_start(), cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.row_end(), cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.row_start(), cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.row_end(), cx);
    }

    fn document_start(&mut self, _: &DocumentStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn document_end(&mut self, _: &DocumentEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn select_document_start(
        &mut self,
        _: &SelectDocumentStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(0, cx);
    }

    fn select_document_end(
        &mut self,
        _: &SelectDocumentEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.content.len(), cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete_word(&mut self, _: &DeleteWord, window: &mut Window, cx: &mut Context<Self>) {
        self.coalesce_next = false;
        if self.selected_range.is_empty() {
            self.select_to(self.previous_word_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    #[allow(
        clippy::unused_self,
        reason = "GPUI action listeners use the entity receiver."
    )]
    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        let Some(delegate) = self.paste_delegate.clone() else {
            self.paste_from_clipboard(window, cx);
            return;
        };
        let input = cx.entity();
        window.defer(cx, move |window, cx| {
            let consumed = delegate.borrow_mut()(window, cx);
            if !consumed {
                input.update(cx, |input, cx| input.paste_from_clipboard(window, cx));
            }
        });
    }

    fn paste_from_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = normalized_paste_text(text, self.multiline);
            self.coalesce_next = false;
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn insert_newline(&mut self, _: &InsertNewline, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(None, "\n", window, cx);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, false, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1, false, cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, true, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1, true, cx);
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-self.rows_per_page(), false, cx);
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(self.rows_per_page(), false, cx);
    }

    fn select_page_up(&mut self, _: &SelectPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-self.rows_per_page(), true, cx);
    }

    fn select_page_down(&mut self, _: &SelectPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(self.rows_per_page(), true, cx);
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "A pixel height becomes a whole row count."
    )]
    fn rows_per_page(&self) -> isize {
        let Some(layout) = self.last_layout.as_ref() else {
            return 1;
        };
        if layout.line_height <= px(0.0) {
            return 1;
        }
        ((f32::from(layout.bounds.size.height) / f32::from(layout.line_height)).floor() as isize)
            .max(1)
    }

    fn vertical(&mut self, delta: isize, extend: bool, cx: &mut Context<Self>) {
        let target = self.offset_in_row(delta);
        let goal = self.goal_x;
        if extend {
            self.select_to(target, cx);
        } else {
            self.move_to(target, cx);
        }
        self.goal_x = goal;
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "GPUI measures row positions with f32 coordinates."
    )]
    fn offset_in_row(&mut self, delta: isize) -> usize {
        let offset = self.cursor_offset();
        let length = self.content.len();
        let Some(layout) = self.last_layout.as_ref() else {
            return offset;
        };
        let goal = self
            .goal_x
            .unwrap_or_else(|| layout.position_for_offset(offset).x);
        let total = isize::try_from(layout.total_rows()).unwrap_or(isize::MAX);
        let row = isize::try_from(layout.row_of_offset(offset))
            .unwrap_or(isize::MAX)
            .saturating_add(delta);
        let target = if row < 0 {
            0
        } else if row >= total {
            length
        } else {
            layout.offset_for_position(point(goal, layout.line_height * row as f32))
        };
        self.goal_x = Some(goal);
        self.valid(target)
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.coalesce_next = false;
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "GPUI action listeners use the entity receiver."
    )]
    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Submitted);
    }

    #[allow(
        clippy::unused_self,
        reason = "GPUI action listeners use the entity receiver."
    )]
    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Cancelled);
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.is_selecting = true;
        self.history.coalesce = false;
        let offset = self.index_for_mouse_position(event.position);
        self.granularity = match event.click_count {
            0 | 1 => Granularity::Character,
            2 => Granularity::Word,
            _ => Granularity::Line,
        };
        if event.modifiers.shift && self.granularity == Granularity::Character {
            self.anchor = self.selected_range.clone();
            self.select_to(offset, cx);
            return;
        }
        let range = self.expanded(offset);
        self.anchor = range.clone();
        self.selected_range = range;
        self.selection_reversed = false;
        self.goal_x = None;
        self.follow_caret = true;
        cx.notify();
    }

    fn restart_blink(&mut self, cx: &mut Context<Self>) {
        self.cursor_visible = true;
        self.blink = Some(cx.spawn(async move |input, cx| {
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                let updated = input.update(cx, |input, cx| {
                    input.cursor_visible = !input.cursor_visible;
                    cx.notify();
                });
                if updated.is_err() {
                    break;
                }
            }
        }));
    }

    fn stop_blink(&mut self) {
        self.blink = None;
        self.cursor_visible = true;
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
        self.drag_position = None;
        self.autoscroll = None;
    }

    fn expanded(&self, offset: usize) -> Range<usize> {
        match self.granularity {
            Granularity::Character => offset..offset,
            Granularity::Word => word_range_at(&self.content, offset),
            Granularity::Line => hard_line_range_at(&self.content, offset),
        }
    }

    fn drag_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let offset = self.index_for_mouse_position(position);
        let range = self.expanded(offset);
        let reversed = range.start < self.anchor.start;
        self.selected_range = self.anchor.start.min(range.start)..self.anchor.end.max(range.end);
        self.selection_reversed = reversed;
        self.goal_x = None;
        cx.notify();
    }

    fn on_drag_move(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.drag_position = Some(position);
        self.drag_to(position, cx);
        let outside = self
            .last_layout
            .as_ref()
            .is_some_and(|layout| layout.bounds.localize(&position).is_none());
        if !outside {
            self.autoscroll = None;
            return;
        }
        if self.autoscroll.is_some() {
            return;
        }
        self.autoscroll = Some(cx.spawn(async move |input, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                let Ok(running) = input.update(cx, TextInput::autoscroll_tick) else {
                    break;
                };
                if !running {
                    break;
                }
            }
        }));
    }

    fn autoscroll_tick(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.is_selecting {
            return false;
        }
        let Some(position) = self.drag_position else {
            return false;
        };
        let Some(layout) = self.last_layout.as_ref() else {
            return false;
        };
        let bounds = layout.bounds;
        let step = layout.line_height;
        if bounds.localize(&position).is_some() {
            return false;
        }
        let mut scroll = self.scroll;
        if position.y < bounds.top() {
            scroll.y -= step;
        } else if position.y > bounds.bottom() {
            scroll.y += step;
        }
        if position.x < bounds.left() {
            scroll.x -= step;
        } else if position.x > bounds.right() {
            scroll.x += step;
        }
        self.scroll = clamp_scroll(scroll, self.last_content, bounds.size);
        self.drag_to(position, cx);
        true
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.history.undo.pop() else {
            return;
        };
        let at = self.valid(edit.at);
        let end = self.valid(at + edit.inserted.len());
        self.content =
            (self.content[..at].to_owned() + &edit.removed + &self.content[end..]).into();
        let length = self.content.len();
        self.selected_range = edit.before.start.min(length)..edit.before.end.min(length);
        self.selection_reversed = edit.reversed;
        self.history.redo.push(edit);
        self.finish_history_step(cx);
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.history.redo.pop() else {
            return;
        };
        let at = self.valid(edit.at);
        let end = self.valid(at + edit.removed.len());
        self.content =
            (self.content[..at].to_owned() + &edit.inserted + &self.content[end..]).into();
        let cursor = (at + edit.inserted.len()).min(self.content.len());
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.history.undo.push(edit);
        if self.history.undo.len() > UNDO_LIMIT {
            self.history.undo.remove(0);
        }
        self.finish_history_step(cx);
    }

    fn finish_history_step(&mut self, cx: &mut Context<Self>) {
        self.history.coalesce = false;
        self.history.composing = None;
        self.marked_range = None;
        self.goal_x = None;
        self.follow_caret = true;
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(layout) = self.last_layout.as_ref() else {
            return;
        };
        let viewport = layout.bounds.size;
        let delta = event.delta.pixel_delta(self.style.line_height);
        let before = self.scroll;
        let requested = if self.multiline {
            point(before.x, before.y - delta.y)
        } else {
            point(before.x - delta.x, before.y)
        };
        let clamped = clamp_scroll(requested, self.last_content, viewport);
        let moved = if self.multiline {
            clamped.y != before.y
        } else {
            clamped.x != before.x
        };
        if !moved {
            return;
        }
        self.scroll = clamped;
        cx.stop_propagation();
        cx.notify();
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let Some(layout) = self.last_layout.as_ref() else {
            return 0;
        };
        let local = point(
            position.x - layout.bounds.left() + self.scroll.x,
            position.y - layout.bounds.top() + self.scroll.y,
        );
        self.valid(layout.offset_for_position(local))
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(index, _)| (index < offset).then_some(index))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(index, _)| (index > offset).then_some(index))
            .unwrap_or(self.content.len())
    }

    fn previous_word_boundary(&self, offset: usize) -> usize {
        self.content
            .split_word_bound_indices()
            .filter(|(index, word)| *index < offset && !word.trim().is_empty())
            .map(|(index, _)| index)
            .next_back()
            .unwrap_or(0)
    }

    fn next_word_boundary(&self, offset: usize) -> usize {
        self.content
            .split_word_bound_indices()
            .find(|(index, word)| *index > offset && !word.trim().is_empty())
            .map_or(self.content.len(), |(index, _)| index)
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        utf8_offset_from_utf16(&self.content, offset)
    }

    fn apply_edit(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        committing: bool,
        coalescing: bool,
        cx: &mut Context<Self>,
    ) {
        let removed = self.content[range.clone()].to_string();
        let before = self.selected_range.clone();
        let reversed = self.selection_reversed;
        self.content = replace_content(&self.content, range.clone(), new_text);
        let cursor = range.start + new_text.len();
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.marked_range.take();
        self.follow_caret = true;
        self.goal_x = None;
        self.coalesce_next = true;
        if committing {
            self.history.end_composition(new_text.to_owned());
        } else {
            self.history.record(
                Edit {
                    at: range.start,
                    removed,
                    inserted: new_text.to_owned(),
                    before,
                    reversed,
                },
                coalescing,
            );
        }
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        utf16_offset_from_utf8(&self.content, offset)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        if let Some(marked) = self.marked_range.take() {
            let inserted = self.content[marked].to_string();
            self.history.end_composition(inserted);
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        let committing = self.marked_range.is_some();
        let coalescing = self.coalesce_next;
        self.apply_edit(range, new_text, committing, coalescing, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        if self.marked_range.is_none() {
            self.history.begin_composition(
                range.start,
                self.content[range.clone()].to_string(),
                self.selected_range.clone(),
                self.selection_reversed,
            );
        }

        self.content = replace_content(&self.content, range.clone(), new_text);
        self.marked_range =
            (!new_text.is_empty()).then(|| range.start..range.start + new_text.len());
        if self.marked_range.is_none() {
            self.history.end_composition(String::new());
        }
        self.selected_range = new_selected_range_utf16.as_ref().map_or_else(
            || {
                let cursor = range.start + new_text.len();
                cursor..cursor
            },
            |selected| {
                range.start + utf8_offset_from_utf16(new_text, selected.start)
                    ..range.start + utf8_offset_from_utf16(new_text, selected.end)
            },
        );
        self.selection_reversed = false;
        self.follow_caret = true;
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let start = layout.position_for_offset(range.start);
        let end = layout.position_for_offset(range.end);
        let origin = point(bounds.left() - self.scroll.x, bounds.top() - self.scroll.y);
        Some(Bounds::from_corners(
            point(origin.x + start.x, origin.y + start.y),
            point(origin.x + end.x, origin.y + end.y + layout.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let layout = self.last_layout.as_ref()?;
        layout.bounds.localize(&position)?;
        let local = point(
            position.x - layout.bounds.left() + self.scroll.x,
            position.y - layout.bounds.top() + self.scroll.y,
        );
        let index = layout.offset_for_position(local);
        Some(self.offset_to_utf16(index))
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let style = self.style;
        let mut container = div()
            .flex()
            .key_context(self.key_context)
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .text_size(style.font_size)
            .line_height(style.line_height)
            .text_color(style.text)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::insert_newline))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::select_page_up))
            .on_action(cx.listener(Self::select_page_down))
            .on_action(cx.listener(Self::document_start))
            .on_action(cx.listener(Self::document_end))
            .on_action(cx.listener(Self::select_document_start))
            .on_action(cx.listener(Self::select_document_end))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel));

        container = if self.multiline {
            container.flex_col().flex_grow().min_h(px(0.0))
        } else {
            container.flex_grow()
        };

        if let Some(family) = self.font_family.clone() {
            container = container.font_family(family);
        }

        container.child(TextElement { input: cx.entity() })
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    layout: Layout,
    ghost: Option<ShapedLine>,
    ghost_origin: Point<Pixels>,
    cursor: Option<PaintQuad>,
    selections: Vec<PaintQuad>,
    scrollbar: Option<PaintQuad>,
    scroll: Point<Pixels>,
    content: Size<Pixels>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let multiline = self.input.read(cx).multiline;
        let mut style = Style::default();
        style.size.width = gpui::relative(1.0).into();
        if multiline {
            style.size.height = gpui::relative(1.0).into();
            style.flex_grow = 1.0;
            style.min_size.height = window.line_height().into();
        } else {
            style.size.height = window.line_height().into();
        }
        (window.request_layout(style, [], cx), ())
    }

    #[allow(
        clippy::too_many_lines,
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "GPUI prepaint shapes text and maps visible rows to f32 pixel coordinates."
    )]
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        (): &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let input_style = input.style;
        let multiline = input.multiline;
        let selected_range = input.selected_range.clone();
        let cursor_offset = input.cursor_offset();
        let marked_range = input.marked_range.clone();
        let ghost = input.ghost.clone();
        let mut scroll = input.scroll;
        let follow_caret = input.follow_caret;
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();

        let shows_placeholder = input.content.is_empty();
        let display_text: SharedString = if shows_placeholder {
            input.placeholder.clone()
        } else {
            input.content.clone()
        };
        let color = if shows_placeholder {
            input_style.placeholder
        } else {
            input_style.text
        };

        let run = TextRun {
            len: display_text.len(),
            font: text_style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match marked_range {
            Some(marked) if !shows_placeholder => vec![
                TextRun {
                    len: marked.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked.end - marked.start,
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: display_text.len().saturating_sub(marked.end),
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect(),
            _ => vec![run],
        };

        let wrap_width = multiline.then_some(bounds.size.width);
        let shaped = window
            .text_system()
            .shape_text(display_text.clone(), font_size, &runs, wrap_width, None)
            .unwrap_or_default();

        let ranges = line_ranges(&display_text);
        let lines: Vec<WrappedLine> = shaped.into_iter().collect();
        let rows: Vec<usize> = lines
            .iter()
            .map(|line| 1 + line.wrap_boundaries().len())
            .collect();
        let layout = Layout {
            lines,
            ranges,
            rows,
            line_height,
            bounds,
        };

        let mut content = layout.content_size();
        content.width += px(2.0);
        if follow_caret {
            let caret =
                layout.position_for_offset(if shows_placeholder { 0 } else { cursor_offset });
            if caret.x < scroll.x {
                scroll.x = caret.x;
            } else if caret.x + px(3.0) > scroll.x + bounds.size.width {
                scroll.x = caret.x + px(3.0) - bounds.size.width;
            }
            if caret.y < scroll.y {
                scroll.y = caret.y;
            } else if caret.y + line_height > scroll.y + bounds.size.height {
                scroll.y = caret.y + line_height - bounds.size.height;
            }
        }
        let scroll = clamp_scroll(scroll, content, bounds.size);
        let origin = point(bounds.left() - scroll.x, bounds.top() - scroll.y);

        let cursor = selected_range.is_empty().then(|| {
            let position =
                layout.position_for_offset(if shows_placeholder { 0 } else { cursor_offset });
            fill(
                Bounds::new(
                    point(origin.x + position.x, origin.y + position.y),
                    size(px(1.5), line_height),
                ),
                input_style.cursor,
            )
        });

        let mut selections = Vec::new();
        if !selected_range.is_empty() && !shows_placeholder {
            let first = layout.row_of_offset(selected_range.start);
            let last = layout.row_of_offset(selected_range.end);
            let visible_top = if line_height > px(0.0) {
                (f32::from(scroll.y) / f32::from(line_height))
                    .floor()
                    .max(0.0) as usize
            } else {
                0
            };
            let visible_rows = if line_height > px(0.0) {
                (f32::from(bounds.size.height) / f32::from(line_height)).ceil() as usize + 1
            } else {
                1
            };
            for row in first.max(visible_top)..=last.min(visible_top + visible_rows) {
                let Some((line, span, base)) = layout.row_span(row) else {
                    continue;
                };
                let from = selected_range.start.max(span.start);
                let to = selected_range.end.min(span.end);
                if to < from {
                    continue;
                }
                let left = layout.x_of_offset(line, from) - base;
                let mut right = layout.x_of_offset(line, to) - base;
                if selected_range.end > span.end {
                    right += px(4.0);
                }
                if right <= left {
                    continue;
                }
                let top = origin.y + line_height * row as f32;
                selections.push(fill(
                    Bounds::from_corners(
                        point(origin.x + left, top),
                        point(origin.x + right, top + line_height),
                    ),
                    input_style.selection,
                ));
            }
        }

        let ghost_line = (!ghost.is_empty() && !multiline).then(|| {
            let run = TextRun {
                len: ghost.len(),
                font: text_style.font(),
                color: input_style.ghost,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(ghost, font_size, &[run], None)
        });
        let ghost_origin = point(
            origin.x + layout.position_for_offset(input.content.len()).x,
            origin.y,
        );

        let scrollbar = (multiline && content.height > bounds.size.height)
            .then(|| {
                let inset = f64::from(TRACK_INSET);
                let track = f64::from(f32::from(bounds.size.height)) - inset * 2.0;
                ThumbGeometry::from_lengths(
                    f64::from(f32::from(content.height)),
                    f64::from(f32::from(bounds.size.height)),
                    f64::from(f32::from(scroll.y)),
                    track,
                    MINIMUM_THUMB_LENGTH,
                )
            })
            .flatten()
            .map(|geometry| {
                fill(
                    Bounds::new(
                        point(
                            bounds.right() - px(WIDTH) - px(TRACK_INSET),
                            bounds.top() + px(TRACK_INSET) + px(geometry.origin as f32),
                        ),
                        size(px(WIDTH), px(geometry.length as f32)),
                    ),
                    input_style.scrollbar,
                )
                .corner_radii(px(WIDTH / 2.0))
            });

        PrepaintState {
            layout,
            ghost: ghost_line,
            ghost_origin,
            cursor,
            selections,
            scrollbar,
            scroll,
            content,
        }
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "Selection row indices become GPUI f32 pixel coordinates."
    )]
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        (): &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        let focused = focus_handle.is_focused(window) && window.is_window_active();
        let cursor_visible = self.input.update(cx, |input, cx| {
            if input.focused != focused {
                input.focused = focused;
                if focused {
                    input.restart_blink(cx);
                } else {
                    input.stop_blink();
                }
            } else if focused && input.follow_caret {
                input.restart_blink(cx);
            }
            input.cursor_visible
        });
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );

        let dragging = self.input.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
            if phase != DispatchPhase::Bubble || !dragging.read(cx).is_selecting {
                return;
            }
            let position = event.position;
            dragging.update(cx, |input, cx| input.on_drag_move(position, cx));
        });

        let line_height = prepaint.layout.line_height;
        let scroll = prepaint.scroll;

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for selection in prepaint.selections.drain(..) {
                window.paint_quad(selection);
            }

            let mut row = 0usize;
            for (index, line) in prepaint.layout.lines.iter().enumerate() {
                let base = line_height * row as f32;
                let span = prepaint.layout.rows[index];
                row += span;
                let top = bounds.top() + base - scroll.y;
                let height = line_height * span as f32;
                if top + height < bounds.top() || top > bounds.bottom() {
                    continue;
                }
                let _ = line.paint(
                    point(bounds.left() - scroll.x, top),
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }

            if let Some(ghost) = prepaint.ghost.take() {
                let _ = ghost.paint(prepaint.ghost_origin, line_height, window, cx);
            }

            if focused
                && cursor_visible
                && let Some(cursor) = prepaint.cursor.take()
            {
                window.paint_quad(cursor);
            }

            if let Some(scrollbar) = prepaint.scrollbar.take() {
                window.paint_quad(scrollbar);
            }
        });

        let layout = std::mem::replace(
            &mut prepaint.layout,
            Layout {
                lines: Vec::new(),
                ranges: Vec::new(),
                rows: Vec::new(),
                line_height,
                bounds,
            },
        );
        let content = prepaint.content;
        self.input.update(cx, |input, _| {
            input.last_layout = Some(layout);
            input.last_content = content;
            input.scroll = scroll;
            input.follow_caret = false;
        });
    }
}

impl std::fmt::Debug for TextInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextInput")
            .field("content", &self.content)
            .field("selected_range", &self.selected_range)
            .field("marked_range", &self.marked_range)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "These geometry cases use exactly representable values."
)]
mod tests {
    use gpui::{point, px, size};

    #[test]
    fn line_ranges_splits_on_every_newline_and_keeps_a_trailing_empty_line() {
        assert_eq!(super::line_ranges(""), vec![0..0]);
        assert_eq!(super::line_ranges("abc"), vec![0..3]);
        assert_eq!(super::line_ranges("ab\ncd"), vec![0..2, 3..5]);
        assert_eq!(super::line_ranges("ab\n"), vec![0..2, 3..3]);
        assert_eq!(super::line_ranges("\n\n"), vec![0..0, 1..1, 2..2]);
    }

    #[test]
    fn a_line_index_is_found_for_every_offset_including_line_ends() {
        let ranges = super::line_ranges("ab\ncd\nef");
        assert_eq!(super::line_index_for(&ranges, 0), 0);
        assert_eq!(super::line_index_for(&ranges, 2), 0);
        assert_eq!(super::line_index_for(&ranges, 3), 1);
        assert_eq!(super::line_index_for(&ranges, 5), 1);
        assert_eq!(super::line_index_for(&ranges, 8), 2);
    }

    #[test]
    fn first_row_of_sums_the_visual_rows_of_every_earlier_hard_line() {
        let rows = [1, 3, 2];
        assert_eq!(super::first_row_of(&rows, 0), 0);
        assert_eq!(super::first_row_of(&rows, 1), 1);
        assert_eq!(super::first_row_of(&rows, 2), 4);
        assert_eq!(super::first_row_of(&rows, 3), 6);
    }

    #[test]
    fn line_at_row_resolves_the_hard_line_and_the_row_within_it() {
        let rows = [1, 3, 2];
        assert_eq!(super::line_at_row(&rows, 0), (0, 0));
        assert_eq!(super::line_at_row(&rows, 1), (1, 0));
        assert_eq!(super::line_at_row(&rows, 2), (1, 1));
        assert_eq!(super::line_at_row(&rows, 3), (1, 2));
        assert_eq!(super::line_at_row(&rows, 4), (2, 0));
        assert_eq!(super::line_at_row(&rows, 5), (2, 1));
        assert_eq!(super::line_at_row(&rows, 9), (2, 1));
    }

    #[test]
    fn clamp_scroll_pins_short_content_and_stops_at_the_last_screenful() {
        let viewport = size(px(100.0), px(50.0));
        let short = size(px(80.0), px(20.0));
        assert_eq!(
            super::clamp_scroll(point(px(30.0), px(30.0)), short, viewport),
            point(px(0.0), px(0.0))
        );

        let tall = size(px(300.0), px(250.0));
        assert_eq!(
            super::clamp_scroll(point(px(900.0), px(900.0)), tall, viewport),
            point(px(200.0), px(200.0))
        );
        assert_eq!(
            super::clamp_scroll(point(px(-40.0), px(-40.0)), tall, viewport),
            point(px(0.0), px(0.0))
        );
    }

    fn edit(at: usize, removed: &str, inserted: &str) -> super::Edit {
        super::Edit {
            at,
            removed: removed.to_owned(),
            inserted: inserted.to_owned(),
            before: at..at,
            reversed: false,
        }
    }

    #[test]
    fn a_layout_offset_is_clamped_into_the_content_it_will_index() {
        assert_eq!(super::valid_offset("", 18), 0);
        assert_eq!(super::valid_offset("abc", 99), 3);
        assert_eq!(super::valid_offset("abc", 2), 2);
        assert_eq!(super::valid_offset("héllo", 2), 1);
        assert_eq!(super::valid_offset("日本", 2), 0);
        assert_eq!(super::valid_offset("日本", 3), 3);
    }

    #[test]
    fn selection_helpers_preserve_unicode_ranges_and_direction() {
        let text = "aé👩‍💻日";
        let range = "a".len().."aé👩‍💻".len();
        assert_eq!(&text[range.clone()], "é👩‍💻");
        assert_eq!(super::selection_cursor(&range, false), range.end);
        assert_eq!(super::selection_cursor(&range, true), range.start);
        assert!(text.is_char_boundary(range.start));
        assert!(text.is_char_boundary(range.end));
    }

    #[test]
    fn unicode_offsets_round_trip_across_utf8_and_utf16_boundaries() {
        let text = "a😀日本z";
        for (utf8, utf16) in [(0, 0), (1, 1), (5, 3), (8, 4), (11, 5), (12, 6)] {
            assert_eq!(super::utf16_offset_from_utf8(text, utf8), utf16);
            assert_eq!(super::utf8_offset_from_utf16(text, utf16), utf8);
        }
    }

    #[test]
    fn ime_relative_selection_stays_within_the_inserted_unicode_text() {
        let insertion_start = 5;
        let marked = "日本";
        let selected_utf16 = 1..2;
        let selected_utf8 = insertion_start
            + super::utf8_offset_from_utf16(marked, selected_utf16.start)
            ..insertion_start + super::utf8_offset_from_utf16(marked, selected_utf16.end);
        assert_eq!(selected_utf8, 8..11);
    }

    #[test]
    fn selection_replacement_records_one_undoable_edit() {
        let content = "alpha βeta";
        let range = 6..8;
        let replaced = super::replace_content(content, range.clone(), "B");
        assert_eq!(replaced.as_ref(), "alpha Beta");

        let mut history = super::History::default();
        history.record(
            super::Edit {
                at: range.start,
                removed: content[range.clone()].to_owned(),
                inserted: "B".to_owned(),
                before: range,
                reversed: false,
            },
            false,
        );
        assert_eq!(history.undo.len(), 1);
        assert_eq!(history.undo[0].removed, "β");
        assert_eq!(history.undo[0].inserted, "B");
    }

    #[test]
    fn ordinary_paste_fallback_preserves_multiline_and_flattens_single_line() {
        let text = "first\nsecond".to_owned();
        assert_eq!(super::normalized_paste_text(text.clone(), true), text);
        assert_eq!(super::normalized_paste_text(text, false), "first second");
    }

    #[test]
    fn word_range_at_prefers_the_word_over_an_adjacent_separator() {
        let text = "foo bar-baz";
        assert_eq!(super::word_range_at(text, 1), 0..3);
        assert_eq!(super::word_range_at(text, 3), 0..3);
        assert_eq!(super::word_range_at(text, 5), 4..7);
        assert_eq!(super::word_range_at(text, 8), 7..8);
        assert_eq!(super::word_range_at(text, 11), 8..11);
    }

    #[test]
    fn hard_line_range_at_covers_every_line_including_a_trailing_empty_one() {
        let text = "a\nbb\n";
        assert_eq!(super::hard_line_range_at(text, 0), 0..1);
        assert_eq!(super::hard_line_range_at(text, 1), 0..1);
        assert_eq!(super::hard_line_range_at(text, 3), 2..4);
        assert_eq!(super::hard_line_range_at(text, 5), 5..5);
    }

    #[test]
    fn a_typing_run_coalesces_until_a_space_a_newline_or_a_caret_move_breaks_it() {
        let mut history = super::History::default();
        history.record(edit(0, "", "a"), true);
        history.record(edit(1, "", "b"), true);
        assert_eq!(history.undo.len(), 1);
        assert_eq!(history.undo[0].inserted, "ab");

        history.record(edit(2, "", " "), true);
        assert_eq!(history.undo.len(), 2);
        history.record(edit(3, "", "c"), true);
        assert_eq!(history.undo.len(), 3);

        history.record(edit(4, "", "\n"), true);
        assert_eq!(history.undo.len(), 4);

        history.coalesce = false;
        history.record(edit(5, "", "d"), true);
        assert_eq!(history.undo.len(), 5);
    }

    #[test]
    fn a_paste_and_a_whole_document_replacement_are_never_merged() {
        let mut history = super::History::default();
        history.record(edit(0, "", "a"), true);
        history.record(edit(1, "", "pasted"), false);
        assert_eq!(history.undo.len(), 2);
        history.record(edit(7, "", "b"), true);
        assert_eq!(history.undo.len(), 3);

        let mut history = super::History::default();
        history.record(edit(0, "", "a"), true);
        history.record(edit(0, "a", "whole"), false);
        assert_eq!(history.undo.len(), 2);
    }

    #[test]
    fn a_composition_records_one_entry_holding_the_pre_composition_text() {
        let mut history = super::History::default();
        history.begin_composition(2, "old".to_owned(), 2..5, false);
        history.begin_composition(2, "ignored".to_owned(), 2..5, false);
        history.end_composition("committed".to_owned());
        assert_eq!(history.undo.len(), 1);
        assert_eq!(history.undo[0].removed, "old");
        assert_eq!(history.undo[0].inserted, "committed");
        assert!(history.composing.is_none());
    }

    #[test]
    fn canceled_composition_does_not_move_the_next_undo_target() {
        let mut history = super::History::default();
        history.redo.push(edit(0, "", "redo"));
        history.begin_composition(2, String::new(), 2..2, false);
        history.end_composition(String::new());
        assert!(history.composing.is_none());
        assert!(history.undo.is_empty());
        assert_eq!(history.redo.len(), 1);
        history.begin_composition(0, String::new(), 0..0, false);
        history.end_composition("Y".into());
        assert_eq!(history.undo.len(), 1);
        let edit = &history.undo[0];
        let restored =
            super::replace_content("Yab", edit.at..edit.at + edit.inserted.len(), &edit.removed);
        assert_eq!(restored, "ab");
    }

    #[test]
    fn canceled_selection_replacement_remains_undoable() {
        let mut history = super::History::default();
        history.begin_composition(1, "b".into(), 1..2, false);
        history.end_composition(String::new());
        assert_eq!(history.undo.len(), 1);
        let edit = &history.undo[0];
        assert_eq!(
            super::replace_content("a", edit.at..edit.at, &edit.removed),
            "ab"
        );
    }

    #[test]
    fn recording_clears_the_redo_stack_and_the_undo_stack_is_capped() {
        let mut history = super::History::default();
        history.redo.push(edit(0, "", "x"));
        history.record(edit(0, "", "y"), false);
        assert!(history.redo.is_empty());

        let mut history = super::History::default();
        for index in 0..(super::UNDO_LIMIT + 10) {
            history.record(edit(index, "", "pasted"), false);
        }
        assert_eq!(history.undo.len(), super::UNDO_LIMIT);
        assert_eq!(history.undo[0].at, 10);
    }

    #[test]
    fn backward_deletes_coalesce_into_a_single_entry() {
        let mut history = super::History::default();
        history.record(edit(4, "d", ""), true);
        history.record(edit(3, "c", ""), true);
        assert_eq!(history.undo.len(), 1);
        assert_eq!(history.undo[0].removed, "cd");
        assert_eq!(history.undo[0].at, 3);
    }
}

#[cfg(test)]
mod gpui_regression_tests {
    use super::*;
    use crate::theme::ColorScheme;
    use gpui::{TestAppContext, VisualTestContext};

    fn open<'a>(
        cx: &'a mut TestAppContext,
        text: &str,
    ) -> (Entity<TextInput>, &'a mut VisualTestContext) {
        cx.update(|cx| cx.bind_keys(key_bindings()));
        let text = text.to_owned();
        cx.add_window_view(|window, cx| {
            let input = TextInput::new(
                InputStyle::field(
                    &Theme::from_scheme(&ColorScheme::default()),
                    &Metrics::new(1.0),
                ),
                cx,
            )
            .with_text(text);
            window.focus(&input.focus_handle);
            input
        })
    }

    fn mark(input: &Entity<TextInput>, cx: &mut VisualTestContext, text: &str) {
        cx.update(|window, cx| {
            input.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, text, None, window, cx);
            });
        });
        cx.run_until_parked();
    }

    fn commit(input: &Entity<TextInput>, cx: &mut VisualTestContext, text: &str) {
        cx.update(|window, cx| {
            input.update(cx, |input, cx| {
                input.replace_text_in_range(None, text, window, cx);
            });
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn canceled_ime_does_not_corrupt_the_next_composition_undo(cx: &mut TestAppContext) {
        let (input, cx) = open(cx, "ab");
        mark(&input, cx, "x");
        assert_eq!(
            input.read_with(cx, |input, _| input.text().to_owned()),
            "abx"
        );
        mark(&input, cx, "");
        assert_eq!(
            input.read_with(cx, |input, _| input.text().to_owned()),
            "ab"
        );
        cx.simulate_keystrokes("cmd-left");
        assert_eq!(input.read_with(cx, |input, _| input.selected_range()), 0..0);
        mark(&input, cx, "y");
        commit(&input, cx, "Y");
        assert_eq!(
            input.read_with(cx, |input, _| input.text().to_owned()),
            "Yab"
        );
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(
            input.read_with(cx, |input, _| input.text().to_owned()),
            "ab"
        );
        assert_eq!(input.read_with(cx, |input, _| input.selected_range()), 0..0);
        assert!(input.read_with(cx, |input, _| input.marked_range.is_none()));
    }

    #[gpui::test]
    fn canceled_ime_preserves_redo_for_an_unchanged_document(cx: &mut TestAppContext) {
        let (input, cx) = open(cx, "ab");
        cx.simulate_input("x");
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(
            input.read_with(cx, |input, _| input.text().to_owned()),
            "ab"
        );
        mark(&input, cx, "x");
        mark(&input, cx, "");
        cx.simulate_keystrokes("cmd-shift-z");
        assert_eq!(
            input.read_with(cx, |input, _| input.text().to_owned()),
            "abx"
        );
        assert_eq!(input.read_with(cx, |input, _| input.selected_range()), 3..3);
    }

    #[gpui::test]
    fn canceled_ime_replacing_non_bmp_selection_remains_undoable(cx: &mut TestAppContext) {
        let (input, cx) = open(cx, "a😀b");
        cx.simulate_keystrokes("cmd-left right shift-right");
        assert_eq!(input.read_with(cx, |input, _| input.selected_range()), 1..5);
        cx.update(|window, cx| {
            input.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(Some(1..3), "x", Some(1..1), window, cx);
                assert_eq!(input.marked_text_range(window, cx), Some(1..2));
            });
        });
        cx.run_until_parked();
        mark(&input, cx, "");
        assert_eq!(
            input.read_with(cx, |input, _| input.text().to_owned()),
            "ab"
        );
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(
            input.read_with(cx, |input, _| input.text().to_owned()),
            "a😀b"
        );
        assert_eq!(input.read_with(cx, |input, _| input.selected_range()), 1..5);
        cx.update(|window, cx| {
            input.update(cx, |input, cx| {
                let selection = input.selected_text_range(false, window, cx);
                assert_eq!(selection.map(|selection| selection.range), Some(1..3));
            });
        });
        cx.simulate_keystrokes("cmd-shift-z");
        assert_eq!(
            input.read_with(cx, |input, _| input.text().to_owned()),
            "ab"
        );
    }
}
