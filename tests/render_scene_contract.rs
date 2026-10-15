#[path = "fixtures/renderers/common.rs"]
mod common;
use common::*;
use turninator::render::{contract::*, materials::resolve_materials};
#[test]
fn versioned_presets_retain_legacy_physics() {
    let mut s = scene(cube([0.; 3], 10.));
    for (preset, metal, aniso, coat) in [
        ("matte_pla", 0., 0., 0.),
        ("silk_pla", 0.18, 0.65, 0.),
        ("glossy_plastic", 0., 0., 0.35),
        ("clay", 0., 0., 0.),
    ] {
        s.appearance.material_preset = preset.into();
        let m = resolve_materials(&s).unwrap();
        let p = &m.materials[0];
        assert_eq!(p.base_color_linear, s.appearance.color);
        near(p.roughness as f64, 0.45);
        near(p.metallic as f64, metal);
        near(p.anisotropy as f64, aniso);
        near(p.coat as f64, coat);
        assert_eq!(p.scattering_per_mm, 0.);
    }
}
#[test]
fn resin_absorption_is_per_mm_and_cloudiness_does_not_add_scattering() {
    let mut s = scene(cube([0.; 3], 10.));
    s.appearance.material_preset = "resin".into();
    s.appearance.resin_clarity = 0.8;
    s.appearance.resin_cloudiness = 0.4;
    let m = resolve_materials(&s).unwrap();
    let p = &m.materials[0];
    near(p.ior as f64, 1.46);
    near(p.transmission as f64, 0.8);
    near(p.roughness as f64, 0.2);
    near(p.absorption_density_per_mm as f64, 0.006);
    assert_eq!(p.absorption_color_linear, s.appearance.color);
    assert_eq!(p.scattering_per_mm, 0.);
    let mut synthetic = p.clone();
    synthetic.scattering_per_mm = 0.02;
    synthetic.scattering_anisotropy = 0.7;
    let decoded: PhysicalMaterial =
        serde_json::from_str(&serde_json::to_string(&synthetic).unwrap()).unwrap();
    assert_eq!(decoded.scattering_per_mm, 0.02);
}
#[test]
fn face_groups_share_tool_color_material_and_leave_unassigned_faces_base() {
    let mut s = scene(cube([0.; 3], 10.));
    s.metadata.face_groups = vec![
        FaceGroup {
            start_triangle: 2,
            end_triangle: 4,
            tool: 1,
            label: "red".into(),
            color: [1., 0., 0.],
        },
        FaceGroup {
            start_triangle: 6,
            end_triangle: 8,
            tool: 1,
            label: "same color".into(),
            color: [1., 0., 0.],
        },
    ];
    let m = resolve_materials(&s).unwrap();
    assert_eq!(m.materials.len(), 3);
    assert_eq!(m.triangle_materials, [0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 0, 0]);
    assert_eq!(m.materials[1].base_color_linear, [1., 0., 0.]);
    assert_eq!(m.materials[2].name, "SectionCutSurface");
}
#[test]
fn conformance_thresholds_are_frozen_from_design() {
    let c: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/renderers/conformance.json")).unwrap();
    assert_eq!(c["geometry"]["relative_error_max"], 0.00001);
    assert_eq!(c["diagnostic_mask"]["intersection_over_union_min"], 0.995);
    assert_eq!(c["cpu_transfer"]["code_value_error_max"], 1);
}
#[test]
fn cut_material_matches_legacy_blender_cut_surface() {
    let s = scene(cube([0.; 3], 10.));
    let m = resolve_materials(&s).unwrap();
    let cut = m.materials.last().unwrap();
    assert_eq!(cut.base_color_linear, [0.5, 0.18, 0.065]);
    near(cut.roughness as f64, 0.65);
    assert_eq!(cut.transmission, 0.);
}
