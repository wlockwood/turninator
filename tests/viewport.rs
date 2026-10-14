use turninator::{
    mesh::Bounds,
    viewport::{CanonicalView, OrbitCamera},
};

#[test]
fn canonical_views_have_agreed_axes() {
    let mut camera = OrbitCamera::default();
    for (view, direction, up) in [
        (CanonicalView::Front, [0., -1., 0.], [0., 0., 1.]),
        (CanonicalView::Right, [1., 0., 0.], [0., 0., 1.]),
        (CanonicalView::Top, [0., 0., 1.], [0., 1., 0.]),
    ] {
        camera.canonical(view);
        let spec = camera.camera_spec();
        for i in 0..3 {
            assert!((spec["direction"][i].as_f64().unwrap() - direction[i]).abs() < 1e-8);
            assert!((spec["up"][i].as_f64().unwrap() - up[i]).abs() < 1e-8);
        }
    }
}

#[test]
fn fit_contains_tall_elongated_model_through_full_rotation() {
    let mut camera = OrbitCamera::default();
    camera.fit(&Bounds {
        min: [-30., -2., 0.],
        max: [30., 2., 80.],
    });
    for ortho in [false, true] {
        camera.orthographic = ortho;
        for angle in 0..360 {
            let a = (angle as f64).to_radians();
            for x in [-30., 30.] {
                for y in [-2., 2.] {
                    for z in [0., 80.] {
                        let p = [x * a.cos() - y * a.sin(), x * a.sin() + y * a.cos(), z];
                        let q = camera.project(p, 1.).unwrap();
                        assert!(
                            q[0].abs() < 0.92 && q[1].abs() < 0.92 && q[2] > 0. && q[2] < 1.,
                            "clipped {q:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn invalid_camera_state_and_controls_stay_finite() {
    let mut camera = OrbitCamera {
        distance_mm: f64::NAN,
        ..Default::default()
    };
    assert!(camera.validate().is_err());
    camera = OrbitCamera::default();
    camera.vertical_fov_degrees = 180.;
    assert!(camera.validate().is_err());
    camera = OrbitCamera::default();
    camera.orbit(1e6, 1e6);
    camera.zoom(1e9);
    camera.pan(1., 2., 400.);
    assert!(camera.validate().is_ok());
    assert!(camera.project([0., 0., 0.], 0.).is_err());
}

#[test]
fn layer_settings_follow_the_physical_scene_contract() {
    use turninator::viewport::ViewportSettings;
    let mut settings = ViewportSettings::default();
    assert!(settings.validate().is_ok());
    settings.groove_depth_mm = 0.051;
    assert!(settings.validate().is_err());
    settings.groove_depth_mm = 0.01;
    settings.layer_height_mm = 0.001;
    assert!(settings.validate().is_err());
    settings.layer_height_mm = 0.2;
    settings.first_layer_height_mm = 3.;
    assert!(settings.validate().is_err());
}

#[test]
fn inspection_settings_reject_nonfinite_planes_and_out_of_range_axes() {
    use turninator::viewport::{SectionPlane, ViewportSettings};
    let mut settings = ViewportSettings {
        section: Some(SectionPlane {
            axis: 3,
            position_mm: 1.,
            keep_positive: true,
        }),
        ..Default::default()
    };
    assert!(settings.validate().is_err());
    settings.section = Some(SectionPlane {
        axis: 2,
        position_mm: f32::NAN,
        keep_positive: false,
    });
    assert!(settings.validate().is_err());
    settings.section = None;
    settings.buildup_height_mm = Some(-1.);
    assert!(settings.validate().is_err());
    settings.buildup_height_mm = None;
    settings.opacity = 1.1;
    assert!(settings.validate().is_err());
}
