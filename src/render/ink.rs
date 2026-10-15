//! Shared sheet ink realization. SDKs upload the resulting ordinary triangles;
//! crease extraction is exclusively geometry::realize_frame's responsibility.
use super::{
    contract::*,
    geometry::{check_derived_geometry_budget, resolve_display_transform},
};
use crate::{Result, mesh::Mesh};
use std::sync::Arc;
type V = [f64; 3];
fn add(a: V, b: V) -> V {
    std::array::from_fn(|i| a[i] + b[i])
}
fn sub(a: V, b: V) -> V {
    std::array::from_fn(|i| a[i] - b[i])
}
fn mul(a: V, s: f64) -> V {
    a.map(|v| v * s)
}
fn dot(a: V, b: V) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn cross(a: V, b: V) -> V {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: V) -> Result<V> {
    let n = dot(a, a).sqrt();
    if !n.is_finite() || n <= 0. {
        Err("invalid design-edge direction".into())
    } else {
        Ok(mul(a, 1. / n))
    }
}

/// Append camera-facing square tubes at Blender's 0.4-pixel radius (0.8 diameter).
/// Coordinates return to build space before upload; normal depth testing hides
/// back edges. No bias, topology welding, or additional crease decision occurs.
/// Call after realize_frame for native sheet panels, with resolve_materials.
/// Any sheet camera, angle, dimensions or geometry change invalidates this result.
/// Base geometry and section/build analyses remain intact. Source quotas are not
/// derived quotas: the fixed mesh ceiling and explicit host/GPU budgets apply.
pub fn expand_design_edges(
    scene: &RenderSceneSnapshot,
    frame: &FrameSpec,
    geometry: &mut RealizedScene,
    materials: &mut MaterialSet,
    limits: &RenderLimits,
) -> Result<()> {
    let edges = geometry.design_edges_mm.len();
    if edges == 0 {
        return Ok(());
    }
    let nv = geometry
        .mesh
        .positions
        .len()
        .checked_add(edges.checked_mul(8).ok_or("ink capacity overflow")?)
        .ok_or("ink capacity overflow")?;
    let nt = geometry
        .mesh
        .triangles
        .len()
        .checked_add(edges.checked_mul(12).ok_or("ink capacity overflow")?)
        .ok_or("ink capacity overflow")?;
    check_derived_geometry_budget(nv, nt)?;
    let gpu = nv as u64 * 36 + nt as u64 * 16;
    let host = nv as u64 * 80 + nt as u64 * 32;
    if edges > 200_000
        || materials.materials.is_empty()
        || materials.materials.len() >= 259
        || gpu > limits.gpu_bytes
        || host > limits.host_bytes
        || frame.width == 0
        || frame.height == 0
        || geometry.normals.len() != geometry.mesh.positions.len()
        || geometry.build_positions_mm.len() != geometry.mesh.positions.len()
        || geometry.triangle_materials.len() != geometry.mesh.triangles.len()
    {
        return Err("design-edge expansion exceeds capacity or has invalid attributes".into());
    }
    let transform = resolve_display_transform(scene, frame.angle_degrees)?;
    let display = |p: V| -> V {
        std::array::from_fn(|i| {
            transform[i][3] + (0..3).map(|j| transform[i][j] * p[j]).sum::<f64>()
        })
    };
    let inverse_vector =
        |p: V| -> V { std::array::from_fn(|i| (0..3).map(|j| transform[j][i] * p[j]).sum()) };
    let inverse = |p: V| inverse_vector(std::array::from_fn(|i| p[i] - transform[i][3]));
    let forward = unit(sub(frame.camera.target_mm, frame.camera.position_mm))?;
    let right = unit(cross(forward, frame.camera.up))?;
    let radius = |p: V| -> Result<f64> {
        let span = match frame.camera.projection {
            RenderProjection::Orthographic { vertical_span_mm } => vertical_span_mm,
            RenderProjection::Perspective {
                vertical_fov_degrees,
            } => {
                let depth = dot(sub(p, frame.camera.position_mm), forward);
                if depth <= 0. || !(1.0..=150.).contains(&vertical_fov_degrees) {
                    return Err("design edge crosses camera plane or invalid projection".into());
                }
                2. * depth * (vertical_fov_degrees.to_radians() * 0.5).tan()
            }
        };
        if !span.is_finite() || span <= 0. {
            return Err("invalid design edge projection".into());
        }
        Ok((span / f64::from(frame.height) * 0.4).max(0.00001))
    };
    let mut ink = Mesh::default();
    let mut normals = Vec::new();
    ink.positions.try_reserve_exact(edges * 8)?;
    ink.triangles.try_reserve_exact(edges * 12)?;
    normals.try_reserve_exact(edges * 8)?;
    for edge in &geometry.design_edges_mm {
        let a = display(edge[0].map(f64::from));
        let b = display(edge[1].map(f64::from));
        let axis = unit(sub(b, a))?;
        let side = unit(cross(axis, forward)).unwrap_or(right);
        let across = unit(cross(axis, side))?;
        let base = ink.positions.len() as u32;
        for p in [a, b] {
            let radius = radius(p)?;
            for [x, y] in [[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]] {
                let radial = add(mul(side, x), mul(across, y));
                let position = inverse(add(p, mul(radial, radius)));
                if position.iter().any(|&n| !(n as f32).is_finite()) {
                    return Err("design edge exceeds f32 capacity".into());
                }
                ink.positions.push(position);
                normals.push(unit(inverse_vector(radial))?.map(|v| v as f32));
            }
        }
        for t in [
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 0, 4],
            [3, 4, 7],
        ] {
            ink.triangles.push(t.map(|i| base + i));
        }
    }
    // Preflight every f32 triangle before mutation, including tiny/large-coordinate ink.
    for t in &ink.triangles {
        let [a, b, c] = t.map(|i| ink.positions[i as usize].map(|v| v as f32 as f64));
        if cross(sub(b, a), sub(c, a)) == [0.; 3] {
            return Err("design edge collapses at f32 precision".into());
        }
    }
    let mut material = materials.materials[0].clone();
    material.name = "DesignEdgeInk".into();
    material.unlit = true;
    material.base_color_linear = [0.018, 0.022, 0.03];
    material.absorption_color_linear = material.base_color_linear;
    material.transmission = 0.;
    material.absorption_density_per_mm = 0.;
    material.scattering_per_mm = 0.;
    let material_index = materials.materials.len() as u32;
    let mesh = Arc::make_mut(&mut geometry.mesh);
    let offset = mesh.positions.len() as u32;
    mesh.positions.try_reserve_exact(ink.positions.len())?;
    mesh.triangles.try_reserve_exact(ink.triangles.len())?;
    geometry.normals.try_reserve_exact(normals.len())?;
    geometry
        .build_positions_mm
        .try_reserve_exact(ink.positions.len())?;
    geometry
        .triangle_materials
        .try_reserve_exact(ink.triangles.len())?;
    materials.materials.try_reserve(1)?;
    geometry
        .build_positions_mm
        .extend(ink.positions.iter().map(|p| p.map(|v| v as f32)));
    mesh.positions.extend(ink.positions);
    mesh.triangles
        .extend(ink.triangles.iter().map(|t| t.map(|i| i + offset)));
    geometry.normals.extend(normals);
    geometry
        .triangle_materials
        .extend(std::iter::repeat_n(material_index, ink.triangles.len()));
    materials.materials.push(material);
    materials.triangle_materials = geometry.triangle_materials.clone();
    geometry.design_edges_mm.clear();
    Ok(())
}
