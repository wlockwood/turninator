use turninator::{
    animation::{CameraKeyframe, Easing, Timeline, Transition},
    viewport::OrbitCamera,
};

fn key(time: f64, yaw: f64, pitch: f64) -> CameraKeyframe {
    CameraKeyframe {
        time_seconds: time,
        camera: OrbitCamera {
            yaw_degrees: yaw,
            pitch_degrees: pitch,
            ..Default::default()
        },
        ..Default::default()
    }
}
#[test]
fn timeline_shortest_orbit_pan_and_endpoints_are_continuous() {
    let mut end = key(2., 10., 90.);
    end.camera.target_mm = [10., 20., 30.];
    end.camera.distance_mm = 200.;
    let timeline = Timeline {
        duration_seconds: 2.,
        keyframes: vec![key(0., 350., 0.), end],
        ..Default::default()
    };
    let halfway = timeline.sample(1.).unwrap().camera;
    assert!((halfway.yaw_degrees - 360.).abs() < 1e-9);
    assert_eq!(halfway.target_mm, [5., 10., 15.]);
    assert_eq!(halfway.pitch_degrees, 45.);
    assert!(halfway.distance_mm > 100. && halfway.distance_mm < 200.);
    assert_eq!(timeline.frame_count().unwrap(), 40);
    assert_eq!(timeline.sample_frame(39).unwrap().camera.pitch_degrees, 90.);
    assert!(timeline.sample_frame(40).is_err());
    for i in 0..40 {
        timeline.sample_frame(i).unwrap().camera.validate().unwrap();
    }
}
#[test]
fn spiral_approaches_pole_while_preserving_requested_turns_and_smooth_easing() {
    let mut first = key(0., 0., 0.);
    first.transition = Transition::Spiral;
    first.spiral_turns = 2.;
    first.easing = Easing::SmoothStep;
    let timeline = Timeline {
        duration_seconds: 4.,
        keyframes: vec![first, key(4., 0., 90.)],
        ..Default::default()
    };
    assert_eq!(timeline.sample(2.).unwrap().camera.yaw_degrees, 360.);
    assert_eq!(timeline.sample(2.).unwrap().camera.pitch_degrees, 45.);
    assert!(timeline.sample(0.01).unwrap().camera.yaw_degrees < 0.02);
    let last = timeline.sample_frame(79).unwrap().camera;
    assert!((last.camera_spec()["direction"][2].as_f64().unwrap() - 1.).abs() < 1e-9);
}
#[test]
fn loops_close_without_duplicate_frame_and_buildup_finishes() {
    let timeline = Timeline {
        duration_seconds: 2.,
        looping: true,
        turntable: true,
        buildup: true,
        keyframes: vec![key(0., 0., 0.), key(1., 90., 30.)],
    };
    let last = timeline.sample_frame(39).unwrap();
    assert!(last.turntable_degrees < 360.);
    assert_eq!(last.buildup_fraction, Some(1.));
    assert!(timeline.sample(1.999999).unwrap().camera.yaw_degrees.abs() < 0.001);
    assert_eq!(timeline.sample(2.).unwrap().camera.yaw_degrees, 0.);
}
#[test]
fn invalid_timeline_cannot_create_nan_discontinuities_or_excess_frames() {
    let base = Timeline {
        keyframes: vec![key(0., 0., 0.), key(4., 90., 0.)],
        ..Default::default()
    };
    for duration in [f64::NAN, 0., 501.] {
        let mut t = base.clone();
        t.duration_seconds = duration;
        assert!(t.validate().is_err());
    }
    let mut t = base.clone();
    t.keyframes[1].time_seconds = 0.;
    assert!(t.validate().is_err());
    let mut t = base.clone();
    t.keyframes[1].camera.orthographic = true;
    assert!(t.validate().is_err());
    let mut t = base.clone();
    t.looping = true;
    assert!(t.validate().is_err());
    let mut t = base.clone();
    t.keyframes[0].spiral_turns = f64::INFINITY;
    assert!(t.validate().is_err());
    let mut t = base.clone();
    t.keyframes[0].spiral_turns = 0.5;
    assert!(
        t.validate().is_err(),
        "fractional extra turns would jump at the endpoint"
    );
    assert!(base.sample(f64::NAN).is_err());
    let saved = serde_json::to_vec(&base).unwrap();
    let restored: Timeline = serde_json::from_slice(&saved).unwrap();
    assert_eq!(restored.frame_count().unwrap(), 80);
}
