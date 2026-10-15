use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use turninator::{
    animation::Timeline,
    export::BuildStage,
    presentation::SceneSettings,
    project::{ProjectFile, SourceStatus, load_project, save_project},
    viewport::OrbitCamera,
};

fn fixture() -> (PathBuf, Value) {
    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "turninator-project-timeline-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir_all(&root).unwrap();
    let source = root.join("part.stl");
    fs::write(&source, b"original source").unwrap();
    let project = ProjectFile {
        schema_version: 1,
        source_path: source,
        source_hash: format!("{:x}", Sha256::digest(b"original source")),
        scene: SceneSettings::default(),
        camera: OrbitCamera::default(),
        export_size: 960,
        engine: "eevee".into(),
        output_options: Default::default(),
    };
    (root, serde_json::to_value(project).unwrap())
}

fn timeline() -> Value {
    let mut first = serde_json::to_value(OrbitCamera::default()).unwrap();
    first["yaw_degrees"] = json!(350.);
    first["pitch_degrees"] = json!(0.);
    let mut last = first.clone();
    last["yaw_degrees"] = json!(10.);
    last["pitch_degrees"] = json!(60.);
    last["target_mm"] = json!([10., 20., 30.]);
    last["distance_mm"] = json!(400.);
    json!({"duration_seconds":2.,"looping":false,"turntable":true,"buildup":true,
    "keyframes":[
        {"time_seconds":0.,"camera":first,"transition":"Spiral","easing":"SmoothStep","spiral_turns":1.},
        {"time_seconds":2.,"camera":last,"transition":"Orbit","easing":"Linear","spiral_turns":0.}
    ]})
}

