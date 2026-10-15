use crate::{Result, mesh::Mesh};
use std::f64::consts::{FRAC_PI_2, PI};

pub const MAX_BEAD_SEGMENTS: usize = 250_000;
const HALF_ARC_STEPS: usize = 12;

/// Ring size is fixed for joined_stadium_v1. Open paths add two terminal fan centers.
pub const JOINED_RING_VERTICES: usize = 32;

pub struct JoinedBeadMesh {
    pub mesh: Mesh,
    pub ring_vertices: usize,
    pub omitted_collapsed_triangles: usize,
    /// Half-open triangle index ranges for side faces; all other faces are flat caps.
    pub smooth_face_ranges: Vec<[usize; 2]>,
}

/// Sweeps ordered contiguous paths with shared, clamped-miter sections. Width and
/// height at an internal joint are the mean of its incident segment dimensions.
/// This is a visualization model, not a union or a physical corner-flow model.
pub fn build_joined_bead_mesh(segments: &[BeadSegment]) -> Result<Mesh> {
    Ok(build_joined_bead_mesh_with_report(segments)?.mesh)
}
pub fn build_joined_bead_mesh_with_report(segments: &[BeadSegment]) -> Result<JoinedBeadMesh> {
    if segments.is_empty() || segments.len() > MAX_BEAD_SEGMENTS {
        return Err("joined bead segment count is empty or exceeds budget".into());
    }
    for segment in segments {
        validate_segment(segment)?;
    }
    let paths: Vec<_> = {
        let mut ranges = Vec::new();
        let mut begin = 0;
        while begin < segments.len() {
            let mut end = begin + 1;
            while end < segments.len() && compatible(&segments[end - 1], &segments[end]) {
                end += 1;
            }
            ranges.push(begin..end);
            begin = end;
        }
        ranges
    };
    let open = paths
        .iter()
        .filter(|range| {
            let path = &segments[(*range).clone()];
            !(path.len() >= 3 && compatible(path.last().unwrap(), &path[0]))
        })
        .count();
    let ring_vertices = [32, 24, 16, 12, 8]
        .into_iter()
        .find(|&ring| {
            (segments.len() + open) * ring + 2 * open <= crate::mesh::MAX_VERTICES
                && (segments.len() + open) * ring * 2 <= crate::mesh::MAX_TRIANGLES
        })
        .ok_or(
            "toolpath exceeds mesh budget even at 8 vertices per bead ring; use a smaller toolpath",
        )?;
    let mut mesh = Mesh::default();
    let mut smooth_face_ranges = Vec::new();
    let mut begin = 0;
    while begin < segments.len() {
        let mut end = begin + 1;
        while end < segments.len() && compatible(&segments[end - 1], &segments[end]) {
            end += 1;
        }
        let first = mesh.triangles.len();
        append_joined_path(&mut mesh, &segments[begin..end], ring_vertices)?;
        smooth_face_ranges.push([first, first + (end - begin) * ring_vertices * 2]);
        begin = end;
    }
    // Blender and the native viewport share f32 coordinates. Remove only faces
    // whose area is exactly zero in that representation, with no tolerance or
    // path simplification, and remap the explicit side ranges to the kept faces.
    let original_count = mesh.triangles.len();
    let mut kept = Vec::with_capacity(original_count + 1);
    kept.push(0);
    mesh.triangles.retain(|triangle| {
        let [a, b, c] = triangle.map(|i| mesh.positions[i as usize].map(|v| v as f32 as f64));
        let u: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
        let v: [f64; 3] = std::array::from_fn(|i| c[i] - a[i]);
        let area = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let keep = area.iter().any(|&component| component != 0.);
        kept.push(kept.last().unwrap() + usize::from(keep));
        keep
    });
    let omitted_collapsed_triangles = original_count - mesh.triangles.len();
    for range in &mut smooth_face_ranges {
        *range = [kept[range[0]], kept[range[1]]];
        if range[0] == range[1] {
            return Err("a depositing path is too small for GPU precision".into());
        }
    }
    mesh.bounds()?;
    Ok(JoinedBeadMesh {
        mesh,
        ring_vertices,
        omitted_collapsed_triangles,
        smooth_face_ranges,
    })
}

