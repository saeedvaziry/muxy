mod block;
mod padding;

use std::sync::Arc;

use gpui::{
    App, Bounds, Entity, FontFeatures, FontStyle, FontWeight, Hsla, IntoElement, LineLayout,
    Pixels, Point, ShapedLine, Styled, TextRun, Window, canvas, fill, font, point, px, rgb, size,
};
use muxy_protocol::{MAX_COLS, MAX_ROWS, Run, Size, Style};

use super::{colors::Palette, pane::TerminalPane};

#[derive(Default)]
struct Painting {
    lines: Vec<(Point<Pixels>, ShapedLine)>,
    backgrounds: Vec<(Bounds<Pixels>, Hsla)>,
    blocks: Vec<(Bounds<Pixels>, Hsla)>,
    selections: Vec<Bounds<Pixels>>,
    matches: Vec<(Bounds<Pixels>, bool)>,
    decorations: Vec<(Bounds<Pixels>, Hsla)>,
    cursor: Option<Bounds<Pixels>>,
    cell: gpui::Size<Pixels>,
}

pub(crate) fn terminal(view: Entity<TerminalPane>, palette: Palette) -> impl IntoElement {
    let mouse_view = view.clone();
    canvas(
        move |bounds, window, cx| {
            let terminal = &view.read(cx).terminal;
            let font_size = px(terminal.font_size);
            let mut base_font = font(
                terminal
                    .font_families
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "Menlo".into()),
            );
            base_font.fallbacks = Some(gpui::FontFallbacks::from_fonts(
                terminal.font_families.iter().skip(1).cloned().collect(),
            ));
            base_font.features = FontFeatures::disable_ligatures();
            let sample = window.text_system().shape_line(
                "M".into(),
                font_size,
                &[text_run(1, Style::default(), palette, &base_font)],
                None,
            );
            let natural_height = f32::from(sample.ascent + sample.descent);
            let cell = size(
                sample.width.ceil().max(px(1.0)),
                px(terminal
                    .cell_height
                    .apply(natural_height, window.scale_factor())),
            );
            let frame = padding::Frame::new(bounds, cell);
            let viewport = frame.viewport;
            #[cfg(target_os = "macos")]
            if let Some(native) = &view.read(cx).native_scroll {
                let pane = view.read(cx);
                native.sync(muxy_ui::native_scroll::ScrollGeometry {
                    bounds,
                    content_height: f64::from(f32::from(bounds.size.height))
                        + pane.scrollable_rows(viewport.rows) * f64::from(f32::from(cell.height)),
                    from_bottom: pane.scroll.requested_pixels(f32::from(cell.height)),
                    line_height: f64::from(f32::from(cell.height)),
                    revision: pane.scroll.revision,
                    dark: ((palette.background >> 16) & 0xff) * 299
                        + ((palette.background >> 8) & 0xff) * 587
                        + (palette.background & 0xff) * 114
                        < 128_000,
                });
            }
            let weak = view.downgrade();
            window.defer(cx, move |_, cx| {
                let _ = weak.update(cx, |pane, cx| {
                    pane.cell_height = f32::from(cell.height);
                    pane.set_viewport(viewport, cx);
                    pane.geometry = Some((frame.content, cell));
                });
            });
            view.update(cx, |pane, cx| pane.sync_cursor_blink(window, cx));
            let painting = prepare(
                view.read(cx),
                frame.content.origin,
                cell,
                palette,
                &base_font,
                font_size,
                window,
            );
            (painting, frame, frame.backgrounds(view.read(cx), palette))
        },
        move |bounds, (painting, frame, padding), window, cx| {
            let focus_border = mouse_view.read(cx).focus_border;
            let mouse_view = mouse_view.clone();
            window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, _, cx| {
                if phase.bubble() {
                    mouse_view.update(cx, |pane, cx| pane.mouse_move(event, cx));
                }
            });
            for (bounds, color) in padding {
                window.paint_quad(fill(bounds, color));
            }
            window.with_content_mask(
                Some(gpui::ContentMask {
                    bounds: frame.grid.intersect(&frame.content),
                }),
                |window| paint(painting, palette, window, cx),
            );
            if let Some(color) = focus_border {
                window.paint_quad(gpui::outline(bounds, color, gpui::BorderStyle::Solid));
            }
        },
    )
    .size_full()
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn viewport_size(bounds: gpui::Size<Pixels>, cell: gpui::Size<Pixels>) -> Size {
    Size {
        cols: (bounds.width / cell.width)
            .floor()
            .clamp(1.0, f32::from(MAX_COLS)) as u16,
        rows: (bounds.height / cell.height)
            .floor()
            .clamp(1.0, f32::from(MAX_ROWS)) as u16,
    }
}