#[test]
fn project_roundtrip_preserves_composed_timeline_and_playback() {
    let (root, mut value) = fixture();
    value["output_options"]["timeline"] = timeline();
    value["output_options"]["section"] =
        json!({"axis":0,"position_mm":0.75,"keep_positive":true,"cap":true});
    value["scene"]["display_rotation_degrees"] = json!([20., 15., 35.]);
    let project: ProjectFile = serde_json::from_value(value).unwrap();
    let path = root.join("camera.tnproj");
    save_project(&path, &project).unwrap();
    let (loaded, status) = load_project(&path).unwrap();
    assert_eq!(status, SourceStatus::Current);
    assert_eq!(loaded.scene.display_rotation_degrees, [20., 15., 35.]);
    assert_eq!(
        loaded.output_options.section.as_ref().unwrap().position_mm,
        0.75
    );
    let restored = serde_json::to_value(loaded).unwrap();
    let restored: Timeline = serde_json::from_value(restored["output_options"]["timeline"].clone())
        .expect("saved project must retain the camera timeline");
    let half = restored.sample(1.).unwrap();
    assert_eq!(half.camera.yaw_degrees, 540.);
    assert_eq!(half.camera.pitch_degrees, 30.);
    assert_eq!(half.camera.target_mm, [5., 10., 15.]);
    assert!((half.camera.distance_mm - 200.).abs() < 1e-9);
    assert_eq!(half.turntable_degrees, 180.);
    assert_eq!(half.buildup_fraction, Some(0.5));
    assert_eq!(restored.frame_count().unwrap(), 40);
    assert_eq!(
        restored.sample_frame(39).unwrap().buildup_fraction,
        Some(1.)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_projects_without_timeline_or_output_options_still_load() {
    let (root, original) = fixture();
    let path = root.join("legacy.tnproj");
    for omit_all_options in [false, true] {
        let mut value = original.clone();
        if omit_all_options {
            value.as_object_mut().unwrap().remove("output_options");
        } else {
            for key in ["timeline", "build_stage", "start_angle_degrees"] {
                value["output_options"].as_object_mut().unwrap().remove(key);
            }
        }
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let (loaded, status) = load_project(&path).unwrap();
        assert_eq!(status, SourceStatus::Current);
        assert!(loaded.output_options.build_stage.is_none());
        assert_eq!(loaded.output_options.start_angle_degrees, 0.);
        let serialized = serde_json::to_value(loaded).unwrap();
        assert!(serialized["output_options"]["timeline"].is_null());
        assert_eq!(serialized["output_options"]["animation"], "Turntable");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_preserves_basic_turntable_timing_and_defaults_legacy_files() {
    let (root, mut value) = fixture();
    let path = root.join("basic.turninator");
    value["output_options"]["turntable_timing"] =
        json!({"seconds_per_turn":2.5,"duration_seconds":7.5});
    let project: ProjectFile = serde_json::from_value(value.clone()).unwrap();
    save_project(&path, &project).unwrap();
    let (restored, _) = load_project(&path).unwrap();
    assert_eq!(
        restored.output_options.turntable_timing.seconds_per_turn,
        2.5
    );
    assert_eq!(
        restored
            .output_options
            .turntable_timing
            .total_turns()
            .unwrap(),
        3.
    );
    assert!(restored.output_options.timeline.is_none());
    for key in ["seconds_per_turn", "duration_seconds"] {
        let mut invalid = project.clone();
        if key == "seconds_per_turn" {
            invalid.output_options.turntable_timing.seconds_per_turn = 0.;
        } else {
            invalid.output_options.turntable_timing.duration_seconds = 501.;
        }
        let previous = fs::read(&path).unwrap();
        assert!(save_project(&path, &invalid).is_err());
        assert_eq!(fs::read(&path).unwrap(), previous);
    }
    value["output_options"]
        .as_object_mut()
        .unwrap()
        .remove("turntable_timing");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let (legacy, _) = load_project(&path).unwrap();
    assert_eq!(legacy.output_options.turntable_timing.seconds_per_turn, 4.);
    assert_eq!(legacy.output_options.turntable_timing.duration_seconds, 4.);
    fs::remove_dir_all(root).unwrap();
}

fn invalid_timelines() -> Vec<(&'static str, Value)> {
    let mut cases = Vec::new();
    let mut add = |name, pointer, replacement| {
        let mut value = timeline();
        *value.pointer_mut(pointer).unwrap() = replacement;
        cases.push((name, value));
    };
    add("zero duration", "/duration_seconds", json!(0.));
    add("excess duration", "/duration_seconds", json!(501.));
    add("empty keys", "/keyframes", json!([]));
    add(
        "excess keys",
        "/keyframes",
        json!(vec![timeline()["keyframes"][0].clone(); 257]),
    );
    add("duplicate time", "/keyframes/1/time_seconds", json!(0.));
    add(
        "first key after zero",
        "/keyframes/0/time_seconds",
        json!(0.1),
    );
    add("key after duration", "/keyframes/1/time_seconds", json!(3.));
    add(
        "mixed projections",
        "/keyframes/1/camera/orthographic",
        json!(true),
    );
    add("invalid zoom", "/keyframes/1/camera/distance_mm", json!(0.));
    add("fractional spiral", "/keyframes/0/spiral_turns", json!(0.5));
    add("discontinuous loop endpoint", "/looping", json!(true));
    cases
}

#[test]
fn invalid_saved_timelines_are_rejected_before_project_use() {
    let (root, original) = fixture();
    let path = root.join("invalid.tnproj");
    for (name, timeline) in invalid_timelines() {
        let mut value = original.clone();
        value["output_options"]["timeline"] = timeline;
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(load_project(&path).is_err(), "accepted {name}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_timeline_cannot_overwrite_a_valid_project() {
    let (root, original) = fixture();
    let path = root.join("keep.tnproj");
    let valid: ProjectFile = serde_json::from_value(original.clone()).unwrap();
    save_project(&path, &valid).unwrap();
    let before = fs::read(&path).unwrap();
    for (name, timeline) in invalid_timelines() {
        let mut value = original.clone();
        value["output_options"]["timeline"] = timeline;
        let invalid: ProjectFile = serde_json::from_value(value).unwrap();
        assert!(save_project(&path, &invalid).is_err(), "saved {name}");
        assert_eq!(
            fs::read(&path).unwrap(),
            before,
            "overwrote valid project for {name}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn nonfinite_timeline_cannot_serialize_to_an_unreloadable_project() {
    let (root, original) = fixture();
    let path = root.join("finite.tnproj");
    let mut project: ProjectFile = serde_json::from_value(original).unwrap();
    save_project(&path, &project).unwrap();
    let before = fs::read(&path).unwrap();
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut timeline: Timeline = serde_json::from_value(timeline()).unwrap();
        timeline.keyframes[1].camera.target_mm[2] = bad;
        project.output_options.timeline = Some(timeline);
        assert!(save_project(&path, &project).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn fixed_build_stage_and_sampled_spin_roundtrip_with_or_without_a_timeline() {
    let (root, original) = fixture();
    let path = root.join("stage.tnproj");
    for has_timeline in [false, true] {
        for (count, angle) in [(0, -135.25), (17, 287.5), (50_000, 1e9)] {
            let mut project: ProjectFile = serde_json::from_value(original.clone()).unwrap();
            project.output_options.build_stage = Some(BuildStage {
                visible_layers: count,
            });
            project.output_options.start_angle_degrees = angle;
            project.output_options.timeline =
                has_timeline.then(|| serde_json::from_value(timeline()).unwrap());
            save_project(&path, &project).unwrap();
            let (loaded, status) = load_project(&path).unwrap();
            assert_eq!(status, SourceStatus::Current);
            assert_eq!(
                loaded.output_options.build_stage.unwrap().visible_layers,
                count
            );
            assert_eq!(loaded.output_options.start_angle_degrees, angle);
            assert_eq!(loaded.output_options.timeline.is_some(), has_timeline);
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_build_stage_or_sampled_spin_cannot_replace_an_existing_project() {
    let (root, original) = fixture();
    let path = root.join("keep-stage.tnproj");
    let valid: ProjectFile = serde_json::from_value(original).unwrap();
    save_project(&path, &valid).unwrap();
    let before = fs::read(&path).unwrap();
    for count in [50_001, u32::MAX] {
        let mut invalid = valid.clone();
        invalid.output_options.build_stage = Some(BuildStage {
            visible_layers: count,
        });
        assert!(save_project(&path, &invalid).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    for angle in [
        1e9 + 1.,
        -1e9 - 1.,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        let mut invalid = valid.clone();
        invalid.output_options.start_angle_degrees = angle;
        assert!(save_project(&path, &invalid).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_or_out_of_budget_saved_stage_and_angle_are_rejected_on_load() {
    let (root, original) = fixture();
    let path = root.join("bad-stage.tnproj");
    for (key, invalid) in [
        ("build_stage", json!({"visible_layers":50_001})),
        ("build_stage", json!({"visible_layers":-1})),
        ("build_stage", json!({"visible_layers":1.5})),
        ("start_angle_degrees", json!(1e9 + 1.)),
        ("start_angle_degrees", json!(-1e9 - 1.)),
        ("start_angle_degrees", Value::Null),
    ] {
        let mut value = original.clone();
        value["output_options"][key] = invalid;
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(load_project(&path).is_err(), "accepted invalid {key}");
    }
    fs::remove_dir_all(root).unwrap();
}