pub fn compatible(a: &BeadSegment, b: &BeadSegment) -> bool {
    if a.end != b.start
        || a.tool != b.tool
        || a.layer_index != b.layer_index
        || a.feature != b.feature
    {
        return false;
    }
    let sa = segment_side(a);
    let sb = segment_side(b);
    // A reversal has no bounded miter. Start a separately capped path instead.
    sa[0] * sb[0] + sa[1] * sb[1] > -0.9999
}
fn segment_side(segment: &BeadSegment) -> [f64; 2] {
    let dx = segment.end[0] - segment.start[0];
    let dy = segment.end[1] - segment.start[1];
    let length = dx.hypot(dy);
    [-dy / length, dx / length]
}
fn append_joined_path(mesh: &mut Mesh, path: &[BeadSegment], ring_vertices: usize) -> Result<()> {
    let closed = path.len() >= 3 && compatible(path.last().unwrap(), &path[0]);
    let rings = path.len() + usize::from(!closed);
    let base = mesh.positions.len() as u32;
    let count = ring_vertices as u32;
    for i in 0..rings {
        let point = if i < path.len() {
            path[i].start
        } else {
            path[i - 1].end
        };
        let previous = if i > 0 {
            Some(&path[i - 1])
        } else if closed {
            path.last()
        } else {
            None
        };
        let next = path.get(i);
        let (side, width, height) = match (previous, next) {
            (Some(a), Some(b)) => {
                let sa = segment_side(a);
                let sb = segment_side(b);
                let sum = [sa[0] + sb[0], sa[1] + sb[1]];
                let length = sum[0].hypot(sum[1]);
                if length < 1e-8 {
                    return Err(
                        "joined path reverses direction at a joint; split the path explicitly"
                            .into(),
                    );
                }
                let direction = [sum[0] / length, sum[1] / length];
                let scale = (1. / (direction[0] * sb[0] + direction[1] * sb[1])).min(4.);
                (
                    [direction[0] * scale, direction[1] * scale],
                    0.5 * (a.width_mm + b.width_mm),
                    0.5 * (a.height_mm + b.height_mm),
                )
            }
            (Some(s), None) | (None, Some(s)) => (segment_side(s), s.width_mm, s.height_mm),
            _ => unreachable!(),
        };
        let radius = height * 0.5;
        let offset = (width - height) * 0.5;
        for sample in 0..ring_vertices {
            let angle = -FRAC_PI_2 + 2. * PI * sample as f64 / ring_vertices as f64;
            let (s, c) = angle.sin_cos();
            // Radial intersection with an exact stadium avoids duplicate vertices
            // when width==height, while retaining exact cardinal dimensions.
            let distance = if s.abs() > 1e-12 && (radius * c / s).abs() <= offset {
                radius / s.abs()
            } else {
                offset * c.abs() + (radius * radius - offset * offset * s * s).max(0.).sqrt()
            };
            let lateral = distance * c;
            let vertical = distance * s;
            mesh.positions.push([
                point[0] + side[0] * lateral,
                point[1] + side[1] * lateral,
                point[2] - radius + vertical,
            ]);
        }
    }
    for i in 0..path.len() {
        let a = base + i as u32 * count;
        let b = base + ((i + 1) % rings) as u32 * count;
        for j in 0..count {
            let k = (j + 1) % count;
            mesh.triangles
                .extend([[a + j, a + k, b + k], [a + j, b + k, b + j]]);
        }
    }
    if !closed {
        let first = mesh.positions.len() as u32;
        let last = first + 1;
        let a = &path[0];
        let b = path.last().unwrap();
        mesh.positions
            .push([a.start[0], a.start[1], a.start[2] - a.height_mm * 0.5]);
        mesh.positions
            .push([b.end[0], b.end[1], b.end[2] - b.height_mm * 0.5]);
        let end = base + (rings as u32 - 1) * count;
        for j in 0..count {
            let k = (j + 1) % count;
            mesh.triangles
                .extend([[first, base + k, base + j], [last, end + j, end + k]]);
        }
    }
    Ok(())
}

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
    if segments.len() * 54 > crate::mesh::MAX_VERTICES
        || segments.len() * 104 > crate::mesh::MAX_TRIANGLES
    {
        return Err(
            "independent bead mesh exceeds geometry budget; select joined beads for larger files"
                .into(),
        );
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
    if !segment.height_mm.is_finite() || !(0.00001..=2.).contains(&segment.height_mm) {
        return Err("bead height must be finite and in 0.00001..=2 mm".into());
    }
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
