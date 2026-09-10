use gpui::{Bounds, Hsla, Pixels, point, px};

pub(super) fn quads(
    text: &str,
    bounds: Bounds<Pixels>,
    mut color: Hsla,
    scale: f32,
) -> Option<impl Iterator<Item = (Bounds<Pixels>, Hsla)>> {
    let eighths: &[[u8; 4]] = match text {
        "▀" => &[[0, 0, 8, 4]],
        "▁" => &[[0, 7, 8, 8]],
        "▂" => &[[0, 6, 8, 8]],
        "▃" => &[[0, 5, 8, 8]],
        "▄" => &[[0, 4, 8, 8]],
        "▅" => &[[0, 3, 8, 8]],
        "▆" => &[[0, 2, 8, 8]],
        "▇" => &[[0, 1, 8, 8]],
        "█" | "░" | "▒" | "▓" => &[[0, 0, 8, 8]],
        "▉" => &[[0, 0, 7, 8]],
        "▊" => &[[0, 0, 6, 8]],
        "▋" => &[[0, 0, 5, 8]],
        "▌" => &[[0, 0, 4, 8]],
        "▍" => &[[0, 0, 3, 8]],
        "▎" => &[[0, 0, 2, 8]],
        "▏" => &[[0, 0, 1, 8]],
        "▐" => &[[4, 0, 8, 8]],
        "▔" => &[[0, 0, 8, 1]],
        "▕" => &[[7, 0, 8, 8]],
        "▖" => &[[0, 4, 4, 8]],
        "▗" => &[[4, 4, 8, 8]],
        "▘" => &[[0, 0, 4, 4]],
        "▙" => &[[0, 0, 4, 4], [0, 4, 8, 8]],
        "▚" => &[[0, 0, 4, 4], [4, 4, 8, 8]],
        "▛" => &[[0, 0, 8, 4], [0, 4, 4, 8]],
        "▜" => &[[0, 0, 8, 4], [4, 4, 8, 8]],
        "▝" => &[[4, 0, 8, 4]],
        "▞" => &[[4, 0, 8, 4], [0, 4, 4, 8]],
        "▟" => &[[4, 0, 8, 4], [0, 4, 8, 8]],
        _ => return None,
    };
    color.a *= match text {
        "░" => 64.0 / 255.0,
        "▒" => 128.0 / 255.0,
        "▓" => 192.0 / 255.0,
        _ => 1.0,
    };
    Some(
        eighths
            .iter()
            .filter_map(move |&[left, top, right, bottom]| {
                let x = |fraction| {
                    snap(
                        bounds.left() + bounds.size.width * f32::from(fraction) / 8.0,
                        scale,
                    )
                };
                let y = |fraction| {
                    snap(
                        bounds.top() + bounds.size.height * f32::from(fraction) / 8.0,
                        scale,
                    )
                };
                let quad = Bounds::from_corners(point(x(left), y(top)), point(x(right), y(bottom)));
                (quad.size.width > px(0.0) && quad.size.height > px(0.0)).then_some((quad, color))
            }),
    )
}