fn prepare(
    view: &TerminalPane,
    origin: Point<Pixels>,
    cell: gpui::Size<Pixels>,
    palette: Palette,
    base_font: &gpui::Font,
    font_size: Pixels,
    window: &mut Window,
) -> Painting {
    let mut painting = Painting {
        cell,
        ..Painting::default()
    };
    let Some(grid) = view.displayed_grid() else {
        return painting;
    };
    let height = view.viewport().map_or(grid.size.rows, |size| size.rows);
    let scrolled = view.scroll.view.is_some();
    let start = view.visible_start(grid);
    let remainder = view.scroll.pixel_remainder(f32::from(cell.height));
    let origin = origin + point(px(0.0), px(remainder));
    for index in 0..usize::from(height) + usize::from(remainder < 0.0) {
        let Some(runs) = grid.content_row(start + index) else {
            continue;
        };
        let row = u16::try_from(index).unwrap_or(MAX_ROWS);
        let position = origin + point(px(0.0), cell.height * f32::from(row));
        search_highlights(view, start + index, position, cell, &mut painting);
        if let Some(bounds) = selection_bounds(view, start + index, position, cell) {
            painting.selections.push(bounds);
        }
        let mut column = 0_u16;
        let mut text = String::new();
        let mut styles = Vec::new();
        let mut columns = Vec::new();
        for run in runs {
            let (foreground, background) = palette.style(run.style);
            let bounds = Bounds::new(
                position + point(cell.width * f32::from(column), px(0.0)),
                size(cell.width * f32::from(run.width), cell.height),
            );
            if background != palette.background {
                push_quad(&mut painting.backgrounds, bounds, rgb(background).into());
            }
            let mut foreground: Hsla = rgb(foreground).into();
            if run.style.faint {
                foreground.a = 0.5;
            }
            if run.style.underline {
                painting.decorations.push((
                    Bounds::new(
                        bounds.origin + point(px(0.0), cell.height - px(2.0)),
                        size(bounds.size.width, px(1.0)),
                    ),
                    foreground,
                ));
            }
            if run.style.strikethrough {
                painting.decorations.push((
                    Bounds::new(
                        bounds.origin + point(px(0.0), cell.height / 2.0),
                        size(bounds.size.width, px(1.0)),
                    ),
                    foreground,
                ));
            }
            if let Some(quads) = block::quads(&run.text, bounds, foreground, window.scale_factor())
            {
                for (bounds, color) in quads {
                    push_quad(&mut painting.blocks, bounds, color);
                }
            } else {
                append_run(
                    run,
                    column,
                    &mut text,
                    &mut styles,
                    &mut columns,
                    palette,
                    base_font,
                );
            }
            column = column.saturating_add(run.width);
        }
        if !text.is_empty() {
            let mut line = window
                .text_system()
                .shape_line(text.into(), font_size, &styles, None);
            align_to_cells(&mut line, &columns, cell.width, column);
            painting.lines.push((position, line));
        }
    }
    if !scrolled
        && view.focused
        && view.cursor_blink.visible
        && grid.cursor.visible
        && grid.cursor.col < grid.size.cols
        && grid.cursor.row < height
    {
        painting.cursor = Some(Bounds::new(
            origin
                + point(
                    cell.width * f32::from(grid.cursor.col),
                    cell.height * f32::from(grid.cursor.row),
                ),
            cell,
        ));
    }
    painting
}

