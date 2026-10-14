use crate::{Result, mesh::Bounds};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug)]
pub enum CanonicalView {
    Front,
    Right,
    Top,
    Iso,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrbitCamera {
    pub yaw_degrees: f64,
    pub pitch_degrees: f64,
    pub distance_mm: f64,
    pub target_mm: [f64; 3],
    pub orthographic: bool,
    pub vertical_fov_degrees: f64,
    pub ortho_span_mm: f64,
}
impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            yaw_degrees: 45.,
            pitch_degrees: 30.,
            distance_mm: 100.,
            target_mm: [0., 0., 0.],
            orthographic: false,
            vertical_fov_degrees: 35.,
            ortho_span_mm: 100.,
        }
    }
}
impl OrbitCamera {
    pub fn validate(&self) -> Result<()> {
        if !self.yaw_degrees.is_finite()
            || !self.pitch_degrees.is_finite()
            || self.pitch_degrees.abs() > 90.
            || !self.distance_mm.is_finite()
            || !(0.0001..=1e9).contains(&self.distance_mm)
            || !self.ortho_span_mm.is_finite()
            || !(0.0001..=1e9).contains(&self.ortho_span_mm)
            || !self.vertical_fov_degrees.is_finite()
            || !(1.0..=120.0).contains(&self.vertical_fov_degrees)
            || self
                .target_mm
                .iter()
                .any(|v| !v.is_finite() || v.abs() > 1e12)
        {
            return Err("invalid physical camera parameters".into());
        }
        Ok(())
    }
    pub fn fit(&mut self, bounds: &Bounds) {
        self.fit_aspect(bounds, 1.);
    }
    pub fn fit_aspect(&mut self, bounds: &Bounds, aspect: f64) {
        if bounds
            .min
            .iter()
            .chain(bounds.max.iter())
            .any(|v| !v.is_finite())
        {
            return;
        }
        self.target_mm = std::array::from_fn(|i| (bounds.min[i] + bounds.max[i]) * 0.5);
        let radius = (0..3)
            .map(|i| ((bounds.max[i] - bounds.min[i]) * 0.5).powi(2))
            .sum::<f64>()
            .sqrt()
            .max(0.001);
        let aspect = if aspect.is_finite() {
            aspect.clamp(0.01, 100.)
        } else {
            1.
        };
        let half = (self.vertical_fov_degrees.clamp(1., 120.) * 0.5).to_radians();
        let fit_half = (half.tan() * aspect.min(1.)).atan();
        self.distance_mm = (radius / fit_half.sin() * 1.15).clamp(0.0001, 1e9);
        self.ortho_span_mm = (radius * 2. * 1.15 / aspect.min(1.)).clamp(0.0001, 1e9);
    }
    pub fn orbit(&mut self, dx: f64, dy: f64) {
        if dx.is_finite() && dy.is_finite() {
            self.yaw_degrees = (self.yaw_degrees - dx * 0.35).rem_euclid(360.);
            self.pitch_degrees = (self.pitch_degrees + dy * 0.35).clamp(-89.9, 89.9);
        }
    }
    pub fn pan(&mut self, dx: f64, dy: f64, viewport_height: f64) {
        if !dx.is_finite()
            || !dy.is_finite()
            || !viewport_height.is_finite()
            || viewport_height <= 0.
        {
            return;
        }
        let (right, up, _) = self.basis();
        let span = if self.orthographic {
            self.ortho_span_mm
        } else {
            2. * self.distance_mm * (self.vertical_fov_degrees * 0.5).to_radians().tan()
        };
        for i in 0..3 {
            self.target_mm[i] = (self.target_mm[i]
                + (-dx * right[i] + dy * up[i]) * span / viewport_height)
                .clamp(-1e12, 1e12);
        }
    }
    pub fn zoom(&mut self, delta: f64) {
        if !delta.is_finite() {
            return;
        }
        let scale = (-delta.clamp(-1e4, 1e4) * 0.002).exp();
        self.distance_mm = (self.distance_mm * scale).clamp(0.0001, 1e9);
        self.ortho_span_mm = (self.ortho_span_mm * scale).clamp(0.0001, 1e9);
    }
    pub fn canonical(&mut self, view: CanonicalView) {
        (self.yaw_degrees, self.pitch_degrees) = match view {
            CanonicalView::Front => (0., 0.),
            CanonicalView::Right => (90., 0.),
            CanonicalView::Top => (0., 90.),
            CanonicalView::Iso => (45., 35.264389682754654),
        };
    }
    pub fn basis(&self) -> ([f64; 3], [f64; 3], [f64; 3]) {
        let (sy, cy) = self.yaw_degrees.to_radians().sin_cos();
        let (sp, cp) = self.pitch_degrees.to_radians().sin_cos();
        (
            [cy, sy, 0.],
            [-sy * sp, cy * sp, cp],
            [sy * cp, -cy * cp, sp],
        )
    }
    pub fn camera_spec(&self) -> serde_json::Value {
        let (_, up, direction) = self.basis();
        serde_json::json!({"target_mm":self.target_mm,"direction":direction,"up":up,"projection":if self.orthographic {"orthographic"} else {"perspective"},"distance_mm":self.distance_mm,"vertical_fov_degrees":self.vertical_fov_degrees,"ortho_span_mm":self.ortho_span_mm})
    }
    pub fn project(&self, point: [f64; 3], aspect: f64) -> Result<[f64; 3]> {
        self.validate()?;
        if !aspect.is_finite() || aspect <= 0. || point.iter().any(|v| !v.is_finite()) {
            return Err("invalid projection input".into());
        }
        let (right, up, back) = self.basis();
        let relative = std::array::from_fn::<_, 3, _>(|i| point[i] - self.target_mm[i]);
        let dot = |v: [f64; 3]| (0..3).map(|i| relative[i] * v[i]).sum::<f64>();
        let depth = self.distance_mm - dot(back);
        let half = if self.orthographic {
            self.ortho_span_mm * 0.5
        } else {
            depth * (self.vertical_fov_degrees * 0.5).to_radians().tan()
        };
        let near = (self.distance_mm * 0.0001).max(1e-5);
        let far = self.distance_mm * 100. + 100.;
        let z = if self.orthographic {
            (depth - near) / (far - near)
        } else {
            far / (far - near) - far * near / ((far - near) * depth)
        };
        Ok([dot(right) / (half * aspect), dot(up) / half, z])
    }
}

pub(crate) fn rotation_rows(degrees: [f32; 3]) -> [[f32; 3]; 3] {
    let (sx, cx) = degrees[0].to_radians().sin_cos();
    let (sy, cy) = degrees[1].to_radians().sin_cos();
    let (sz, cz) = degrees[2].to_radians().sin_cos();
    [
        [cz * cy, cz * sy * sx - sz * cx, cz * sy * cx + sz * sx],
        [sz * cy, sz * sy * sx + cz * cx, sz * sy * cx - cz * sx],
        [-sy, cy * sx, cy * cx],
    ]
}
