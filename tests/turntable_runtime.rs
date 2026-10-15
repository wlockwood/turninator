//! Opt-in real renderer/encoder acceptance for repeated basic rotations.
#[test]
#[ignore = "requires Blender, GPU and FFmpeg; writes artifacts/turntable-timing-proof"]
fn repeated_turns_render_once_and_preserve_order_duration_and_retry() {
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
        .join(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis()
                .to_string(),
        );
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
        "layer_height_mm":0.2,"studio_preset":"dark","warnings":[]});
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
        samples: 8,
        turntable_timing: Some(TurntableTiming {
            seconds_per_turn: 0.2,
            duration_seconds: 0.6,
        }),
        ..Default::default()
    };
    let result = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    assert_eq!((result.frame_count, result.duration_ms), (12, 600));
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
    assert_eq!(first["unique_frame_count"], 4);
    assert_eq!(first["rendered_frame_count"], 4);
    assert_eq!(first["copied_frame_count"], 8);
    for i in 0..12 {
        assert_eq!(
            fs::read(render_dir.join(format!("frames/frame_{i:04}.png"))).unwrap(),
            fs::read(render_dir.join(format!("frames/frame_{:04}.png", i % 4))).unwrap()
        );
    }
    // Retrying validated jobs renders and copies nothing.
    request.destination = dir.join("retry.mp4");
    request.kind = ExportKind::Mp4;
    let retry = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    assert_eq!((retry.frame_count, retry.duration_ms), (12, 600));
    let retried = details();
    assert_eq!(retried["rendered_frame_count"], 0);
    assert_eq!(retried["copied_frame_count"], 0);
    assert_eq!(retried["verified_frame_count"], 12);
    // A fractional final turn uses the matching prefix of the rendered rotation.
    request.destination = dir.join("partial.gif");
    request.kind = ExportKind::Gif;
    request.turntable_timing.as_mut().unwrap().duration_seconds = 0.5;
    let partial = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    assert_eq!((partial.frame_count, partial.duration_ms), (10, 500));
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
        (256, 10, 500)
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
        assert_eq!(stats["rendered_frame_count"], 4);
        assert_eq!(stats["copied_frame_count"], 6);
        let analysis = stats["frame_analysis"].as_array().unwrap();
        assert_eq!(analysis[0], analysis[4]);
    }
    fs::write(
        dir.join("acceptance.json"),
        serde_json::to_vec_pretty(&json!({"first":first,"retry":retried,
        "gif":result,"mp4":retry,"partial":partial,"paired":paired}))
        .unwrap(),
    )
    .unwrap();
    println!("Turntable acceptance: {}", dir.display());
}
