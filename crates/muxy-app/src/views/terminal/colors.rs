#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Palette {
    pub(crate) background: u32,
    pub(crate) foreground: u32,
    pub(crate) cursor: u32,
    colors: [u32; 16],
}

impl Palette {
    pub(crate) fn terminal_colors(self) -> muxy_protocol::TerminalColors {
        let rgb = |color: u32| {
            let [_, r, g, b] = color.to_be_bytes();
            [r, g, b]
        };
        muxy_protocol::TerminalColors {
            foreground: rgb(self.foreground),
            background: rgb(self.background),
            cursor: rgb(self.cursor),
            ansi: self.colors.map(rgb),
        }
    }

    pub(crate) fn from_scheme(scheme: &muxy_ui::theme::ColorScheme, dark: bool) -> Self {
        let mut palette = Self::new(dark);
        if let Some(color) = scheme.background {
            palette.background = u32::from(color) >> 8;
        }
        if let Some(color) = scheme.foreground {
            palette.foreground = u32::from(color) >> 8;
        }
        palette.cursor = scheme
            .cursor_color
            .map_or(palette.foreground, |color| u32::from(color) >> 8);
        for (index, color) in palette.colors.iter_mut().enumerate() {
            if let Some(value) = scheme.palette_color(index) {
                *color = u32::from(value) >> 8;
            }
        }
        palette
    }

    pub(crate) fn resolve(self, color: muxy_protocol::Color, default: u32) -> u32 {
        match color {
            muxy_protocol::Color::Default => default,
            muxy_protocol::Color::Indexed(index) => self.indexed(index),
            muxy_protocol::Color::Rgb(red, green, blue) => {
                (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)
            }
        }
    }

    pub(crate) fn style(self, style: muxy_protocol::Style) -> (u32, u32) {
        let foreground = self.resolve(style.fg, self.foreground);
        let background = self.resolve(style.bg, self.background);
        if style.inverse {
            (background, foreground)
        } else {
            (foreground, background)
        }
    }

    pub(crate) const fn new(dark: bool) -> Self {
        if dark {
            Self {
                background: 0x19_17_1f,
                foreground: 0xc9_c2_d9,
                cursor: 0xc9_c2_d9,
                colors: [
                    0x46_40_56, 0xec_48_99, 0x34_d3_99, 0xe0_af_68, 0xc3_70_d3, 0x63_66_f1,
                    0x22_d3_ee, 0xa9_b1_d6, 0x7c_73_93, 0xf4_72_b6, 0x6e_e7_b7, 0xfb_bf_24,
                    0xd9_9b_e5, 0x81_8c_f8, 0x67_e8_f9, 0xc9_c2_d9,
                ],
            }
        } else {
            Self {
                background: 0xf0_f0_f5,
                foreground: 0x1e_1e_2e,
                cursor: 0x1e_1e_2e,
                colors: [
                    0xd5_d6_db, 0xa3_2d_68, 0x1a_7a_4e, 0x9a_70_24, 0x47_96_f0, 0x7c_3a_ed,
                    0x0b_71_89, 0x3b_3f_5c, 0x7a_7e_94, 0xec_48_99, 0x34_d3_99, 0xe0_af_68,
                    0x6b_ab_f5, 0xa7_8b_fa, 0x22_d3_ee, 0x1e_1e_2e,
                ],
            }
        }
    }

    pub(crate) fn indexed(self, index: u8) -> u32 {
        match index {
            0..=15 => self.colors[usize::from(index)],
            16..=231 => {
                let cube = u32::from(index) - 16;
                let component = |level| if level == 0 { 0 } else { 55 + 40 * level };
                (component(cube / 36) << 16)
                    | (component((cube / 6) % 6) << 8)
                    | component(cube % 6)
            }
            232..=255 => {
                let level = 8 + 10 * (u32::from(index) - 232);
                (level << 16) | (level << 8) | level
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Palette;

    #[test]
    fn indexed_colors_cover_the_cube_and_grayscale_boundaries() {
        for dark in [false, true] {
            let palette = Palette::new(dark);
            for (index, rgb) in [
                (16, 0x00_00_00),
                (17, 0x00_00_5f),
                (21, 0x00_00_ff),
                (46, 0x00_ff_00),
                (196, 0xff_00_00),
                (231, 0xff_ff_ff),
                (232, 0x08_08_08),
                (255, 0xee_ee_ee),
            ] {
                assert_eq!(palette.indexed(index), rgb);
            }
        }
    }

    #[test]
    fn cursor_color_comes_from_the_selected_theme() -> Result<(), Box<dyn std::error::Error>> {
        let (_, source) = muxy_ui::assets::Assets::themes()
            .find(|(name, _)| *name == "Dracula")
            .ok_or("missing Dracula theme")?;
        let scheme = muxy_ui::theme::ColorScheme::parse(source);
        let palette = Palette::from_scheme(&scheme, true);
        assert_eq!(palette.cursor, 0xf8_f8_f2);
        assert_ne!(palette.cursor, palette.indexed(4));
        let scheme = muxy_ui::theme::ColorScheme::parse("background=123456\nforeground=abcdef");
        assert_eq!(Palette::from_scheme(&scheme, true).cursor, 0xab_cd_ef);
        Ok(())
    }

    #[test]
    fn server_colors_match_the_selected_render_palette() {
        let scheme = muxy_ui::theme::ColorScheme::parse(
            "background=123456\nforeground=abcdef\ncursor-color=789abc\npalette=6=fedcba",
        );
        for palette in [
            Palette::new(true),
            Palette::new(false),
            Palette::from_scheme(&scheme, true),
        ] {
            let colors = palette.terminal_colors();
            let packed = |[r, g, b]: [u8; 3]| u32::from_be_bytes([0, r, g, b]);
            assert_eq!(packed(colors.foreground), palette.foreground);
            assert_eq!(packed(colors.background), palette.background);
            assert_eq!(packed(colors.cursor), palette.cursor);
            for (index, color) in (0_u8..16).zip(colors.ansi) {
                assert_eq!(packed(color), palette.indexed(index));
            }
        }
    }

    #[test]
    fn muxy_themes_keep_their_own_base_palette() {
        let dark = Palette::new(true);
        let light = Palette::new(false);
        assert_eq!(dark.indexed(4), 0xc3_70_d3);
        assert_eq!(light.indexed(4), 0x47_96_f0);
        assert_eq!(dark.indexed(15), dark.foreground);
        assert_eq!(light.indexed(15), light.foreground);
        assert_ne!(dark.background, light.background);
    }
}
