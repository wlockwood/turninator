//! Opt-in real renderer/encoder acceptance for repeated basic rotations.
#[test]
#[ignore = "requires Blender, GPU and FFmpeg; writes artifacts/turntable-timing-proof"]
fn repeated_turns_render_once_and_preserve_order_duration_and_retry() {
    prove_repeated_turns(20);
}

#[test]
#[ignore = "requires Blender/GPU and FFmpeg; validates selected30fps plus repeated-frame reuse"]
fn selected_frame_rate_preserves_turns_duration_reuse_and_paired_sections() {
    prove_repeated_turns(30);
}

fn prove_repeated_turns(frame_rate: u32) {
    use serde_json::{Value, json};
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::AtomicBool,
        time::{SystemTime, UNIX_EPOCH},
    };
    use turninator::{
        animation::TurntableTiming,
        export::{ExportKind, ExportLayout, ExportRequest, SectionSpec, run_export},
        mesh::Mesh,
        render::{ToolPaths, WorkerClient},
    };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts/turntable-timing-proof")
        .join(format!(
            "{}-{frame_rate}fps",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
    fs::create_dir_all(&dir).unwrap();
    let mesh = Mesh {
        positions: vec![
            [0., 0., 0.],
            [20., 0., 0.],
            [20., 12., 0.],
            [0., 12., 0.],
            [0., 0., 10.],
            [20., 0., 10.],
            [20., 12., 10.],
            [0., 12., 10.],
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
    };
    mesh.write_binary(&dir.join("mesh.bin")).unwrap();
    let scene = json!({"schema_version":1,"mesh_path":dir.join("mesh.bin"),"geometry_kind":"original",
        "display_rotation_degrees":[0,0,0],"color":[0.04,0.2,0.65],"roughness":0.4,
        "layer_height_mm":0.2,"solid_layer_tops_mm":[2.,4.,6.,8.,10.],"studio_preset":"dark","warnings":[]});
    fs::write(dir.join("scene.json"), serde_json::to_vec(&scene).unwrap()).unwrap();
    let mut tools = ToolPaths::default();
    if let Some(blender) = std::env::var_os("TURNINATOR_TEST_BLENDER") {
        tools.blender = blender.into();
    }
    let mut worker = WorkerClient::spawn(&tools).unwrap();
    let cancel = AtomicBool::new(false);
    let mut request = ExportRequest {
        job_dir: dir.join("jobs"),
        scene_path: dir.join("scene.json"),
        destination: dir.join("three-turns.gif"),
        kind: ExportKind::Gif,
        size: 128,
        frame_rate,
        samples: 8,
        turntable_timing: Some(TurntableTiming {
            seconds_per_turn: 0.2,
            duration_seconds: 0.6,
        }),
        ..Default::default()
    };
    let result = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    let total_frames = frame_rate * 3 / 5;
    let unique_frames = frame_rate / 5;
    let partial_frames = frame_rate / 2;
    assert_eq!(
        (result.frame_count, result.duration_ms),
        (total_frames, 600)
    );
    let render_dir: PathBuf = fs::read_dir(&request.job_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|path| path.join("frames/render-result.json").is_file())
        .unwrap();
    let details = || -> Value {
        serde_json::from_slice(&fs::read(render_dir.join("frames/render-result.json")).unwrap())
            .unwrap()
    };
    let first = details();
    assert_eq!(first["unique_frame_count"], unique_frames);
    assert_eq!(first["rendered_frame_count"], unique_frames);
    assert_eq!(first["copied_frame_count"], total_frames - unique_frames);
    assert_eq!(first["frame_rate"], frame_rate);
    assert_eq!(first["render_strategy"], "animation");
    assert_eq!(first["frame_pipeline"]["validated_frames"], unique_frames);
    assert!(first["frame_pipeline"]["workers"].as_u64().unwrap() >= 1);
    for i in 0..total_frames {
        assert_eq!(
            fs::read(render_dir.join(format!("frames/frame_{i:04}.png"))).unwrap(),
            fs::read(render_dir.join(format!("frames/frame_{:04}.png", i % unique_frames)))
                .unwrap()
        );
    }
    // Retrying validated jobs renders and copies nothing.
    worker.cancel();
    assert!(!worker.is_running());
    request.destination = dir.join("retry.mp4");
    request.kind = ExportKind::Mp4;
    let mut retained_frames = 0;
    let retry = run_export(&mut worker, &tools, &request, &cancel, |event| {
        if let turninator::export::JobEvent::Progress { stage, .. } = event
            && stage == "Reusing retained frames"
        {
            retained_frames += 1;
        }
    })
    .unwrap();
    assert_eq!((retry.frame_count, retry.duration_ms), (total_frames, 600));
    let retried = details();
    assert_eq!(
        retried, first,
        "native reuse must not invoke Blender or rewrite its result"
    );
    assert_eq!(retained_frames, total_frames);
    // Retry validates each PNG completely; one corrupt image must be regenerated
    // without publishing progress out of order or touching its valid siblings.
    let untouched = fs::read(render_dir.join("frames/frame_0000.png")).unwrap();
    fs::write(render_dir.join("frames/frame_0001.png"), b"broken PNG").unwrap();
    worker = WorkerClient::spawn(&tools).unwrap();
    request.destination = dir.join("repaired.mp4");
    run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    let repaired = details();
    assert_eq!(repaired["render_strategy"], "individual_frames");
    assert_eq!(repaired["rendered_frame_count"], 1);
    assert_eq!(repaired["verified_frame_count"], total_frames - 1);
    assert_eq!(repaired["frame_pipeline"]["validated_frames"], 1);
    assert_eq!(
        fs::read(render_dir.join("frames/frame_0000.png")).unwrap(),
        untouched
    );
    // A fractional final turn uses the matching prefix of the rendered rotation.
    request.destination = dir.join("partial.gif");
    request.kind = ExportKind::Gif;
    request.turntable_timing.as_mut().unwrap().duration_seconds = 0.5;
    let partial = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    assert_eq!(
        (partial.frame_count, partial.duration_ms),
        (partial_frames, 500)
    );
    // Both full and cutaway panels independently share their repeated poses.
    request.destination = dir.join("paired.gif");
    request.layout = ExportLayout::SideBySide;
    request.section = Some(SectionSpec {
        axis: 0,
        position_mm: 10.,
        keep_positive: true,
        cap: true,
    });
    let paired = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    assert_eq!(
        (paired.width, paired.frame_count, paired.duration_ms),
        (256, partial_frames, 500)
    );
    let panel_dir = fs::read_dir(&request.job_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|path| path.join("full/render-result.json").is_file())
        .unwrap();
    for panel in ["full", "section"] {
        let stats: Value = serde_json::from_slice(
            &fs::read(panel_dir.join(panel).join("render-result.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(stats["rendered_frame_count"], unique_frames);
        assert_eq!(stats["copied_frame_count"], partial_frames - unique_frames);
        let analysis = stats["frame_analysis"].as_array().unwrap();
        assert_eq!(analysis[0], analysis[unique_frames as usize]);
    }
    worker.cancel();
    request.kind = ExportKind::Mp4;
    request.destination = dir.join("paired-retained.mp4");
    let mut analyses = 0;
    run_export(&mut worker, &tools, &request, &cancel, |event| {
        if matches!(event, turninator::export::JobEvent::Analysis(_)) {
            analyses += 1;
        }
    })
    .unwrap();
    assert_eq!(
        analyses, 1,
        "retained section metadata must remain available"
    );
    assert!(!worker.is_running());
    if frame_rate != 20 {
        worker = WorkerClient::spawn(&tools).unwrap();
        use turninator::animation::{CameraKeyframe, Timeline};
        use turninator::viewport::OrbitCamera;
        let camera = OrbitCamera {
            target_mm: [10., 6., 5.],
            distance_mm: 100.,
            ortho_span_mm: 40.,
            ..Default::default()
        };
        request.turntable_timing = None;
        request.section = None;
        request.layout = ExportLayout::Single;
        request.kind = ExportKind::Mp4;
        request.destination = dir.join("selected-rate-camera.mp4");
        request.timeline = Some(Timeline {
            duration_seconds: 0.2,
            keyframes: vec![
                CameraKeyframe {
                    camera: camera.clone(),
                    ..Default::default()
                },
                CameraKeyframe {
                    time_seconds: 0.2,
                    camera: OrbitCamera {
                        yaw_degrees: camera.yaw_degrees + 30.,
                        ..camera
                    },
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        let moving = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
        assert_eq!(
            (moving.frame_count, moving.duration_ms),
            (frame_rate / 5, 200)
        );
        request.timeline = None;
        request.animation = turninator::export::AnimationMode::BuildUp;
        request.frame_count = frame_rate / 5;
        request.kind = ExportKind::Gif;
        request.destination = dir.join("selected-rate-build-up.gif");
        let growing = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
        assert_eq!(
            (growing.frame_count, growing.duration_ms),
            (frame_rate / 5, 200)
        );
        // PNG and four-view exports share the same retained-image path, and
        // neither should require a running Blender process on repeat export.
        for kind in [ExportKind::Still, ExportKind::Diagram] {
            request.animation = turninator::export::AnimationMode::Turntable;
            request.kind = kind;
            request.size = if kind == ExportKind::Diagram {
                256
            } else {
                128
            };
            request.destination = dir.join(format!("{kind:?}.png"));
            let first_png = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
            let pixels = fs::read(&first_png.path).unwrap();
            worker.cancel();
            request.destination = dir.join(format!("{kind:?}-retained.png"));
            let retained = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
            assert_eq!(fs::read(&retained.path).unwrap(), pixels);
            assert!(!worker.is_running());
            worker = WorkerClient::spawn(&tools).unwrap();
        }
    }
    fs::write(
        dir.join("acceptance.json"),
        serde_json::to_vec_pretty(
            &json!({"first":first,"retry":retried,"repaired":repaired,"frame_rate":frame_rate,
        "gif":result,"mp4":retry,"partial":partial,"paired":paired}),
        )
        .unwrap(),
    )
    .unwrap();
    println!("Turntable acceptance: {}", dir.display());
}
