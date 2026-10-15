//! HDR: turninator-agx-v1, scene-linear Rec.709 to display-linear or display-sRGB.
//! Adapted from Filament (Apache-2.0/MIT); see assets/render/color/NOTICE.md.
//! The separate 8-bit normalizer unpremultiplies in the producer domain and
//! encodes IEC sRGB only for Linear input. HDR mapping must precede quantization.
use super::contract::*;
use crate::Result;
use sha2::{Digest, Sha256};

pub const AGX_RECIPE_VERSION: &str = "turninator-agx-v1";
pub const AGX_RECIPE_JSON: &str = include_str!("../../assets/render/color/recipe.json");
pub const AGX_WGSL: &str = include_str!("../../assets/render/color/turninator_agx.wgsl");
pub const AGX_CPP: &str = include_str!("../../assets/render/color/turninator_agx.h");

/// Scene-linear Rec.709 HDR input; straight alpha is independent of RGB.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AgxRecipe {
    /// Stops applied before AgX. Default 0; valid range -32..=32.
    pub exposure_ev: f32,
}
impl AgxRecipe {
    /// Returns display-linear Rec.709 D65 RGB in 0..=1, with straight alpha
    /// unchanged (even at zero). Use with hardware sRGB encoding or quantize as
    /// NativePixels/Linear. Never feed already display-encoded RGB to this helper.
    pub fn map_linear_rgba(self, rgba: [f32; 4]) -> Result<[f32; 4]> {
        if !self.exposure_ev.is_finite()
            || !(-32.0..=32.0).contains(&self.exposure_ev)
            || rgba.iter().any(|v| !v.is_finite())
            || !(0.0..=1.0).contains(&rgba[3])
        {
            return Err("invalid HDR color, alpha, or exposure".into());
        }
        let gain = self.exposure_ev.exp2();
        let mut rgb = [rgba[0], rgba[1], rgba[2]].map(|v| v.max(0.) * gain);
        if rgb.iter().any(|v| !v.is_finite()) {
            return Err("HDR exposure overflow".into());
        }
        // Every positive inset coefficient saturates its log window well before
        // this cap; it bounds arithmetic without changing the saturated result.
        rgb = rgb.map(|v| v.min(65504.));
        use agx_matrices::*;
        rgb = multiply(INSET, multiply(REC709_TO_REC2020, rgb));
        rgb = rgb.map(|v| {
            let x = ((v.max(1e-10).log2() - (-12.47393)) / (4.026069 - (-12.47393))).clamp(0., 1.);
            let x2 = x * x;
            let x4 = x2 * x2;
            let x6 = x4 * x2;
            -17.86 * x6 * x + 78.01 * x6 - 126.7 * x4 * x + 92.06 * x4 - 28.72 * x2 * x + 4.361 * x2
                - 0.1718 * x
                + 0.002857
        });
        rgb = multiply(OUTSET, rgb).map(|v| v.max(0.).powf(2.2));
        rgb = multiply(REC2020_TO_REC709, rgb).map(|v| v.clamp(0., 1.));
        Ok([rgb[0], rgb[1], rgb[2], rgba[3]])
    }
    /// Returns display-sRGB straight RGBA in 0..=1. Quantize with round(v*255)
    /// and declare NativePixels/Srgb; an additional OETF would encode twice.
    /// normalize_rgba owns the final zero-alpha RGB policy at the byte boundary.
    pub fn map_srgb_rgba(self, rgba: [f32; 4]) -> Result<[f32; 4]> {
        let mut output = self.map_linear_rgba(rgba)?;
        for value in &mut output[..3] {
            *value = linear_to_srgb(*value as f64) as f32;
        }
        Ok(output)
    }
    /// SHA-256 over the version, frozen numeric specification, portable assets,
    /// (CRLF normalized to LF), then exposure's little-endian f32 bits
    /// (signed zero canonicalized).
    /// Include alongside the adapter's code/shader digest in pixel identity.
    pub fn digest(self) -> String {
        let mut digest = Sha256::new();
        for part in [AGX_RECIPE_VERSION, AGX_RECIPE_JSON, AGX_WGSL, AGX_CPP] {
            let part = part.replace("\r\n", "\n");
            digest.update((part.len() as u64).to_le_bytes());
            digest.update(part.as_bytes());
        }
        let exposure = if self.exposure_ev == 0. {
            0.
        } else {
            self.exposure_ev
        };
        digest.update(exposure.to_le_bytes());
        format!("{:x}", digest.finalize())
    }
}

fn multiply(matrix: [[f32; 3]; 3], rgb: [f32; 3]) -> [f32; 3] {
    matrix.map(|row| (row[0] * rgb[0] + row[1] * rgb[1]) + row[2] * rgb[2])
}

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

// f32 values frozen from the pinned SDK; rows multiply column RGB vectors.
#[allow(clippy::excessive_precision)]
mod agx_matrices {
    pub const REC709_TO_REC2020: [[f32; 3]; 3] = [
        [0.627507389, 0.329277724, 0.0433038622],
        [0.0691077262, 0.919504464, 0.0113590918],
        [0.0163964797, 0.0880243331, 0.895513475],
    ];
    pub const INSET: [[f32; 3]; 3] = [
        [0.856627153315983, 0.0951212405381588, 0.0482516061458583],
        [0.137318972929847, 0.761241990602591, 0.101439036467562],
        [0.11189821299995, 0.0767994186031903, 0.811302368396859],
    ];
    pub const OUTSET: [[f32; 3]; 3] = [
        [1.12710059, -0.110606641, -0.0164939389],
        [-0.141329765, 1.15782368, -0.016493937],
        [-0.141329765, -0.110606641, 1.25193644],
    ];
    pub const REC2020_TO_REC709: [[f32; 3]; 3] = [
        [1.66021347, -0.58756429, -0.0728275776],
        [-0.124552041, 1.13294613, -0.00834833086],
        [-0.0181550235, -0.100604787, 1.11883187],
    ];
}
