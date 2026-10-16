//! Explicit prepared geometry protocol. Never overwrites immutable source snapshots.
use super::{contract::*, geometry::check_derived_geometry_budget};
use crate::Result;
use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::Path,
};

pub const GEOMETRY_RECIPE: &str = "shared-realization-v1";
pub const BLENDER_ADAPTER_VERSION: u32 = 2;

/// Geometry reuse within one immutable scene snapshot. Native sheet ink depends
/// on camera, projection, pose and output pixels; ordinary transform-only frames
/// and Blender's curve-based ink reuse their build-space mesh upload.
pub fn frame_geometry_key(frame: &FrameSpec, expanded_ink: bool) -> Result<String> {
    use sha2::{Digest, Sha256};
    let edges = matches!(
        frame.key.panel,
        RenderPanel::Top | RenderPanel::Iso | RenderPanel::Front | RenderPanel::Right
    );
    let ink = if edges && expanded_ink {
        serde_json::json!({"camera":frame.camera,"angle":frame.angle_degrees,"width":frame.width,"height":frame.height})
    } else {
        serde_json::Value::Null
    };
    let recipe = serde_json::json!({"layers":frame.visible_layers,"section":frame.section,"edges":edges,"ink":ink});
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&recipe)?)
    ))
}

pub fn recipe_versions(identity: &BackendIdentity) -> Result<(&'static str, u32)> {
    use super::BackendId;
    match (identity.backend, identity.adapter_recipe_version) {
        (BackendId::BlenderEevee | BackendId::BlenderCycles, 1) => Ok(("legacy-realization-v1", 1)),
        (BackendId::BlenderEevee | BackendId::BlenderCycles, 2) => Ok((GEOMETRY_RECIPE, 2)),
        (BackendId::Wgpu | BackendId::Filament | BackendId::LuxCore, 1) => Ok((GEOMETRY_RECIPE, 2)),
        _ => Err("unsupported renderer geometry/frame-plan recipe version".into()),
    }
}

/// Canonical host fit for scenes without an explicit camera. Reproduces Blender's
/// 70mm / 36mm horizontal sensor, full-source radial envelope and 10% margin.
pub fn fit_legacy_camera(
    scene: &RenderSceneSnapshot,
    width: u32,
    height: u32,
) -> Result<RenderCamera> {
    if width == 0 || height == 0 {
        return Err("invalid camera aspect".into());
    }
    let transform = super::geometry::resolve_display_transform(scene, 0.)?;
    let mut radius = 0f64;
    let mut top = 0f64;
    for p in &scene.mesh.positions {
        let q: [f64; 3] = std::array::from_fn(|i| {
            transform[i][3] + (0..3).map(|j| transform[i][j] * p[j]).sum::<f64>()
        });
        radius = radius.max(q[0].hypot(q[1]));
        top = top.max(q[2]);
    }
    let aspect = f64::from(width) / f64::from(height);
    let half = (36. / 140. * (1. / aspect).min(1.)).atan();
    let distance = radius.hypot(top * 0.5).max(0.1) / half.sin() * 1.1;
    let direction = [1.25, -1.7, 1.];
    let length = direction.iter().map(|v| v * v).sum::<f64>().sqrt();
    let target = [0., 0., top * 0.5];
    RenderCamera::from_legacy(
        &serde_json::json!({"target_mm":target,"direction":direction.map(|v|v/length),"up":[0.,0.,1.],
        "distance_mm":distance,"projection":"perspective","vertical_fov_degrees":2.*(36./140./aspect).atan().to_degrees(),"ortho_span_mm":1.}),
    )
}

/// Fixed little-endian TNPRP001 layout: header (magic/version/V/T/E),
/// V*(position3f,normal3f,build3f), T*(indices3u,materialu), E*(endpoints6f).
pub fn write_geometry(
    path: &Path,
    geometry: &RealizedScene,
    material_count: usize,
    limits: &RenderLimits,
) -> Result<u64> {
    let nv = geometry.mesh.positions.len();
    let nt = geometry.mesh.triangles.len();
    let ne = geometry.design_edges_mm.len();
    check_derived_geometry_budget(nv, nt)?;
    let bytes = 24u64 + nv as u64 * 36 + nt as u64 * 16 + ne as u64 * 24;
    if !path.is_absolute()
        || ne > 200_000
        || !(1..=258).contains(&material_count)
        || geometry.normals.len() != nv
        || geometry.build_positions_mm.len() != nv
        || geometry.triangle_materials.len() != nt
        || (nv == 0) != (nt == 0)
        || bytes > limits.host_bytes
        || bytes > limits.scratch_bytes
    {
        return Err("invalid prepared geometry attributes or budget".into());
    }
    if geometry
        .mesh
        .positions
        .iter()
        .flatten()
        .any(|&v| !(v as f32).is_finite())
        || geometry.normals.iter().any(|n| {
            n.iter().any(|v| !v.is_finite())
                || (n.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>() - 1.).abs() > 0.0001
        })
        || geometry
            .build_positions_mm
            .iter()
            .flatten()
            .any(|v| !v.is_finite())
        || geometry
            .design_edges_mm
            .iter()
            .flatten()
            .flatten()
            .any(|v| !v.is_finite())
        || geometry
            .mesh
            .triangles
            .iter()
            .flatten()
            .any(|&i| i as usize >= nv)
        || geometry
            .triangle_materials
            .iter()
            .any(|&i| i as usize >= material_count)
    {
        return Err("invalid prepared geometry values".into());
    }
    let mut out = BufWriter::new(OpenOptions::new().write(true).create_new(true).open(path)?);
    out.write_all(b"TNPRP001")?;
    for n in [1, nv as u32, nt as u32, ne as u32] {
        out.write_all(&n.to_le_bytes())?;
    }
    for i in 0..nv {
        for n in geometry.mesh.positions[i]
            .map(|v| v as f32)
            .into_iter()
            .chain(geometry.normals[i])
            .chain(geometry.build_positions_mm[i])
        {
            out.write_all(&n.to_le_bytes())?;
        }
    }
    for (t, &m) in geometry
        .mesh
        .triangles
        .iter()
        .zip(&geometry.triangle_materials)
    {
        for n in t.iter().copied().chain([m]) {
            out.write_all(&n.to_le_bytes())?;
        }
    }
    for n in geometry.design_edges_mm.iter().flatten().flatten() {
        out.write_all(&n.to_le_bytes())?;
    }
    out.flush()?;
    Ok(bytes)
}
