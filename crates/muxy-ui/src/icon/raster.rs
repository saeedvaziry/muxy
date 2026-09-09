use super::Icon;
use super::sfsymbol::{self, Mask};
use gpui::{Hsla, Pixels, RenderImage, Rgba, SharedString, px};
use image::{Frame, RgbaImage};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

const SEMIBOLD: f32 = 0.3;

const APPKIT_SYMBOL_CORRECTION: f32 = 0.97;

#[derive(PartialEq, Eq, Hash, Clone)]
struct MaskKey {
    symbol: SharedString,
    point_size: u32,
    scale: u32,
}

#[derive(PartialEq, Eq, Hash, Clone)]
struct TintKey {
    mask: MaskKey,
    color: u32,
}

thread_local! {
    static MASKS: RefCell<HashMap<MaskKey, Option<Arc<Mask>>>> = RefCell::new(HashMap::new());
    static TINTED: RefCell<HashMap<TintKey, Arc<RenderImage>>> = RefCell::new(HashMap::new());
}

#[derive(Debug)]
pub struct Glyph {
    pub image: Arc<RenderImage>,
    pub width: Pixels,
    pub height: Pixels,
}

pub fn tinted(icon: Icon, size: Pixels, color: Hsla, scale: f32) -> Option<Glyph> {
    tinted_symbol(&SharedString::from(icon.sf_symbol()), size, color, scale)
}

pub fn tinted_symbol(
    symbol: &SharedString,
    size: Pixels,
    color: Hsla,
    scale: f32,
) -> Option<Glyph> {
    let key = MaskKey {
        symbol: symbol.clone(),
        point_size: f32::from(size).to_bits(),
        scale: scale.to_bits(),
    };
    let tint = TintKey {
        mask: key.clone(),
        color: pack(color),
    };

    let cached = TINTED.with(|cache| cache.borrow().get(&tint).cloned());

    let mask = MASKS.with(|cache| {
        cache
            .borrow_mut()
            .entry(key.clone())
            .or_insert_with(|| {
                sfsymbol::rasterize(
                    &key.symbol,
                    f32::from(size) * APPKIT_SYMBOL_CORRECTION,
                    SEMIBOLD,
                    scale,
                )
                .map(Arc::new)
            })
            .clone()
    })?;

    let image = if let Some(image) = cached {
        image
    } else {
        let image = Arc::new(compose(&mask, color));
        TINTED.with(|cache| cache.borrow_mut().insert(tint, image.clone()));
        image
    };

    Some(Glyph {
        image,
        width: px(mask.logical_width),
        height: px(mask.logical_height),
    })
}

fn compose(mask: &Mask, color: Hsla) -> RenderImage {
    let rgba: Rgba = color.into();
    let (r, g, b) = (channel(rgba.r), channel(rgba.g), channel(rgba.b));
    let tint_alpha = rgba.a.clamp(0.0, 1.0);

    let mut buffer = RgbaImage::new(mask.width, mask.height);
    for (index, pixel) in buffer.pixels_mut().enumerate() {
        let coverage = (f32::from(mask.alpha[index]) / 255.0) * tint_alpha;
        pixel.0 = [b, g, r, channel(coverage)];
    }
    RenderImage::new([Frame::new(buffer)])
}

fn pack(color: Hsla) -> u32 {
    let rgba: Rgba = color.into();
    u32::from_be_bytes([
        channel(rgba.r),
        channel(rgba.g),
        channel(rgba.b),
        channel(rgba.a),
    ])
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}
