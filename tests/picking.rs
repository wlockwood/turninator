use turninator::{
    mesh::Mesh,
    model::orientation::{rotation_matrix, settled_display_rotation},
    viewport::{CanonicalView, OrbitCamera, PickingMesh, SectionPlane, ViewportSettings},
};

fn cube() -> Mesh {
    Mesh {
        positions: vec![
            [10., 20., 30.],
            [14., 20., 30.],
            [14., 24., 30.],
            [10., 24., 30.],
            [10., 20., 34.],
            [14., 20., 34.],
            [14., 24., 34.],
            [10., 24., 34.],
        ],
        triangles: vec![
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
        ],
    }
}
fn apply(matrix: [[f64; 3]; 3], vector: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| (0..3).map(|j| matrix[i][j] * vector[j]).sum())
}
fn close(a: [f64; 3], b: [f64; 3]) {
    for i in 0..3 {
        assert!((a[i] - b[i]).abs() < 1e-8, "{a:?} != {b:?}");
    }
}

#[test]
fn nearest_face_uses_build_coordinates_for_both_projections_and_shared_edges() {
    let picker = PickingMesh::new(&cube()).unwrap();
    let mut camera = OrbitCamera::default();
    camera.canonical(CanonicalView::Front);
    camera.target_mm = [0., 0., 2.];
    camera.distance_mm = 20.;
    camera.ortho_span_mm = 10.;
    for orthographic in [false, true] {
        camera.orthographic = orthographic;
        let hit = picker
            .pick(&camera, &ViewportSettings::default(), [0., 0.], 1.5)
            .unwrap();
        assert!([4, 5].contains(&hit.triangle_index));
        close(hit.normal_mm, [0., -1., 0.]);
        close(hit.point_mm, [12., 20., 32.]);
        assert!(
            picker
                .pick(&camera, &ViewportSettings::default(), [0.99, 0.99], 1.5)
                .is_none()
        );
    }
}

#[test]
fn discarded_surfaces_do_not_occlude_retained_faces() {
    let mut picker = PickingMesh::new(&cube()).unwrap();
    let mut camera = OrbitCamera::default();
    camera.canonical(CanonicalView::Front);
    camera.target_mm = [0., 0., 2.];
    camera.distance_mm = 20.;
    let mut settings = ViewportSettings {
        section: Some(SectionPlane {
            axis: 1,
            position_mm: 22.,
            keep_positive: true,
        }),
        ..Default::default()
    };
    let hit = picker.pick(&camera, &settings, [0., 0.], 1.).unwrap();
    close(hit.point_mm, [12., 24., 32.]);
    close(hit.normal_mm, [0., 1., 0.]);
    settings.section = None;
    settings.buildup_height_mm = Some(31.);
    assert!(picker.pick(&camera, &settings, [0., 0.], 1.).is_none());
    settings.buildup_height_mm = Some(33.);
    picker.set_triangle_layer_tops(&[34.; 12]).unwrap();
    assert!(picker.pick(&camera, &settings, [0., 0.], 1.).is_none());
    assert!(picker.set_triangle_layer_tops(&[f32::NAN; 12]).is_err());
    assert!(picker.pick(&camera, &settings, [0., 0.], 1.).is_none());
    picker.set_triangle_layer_tops(&[]).unwrap();
    assert!(picker.pick(&camera, &settings, [0., 0.], 1.).is_some());
}

#[test]
fn displayed_pose_spin_and_camera_pan_pick_the_visible_surface() {
    let picker = PickingMesh::new(&cube()).unwrap();
    let settings = ViewportSettings {
        display_rotation_degrees: [0., 0., 90.],
        turntable_angle_degrees: 90.,
        ..Default::default()
    };
    let mut camera = OrbitCamera::default();
    camera.canonical(CanonicalView::Front);
    camera.target_mm = [0., 0., 2.];
    camera.distance_mm = 20.;
    camera.orthographic = true;
    camera.ortho_span_mm = 10.;
    let hit = picker.pick(&camera, &settings, [0., 0.], 1.).unwrap();
    close(hit.normal_mm, [0., 1., 0.]);
    assert!((hit.point_mm[1] - 24.).abs() < 1e-6);
    camera.pan(10., -10., 100.);
    let hit = picker.pick(&camera, &settings, [0., 0.], 1.).unwrap();
    assert!((hit.point_mm[0] - 13.).abs() < 1e-5 && (hit.point_mm[2] - 31.).abs() < 1e-5);
}

