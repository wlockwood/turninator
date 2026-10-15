#[path = "fixtures/renderers/common.rs"]
mod common;
use common::*;
use turninator::{
    export::SectionSpec,
    mesh::Mesh,
    render::{contract::*, geometry::*},
};
fn cut(axis: u8, pos: f64, positive: bool) -> SectionSpec {
    SectionSpec {
        axis,
        position_mm: pos,
        keep_positive: positive,
        cap: true,
    }
}
fn area_at(s: &RealizedScene, axis: usize, pos: f64) -> f64 {
    s.mesh
        .triangles
        .iter()
        .filter_map(|t| {
            let [a, b, c] = t.map(|i| s.mesh.positions[i as usize]);
            if [a, b, c].iter().any(|p| (p[axis] - pos).abs() > 1e-6) {
                return None;
            }
            let u = std::array::from_fn::<_, 3, _>(|i| b[i] - a[i]);
            let v = std::array::from_fn::<_, 3, _>(|i| c[i] - a[i]);
            let n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            Some(n.iter().map(|x| x * x).sum::<f64>().sqrt() * 0.5)
        })
        .sum()
}
#[test]
fn complete_cube_sheet_has_flat_normals_edges_and_immutable_build_coordinates() {
    let s = scene(cube([0.; 3], 10.));
    let source = s.mesh.snapshot_hash().unwrap();
    let mut f = frame(&s);
    f.key.panel = RenderPanel::Top;
    let r = realize_frame(&s, &f).unwrap();
    assert_eq!(s.mesh.snapshot_hash().unwrap(), source);
    near(r.mesh.bounds().unwrap().max[2], 10.);
    assert_eq!(r.normals.len(), r.mesh.positions.len());
    assert_eq!(r.build_positions_mm.len(), r.mesh.positions.len());
    for (p, b) in r.mesh.positions.iter().zip(&r.build_positions_mm) {
        assert_eq!(*b, p.map(|x| x as f32));
    }
    for n in &r.normals {
        near(n.iter().map(|v| (*v as f64).powi(2)).sum::<f64>(), 1.);
        assert_eq!(n.iter().filter(|v| v.abs() > 0.001).count(), 1);
    }
    assert_eq!(r.design_edges_mm.len(), 12);
}
#[test]
fn cube_sections_all_axes_both_directions_have_physical_area_volume_and_outward_caps() {
    let s = scene(cube([0.; 3], 10.));
    for axis in 0..3 {
        for positive in [false, true] {
            let mut f = frame(&s);
            f.section = Some(cut(axis, 5., positive));
            let r = realize_frame(&s, &f).unwrap();
            near(area_at(&r, axis as usize, 5.), 100.);
            let b = r.mesh.bounds().unwrap();
            near(
                if positive {
                    b.min[axis as usize]
                } else {
                    b.max[axis as usize]
                },
                5.,
            );
            let a = r.analysis.as_ref().unwrap();
            assert!(a.cap_supported);
            assert_eq!(a.legacy["closed_input"], true);
            near(a.legacy["cross_section_area_mm2"].as_f64().unwrap(), 100.);
            near(a.legacy["original_volume_mm3"].as_f64().unwrap(), 1000.);
            near(a.legacy["remaining_volume_mm3"].as_f64().unwrap(), 500.);
            let cap_material = *r.triangle_materials.iter().max().unwrap();
            for (t, m) in r.mesh.triangles.iter().zip(&r.triangle_materials) {
                if *m == cap_material {
                    let n = r.normals[t[0] as usize];
                    near(n[axis as usize] as f64, if positive { -1. } else { 1. });
                }
            }
            assert_eq!(r.reference_bounds_mm.min, s.reference_bounds_mm.min);
            assert_eq!(r.reference_bounds_mm.max, s.reference_bounds_mm.max);
        }
    }
}
#[test]
fn closed_hollow_shell_keeps_its_hole_on_all_axes() {
    let mut mesh = cube([0.; 3], 10.);
    let mut cavity = cube([3.; 3], 4.);
    for t in &mut cavity.triangles {
        t.swap(1, 2);
    }
    append(&mut mesh, cavity);
    let s = scene(mesh);
    for axis in 0..3 {
        let mut f = frame(&s);
        f.section = Some(cut(axis, 5., false));
        let r = realize_frame(&s, &f).unwrap();
        near(area_at(&r, axis as usize, 5.), 84.);
        near(
            r.analysis.as_ref().unwrap().legacy["remaining_volume_mm3"]
                .as_f64()
                .unwrap(),
            468.,
        );
        for t in &r.mesh.triangles {
            let p = t.map(|i| r.mesh.positions[i as usize]);
            if p.iter().all(|p| p[axis as usize] == 5.) {
                let centroid =
                    std::array::from_fn::<_, 3, _>(|a| (p[0][a] + p[1][a] + p[2][a]) / 3.);
                assert!(
                    (0..3)
                        .filter(|a| *a != axis as usize)
                        .any(|a| centroid[a] <= 3. || centroid[a] >= 7.)
                );
            }
        }
    }
}
#[test]
fn open_input_remains_uncapped_with_unavailable_measurements() {
    let mut mesh = cube([0.; 3], 10.);
    mesh.triangles.pop();
    let s = scene(mesh);
    let mut f = frame(&s);
    f.section = Some(cut(2, 5., false));
    let r = realize_frame(&s, &f).unwrap();
    near(area_at(&r, 2, 5.), 0.);
    let a = r.analysis.as_ref().unwrap();
    assert!(!a.cap_supported);
    assert_eq!(a.legacy["capped"], false);
    for field in [
        "cross_section_area_mm2",
        "original_volume_mm3",
        "remaining_volume_mm3",
    ] {
        assert!(a.legacy[field].is_null());
    }
    assert!(!a.legacy["warnings"].as_array().unwrap().is_empty());
}
#[test]
fn exact_stl_corner_duplicates_are_welded_without_merging_nearby_shells() {
    let input = cube([0.; 3], 10.);
    let mut mesh = Mesh::default();
    for t in input.triangles {
        let start = mesh.positions.len() as u32;
        mesh.positions
            .extend(t.map(|i| input.positions[i as usize]));
        mesh.triangles.push([start, start + 1, start + 2]);
    }
    let s = scene(mesh);
    let mut f = frame(&s);
    f.section = Some(cut(0, 5., false));
    let r = realize_frame(&s, &f).unwrap();
    near(
        r.analysis.as_ref().unwrap().legacy["remaining_volume_mm3"]
            .as_f64()
            .unwrap(),
        500.,
    );
}
#[test]
fn adaptive_original_build_stages_are_relative_to_source_minimum() {
    let mut s = scene(cube([-20., -30., -40.], 10.));
    s.metadata.solid_layer_tops_mm = vec![0.3, 2.7, 10.];
    for (count, expected) in [
        (0, None),
        (1, Some(-39.7)),
        (2, Some(-37.3)),
        (3, Some(-30.)),
    ] {
        let mut f = frame(&s);
        f.visible_layers = Some(count);
        let r = realize_frame(&s, &f).unwrap();
        assert_eq!(r.reference_bounds_mm.min, [-20., -30., -40.]);
        if let Some(top) = expected {
            near(r.mesh.bounds().unwrap().max[2], top);
            near(area_at(&r, 2, top), 100.);
        } else {
            assert!(r.mesh.positions.is_empty());
            assert!(r.normals.is_empty());
        }
    }
    let mut f = frame(&s);
    f.visible_layers = Some(4);
    assert!(realize_frame(&s, &f).is_err());
}
#[test]
fn section_after_partial_build_preserves_build_cap() {
    let mut s = scene(cube([0.; 3], 10.));
    s.metadata.solid_layer_tops_mm = vec![5., 10.];
    let mut f = frame(&s);
    f.visible_layers = Some(1);
    f.section = Some(cut(0, 5., false));
    let r = realize_frame(&s, &f).unwrap();
    near(area_at(&r, 0, 5.), 50.);
    near(area_at(&r, 2, 5.), 50.);
    near(
        r.analysis.as_ref().unwrap().legacy["remaining_volume_mm3"]
            .as_f64()
            .unwrap(),
        250.,
    );
}
#[test]
fn stepped_layers_keep_independent_touching_interfaces_and_materials() {
    let mut mesh = cube([0.; 3], 2.);
    append(&mut mesh, cube([0., 0., 2.], 2.));
    let mut s = scene(mesh);
    s.metadata.kind = GeometryKind::Stepped;
    s.metadata.layer_ranges = vec![
        LayerRange {
            start_triangle: 0,
            end_triangle: 12,
            layer_index: 4,
            z_top_mm: 2.,
        },
        LayerRange {
            start_triangle: 12,
            end_triangle: 24,
            layer_index: 9,
            z_top_mm: 4.,
        },
    ];
    s.metadata.face_groups = vec![FaceGroup {
        start_triangle: 12,
        end_triangle: 24,
        tool: 2,
        label: "second".into(),
        color: [1., 0., 0.],
    }];
    let mut f = frame(&s);
    f.visible_layers = Some(1);
    f.section = Some(cut(0, 1., false));
    let r = realize_frame(&s, &f).unwrap();
    near(r.mesh.bounds().unwrap().max[2], 2.);
    near(
        r.analysis.as_ref().unwrap().legacy["remaining_volume_mm3"]
            .as_f64()
            .unwrap(),
        4.,
    );
    assert!(!r.triangle_materials.contains(&1));
    f.visible_layers = None;
    let r = realize_frame(&s, &f).unwrap();
    near(
        r.analysis.as_ref().unwrap().legacy["remaining_volume_mm3"]
            .as_f64()
            .unwrap(),
        8.,
    );
    assert!(r.triangle_materials.contains(&1));
    near(area_at(&r, 2, 2.), 4.);
}
#[test]
fn disconnected_bead_strands_keep_a_submicron_gap_without_envelope_fill() {
    let mut mesh = cube([0.; 3], 1.);
    append(&mut mesh, cube([0., 1.0000001, 0.], 1.));
    let mut s = scene(mesh);
    s.metadata.kind = GeometryKind::Beads;
    let mut f = frame(&s);
    f.section = Some(cut(0, 0.5, false));
    let r = realize_frame(&s, &f).unwrap();
    near(area_at(&r, 0, 0.5), 2.);
    for t in &r.mesh.triangles {
        let p = t.map(|i| r.mesh.positions[i as usize]);
        let min = p.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
        let max = p.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max);
        assert!(max <= 1. || min >= 1.0000001);
    }
    assert!(
        r.analysis.as_ref().unwrap().legacy["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x.as_str().unwrap().contains("overlap"))
    );
}
#[test]
fn large_negative_coordinates_obey_analytic_area_and_volume() {
    let s = scene(cube([-1_000_000.; 3], 10.));
    let mut f = frame(&s);
    f.section = Some(cut(1, -999_995., true));
    let r = realize_frame(&s, &f).unwrap();
    near(area_at(&r, 1, -999_995.), 100.);
    near(
        r.analysis.as_ref().unwrap().legacy["remaining_volume_mm3"]
            .as_f64()
            .unwrap(),
        500.,
    );
}
#[test]
fn outside_sections_are_empty_or_complete_and_boundary_faces_are_not_duplicated() {
    let s = scene(cube([0.; 3], 10.));
    for (position, empty) in [(-1., true), (0., true), (10., false), (11., false)] {
        let mut f = frame(&s);
        f.section = Some(cut(2, position, false));
        let r = realize_frame(&s, &f).unwrap();
        if empty {
            assert!(r.mesh.triangles.is_empty());
        } else {
            assert_eq!(r.mesh.triangles.len(), 12);
            near(area_at(&r, 2, 10.), 100.);
        }
    }
}
#[test]
fn display_transform_uses_full_source_floor_center_and_build_coordinates() {
    let mut s = scene(cube([-5., -3., -8.], 10.));
    s.appearance.display_rotation_degrees = [30., 20., 40.];
    let transform = resolve_display_transform(&s, 70.).unwrap();
    let mapped: Vec<[f64; 3]> = s
        .mesh
        .positions
        .iter()
        .map(|p| {
            std::array::from_fn(|i| {
                transform[i][3] + (0..3).map(|j| transform[i][j] * p[j]).sum::<f64>()
            })
        })
        .collect();
    near(
        mapped.iter().map(|p| p[2]).fold(f64::INFINITY, f64::min),
        0.,
    );
    s.metadata.solid_layer_tops_mm = vec![1., 10.];
    let mut f = frame(&s);
    f.visible_layers = Some(1);
    f.angle_degrees = 70.;
    let r = realize_frame(&s, &f).unwrap();
    assert_eq!(
        resolve_display_transform(&s, f.angle_degrees).unwrap(),
        transform
    );
    assert_eq!(r.reference_bounds_mm.max, s.reference_bounds_mm.max);
    for (p, b) in r.mesh.positions.iter().zip(r.build_positions_mm) {
        assert_eq!(b, p.map(|v| v as f32));
    }
}
#[test]
fn physical_layer_pitch_and_normal_gradient_use_build_z() {
    let mut s = scene(cube([0.; 3], 10.));
    s.appearance.layer_appearance = true;
    s.appearance.first_layer_height_mm = 0.3;
    s.appearance.layer_height_mm = 0.2;
    s.appearance.groove_depth_mm = 0.01;
    let a = layer_surface_normal(&s.appearance, 0.35, [1., 0., 0.], 1.).unwrap();
    let b = layer_surface_normal(&s.appearance, 0.55, [1., 0., 0.], 1.).unwrap();
    for i in 0..3 {
        near(a[i], b[i]);
    }
    assert!(a[2] < 0.);
    let flat = layer_surface_normal(&s.appearance, 0.35, [0., 0., 1.], 1.).unwrap();
    assert_eq!(flat, [0., 0., 1.]);
    s.appearance.display_rotation_degrees = [90., 45., 32.];
    assert_eq!(
        a,
        layer_surface_normal(&s.appearance, 0.35, [1., 0., 0.], 1.).unwrap()
    );
}
#[test]
fn derived_allocation_counts_are_checked_before_expansion() {
    assert!(check_derived_geometry_budget(usize::MAX, 1).is_err());
    assert!(check_derived_geometry_budget(2_000_001, 1).is_err());
    assert!(check_derived_geometry_budget(1, 4_000_001).is_err());
    assert!(check_derived_geometry_budget(0, 0).is_ok());
    let mut mesh = Mesh::default();
    for _ in 0..166_667 {
        let n = mesh.positions.len() as u32;
        mesh.positions
            .extend([[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]);
        mesh.triangles.extend([
            [n, n + 2, n + 1],
            [n, n + 1, n + 3],
            [n, n + 3, n + 2],
            [n + 1, n + 2, n + 3],
        ]);
    }
    let s = scene(mesh);
    let before = s.mesh.snapshot_hash().unwrap();
    let error = realize_frame(&s, &frame(&s)).unwrap_err().to_string();
    assert!(error.contains("derived render capacity budget"), "{error}");
    assert_eq!(before, s.mesh.snapshot_hash().unwrap());
}

#[test]
fn coincident_build_and_section_caps_are_not_duplicated() {
    let mut s = scene(cube([0.; 3], 10.));
    s.metadata.solid_layer_tops_mm = vec![5., 10.];
    let mut f = frame(&s);
    f.visible_layers = Some(1);
    f.section = Some(cut(2, 5., false));
    let r = realize_frame(&s, &f).unwrap();
    near(area_at(&r, 2, 5.), 100.);
    near(
        r.analysis.as_ref().unwrap().legacy["remaining_volume_mm3"]
            .as_f64()
            .unwrap(),
        500.,
    );
}
#[test]
fn zero_layers_always_means_empty_and_preserves_legacy_empty_analysis() {
    let s = scene(cube([0.; 3], 10.));
    let mut f = frame(&s);
    f.visible_layers = Some(0);
    f.section = Some(cut(0, 5., false));
    let r = realize_frame(&s, &f).unwrap();
    assert!(r.mesh.positions.is_empty());
    let a = r.analysis.unwrap();
    assert_eq!(a.legacy["cross_section_area_mm2"], 0.);
    assert_eq!(a.legacy["original_volume_mm3"], 0.);
    assert_eq!(a.legacy["remaining_volume_mm3"], 0.);
    assert_eq!(a.legacy["visible_layer"], -1);
    assert_eq!(a.legacy["visible_source_triangles"], 0);
    assert!(a.legacy.get("volume_basis").is_none());
}
#[test]
fn clipped_geometry_that_collapses_at_adapter_precision_is_rejected() {
    let s = scene(cube([1_000_000.; 3], 10.));
    let mut f = frame(&s);
    f.section = Some(cut(0, 1_000_000.01, false));
    assert!(
        realize_frame(&s, &f)
            .unwrap_err()
            .to_string()
            .contains("precision")
    );
}
#[test]
fn real_independent_bead_mesh_has_smooth_sides_flat_ends_and_preserved_gap() {
    use turninator::print::beads::{BeadSegment, build_bead_mesh};
    let bead = |y| BeadSegment {
        start: [0., y, 0.2],
        end: [10., y, 0.2],
        width_mm: 0.42,
        height_mm: 0.2,
        volume_mm3: 0.,
        source_line: 1,
        feature: "wall".into(),
        tool: 0,
        layer_index: 0,
        width_source: "fixture".into(),
        height_source: "fixture".into(),
    };
    let mesh = build_bead_mesh(&[bead(0.), bead(1.)]).unwrap();
    let mut s = scene(mesh);
    s.metadata.kind = GeometryKind::Beads;
    s.metadata.bead_layout = Some("independent_stadium_v1".into());
    let f = frame(&s);
    let r = realize_frame(&s, &f).unwrap();
    let mut nonaxis = false;
    for (source, t) in s.mesh.triangles.iter().zip(&r.mesh.triangles) {
        let index = r.mesh.triangles.iter().position(|x| x == t).unwrap();
        let n = r.normals[t[0] as usize];
        if index % 4 < 2 {
            nonaxis |= n[1].abs() > 0.01 && n[2].abs() > 0.01;
        } else {
            near(n[0].abs() as f64, 1.);
        }
        assert_eq!(
            r.mesh.positions[t[0] as usize],
            s.mesh.positions[source[0] as usize]
        );
    }
    assert!(nonaxis);
    let mut f = f;
    f.section = Some(cut(0, 5., false));
    let r = realize_frame(&s, &f).unwrap();
    assert!(r.analysis.as_ref().unwrap().cap_supported);
    for t in &r.mesh.triangles {
        let p = t.map(|i| r.mesh.positions[i as usize]);
        assert!(p.iter().all(|p| p[1] <= 0.21) || p.iter().all(|p| p[1] >= 0.79));
    }
}
#[test]
fn vertex_indexing_reuses_flat_face_corners_but_preserves_cube_creases() {
    let s = scene(cube([0.; 3], 10.));
    let r = realize_frame(&s, &frame(&s)).unwrap();
    assert_eq!(
        r.mesh.positions.len(),
        24,
        "four corners per flat cube face, with six independent face normals"
    );
    let corner: Vec<_> = r
        .mesh
        .positions
        .iter()
        .zip(&r.normals)
        .filter(|(p, _)| **p == [0.; 3])
        .map(|(_, n)| *n)
        .collect();
    assert_eq!(corner.len(), 3);
    assert!(corner.contains(&[-1., 0., 0.]));
    assert!(corner.contains(&[0., -1., 0.]));
    assert!(corner.contains(&[0., 0., -1.]));
}
#[test]
fn declared_legacy_bead_smoothing_rejects_incompatible_indices() {
    let mut s = scene(cube([0.; 3], 10.));
    s.metadata.kind = GeometryKind::Beads;
    s.metadata.bead_layout = Some("independent_stadium_v1".into());
    assert!(realize_frame(&s, &frame(&s)).is_err());
}
#[test]
fn build_zero_filter_and_layer_height_do_not_change_physical_pitch() {
    let s = scene(cube([0.; 3], 10.));
    assert_eq!(layer_filter_gain(0.2, 0.01, 1.).unwrap(), 1.);
    assert_eq!(layer_filter_gain(0.2, 0.2, 1.).unwrap(), 0.);
    assert_eq!(layer_filter_gain(0.2, 0.01, 0.).unwrap(), 0.);
    assert!(layer_filter_gain(0., 0.01, 1.).is_err());
    assert!(layer_filter_gain(0.2, -0.01, 1.).is_err());
    let n = layer_surface_normal(&s.appearance, 0.35, [1., 0., 0.], 0.).unwrap();
    assert_eq!(n, [1., 0., 0.]);
}
#[test]
fn joined_ring_layout_validates_declared_quad_pairs() {
    let mut s = scene(cube([0.; 3], 10.));
    s.metadata.kind = GeometryKind::Beads;
    s.metadata.bead_layout = Some("joined_stadium_v2".into());
    s.metadata.bead_ring_vertices = Some(8);
    s.metadata.bead_side_face_ranges = vec![[4, 12]];
    assert!(realize_frame(&s, &frame(&s)).is_err());
}
#[test]
fn frozen_diagnostic_meshes_and_masks_preserve_closed_hole_geometry() {
    use std::path::Path;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/renderers");
    let config: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/renderers/conformance.json")).unwrap();
    for (name, pixels, volume, area) in
        [("cube", 102400, 1000., 100.), ("hollow", 86016, 840., 84.)]
    {
        let mesh = Mesh::read_binary(&root.join(format!("{name}-10mm.mesh.bin"))).unwrap();
        let s = scene(mesh);
        let mut f = frame(&s);
        f.section = Some(cut(1, 5., false));
        let r = realize_frame(&s, &f).unwrap();
        near(
            r.analysis.as_ref().unwrap().legacy["original_volume_mm3"]
                .as_f64()
                .unwrap(),
            volume,
        );
        near(area_at(&r, 1, 5.), area);
        let mask = image::open(root.join(format!("{name}-mask-512.png")))
            .unwrap()
            .to_luma8();
        assert_eq!(mask.dimensions(), (512, 512));
        assert_eq!(mask.as_raw().iter().filter(|&&p| p == 255).count(), pixels);
        assert_eq!(
            mask.get_pixel(256, 256)[0],
            if name == "hollow" { 0 } else { 255 }
        );
    }
    let value = 1_000_000_f32;
    assert_eq!(f32::from_bits(value.to_bits() + 1) - value, 0.0625);
    assert_eq!(
        config["geometry"]["large_coordinate_fixture"]["f32_ulp_mm"],
        0.0625
    );
}
#[test]
fn single_frames_do_not_require_sheet_design_edge_capacity() {
    let mut mesh = Mesh::default();
    for i in 0..66_667 {
        let x = i as f64 * 2.;
        let n = mesh.positions.len() as u32;
        mesh.positions
            .extend([[x, 0., 0.], [x + 1., 0., 0.], [x, 1., 0.]]);
        mesh.triangles.push([n, n + 1, n + 2]);
    }
    let s = scene(mesh);
    let f = frame(&s);
    let r = realize_frame(&s, &f).unwrap();
    assert!(r.design_edges_mm.is_empty());
    assert_eq!(r.mesh.triangles.len(), 66_667);
    let mut sheet = f;
    sheet.key.panel = RenderPanel::Top;
    assert!(
        realize_frame(&s, &sheet)
            .unwrap_err()
            .to_string()
            .contains("design-edge budget")
    );
}