fn search_highlights(
    view: &TerminalPane,
    index: usize,
    position: Point<Pixels>,
    cell: gpui::Size<Pixels>,
    painting: &mut Painting,
) {
    let Some(grid) = view.displayed_grid() else {
        return;
    };
    if let Some(find) = &view.find {
        for (found, current) in find.results.highlights(grid, index) {
            painting.matches.push((
                Bounds::new(
                    position + point(cell.width * f32::from(found.start), px(0.0)),
                    size(cell.width * f32::from(found.end - found.start), cell.height),
                ),
                current,
            ));
        }
    }
}

fn selection_bounds(
    view: &TerminalPane,
    index: usize,
    position: Point<Pixels>,
    cell: gpui::Size<Pixels>,
) -> Option<Bounds<Pixels>> {
    let selection = view.selection?;
    let grid = view.displayed_grid()?;
    let row = isize::try_from(index).ok()? - isize::try_from(grid.history.len()).ok()?;
    let columns = selection.columns(row, grid);
    (!columns.is_empty()).then(|| {
        Bounds::new(
            position + point(cell.width * f32::from(columns.start), px(0.0)),
            size(
                cell.width * f32::from(columns.end - columns.start),
                cell.height,
            ),
        )
    })
}

fn align_to_cells(line: &mut ShapedLine, columns: &[u16], cell_width: Pixels, width: u16) {
    let mut runs = line.runs.clone();
    let mut anchors = std::collections::HashMap::new();
    for glyph in runs.iter().flat_map(|run| &run.glyphs) {
        if let Some(column) = columns.get(glyph.index) {
            anchors.entry(*column).or_insert(glyph.position.x);
        }
    }
    for run in &mut runs {
        for glyph in &mut run.glyphs {
            if let Some(column) = columns.get(glyph.index) {
                glyph.position.x =
                    cell_width * f32::from(*column) + glyph.position.x - anchors[column];
            }
        }
    }
    **line = Arc::new(LineLayout {
        font_size: line.font_size,
        width: cell_width * f32::from(width),
        ascent: line.ascent,
        descent: line.descent,
        runs,
        len: line.len(),
    });
}

fn append_run(
    run: &Run,
    column: u16,
    text: &mut String,
    styles: &mut Vec<TextRun>,
    columns: &mut Vec<u16>,
    palette: Palette,
    base_font: &gpui::Font,
) {
    if run.text.bytes().all(|byte| byte == b' ') {
        return;
    }
    let start = text.len();
    text.push_str(&run.text);
    text.push('\u{200c}');
    let ascii = run.text.is_ascii();
    columns.extend(run.text.bytes().enumerate().map(|(index, _)| {
        let offset = if ascii {
            u16::try_from(index).unwrap_or(u16::MAX)
        } else {
            0
        };
        column.saturating_add(offset)
    }));
    columns.extend(std::iter::repeat_n(column, '\u{200c}'.len_utf8()));
    styles.push(text_run(text.len() - start, run.style, palette, base_font));
}

