use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use turninator::{
    animation::{Timeline, TurntableTiming},
    export::{
        AnimationMode, BuildStage, ExportKind, ExportLayout, ExportRequest, SectionSpec,
        VideoCodec, identity::retained_render_key,
    },
    mesh::Mesh,
};

fn scene() -> Value {
    json!({"mesh_path":"D:/cache/first/mesh.bin","color":[0.5,0.2,0.1],
        "camera":{"position":[10.,20.,30.]},"layer_height_mm":0.2})
}

fn key(scene: &Value, request: &ExportRequest) -> String {
    retained_render_key(scene, &"a".repeat(64), request).unwrap()
}

#[test]
fn concrete_pixel_identity_invalidates_runtime_device_color_and_denoiser() {
    use turninator::{
        export::identity::pixel_recipe_key,
        render::{BackendId, contract::BackendIdentity},
    };
    let request = ExportRequest::default();
    let identity = BackendIdentity {
        backend: BackendId::BlenderEevee,
        adapter_recipe_version: 1,
        runtime_digest: "runtime".into(),
        bridge_digest: None,
        shader_digest: None,
        material_digest: None,
        color_digest: "color".into(),
        denoiser_digest: None,
        device: "gpu".into(),
        api: "api".into(),
        driver: "driver".into(),
    };
    let key = |r: &ExportRequest, i: &BackendIdentity| {
        pixel_recipe_key(&scene(), &"a".repeat(64), r, i).unwrap()
    };
    let expected = key(&request, &identity);
    let mut codec = request.clone();
    codec.kind = ExportKind::Mp4;
    codec.video_codec = VideoCodec::Av1;
    assert_eq!(expected, key(&codec, &identity));
    for field in [
        "runtime_digest",
        "device",
        "color_digest",
        "denoiser_digest",
        "shader_digest",
        "material_digest",
        "driver",
        "api",
        "bridge_digest",
    ] {
        let mut value = serde_json::to_value(&identity).unwrap();
        value[field] = json!("changed");
        let changed = serde_json::from_value(value).unwrap();
        assert_ne!(expected, key(&request, &changed), "{field}");
    }
    let mut quality = request.clone();
    *quality.selection.quality.samples_mut().unwrap() += 1;
    assert_ne!(expected, key(&quality, &identity));
    let mut backend = request.clone();
    backend.selection =
        turninator::render::RenderSelection::for_backend(BackendId::BlenderCycles, true);
    assert!(pixel_recipe_key(&scene(), &"a".repeat(64), &backend, &identity).is_err());
}

#[test]
fn animation_frames_are_shared_across_formats_codecs_and_output_paths() {
    let original = ExportRequest::default();
    let expected = key(&scene(), &original);
    let mut other = original.clone();
    other.kind = ExportKind::Mp4;
    other.video_codec = VideoCodec::Av1;
    other.destination = "D:/somewhere/another-name.mp4".into();
    other.job_dir = "D:/cache/other-job".into();
    other.scene_path = "D:/cache/other-job/scene.json".into();
    other.transparent = true; // Animated output is always opaque.
    let mut other_scene = scene();
    other_scene["mesh_path"] = json!("D:/cache/other-job/mesh.bin");
    assert_eq!(expected, key(&other_scene, &other));
    assert_eq!(expected.len(), 64);
}

#[test]
fn visual_geometry_and_sampling_changes_invalidate_frames() {
    let original = ExportRequest::default();
    let expected = key(&scene(), &original);
    for (field, value) in [
        ("color", json!([0.1, 0.2, 0.3])),
        ("camera", json!({"position":[11.,20.,30.]})),
        ("layer_height_mm", json!(0.3)),
    ] {
        let mut changed = scene();
        changed[field] = value;
        assert_ne!(expected, key(&changed, &original), "{field}");
    }
    assert_ne!(
        expected,
        retained_render_key(&scene(), &"b".repeat(64), &original).unwrap()
    );
    let variants = [
        ExportRequest {
            size: 512,
            ..original.clone()
        },
        ExportRequest {
            selection: turninator::render::RenderSelection::from_legacy("eevee", 64).unwrap(),
            ..original.clone()
        },
        ExportRequest {
            selection: turninator::render::RenderSelection::from_legacy("cycles", 32).unwrap(),
            ..original.clone()
        },
        ExportRequest {
            frame_rate: 30,
            ..original.clone()
        },
        ExportRequest {
            frame_count: 81,
            ..original.clone()
        },
        ExportRequest {
            start_angle_degrees: 90.,
            ..original.clone()
        },
        ExportRequest {
            animation: AnimationMode::BuildUp,
            ..original.clone()
        },
        ExportRequest {
            build_stage: Some(BuildStage { visible_layers: 2 }),
            ..original.clone()
        },
        ExportRequest {
            section: Some(SectionSpec {
                axis: 2,
                position_mm: 5.,
                keep_positive: true,
                cap: true,
            }),
            ..original.clone()
        },
        ExportRequest {
            layout: ExportLayout::SideBySide,
            ..original.clone()
        },
        ExportRequest {
            timeline: Some(Timeline::default()),
            ..original.clone()
        },
        ExportRequest {
            turntable_timing: Some(TurntableTiming::default()),
            ..original.clone()
        },
    ];
    for changed in variants {
        assert_ne!(expected, key(&scene(), &changed), "{changed:?}");
    }
}

