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
    pub fn explicit(boundaries_mm: Vec<f64>) -> Result<Self> {
        if boundaries_mm.len() < 2 || boundaries_mm.len() > MAX_LAYERS + 1 || boundaries_mm[0] != 0.
        {
            return Err(
                "explicit schedule must start at zero and contain 1..=50000 intervals".into(),
            );
        }
        for pair in boundaries_mm.windows(2) {
            if pair.iter().any(|v| !v.is_finite()) {
                return Err("explicit boundaries must be finite".into());
            }
            let height = pair[1] - pair[0];
            // Subtraction at a large layer index may round an exact .02 boundary.
            let tolerance = 16. * f64::EPSILON * pair[1].abs().max(1.);
            if height <= 0. || height < 0.02 - tolerance || height > 2. + tolerance {
                return Err(
                    "explicit layer intervals must be strictly increasing and 0.02..=2 mm".into(),
                );
            }
        }
        Ok(Self { boundaries_mm })
    }
    /// A sliced envelope ends at the actual model top, even within a scheduled layer.
    pub fn clipped(&self, build_height_mm: f64) -> Result<Self> {
        Self::explicit(self.boundaries_mm.clone())?;
        if !build_height_mm.is_finite()
            || build_height_mm <= 0.
            || build_height_mm > *self.boundaries_mm.last().unwrap()
        {
            return Err("model top must be positive and covered by the layer schedule".into());
        }
        let mut boundaries_mm: Vec<_> = self
            .boundaries_mm
            .iter()
            .copied()
            .take_while(|&z| z < build_height_mm)
            .collect();
        boundaries_mm.push(build_height_mm);
        Ok(Self { boundaries_mm })
    }
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
