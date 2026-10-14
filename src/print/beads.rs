use super::schedule::validate_height;
use crate::{Result, mesh::Mesh};
use std::f64::consts::{FRAC_PI_2, PI};

pub const MAX_BEAD_SEGMENTS: usize = 30_000;
const HALF_ARC_STEPS: usize = 12;

#[derive(Clone, Debug)]
pub struct BeadSegment {
    pub start: [f64; 3],
    pub end: [f64; 3],
    pub width_mm: f64,
    pub height_mm: f64,
    pub volume_mm3: f64,
    pub source_line: usize,
    pub feature: String,
    pub tool: u32,
    pub layer_index: u32,
    pub width_source: String,
    pub height_source: String,
}

pub fn build_bead_mesh(segments: &[BeadSegment]) -> Result<Mesh> {
    if segments.is_empty() {
        return Err("no bead segments to render".into());
    }
    if segments.len() > MAX_BEAD_SEGMENTS {
        return Err(format!("bead budget exceeded: maximum {MAX_BEAD_SEGMENTS} segments").into());
    }
    let mut mesh = Mesh {
        positions: Vec::with_capacity(segments.len() * 54),
        triangles: Vec::with_capacity(segments.len() * 104),
    };
    for segment in segments {
        validate_segment(segment)?;
        let dx = segment.end[0] - segment.start[0];
        let dy = segment.end[1] - segment.start[1];
        let length = dx.hypot(dy);
        let side = [-dy / length, dx / length];
        let radius = segment.height_mm / 2.;
        let offset = (segment.width_mm - segment.height_mm) / 2.;
        let mut ring = Vec::with_capacity(26);
        if offset == 0. {
            for i in 0..2 * HALF_ARC_STEPS {
                let angle = -FRAC_PI_2 + i as f64 * PI / HALF_ARC_STEPS as f64;
                ring.push([radius * angle.cos(), radius * angle.sin()]);
            }
        } else {
            for half in 0..2 {
                let horizontal = if half == 0 { offset } else { -offset };
                for i in 0..=HALF_ARC_STEPS {
                    let angle =
                        -FRAC_PI_2 + half as f64 * PI + i as f64 * PI / HALF_ARC_STEPS as f64;
                    ring.push([horizontal + radius * angle.cos(), radius * angle.sin()]);
                }
            }
        }
        let base = mesh.positions.len() as u32;
        let count = ring.len() as u32;
        for point in [segment.start, segment.end] {
            for r in &ring {
                mesh.positions.push([
                    point[0] + side[0] * r[0],
                    point[1] + side[1] * r[0],
                    point[2] - radius + r[1],
                ]);
            }
        }
        let start_center = base + 2 * count;
        let end_center = start_center + 1;
        mesh.positions.push([
            segment.start[0],
            segment.start[1],
            segment.start[2] - radius,
        ]);
        mesh.positions
            .push([segment.end[0], segment.end[1], segment.end[2] - radius]);
        for i in 0..count {
            let j = (i + 1) % count;
            mesh.triangles.extend([
                [base + i, base + j, base + count + j],
                [base + i, base + count + j, base + count + i],
                [start_center, base + j, base + i],
                [end_center, base + count + i, base + count + j],
            ]);
        }
    }
    mesh.bounds()?;
    Ok(mesh)
}

pub(crate) fn validate_segment(segment: &BeadSegment) -> Result<()> {
    validate_height(segment.height_mm)?;
    if !segment.width_mm.is_finite()
        || segment.width_mm < segment.height_mm
        || segment.width_mm > 10.
    {
        return Err(
            "stadium bead width must be finite, at least its height, and at most 10 mm".into(),
        );
    }
    if segment
        .start
        .iter()
        .chain(segment.end.iter())
        .any(|v| !v.is_finite() || v.abs() > 1e7)
    {
        return Err("bead position is non-finite or exceeds the 1e7 mm coordinate budget".into());
    }
    if (segment.end[2] - segment.start[2]).abs() > 1e-8 {
        return Err("nonplanar deposition is unsupported".into());
    }
    if (segment.end[0] - segment.start[0]).hypot(segment.end[1] - segment.start[1]) <= 1e-10 {
        return Err("bead requires nonzero planar length".into());
    }
    if !segment.volume_mm3.is_finite() || segment.volume_mm3 < 0. {
        return Err("invalid bead volume".into());
    }
    Ok(())
}
