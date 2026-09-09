use gpui::{Rgba, rgb};
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct ColorScheme {
    pub background: Option<Rgba>,
    pub foreground: Option<Rgba>,
    pub cursor_color: Option<Rgba>,
    pub palette: HashMap<usize, Rgba>,
}

impl ColorScheme {
    pub fn parse(source: &str) -> Self {
        let mut theme = Self::default();
        for line in source.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            match key {
                "background" => theme.background = parse_hex(value),
                "foreground" => theme.foreground = parse_hex(value),
                "cursor-color" => theme.cursor_color = parse_hex(value),
                "palette" => {
                    let Some((index, color)) = value.split_once('=') else {
                        continue;
                    };
                    let (Ok(index), Some(color)) = (index.trim().parse(), parse_hex(color)) else {
                        continue;
                    };
                    theme.palette.insert(index, color);
                }
                _ => {}
            }
        }
        theme
    }

    pub fn palette_color(&self, index: usize) -> Option<Rgba> {
        self.palette.get(&index).copied()
    }
}

pub fn parse_hex(value: &str) -> Option<Rgba> {
    let value = value.trim().trim_start_matches('#');
    if value.len() != 6 {
        return None;
    }
    u32::from_str_radix(value, 16).ok().map(rgb)
}

pub(super) fn luminance(color: Rgba) -> f32 {
    0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b
}

pub(super) fn with_alpha(color: Rgba, alpha: f32) -> Rgba {
    Rgba { a: alpha, ..color }
}
