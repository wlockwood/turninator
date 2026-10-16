#[path = "fixtures/renderers/common.rs"]
mod common;
use common::*;
use turninator::render::{contract::*, geometry::*, materials::resolve_materials};

#[test]
fn sheet_ink_revision_tracks_projection_but_static_turntables_reuse_geometry() {
    use turninator::render::prepared::frame_geometry_key;
    let s = scene(cube([0.; 3], 10.));
    let f = frame(&s);
    let mut moved = f.clone();
    moved.angle_degrees = 120.;
    moved.camera.position_mm[0] += 1.;
    assert_eq!(
        frame_geometry_key(&f, true).unwrap(),
        frame_geometry_key(&moved, true).unwrap()
    );
    let mut sheet = f;
    sheet.key.panel = RenderPanel::Top;
    let mut changed = sheet.clone();
    changed.width += 1;
    assert_ne!(
        frame_geometry_key(&sheet, true).unwrap(),
        frame_geometry_key(&changed, true).unwrap()
    );
    assert_eq!(
        frame_geometry_key(&sheet, false).unwrap(),
        frame_geometry_key(&changed, false).unwrap()
    );
}

#[test]
fn ink_is_optional_serialization_and_expands_with_depth_for_occlusion() {
    let s = scene(cube([0.; 3], 10.));
    let mut f = frame(&s);
    f.key.panel = RenderPanel::Front;
    f.camera = RenderCamera {
        position_mm: [0., -30., 5.],
        target_mm: [0., 0., 5.],
        up: [0., 0., 1.],
        projection: RenderProjection::Orthographic {
            vertical_span_mm: 16.,
        },
    };
    let mut r = realize_frame(&s, &f).unwrap();
    let base = r.mesh.triangles.len();
    let positions = r.mesh.positions.clone();
    let mut m = resolve_materials(&s).unwrap();
    assert!(
        serde_json::to_value(&m.materials[0])
            .unwrap()
            .get("unlit")
            .is_none()
    );
    expand_design_edges(&s, &f, &mut r, &mut m, &RenderLimits::default()).unwrap();
    assert!(m.materials.last().unwrap().unlit);
    assert_eq!(r.mesh.triangles.len(), base + 12 * 12);
    assert_eq!(&r.mesh.positions[..positions.len()], positions.as_slice());
    assert!(r.design_edges_mm.is_empty());
    assert_eq!(r.triangle_materials.len(), r.mesh.triangles.len());
    assert!(
        r.triangle_materials[base..]
            .iter()
            .all(|&i| i as usize == m.materials.len() - 1)
    );
    assert_eq!(r.normals.len(), r.mesh.positions.len());
    assert_eq!(r.build_positions_mm.len(), r.mesh.positions.len());
    // Ink retains each source edge's physical depth; back edges stay behind the
    // source solid and therefore participate in normal depth testing/occlusion.
    let ink = &r.mesh.positions[positions.len()..];
    assert!(ink.iter().any(|p| p[1] < 0.));
    assert!(ink.iter().any(|p| p[1] > 10.));
}

#[test]
fn ink_width_is_point_eight_pixels_and_budget_failure_is_atomic() {
    let mut s = scene(cube([0.; 3], 10.));
    s.appearance.display_rotation_degrees = [53., 19., 37.];
    let mut f = frame(&s);
    f.key.panel = RenderPanel::Iso;
    f.angle_degrees = 28.;
    f.camera = RenderCamera {
        position_mm: [0., -30., 5.],
        target_mm: [0., 0., 5.],
        up: [0., 0., 1.],
        projection: RenderProjection::Orthographic {
            vertical_span_mm: 16.,
        },
    };
    let transform = resolve_display_transform(&s, f.angle_degrees).unwrap();
    let mut r = realize_frame(&s, &f).unwrap();
    let start = r.mesh.positions.len();
    let edges = r.design_edges_mm.clone();
    let mut m = resolve_materials(&s).unwrap();
    let tiny = RenderLimits {
        gpu_bytes: 1,
        ..Default::default()
    };
    assert!(expand_design_edges(&s, &f, &mut r, &mut m, &tiny).is_err());
    assert_eq!(r.mesh.positions.len(), start);
    assert_eq!(r.design_edges_mm, edges);
    let source_quotas = RenderLimits {
        max_source_vertices: 1,
        max_source_triangles: 1,
        ..Default::default()
    };
    expand_design_edges(&s, &f, &mut r, &mut m, &source_quotas).unwrap();
    for (edge, vertices) in edges.iter().zip(r.mesh.positions[start..].chunks_exact(8)) {
        let display = |p: [f64; 3]| -> [f64; 3] {
            std::array::from_fn(|i| {
                transform[i][3] + (0..3).map(|j| transform[i][j] * p[j]).sum::<f64>()
            })
        };
        let a = display(edge[0].map(f64::from));
        let b = display(edge[1].map(f64::from));
        let d = [b[0] - a[0], b[2] - a[2]];
        let len = d[0].hypot(d[1]);
        if len < 1e-8 {
            continue;
        }
        let values: Vec<_> = vertices
            .iter()
            .map(|&p| {
                let p = display(p);
                (-d[1] * p[0] + d[0] * p[2]) / len
            })
            .collect();
        let width = values.iter().copied().fold(f64::NEG_INFINITY, f64::max)
            - values.iter().copied().fold(f64::INFINITY, f64::min);
        near(width / 16. * 512., 0.8);
    }
}
