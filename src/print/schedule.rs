use crate::Result;

pub const MAX_LAYERS: usize = 50_000;

pub(crate) fn validate_height(value: f64) -> Result<()> {
    if !value.is_finite() || !(0.02..=2.0).contains(&value) {
        return Err("layer height must be finite and between 0.02 and 2.00 mm".into());
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct LayerSchedule {
    pub boundaries_mm: Vec<f64>,
}

impl LayerSchedule {
    pub fn uniform(height_mm: f64, first_mm: f64, build_height_mm: f64) -> Result<Self> {
        validate_height(height_mm)?;
        validate_height(first_mm)?;
        if !build_height_mm.is_finite() || build_height_mm <= 0. {
            return Err("build height must be positive and finite".into());
        }
        let remaining = ((build_height_mm - first_mm) / height_mm).max(0.);
        if remaining > MAX_LAYERS as f64 {
            return Err("layer schedule exceeds 50000-layer budget".into());
        }
        // Rounding protects an exact decimal layer boundary from a spurious extra interval.
        let tolerance = 16. * f64::EPSILON * remaining.max(1.);
        let extra = (remaining - tolerance).max(0.).ceil() as usize;
        if extra + 1 > MAX_LAYERS {
            return Err("layer schedule exceeds 50000-layer budget".into());
        }
        let mut boundaries_mm = Vec::with_capacity(extra + 2);
        boundaries_mm.push(0.);
        for i in 0..=extra {
            boundaries_mm.push(first_mm + i as f64 * height_mm);
        }
        Ok(Self { boundaries_mm })
    }
}