#[test]
fn settle_composes_xyz_rotation_through_antiparallel_and_gimbal_cases() {
    for previous in [
        [0.; 3],
        [23., -71., 142.],
        [0., 90., 30.],
        [0., -90., 30.],
        [1e5, -1e5, 1e5],
    ] {
        for normal in [
            [0., 0., 1.],
            [0., 0., -1.],
            [1., 0., 0.],
            [0., 1., 0.],
            [1e-12, 0., 1.],
            [0.2672612419124244, 0.5345224838248488, 0.8017837257372732],
        ] {
            let next = settled_display_rotation(previous, normal).unwrap();
            close(apply(rotation_matrix(next).unwrap(), normal), [0., 0., -1.]);
            assert!(next.iter().all(|v| v.is_finite()));
        }
    }
    let previous = [24., 45., -30.];
    let matrix = rotation_matrix(previous).unwrap();
    let down = std::array::from_fn(|i| -matrix[2][i]);
    let next = rotation_matrix(settled_display_rotation(previous, down).unwrap()).unwrap();
    for i in 0..3 {
        close(next[i], matrix[i]);
    }
    assert!(settled_display_rotation([0.; 3], [0.; 3]).is_err());
    assert!(settled_display_rotation([f64::NAN, 0., 0.], [0., 0., 1.]).is_err());
}

#[test]
fn invalid_rays_and_mesh_budget_fail_without_guessing() {
    let picker = PickingMesh::new(&cube()).unwrap();
    let camera = OrbitCamera::default();
    let settings = ViewportSettings::default();
    for (ndc, aspect) in [([f64::NAN, 0.], 1.), ([0., 0.], 0.), ([2., 0.], 1.)] {
        assert!(picker.pick(&camera, &settings, ndc, aspect).is_none());
    }
    let mut broken = cube();
    broken.positions[0][0] = f64::INFINITY;
    assert!(PickingMesh::new(&broken).is_err());
    let oversized = Mesh {
        positions: vec![[0.; 3]; 3],
        triangles: vec![[0, 1, 2]; turninator::mesh::MAX_TRIANGLES + 1],
    };
    assert!(PickingMesh::new(&oversized).is_err());
}

#[test]
fn arbitrary_xyz_pose_uses_actual_centered_and_floored_vertices() {
    let mesh = cube();
    let picker = PickingMesh::new(&mesh).unwrap();
    for pose in [[37., -51., 123.], [-89., 90., -170.], [180., 0., 72.]] {
        let rotation = rotation_matrix(pose).unwrap();
        let points: Vec<_> = mesh.positions.iter().map(|&p| apply(rotation, p)).collect();
        let min: [f64; 3] =
            std::array::from_fn(|i| points.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min));
        let max: [f64; 3] = std::array::from_fn(|i| {
            points
                .iter()
                .map(|p| p[i])
                .fold(f64::NEG_INFINITY, f64::max)
        });
        let shift = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5, min[2]];
        let spin = rotation_matrix([0., 0., 57.]).unwrap();
        let point = apply(rotation, [12., 20., 32.]);
        let displayed = apply(spin, std::array::from_fn(|i| point[i] - shift[i]));
        let normal = apply(spin, apply(rotation, [0., -1., 0.]));
        let mut camera = OrbitCamera {
            yaw_degrees: normal[0].atan2(-normal[1]).to_degrees(),
            pitch_degrees: normal[2].asin().to_degrees(),
            target_mm: displayed,
            distance_mm: 20.,
            ..Default::default()
        };
        let settings = ViewportSettings {
            display_rotation_degrees: pose.map(|v| v as f32),
            turntable_angle_degrees: 57.,
            ..Default::default()
        };
        for orthographic in [false, true] {
            camera.orthographic = orthographic;
            let hit = picker.pick(&camera, &settings, [0., 0.], 0.55).unwrap();
            close(hit.normal_mm, [0., -1., 0.]);
            for (actual, expected) in hit.point_mm.into_iter().zip([12., 20., 32.]) {
                assert!((actual - expected).abs() < 2e-5);
            }
        }
    }
}

#[test]
fn pointer_coordinates_follow_screen_axes_and_reject_outside_clicks() {
    use eframe::egui::{Rect, pos2};
    use turninator::viewport::picking::pointer_coordinates;
    let rect = Rect::from_min_max(pos2(100., 50.), pos2(300., 150.));
    assert_eq!(
        pointer_coordinates(rect, pos2(100., 50.)),
        Some(([-1., 1.], 2.))
    );
    assert_eq!(
        pointer_coordinates(rect, pos2(300., 150.)),
        Some(([1., -1.], 2.))
    );
    assert_eq!(
        pointer_coordinates(rect, rect.center()),
        Some(([0., 0.], 2.))
    );
    assert!(pointer_coordinates(rect, pos2(99., 50.)).is_none());
    assert!(pointer_coordinates(Rect::NOTHING, pos2(0., 0.)).is_none());
}
