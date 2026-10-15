//! Format-independent identities for retained, immutable render jobs.
use super::{ExportKind, ExportRequest};
use crate::Result;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Identify rendered pixels independently of their output filename or encoding.
///
/// The mesh hash must describe the exact binary snapshot, rather than the source
/// file: importing one source with different print geometry changes its pixels.
/// Renderer/runtime versions are checked separately by the inner render recipe.
pub fn retained_render_key(
    scene: &Value,
    mesh_hash: &str,
    request: &ExportRequest,
) -> Result<String> {
    request.selection.validate()?;
    if mesh_hash.len() != 64 || !mesh_hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("retained render requires a SHA-256 mesh snapshot hash".into());
    }
    let mut scene = scene.clone();
    scene
        .as_object_mut()
        .ok_or("retained render scene must be an object")?
        .remove("mesh_path");
    let animated = matches!(
        request.kind,
        ExportKind::Gif | ExportKind::Mp4 | ExportKind::SectionTurntable
    );
    let diagram = request.kind == ExportKind::Diagram;
    let frame_count = if animated {
        let count = request.animation_frame_count()?;
        if !(1..=10000).contains(&count) {
            return Err("retained animation requires 1 to 10000 frames".into());
        }
        count
    } else if diagram {
        4
    } else {
        1
    };
    let start_angle = if diagram || (animated && request.timeline.is_some()) {
        0.
    } else {
        request.start_angle_degrees
    };
    if !start_angle.is_finite() || start_angle.abs() > 1e9 {
        return Err("start angle must be finite and within supported range".into());
    }
    let animation = if animated {
        json!({
            "mode":request.animation,
            "frame_rate":request.frame_rate,
            "timeline":request.timeline,
            "turntable_timing":request.turntable_timing,
        })
    } else {
        Value::Null
    };
    let recipe = json!({
        "retained_render_version":2,
        "scene":scene,
        "mesh_hash":mesh_hash.to_ascii_lowercase(),
        "kind":if animated { "animation" } else if diagram { "diagram" } else { "still" },
        "selection":request.selection,
        "size":request.size,
        "frame_count":frame_count,
        "transparent":if diagram { true } else { request.transparent && !animated },
        "section":request.section,
        "layout":request.layout,
        "animation":animation,
        "build_stage":request.build_stage,
        "start_angle_degrees":start_angle,
    });
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&recipe)?)
    ))
}
