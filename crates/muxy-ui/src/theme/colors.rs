use super::palette::{ColorScheme, luminance, with_alpha};
use gpui::{Hsla, Rgba, rgb};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Theme {
    pub bg: Hsla,
    pub fg: Hsla,
    pub fg_muted: Hsla,
    pub fg_dim: Hsla,
    pub surface: Hsla,
    pub border: Hsla,
    pub hover: Hsla,
    pub accent: Hsla,
    pub accent_soft: Hsla,
    pub accent_foreground: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
    pub diff_add: Hsla,
    pub diff_remove: Hsla,
    pub diff_hunk: Hsla,
}

const FALLBACK_BACKGROUND: u32 = 0x19_17_1f;
const FALLBACK_FOREGROUND: u32 = 0xc9_c2_d9;
const FALLBACK_ACCENT: u32 = 0xc3_70_d3;

impl Theme {
    pub fn from_scheme(theme: &ColorScheme) -> Self {
        let bg = theme.background.unwrap_or(rgb(FALLBACK_BACKGROUND));
        let fg = theme.foreground.unwrap_or(rgb(FALLBACK_FOREGROUND));
        let accent = theme.palette_color(4).unwrap_or(rgb(FALLBACK_ACCENT));
        let warning = theme.palette_color(3).unwrap_or(rgb(0xe0_af_68));
        let danger = theme.palette_color(1).unwrap_or(rgb(0xec_48_99));
        let diff_add = theme.palette_color(2).unwrap_or(rgb(0x22_c5_5e));
        let diff_hunk = theme.palette_color(6).unwrap_or(accent);

        Self {
            bg: bg.into(),
            fg: fg.into(),
            fg_muted: with_alpha(fg, 0.65).into(),
            fg_dim: with_alpha(fg, 0.4).into(),
            surface: with_alpha(fg, 0.08).into(),
            border: with_alpha(fg, 0.12).into(),
            hover: with_alpha(fg, 0.06).into(),
            accent: accent.into(),
            accent_soft: with_alpha(accent, 0.1).into(),
            accent_foreground: contrasting_foreground(accent).into(),
            warning: warning.into(),
            danger: danger.into(),
            diff_add: diff_add.into(),
            diff_remove: danger.into(),
            diff_hunk: diff_hunk.into(),
        }
    }

    pub fn raised(&self) -> Hsla {
        blend(self.surface, self.bg)
    }

    pub fn border_solid(&self) -> Hsla {
        blend(self.border, self.bg)
    }

    pub fn fg_alpha(&self, alpha: f32) -> Hsla {
        let mut color = self.fg;
        color.a = alpha;
        color
    }
}

fn blend(top: Hsla, bottom: Hsla) -> Hsla {
    let top: Rgba = top.into();
    let bottom: Rgba = bottom.into();
    let mix = |top_channel: f32, bottom_channel: f32| {
        top_channel * top.a + bottom_channel * (1.0 - top.a)
    };
    Rgba {
        r: mix(top.r, bottom.r),
        g: mix(top.g, bottom.g),
        b: mix(top.b, bottom.b),
        a: 1.0,
    }
    .into()
}

pub fn contrasting_foreground(color: Rgba) -> Rgba {
    if luminance(color) > 0.6 {
        rgb(0x00_00_00)
    } else {
        rgb(0xff_ff_ff)
    }
}
