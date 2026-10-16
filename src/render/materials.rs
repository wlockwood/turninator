//! Version 1 of the shared physical-material recipe, matching renderer/scene.py.
use super::contract::*;
use crate::Result;
use std::collections::BTreeMap;

/// Base, unique tool/color materials in first-use order, then the section material.
/// The final slot is reserved even when a particular frame has no cut faces.
pub fn resolve_materials(scene: &RenderSceneSnapshot) -> Result<MaterialSet> {
    let a = &scene.appearance;
    let mut base = PhysicalMaterial {
        name: a.material_preset.clone(),
        unlit: false,
        base_color_linear: a.color,
        absorption_color_linear: a.color,
        roughness: a.roughness,
        metallic: 0.,
        anisotropy: 0.,
        coat: 0.,
        coat_roughness: 0.03,
        transmission: 0.,
        ior: 1.5,
        absorption_density_per_mm: 0.,
        scattering_per_mm: 0.,
        scattering_anisotropy: 0.,
    };
    match a.material_preset.as_str() {
        "matte_pla" | "clay" => {}
        "silk_pla" => {
            base.metallic = 0.18;
            base.anisotropy = 0.65;
        }
        "glossy_plastic" => {
            base.coat = 0.35;
            base.coat_roughness = 0.12;
        }
        "resin" => {
            base.ior = 1.46;
            base.transmission = a.resin_clarity;
            base.roughness = (a.resin_cloudiness * 0.5).max(0.025);
            base.absorption_density_per_mm = (20. * (1. - a.resin_clarity) + 2.) / 1000.;
        }
        _ => return Err("unsupported physical material preset".into()),
    }
    let mut result = MaterialSet {
        materials: vec![base.clone()],
        triangle_materials: vec![0; scene.mesh.triangles.len()],
    };
    let mut ids = BTreeMap::new();
    let mut previous = 0;
    for group in &scene.metadata.face_groups {
        if group.start_triangle < previous
            || group.end_triangle <= group.start_triangle
            || group.end_triangle as usize > result.triangle_materials.len()
        {
            return Err("invalid material face range".into());
        }
        previous = group.end_triangle;
        let key = (
            group.tool,
            group.color.map(|v| if v == 0. { 0 } else { v.to_bits() }),
        );
        let index = if let Some(&index) = ids.get(&key) {
            index
        } else {
            if ids.len() >= 256 {
                return Err("material mapping exceeds 256 unique tool/color combinations".into());
            }
            let index = result.materials.len() as u32;
            let mut material = base.clone();
            material.name = format!("Tool{}_{}", group.tool, group.label);
            material.base_color_linear = group.color;
            material.absorption_color_linear = group.color;
            result.materials.push(material);
            ids.insert(key, index);
            index
        };
        result.triangle_materials[group.start_triangle as usize..group.end_triangle as usize]
            .fill(index);
    }
    let mut cap = base;
    cap.name = "SectionCutSurface".into();
    cap.base_color_linear = [0.5, 0.18, 0.065];
    cap.absorption_color_linear = cap.base_color_linear;
    cap.roughness = 0.65;
    cap.metallic = 0.;
    cap.anisotropy = 0.;
    cap.coat = 0.;
    cap.transmission = 0.;
    cap.absorption_density_per_mm = 0.;
    result.materials.push(cap);
    Ok(result)
}
