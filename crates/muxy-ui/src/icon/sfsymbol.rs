use objc2::rc::{Retained, autoreleasepool};
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSBitmapImageRep, NSCompositingOperation, NSDeviceRGBColorSpace, NSGraphicsContext, NSImage,
    NSImageSymbolConfiguration, NSImageSymbolScale,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

#[derive(Debug)]
pub(super) struct Mask {
    pub width: u32,
    pub height: u32,
    pub logical_width: f32,
    pub logical_height: f32,
    pub alpha: Vec<u8>,
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "Validated AppKit dimensions are converted to GPUI f32 coordinates."
)]
pub(super) fn rasterize(symbol: &str, point_size: f32, weight: f32, scale: f32) -> Option<Mask> {
    let _main_thread = MainThreadMarker::new()?;
    if !point_size.is_finite()
        || point_size <= 0.0
        || point_size > 256.0
        || !weight.is_finite()
        || !(-1.0..=1.0).contains(&weight)
        || !scale.is_finite()
        || !(0.5..=8.0).contains(&scale)
    {
        return None;
    }
    autoreleasepool(|_| unsafe {
        let name = NSString::from_str(symbol);
        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&name, None)?;

        let config = NSImageSymbolConfiguration::configurationWithPointSize_weight_scale(
            f64::from(point_size),
            f64::from(weight),
            NSImageSymbolScale::Medium,
        );
        let image = image.imageWithSymbolConfiguration(&config)?;

        let natural = image.size();
        let width = dimension(natural.width * f64::from(scale))?;
        let height = dimension(natural.height * f64::from(scale))?;

        let rep = draw_into_bitmap(&image, width, height)?;
        let data = rep.bitmapData();
        if data.is_null() {
            return None;
        }

        let bytes_per_row = usize::try_from(rep.bytesPerRow()).ok()?;
        let samples = usize::try_from(rep.samplesPerPixel()).ok()?;
        let columns = usize::try_from(width).ok()?;
        let rows = usize::try_from(height).ok()?;
        let length = rows.checked_mul(bytes_per_row)?;
        if samples != 4
            || rep.isPlanar()
            || bytes_per_row < columns.checked_mul(samples)?
            || usize::try_from(rep.bytesPerPlane()).ok()? < length
        {
            return None;
        }
        let pixels = std::slice::from_raw_parts(data, length);
        let alpha = pixels
            .chunks_exact(bytes_per_row)
            .take(rows)
            .flat_map(|row| {
                row[..columns * samples]
                    .chunks_exact(samples)
                    .map(|pixel| pixel[3])
            })
            .collect();
        Some(Mask {
            width,
            height,
            logical_width: natural.width as f32,
            logical_height: natural.height as f32,
            alpha,
        })
    })
}

unsafe fn draw_into_bitmap(
    image: &NSImage,
    width: u32,
    height: u32,
) -> Option<Retained<NSBitmapImageRep>> {
    unsafe {
        let rep = NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
        NSBitmapImageRep::alloc(),
        std::ptr::null_mut(),
        isize::try_from(width).ok()?,
        isize::try_from(height).ok()?,
        8,
        4,
        true,
        false,
        NSDeviceRGBColorSpace,
        isize::try_from(width.checked_mul(4)?).ok()?,
        32,
    )?;

        let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
        NSGraphicsContext::saveGraphicsState_class();
        NSGraphicsContext::setCurrentContext(Some(&context));

        let rect = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(f64::from(width), f64::from(height)),
        );
        image.drawInRect_fromRect_operation_fraction(
            rect,
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
            NSCompositingOperation::SourceOver,
            1.0,
        );

        NSGraphicsContext::restoreGraphicsState_class();
        Some(rep)
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn dimension(value: f64) -> Option<u32> {
    if value.is_finite() && (0.0..=4096.0).contains(&value) && value > 0.0 {
        Some(value.round().max(1.0) as u32)
    } else {
        None
    }
}