fn text_run(len: usize, style: Style, palette: Palette, base_font: &gpui::Font) -> TextRun {
    let mut font = base_font.clone();
    font.weight = if style.bold {
        FontWeight::BOLD
    } else {
        FontWeight::NORMAL
    };
    font.style = if style.italic {
        FontStyle::Italic
    } else {
        FontStyle::Normal
    };
    let mut color: Hsla = rgb(palette.style(style).0).into();
    if style.faint {
        color.a = 0.5;
    }
    TextRun {
        len,
        font,
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

fn push_quad(quads: &mut Vec<(Bounds<Pixels>, Hsla)>, bounds: Bounds<Pixels>, color: Hsla) {
    if let Some((previous, previous_color)) = quads.last_mut()
        && *previous_color == color
        && previous.origin.y == bounds.origin.y
        && previous.right() == bounds.left()
        && previous.size.height == bounds.size.height
    {
        previous.size.width += bounds.size.width;
    } else {
        quads.push((bounds, color));
    }
}

fn paint(painting: Painting, palette: Palette, window: &mut Window, cx: &mut App) {
    for (bounds, color) in painting.backgrounds {
        window.paint_quad(fill(bounds, color));
    }
    let mut selection_color: Hsla = rgb(palette.indexed(4)).into();
    for (bounds, current) in painting.matches {
        let mut color: Hsla = rgb(palette.indexed(3)).into();
        color.a = if current { 0.65 } else { 0.25 };
        window.paint_quad(fill(bounds, color));
    }
    selection_color.a = 0.35;
    for bounds in painting.selections {
        window.paint_quad(fill(bounds, selection_color));
    }
    for (bounds, color) in painting.decorations {
        window.paint_quad(fill(bounds, color));
    }
    for (bounds, color) in painting.blocks {
        window.paint_quad(fill(bounds, color));
    }
    for (origin, line) in painting.lines {
        if let Err(error) = line.paint(origin, painting.cell.height, window, cx) {
            use std::io::Write;
            let _ = writeln!(
                std::io::stderr(),
                "muxy-app: could not paint terminal text: {error}"
            );
        }
    }
    if let Some(cursor) = painting.cursor {
        let mut color: Hsla = rgb(palette.cursor).into();
        color.a = 0.5;
        window.paint_quad(fill(cursor, color));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn cursor_painting_follows_blink_phase_and_terminal_visibility(cx: &mut gpui::TestAppContext) {
        let (pane, cx) = cx.add_window_view(|_, cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        cx.update(|window, cx| {
            pane.update(cx, |pane, _| {
                let mut grid = muxy_client::RunGrid::from_saved(muxy_protocol::SavedScreen {
                    size: Size { cols: 20, rows: 3 },
                    rows: vec![],
                    cursor: muxy_protocol::Cursor {
                        row: 0,
                        col: 0,
                        visible: true,
                    },
                    reason: None,
                });
                grid.cursor.visible = true;
                pane.grid = Some(grid);
                for (phase, terminal_visible, focused, expected) in [
                    (true, true, true, true),
                    (false, true, true, false),
                    (true, false, true, false),
                    (true, true, false, false),
                ] {
                    pane.cursor_blink.visible = phase;
                    pane.focused = focused;
                    if let Some(grid) = &mut pane.grid {
                        grid.cursor.visible = terminal_visible;
                    }
                    let painting = prepare(
                        pane,
                        point(px(0.0), px(0.0)),
                        size(px(8.0), px(16.0)),
                        pane.palette,
                        &font("Menlo"),
                        px(12.0),
                        window,
                    );
                    assert_eq!(painting.cursor.is_some(), expected);
                }
            });
        });
    }

    #[gpui::test]
    fn block_elements_bypass_font_shaping(cx: &mut gpui::TestAppContext) {
        let (pane, cx) = cx.add_window_view(|_, cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        cx.update(|window, cx| {
            pane.update(cx, |pane, _| {
                pane.grid = Some(muxy_client::RunGrid::from_saved(
                    muxy_protocol::SavedScreen {
                        size: Size { cols: 32, rows: 1 },
                        rows: vec![muxy_protocol::Row {
                            index: 0,
                            runs: ('\u{2580}'..='\u{259f}')
                                .map(|character| Run {
                                    text: character.to_string(),
                                    width: 1,
                                    style: Style::default(),
                                })
                                .collect(),
                        }],
                        cursor: muxy_protocol::Cursor {
                            row: 0,
                            col: 0,
                            visible: false,
                        },
                        reason: None,
                    },
                ));
                let painting = prepare(
                    pane,
                    point(px(0.0), px(0.0)),
                    size(px(8.0), px(16.0)),
                    pane.palette,
                    &font("Menlo"),
                    px(12.0),
                    window,
                );
                assert!(painting.lines.is_empty(), "blocks must not use font glyphs");
                assert!(!painting.blocks.is_empty());
            });
        });
    }

    #[gpui::test]
    fn block_graphics_preserve_grid_columns_and_styles(cx: &mut gpui::TestAppContext) {
        let (pane, cx) = cx.add_window_view(|_, cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        cx.update(|window, cx| {
            pane.update(cx, |pane, _| {
                let plain = Style::default();
                let styled = Style {
                    fg: muxy_protocol::Color::Rgb(0x11, 0x22, 0x33),
                    bg: muxy_protocol::Color::Rgb(0xaa, 0xbb, 0xcc),
                    inverse: true,
                    faint: true,
                    bold: true,
                    italic: true,
                    underline: true,
                    strikethrough: true,
                };
                pane.grid = Some(muxy_client::RunGrid::from_saved(
                    muxy_protocol::SavedScreen {
                        size: Size { cols: 9, rows: 1 },
                        rows: vec![muxy_protocol::Row {
                            index: 0,
                            runs: [
                                ("a", 1, plain),
                                ("  ", 2, plain),
                                ("█", 2, styled),
                                ("█", 1, plain),
                                ("█", 1, plain),
                                ("░", 1, styled),
                                ("x", 1, plain),
                            ]
                            .into_iter()
                            .map(|(text, width, style)| Run {
                                text: text.into(),
                                width,
                                style,
                            })
                            .collect(),
                        }],
                        cursor: muxy_protocol::Cursor {
                            row: 0,
                            col: 8,
                            visible: true,
                        },
                        reason: None,
                    },
                ));
                if let Some(grid) = &mut pane.grid {
                    grid.cursor.visible = true;
                }
                let painting = prepare(
                    pane,
                    point(px(0.0), px(0.0)),
                    size(px(8.0), px(16.0)),
                    pane.palette,
                    &font("Menlo"),
                    px(12.0),
                    window,
                );
                let mut color: Hsla = rgb(0xaa_bb_cc).into();
                color.a = 0.5;
                let mut shade = color;
                shade.a *= 64.0 / 255.0;
                let bounds =
                    |x, width| Bounds::new(point(px(x), px(0.0)), size(px(width), px(16.0)));
                assert_eq!(
                    painting.blocks,
                    vec![
                        (bounds(24.0, 16.0), color),
                        (bounds(40.0, 16.0), rgb(pane.palette.foreground).into()),
                        (bounds(56.0, 8.0), shade),
                    ]
                );
                assert_eq!(
                    painting.backgrounds,
                    vec![
                        (bounds(24.0, 16.0), rgb(0x11_22_33).into()),
                        (bounds(56.0, 8.0), rgb(0x11_22_33).into()),
                    ]
                );
                assert_eq!(painting.decorations.len(), 4);
                assert_eq!(painting.cursor, Some(bounds(64.0, 8.0)));
                assert_eq!(painting.lines.len(), 1);
                let line = &painting.lines[0].1;
                assert_eq!(line.text.as_ref(), "a\u{200c}x\u{200c}");
                let positions: Vec<_> = line
                    .runs
                    .iter()
                    .flat_map(|run| &run.glyphs)
                    .filter(|glyph| Some(glyph.index) == line.text.find('x'))
                    .map(|glyph| glyph.position.x)
                    .collect();
                assert_eq!(positions, vec![px(64.0)]);
            });
        });
    }

    #[test]
    fn shaping_preserves_server_cell_boundaries_and_clusters() {
        for runs in [
            vec![("ل", 1), ("ا", 1), ("x", 1)],
            vec![("👩‍", 2), ("💻", 2), ("x", 1)],
            vec![("👩‍💻", 2), ("x", 1)],
            vec![("❤️", 1), ("x", 1)],
            vec![("❤️", 2), ("x", 1)],
            vec![("e\u{301}", 1), ("x", 1)],
        ] {
            let mut text = String::new();
            let mut styles = Vec::new();
            let mut columns = Vec::new();
            let mut column = 0;
            for (value, width) in runs {
                let start = text.len();
                append_run(
                    &Run {
                        text: value.into(),
                        width,
                        style: Style::default(),
                    },
                    column,
                    &mut text,
                    &mut styles,
                    &mut columns,
                    Palette::new(true),
                    &font("Menlo"),
                );
                assert_eq!(&text[start..], format!("{value}\u{200c}"));
                assert_eq!(columns[start], column);
                column += width;
            }
            assert_eq!(text.len(), columns.len());
            assert_eq!(styles.iter().map(|run| run.len).sum::<usize>(), text.len());
            let last = text.find('x').map(|index| columns[index]);
            assert_eq!(last, Some(column - 1));
        }
    }

    #[test]
    fn blank_runs_leave_a_gap_without_entering_the_shaped_line() {
        let palette = Palette::new(true);
        let mut text = String::new();
        let mut styles = Vec::new();
        let mut columns = Vec::new();
        append_run(
            &Run {
                text: "   ".into(),
                width: 3,
                style: Style::default(),
            },
            0,
            &mut text,
            &mut styles,
            &mut columns,
            palette,
            &font("Menlo"),
        );
        append_run(
            &Run {
                text: "x".into(),
                width: 1,
                style: Style::default(),
            },
            3,
            &mut text,
            &mut styles,
            &mut columns,
            palette,
            &font("Menlo"),
        );
        assert_eq!(text, "x\u{200c}");
        assert_eq!(columns, vec![3, 3, 3, 3]);
        assert_eq!(styles.len(), 1);
    }

    #[test]
    fn adjacent_backgrounds_merge_only_with_the_same_color_and_row() {
        let mut quads = Vec::new();
        let color: Hsla = rgb(0xff_00_00).into();
        for x in [0.0, 10.0] {
            push_quad(
                &mut quads,
                Bounds::new(point(px(x), px(0.0)), size(px(10.0), px(15.0))),
                color,
            );
        }
        assert_eq!(quads.len(), 1);
        assert_eq!(quads[0].0.size.width, px(20.0));
        push_quad(
            &mut quads,
            Bounds::new(point(px(0.0), px(15.0)), size(px(10.0), px(15.0))),
            color,
        );
        assert_eq!(quads.len(), 2);
    }

    #[gpui::test]
    #[allow(clippy::unwrap_used)]
    fn terminal_geometry_matches_swift_padding_after_resize(cx: &mut gpui::TestAppContext) {
        let (pane, cx) = cx.add_window_view(|_, cx| {
            TerminalPane::new(
                Palette::new(true),
                muxy_settings::TerminalSettings::default(),
                cx,
            )
        });
        for dimensions in [size(px(816.0), px(416.0)), size(px(643.0), px(379.0))] {
            cx.simulate_resize(dimensions);
            cx.run_until_parked();
            let pane_bounds = cx.debug_bounds("terminal-pane").unwrap();
            pane.read_with(cx, |pane, _| {
                let (bounds, cell) = pane.geometry.unwrap();
                let viewport = pane.viewport().unwrap();
                assert_eq!(bounds.origin, pane_bounds.origin + point(px(2.0), px(2.0)));
                assert_eq!(bounds.size, pane_bounds.size - size(px(4.0), px(4.0)));
                let unused_width = bounds.size.width - cell.width * f32::from(viewport.cols);
                let unused_height = bounds.size.height - cell.height * f32::from(viewport.rows);
                assert!(unused_width >= px(0.0) && unused_width < cell.width);
                assert!(unused_height >= px(0.0) && unused_height < cell.height);
            });
        }
    }

    #[test]
    fn viewport_dimensions_use_full_bounds_and_whole_cells() {
        assert_eq!(
            viewport_size(size(px(816.0), px(416.0)), size(px(8.0), px(16.0))),
            Size {
                cols: 102,
                rows: 26
            }
        );
        assert_eq!(
            viewport_size(size(px(815.5), px(415.5)), size(px(8.0), px(16.0))),
            Size {
                cols: 101,
                rows: 25
            }
        );
        assert_eq!(
            viewport_size(size(px(0.0), px(0.0)), size(px(8.0), px(16.0))),
            Size { cols: 1, rows: 1 }
        );
    }
}