fn snap(value: Pixels, scale: f32) -> Pixels {
    px((f32::from(value) * scale).round() / scale)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use gpui::{rgb, size};

    fn shapes(text: &str, bounds: Bounds<Pixels>, scale: f32) -> Vec<Bounds<Pixels>> {
        quads(text, bounds, rgb(0xff_00_00).into(), scale)
            .unwrap()
            .map(|(bounds, _)| bounds)
            .collect()
    }

    #[test]
    fn full_blocks_tile_on_physical_pixels_at_fractional_origins() {
        for scale in [1.0, 1.5, 2.0, 3.0] {
            for (width, height) in [(7.5, 15.5), (8.0, 17.0), (10.5, 23.0)] {
                let cell = size(px(width), px(height));
                let origin = point(px(0.3), px(-2.7));
                let first = shapes("█", Bounds::new(origin, cell), scale)[0];
                let right = shapes(
                    "█",
                    Bounds::new(origin + point(cell.width, px(0.0)), cell),
                    scale,
                )[0];
                let below = shapes(
                    "█",
                    Bounds::new(origin + point(px(0.0), cell.height), cell),
                    scale,
                )[0];
                assert!(f32::from(first.right() - right.left()).abs() * scale < 0.0001);
                assert!(f32::from(first.bottom() - below.top()).abs() * scale < 0.0001);
                for bounds in [first, right, below] {
                    for edge in [bounds.left(), bounds.top(), bounds.right(), bounds.bottom()] {
                        let physical = f32::from(edge) * scale;
                        assert!((physical - physical.round()).abs() < 0.0001);
                    }
                }
            }
        }
    }

    #[test]
    fn complementary_halves_share_an_edge_even_with_odd_pixel_sizes() {
        for scale in [1.0, 1.5, 2.0] {
            let cell = Bounds::new(point(px(0.3), px(0.7)), size(px(7.5), px(15.5)));
            let upper = shapes("▀", cell, scale)[0];
            let lower = shapes("▄", cell, scale)[0];
            let left = shapes("▌", cell, scale)[0];
            let right = shapes("▐", cell, scale)[0];
            let full = shapes("█", cell, scale)[0];
            assert_eq!(upper.bottom(), lower.top());
            assert_eq!(left.right(), right.left());
            assert_eq!(upper.union(&lower), full);
            assert_eq!(left.union(&right), full);
        }
    }

    #[test]
    fn fractional_blocks_fill_the_expected_eighths() {
        let cell = Bounds::new(point(px(0.0), px(0.0)), size(px(8.0), px(16.0)));
        for (text, eighths) in ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"]
            .into_iter()
            .zip(1_u8..)
        {
            let bounds = shapes(text, cell, 1.0)[0];
            assert_eq!(bounds.size, size(px(8.0), px(f32::from(eighths) * 2.0)));
            assert_eq!(bounds.bottom(), cell.bottom());
        }
        for (text, eighths) in ["▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"]
            .into_iter()
            .zip(1_u8..)
        {
            let bounds = shapes(text, cell, 1.0)[0];
            assert_eq!(bounds.size, size(px(f32::from(eighths)), px(16.0)));
            assert_eq!(bounds.left(), cell.left());
        }
        assert_eq!(shapes("▔", cell, 1.0)[0].size.height, px(2.0));
        assert_eq!(shapes("▕", cell, 1.0)[0].size.width, px(1.0));
        assert_eq!(shapes("▔", cell, 1.0)[0].top(), cell.top());
        assert_eq!(shapes("▕", cell, 1.0)[0].right(), cell.right());
    }

    #[test]
    fn quadrants_fill_only_their_named_corners_without_overlapping() {
        let cell = Bounds::new(point(px(0.0), px(0.0)), size(px(8.0), px(16.0)));
        for (text, expected) in [
            ("▖", [false, false, true, false]),
            ("▗", [false, false, false, true]),
            ("▘", [true, false, false, false]),
            ("▙", [true, false, true, true]),
            ("▚", [true, false, false, true]),
            ("▛", [true, true, true, false]),
            ("▜", [true, true, false, true]),
            ("▝", [false, true, false, false]),
            ("▞", [false, true, true, false]),
            ("▟", [false, true, true, true]),
        ] {
            let quads = shapes(text, cell, 1.0);
            for ((x, y), filled) in [(2.0, 4.0), (6.0, 4.0), (2.0, 12.0), (6.0, 12.0)]
                .into_iter()
                .zip(expected)
            {
                let covering = quads
                    .iter()
                    .filter(|bounds| bounds.contains(&point(px(x), px(y))))
                    .count();
                assert_eq!(covering, usize::from(filled), "{text} at ({x}, {y})");
            }
        }
    }

    #[test]
    fn shade_coverage_preserves_the_foreground_and_faint_alpha() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(8.0), px(16.0)));
        for alpha in [1.0, 0.5] {
            let mut foreground: Hsla = rgb(0x12_34_56).into();
            foreground.a = alpha;
            for (text, coverage) in [("█", 255.0), ("░", 64.0), ("▒", 128.0), ("▓", 192.0)]
            {
                let quads: Vec<_> = quads(text, bounds, foreground, 1.0).unwrap().collect();
                let mut expected = foreground;
                expected.a *= coverage / 255.0;
                assert_eq!(quads, vec![(bounds, expected)]);
            }
        }
    }

    #[test]
    fn ordinary_text_and_combining_clusters_keep_font_shaping() {
        let cell = Bounds::new(point(px(0.0), px(0.0)), size(px(8.0), px(16.0)));
        for text in ["", " ", "hello", "界", "👩‍💻", "e\u{301}", "█\u{301}", "─"] {
            assert!(quads(text, cell, rgb(0).into(), 1.0).is_none(), "{text}");
        }
    }
}