#[test]
fn timed_animation_uses_effective_count_and_rotation_speed() {
    let original = ExportRequest {
        turntable_timing: Some(TurntableTiming::default()),
        ..Default::default()
    };
    let expected = key(&scene(), &original);
    let mut changed = original.clone();
    changed.frame_count = 199;
    assert_eq!(expected, key(&scene(), &changed));
    changed.turntable_timing.as_mut().unwrap().seconds_per_turn = 2.;
    assert_ne!(expected, key(&scene(), &changed));
    changed.turntable_timing = Some(TurntableTiming {
        duration_seconds: 8.,
        ..Default::default()
    });
    assert_ne!(expected, key(&scene(), &changed));
}

#[test]
fn stills_ignore_inactive_animation_settings_and_keep_alpha_and_pose() {
    let original = ExportRequest {
        kind: ExportKind::Still,
        ..Default::default()
    };
    let expected = key(&scene(), &original);
    let changed = ExportRequest {
        kind: ExportKind::SectionStill,
        frame_count: 999,
        frame_rate: 100,
        animation: AnimationMode::BuildUp,
        timeline: Some(Timeline::default()),
        turntable_timing: Some(TurntableTiming::default()),
        ..original.clone()
    };
    assert_eq!(expected, key(&scene(), &changed));
    assert_ne!(
        expected,
        key(
            &scene(),
            &ExportRequest {
                transparent: true,
                ..original.clone()
            }
        )
    );
    assert_ne!(
        expected,
        key(
            &scene(),
            &ExportRequest {
                start_angle_degrees: 10.,
                ..original
            }
        )
    );
}

#[test]
fn timeline_and_diagram_ignore_unused_start_angle() {
    for original in [
        ExportRequest {
            timeline: Some(Timeline::default()),
            ..Default::default()
        },
        ExportRequest {
            kind: ExportKind::Diagram,
            ..Default::default()
        },
    ] {
        let changed = ExportRequest {
            start_angle_degrees: 175.,
            ..original.clone()
        };
        assert_eq!(key(&scene(), &original), key(&scene(), &changed));
    }
    let original = ExportRequest {
        kind: ExportKind::Diagram,
        ..Default::default()
    };
    assert_eq!(
        key(&scene(), &original),
        key(
            &scene(),
            &ExportRequest {
                transparent: true,
                ..original
            }
        )
    );
}

#[test]
fn section_format_variants_share_only_matching_cut_and_layout() {
    let original = ExportRequest {
        kind: ExportKind::SectionTurntable,
        section: Some(SectionSpec {
            axis: 2,
            position_mm: 5.,
            keep_positive: true,
            cap: true,
        }),
        layout: ExportLayout::SideBySide,
        ..Default::default()
    };
    let expected = key(&scene(), &original);
    let mut changed = ExportRequest {
        kind: ExportKind::Mp4,
        ..original.clone()
    };
    assert_eq!(expected, key(&scene(), &changed));
    changed.section.as_mut().unwrap().position_mm = 6.;
    assert_ne!(expected, key(&scene(), &changed));
    changed = original.clone();
    changed.section.as_mut().unwrap().cap = false;
    assert_ne!(expected, key(&scene(), &changed));
    changed = original;
    changed.layout = ExportLayout::Single;
    assert_ne!(expected, key(&scene(), &changed));
}

#[test]
fn invalid_active_identity_inputs_are_rejected() {
    assert!(retained_render_key(&scene(), "not-a-hash", &ExportRequest::default()).is_err());
    assert!(retained_render_key(&Value::Null, &"a".repeat(64), &ExportRequest::default()).is_err());
    for changed in [
        ExportRequest {
            frame_rate: 0,
            ..Default::default()
        },
        ExportRequest {
            frame_count: 0,
            ..Default::default()
        },
        ExportRequest {
            frame_count: 10001,
            ..Default::default()
        },
        ExportRequest {
            start_angle_degrees: f64::NAN,
            ..Default::default()
        },
    ] {
        assert!(retained_render_key(&scene(), &"a".repeat(64), &changed).is_err());
    }
}

#[test]
fn snapshot_hash_matches_the_exact_serialized_float_geometry() {
    let mut mesh = Mesh {
        positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
        triangles: vec![[0, 1, 2]],
    };
    let path = std::env::temp_dir().join(format!(
        "turninator-snapshot-hash-{}.bin",
        std::process::id()
    ));
    mesh.write_binary(&path).unwrap();
    let expected = format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap()));
    std::fs::remove_file(path).unwrap();
    assert_eq!(mesh.snapshot_hash().unwrap(), expected);
    mesh.positions[1][0] += 1e-10;
    assert_eq!(mesh.snapshot_hash().unwrap(), expected);
    mesh.positions[1][0] = 2.;
    assert_ne!(mesh.snapshot_hash().unwrap(), expected);
    mesh.positions[1] = mesh.positions[0];
    assert!(mesh.snapshot_hash().is_err());
}

#[test]
fn snapshot_hash_rejects_invalid_indices_and_unrepresentable_coordinates() {
    let mesh = Mesh {
        positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
        triangles: vec![[0, 1, 2]],
    };
    let mut invalid = mesh.clone();
    invalid.triangles[0][2] = 3;
    assert!(invalid.snapshot_hash().is_err());
    for coordinate in [f64::NAN, f64::INFINITY, f64::MAX] {
        invalid = mesh.clone();
        invalid.positions[1][0] = coordinate;
        assert!(invalid.snapshot_hash().is_err());
    }
    assert!(Mesh::default().snapshot_hash().is_err());
}
