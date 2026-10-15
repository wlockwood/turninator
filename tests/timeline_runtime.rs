//! Explicit GPU acceptance; source geometry and sampled camera identities are retained.
#[test]
#[ignore = "requires Blender OptiX; real resin build-up, capped sections, paired spiral timeline and AV1 reuse"]
fn resin_buildup_sections_follow_one_timeline_and_preserve_bead_voids() {
    use std::{fs, path::Path, sync::atomic::AtomicBool};
    use turninator::{
        animation::{CameraKeyframe, Easing, Timeline, Transition},
        export::{ExportKind, ExportLayout, ExportRequest, SectionSpec, VideoCodec, run_export},
        presentation::{ImportSettings, SceneSettings, load_model, model_scene},
        render::{ToolPaths, WorkerClient},
        viewport::OrbitCamera,
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("artifacts/timeline-section-proof");
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join("separated-layers.gcode");
    let mut gcode = "G90\nM83\n;WIDTH:1.0\n;HEIGHT:0.5\n".to_owned();
    for i in 1..=4 {
        gcode.push_str(&format!(
            "T{}\nG1X0Y0Z{}\nG1X8E1\nG1X0Y6\nG1X8E1\n",
            usize::from(i > 2),
            i as f64 * 0.5
        ));
    }
    fs::write(&source, gcode).unwrap();
    let import = ImportSettings {
        layer_height_mm: 0.5,
        first_layer_height_mm: 0.5,
        allow_tool_changes: true,
        ..Default::default()
    };
    let model = load_model(&source, &import).unwrap();
    model.mesh.write_binary(&dir.join("mesh.bin")).unwrap();
    let settings = SceneSettings {
        import,
        material_preset: "resin".into(),
        color_mode: "tool".into(),
        studio_preset: "dark".into(),
        layer_appearance: false,
        ..Default::default()
    };
    let spec = model_scene(&model, &settings, &dir.join("mesh.bin"), None).unwrap();
    let scene_path = dir.join("scene.json");
    fs::write(&scene_path, serde_json::to_vec(&spec).unwrap()).unwrap();
    let camera = OrbitCamera {
        yaw_degrees: 35.,
        pitch_degrees: 25.,
        distance_mm: 32.,
        target_mm: [0., 0., 1.],
        ..Default::default()
    };
    let timeline = Timeline {
        duration_seconds: 0.2,
        buildup: true,
        keyframes: vec![
            CameraKeyframe {
                camera: camera.clone(),
                transition: Transition::Spiral,
                spiral_turns: 1.,
                easing: Easing::SmoothStep,
                ..Default::default()
            },
            CameraKeyframe {
                time_seconds: 0.2,
                camera: OrbitCamera {
                    pitch_degrees: 90.,
                    target_mm: [0.5, 0., 1.],
                    ..camera
                },
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let tools = ToolPaths::default();
    let cancel = AtomicBool::new(false);
    let mut worker = WorkerClient::spawn(&tools).unwrap();
    let mut request = ExportRequest {
        job_dir: dir.join("jobs"),
        scene_path: scene_path.clone(),
        destination: dir.join("paired-resin.gif"),
        kind: ExportKind::Gif,
        size: 128,
        selection: turninator::render::RenderSelection::from_legacy("cycles", 8).unwrap(),
        layout: ExportLayout::SideBySide,
        section: Some(SectionSpec {
            axis: 0,
            position_mm: 4.,
            keep_positive: true,
            cap: true,
        }),
        timeline: Some(timeline.clone()),
        ..Default::default()
    };
    let result = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    assert_eq!(
        (
            result.width,
            result.height,
            result.frame_count,
            result.duration_ms
        ),
        (256, 128, 4, 200)
    );
    let render_dir = fs::read_dir(&request.job_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.join("frame-analysis.json").is_file())
        .unwrap();
    let analysis: serde_json::Value =
        serde_json::from_slice(&fs::read(render_dir.join("frame-analysis.json")).unwrap()).unwrap();
    let values = analysis.as_array().unwrap();
    assert_eq!(values.len(), 4);
    let first = values[0]["original_volume_mm3"].as_f64().unwrap();
    for (i, stats) in values.iter().enumerate() {
        assert_eq!(stats["visible_layer"], i);
        assert_eq!(stats["capped"], true);
        let before = stats["original_volume_mm3"].as_f64().unwrap();
        let after = stats["remaining_volume_mm3"].as_f64().unwrap();
        assert!(
            (before / first - (i + 1) as f64).abs() < 1e-4,
            "future layers affected volume: {stats}"
        );
        assert!(
            (after / before - 0.5).abs() < 1e-4,
            "section did not preserve half of actual beads: {stats}"
        );
    }
    // Two isolated paths cut twice; no 6 mm envelope bridge can be introduced.
    assert!(values[0]["cross_section_area_mm2"].as_f64().unwrap() < 1.0);
    let full: serde_json::Value =
        serde_json::from_slice(&fs::read(render_dir.join("full/render-manifest.json")).unwrap())
            .unwrap();
    let cut: serde_json::Value =
        serde_json::from_slice(&fs::read(render_dir.join("section/render-manifest.json")).unwrap())
            .unwrap();
    assert_eq!(full["frame_states_hash"], cut["frame_states_hash"]);
    let last = image::open(render_dir.join("frames/frame_0003.png"))
        .unwrap()
        .to_rgba8();
    assert_eq!(last.dimensions(), (256, 128));
    let original_times: Vec<_> = (0..4)
        .map(|i| {
            fs::metadata(render_dir.join(format!("section/frame_{i:04}.png")))
                .unwrap()
                .modified()
                .unwrap()
        })
        .collect();
    request.kind = ExportKind::Mp4;
    request.video_codec = VideoCodec::Av1;
    request.destination = dir.join("paired-resin-av1.mp4");
    let video = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    assert_eq!(video.duration_ms, 200);
    for (i, time) in original_times.iter().enumerate() {
        assert_eq!(
            fs::metadata(render_dir.join(format!("section/frame_{i:04}.png")))
                .unwrap()
                .modified()
                .unwrap(),
            *time
        );
    }
    // Independent transparent PNGs retain alpha even with resin and a section.
    let mut section_spec = spec;
    section_spec["section"] = serde_json::to_value(&request.section).unwrap();
    let section_path = dir.join("alpha-section.json");
    fs::write(&section_path, serde_json::to_vec(&section_spec).unwrap()).unwrap();
    let frames = dir.join(format!(
        "alpha-{}",
        full["worker_hash"].as_str().unwrap().get(..12).unwrap()
    ));
    let complete=worker.request(serde_json::json!({"request_id":"alpha-timeline","scene_revision":1,"kind":"render","scene_path":section_path,"output_dir":frames,"mode":"buildup","frame_count":4,"frame_states":timeline.worker_frames().unwrap(),"engine":"cycles","size":128,"samples":8,"transparent":true,"reuse_verified_frames":frames.join("render-manifest.json").exists()}),&cancel,|_|{}).unwrap();
    assert_eq!(
        complete["payload"]["frame_analysis"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    let image = image::open(frames.join("frame_0003.png"))
        .unwrap()
        .to_rgba8();
    assert!(image.pixels().any(|p| p[3] == 0));
    assert!(image.pixels().any(|p| p[3] > 128));
    fs::write(dir.join("result.json"),serde_json::to_vec_pretty(&serde_json::json!({"gif":result,"mp4":video,"analysis":analysis,"full_manifest":full,"section_manifest":cut,"alpha_complete":complete})).unwrap()).unwrap();
}

#[test]
#[ignore = "requires Blender; fixed partial PNG/sheet and empty section retain full-source framing"]
fn fixed_stage_exports_png_sheet_and_empty_section() {
    use serde_json::json;
    use std::{fs, path::Path, sync::atomic::AtomicBool};
    use turninator::{
        export::{BuildStage, ExportKind, ExportRequest, SectionSpec, run_export},
        presentation::{ImportSettings, SceneSettings, load_model, model_scene},
        render::{ToolPaths, WorkerClient},
    };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/static-build-stage-proof");
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join("layers.gcode");
    fs::write(&source,"G90\nM83\n;WIDTH:1\n;HEIGHT:0.5\nG1X0Z0.5\nG1X8E1\nG1X0Z1\nG1X8E1\nG1X0Z1.5\nG1X8E1\nG1X0Z2\nG1X8E1\n").unwrap();
    let import = ImportSettings {
        layer_height_mm: 0.5,
        first_layer_height_mm: 0.5,
        ..Default::default()
    };
    let model = load_model(&source, &import).unwrap();
    model.mesh.write_binary(&dir.join("mesh.bin")).unwrap();
    let settings = SceneSettings {
        import,
        layer_appearance: false,
        ..Default::default()
    };
    let scene = model_scene(&model, &settings, &dir.join("mesh.bin"), None).unwrap();
    let scene_path = dir.join("scene.json");
    fs::write(&scene_path, serde_json::to_vec(&scene).unwrap()).unwrap();
    let tools = ToolPaths::default();
    let cancel = AtomicBool::new(false);
    let mut worker = WorkerClient::spawn(&tools).unwrap();
    let mut request = ExportRequest {
        job_dir: dir.join("jobs"),
        scene_path,
        destination: dir.join("partial.png"),
        kind: ExportKind::Still,
        size: 256,
        selection: turninator::render::RenderSelection::from_legacy("eevee", 8).unwrap(),
        transparent: true,
        build_stage: Some(BuildStage { visible_layers: 1 }),
        start_angle_degrees: 37.5,
        ..Default::default()
    };
    let partial = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    let partial_image = image::open(&partial.path).unwrap().to_rgba8();
    let alpha_count = |img: &image::RgbaImage| img.pixels().filter(|p| p[3] > 128).count();
    let n = alpha_count(&partial_image);
    assert!(n > 100);
    request.build_stage = Some(BuildStage { visible_layers: 4 });
    request.destination = dir.join("full.png");
    let full = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    assert!(alpha_count(&image::open(full.path).unwrap().to_rgba8()) > n);
    let partial_dir = fs::read_dir(&request.job_dir)
        .unwrap()
        .map(|v| v.unwrap().path())
        .find(|p| {
            fs::read(p.join("frames/render-manifest.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .is_some_and(|m| m["build_stage"]["visible_layers"] == 1)
        })
        .unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(partial_dir.join("frames/render-manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["angles"], json!([37.5]));
    request.kind = ExportKind::Diagram;
    request.build_stage = Some(BuildStage { visible_layers: 1 });
    request.destination = dir.join("partial-sheet.png");
    let sheet = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    assert_eq!((sheet.width, sheet.height), (256, 256));
    request.kind = ExportKind::SectionStill;
    request.build_stage = Some(BuildStage { visible_layers: 0 });
    request.destination = dir.join("empty-section.png");
    request.section = Some(SectionSpec {
        axis: 0,
        position_mm: 4.,
        keep_positive: true,
        cap: true,
    });
    let empty = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    assert_eq!(alpha_count(&image::open(empty.path).unwrap().to_rgba8()), 0);
    request.build_stage = Some(BuildStage { visible_layers: 1 });
    request.destination = dir.join("partial-section.png");
    let cut = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    assert!(alpha_count(&image::open(cut.path).unwrap().to_rgba8()) > 0);
    fs::write(dir.join("result.json"),serde_json::to_vec_pretty(&json!({"partial":partial,"sheet":sheet,"sampled_start_angle":manifest["angles"],"empty_alpha_pixels":0,"first_layer_alpha_pixels":n})).unwrap()).unwrap();
}
