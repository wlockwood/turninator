//! CPU transfer recipe v1: producer-domain unpremultiply, then IEC sRGB encode.
//! HDR exposure/tone mapping belongs upstream of this 8-bit boundary.
use super::contract::*;
use crate::Result;

pub fn normalize_rgba(image: NativePixels) -> Result<RgbaFrame> {
    if image.width == 0 || image.height == 0 || image.width > 8000 || image.height > 8000 {
        return Err("pixel dimensions outside 1..=8000".into());
    }
    let row = (image.width as usize)
        .checked_mul(4)
        .ok_or("pixel row overflow")?;
    let input_len = image
        .row_stride
        .checked_mul(image.height as usize)
        .ok_or("pixel input size overflow")?;
    let output_len = row
        .checked_mul(image.height as usize)
        .ok_or("pixel output size overflow")?;
    if image.row_stride < row || image.bytes.len() != input_len {
        return Err("pixel buffer size or stride does not match dimensions".into());
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(output_len)?;
    for y in 0..image.height as usize {
        let source_y = match image.origin {
            RowOrigin::Top => y,
            RowOrigin::Bottom => image.height as usize - 1 - y,
        };
        let offset = source_y * image.row_stride;
        for pixel in image.bytes[offset..offset + row].chunks_exact(4) {
            let indices = match image.channels {
                PixelChannels::Rgba => [0, 1, 2],
                PixelChannels::Bgra => [2, 1, 0],
            };
            for index in indices {
                let mut value = pixel[index] as f64 / 255.;
                if pixel[3] == 0 {
                    value = 0.;
                } else if matches!(image.alpha, PixelAlpha::Premultiplied) {
                    // NativePixels.transfer defines the producer's premultiplication domain.
                    value = (value * 255. / pixel[3] as f64).min(1.);
                }
                if matches!(image.transfer, PixelTransfer::Linear) {
                    value = linear_to_srgb(value);
                }
                bytes.push((value * 255.).round().clamp(0., 255.) as u8);
            }
            bytes.push(pixel[3]);
        }
    }
    Ok(RgbaFrame {
        width: image.width,
        height: image.height,
        bytes,
    })
}
fn linear_to_srgb(value: f64) -> f64 {
    if value <= 0.0031308 {
        12.92 * value
    } else {
        1.055 * value.powf(1. / 2.4) - 0.055
    }
}
