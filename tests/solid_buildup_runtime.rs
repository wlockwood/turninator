//! Opt-in test of the published outputs, beyond the geometry/renderer helpers.
#[test]
#[ignore = "requires installed Blender/GPU and FFmpeg; verifies solid stages in final PNG/GIF/MP4/sheet"]
fn solid_stages_reach_final_encoded_outputs() {
    use std::{
        fs,
        path::Path,
        sync::atomic::AtomicBool,
        time::{SystemTime, UNIX_EPOCH},
    };
    use turninator::{
        export::{AnimationMode, BuildStage, ExportKind, ExportRequest, run_export},
        presentation::{ImportSettings, SceneSettings, load_model, model_scene},
        render::{ToolPaths, WorkerClient},
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("artifacts/solid-export-proof").join(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
            .to_string(),
    );
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join("solid.stl");
    let points = [
        [0., 0., 0.],
        [4., 0., 0.],
        [4., 3., 0.],
        [0., 3., 0.],
        [0., 0., 4.],
        [4., 0., 4.],
        [4., 3., 4.],
        [0., 3., 4.],
    ];
    let mut stl = String::from("solid proof\n");
    for triangle in [
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
    ] {
        stl.push_str("facet normal 0 0 0\nouter loop\n");
        for index in triangle {
            let [x, y, z] = points[index];
            stl.push_str(&format!("vertex {x} {y} {z}\n"));
        }
        stl.push_str("endloop\nendfacet\n");
    }
    stl.push_str("endsolid proof\n");
    fs::write(&source, &stl).unwrap();
    let import = ImportSettings {
        layer_height_mm: 1.,
        first_layer_height_mm: 1.,
        ..Default::default()
    };
    let model = load_model(&source, &import).unwrap();
    assert_eq!(model.layer_tops_mm, vec![1., 2., 3., 4.]);
    let mesh = dir.join("mesh.bin");
    model.mesh.write_binary(&mesh).unwrap();
    let scene = model_scene(
        &model,
        &SceneSettings {
            import,
            layer_appearance: false,
            ..Default::default()
        },
        &mesh,
        None,
    )
    .unwrap();
    let scene_path = dir.join("scene.json");
    fs::write(&scene_path, serde_json::to_vec_pretty(&scene).unwrap()).unwrap();
    let before = fs::read(&source).unwrap();
    let tools = ToolPaths {
        blender: root.join("artifacts/runtime-tools/blender-4.5.14-windows-x64/blender.exe"),
        ..Default::default()
    };
    let cancel = AtomicBool::new(false);
    let mut worker = WorkerClient::spawn(&tools).unwrap();
    let mut request = ExportRequest {
        job_dir: dir.join("jobs"),
        scene_path,
        destination: dir.join("empty.png"),
        kind: ExportKind::Still,
        size: 160,
        samples: 4,
        transparent: true,
        build_stage: Some(BuildStage { visible_layers: 0 }),
        ..Default::default()
    };
    run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    assert!(
        image::open(&request.destination)
            .unwrap()
            .to_rgba8()
            .pixels()
            .all(|p| p[3] == 0)
    );
    request.build_stage = Some(BuildStage { visible_layers: 2 });
    request.destination = dir.join("partial.png");
    let png = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    assert!(
        image::open(&png.path)
            .unwrap()
            .to_rgba8()
            .pixels()
            .filter(|p| p[3] > 128)
            .count()
            > 500
    );
    request.kind = ExportKind::Diagram;
    request.size = 256;
    request.destination = dir.join("partial-sheet.png");
    run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    request.kind = ExportKind::Gif;
    request.size = 160;
    request.transparent = false;
    request.frame_count = 4;
    request.destination = dir.join("partial-turntable.gif");
    let fixed = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    assert_eq!(fixed.frame_count, 4);
    request.kind = ExportKind::Gif;
    request.size = 160;
    request.transparent = false;
    request.build_stage = None;
    request.animation = AnimationMode::BuildUp;
    request.frame_count = 4;
    request.destination = dir.join("growth.gif");
    let gif = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    assert_eq!(gif.frame_count, 4);
    assert_eq!(gif.duration_ms, 200);
    let mut decoder = gif::DecodeOptions::new();
    decoder.set_color_output(gif::ColorOutput::RGBA);
    let mut reader = decoder
        .read_info(fs::File::open(&gif.path).unwrap())
        .unwrap();
    let mut images = Vec::new();
    while let Some(frame) = reader.read_next_frame().unwrap() {
        images.push(frame.buffer.to_vec());
    }
    assert_eq!(images.len(), 4);
    assert!(
        images.windows(2).all(|pair| pair[0] != pair[1]),
        "encoded build stages did not grow"
    );
    request.kind = ExportKind::Mp4;
    request.destination = dir.join("growth.mp4");
    let mp4 = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    assert_eq!(mp4.frame_count, 4);
    assert_eq!(mp4.duration_ms, 200);
    let mut checked_progression = false;
    let mut checked_partial = false;
    let mut checked_fixed_animation = false;
    for entry in fs::read_dir(&request.job_dir).unwrap() {
        let path = entry.unwrap().path().join("frames/render-result.json");
        if !path.is_file() {
            continue;
        }
        let data: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        if data["frame_count"] == 4 {
            let stages = data["frame_build_analysis"].as_array().unwrap();
            if stages.iter().all(|stage| stage["z_top_mm"] == 2.) {
                assert!(stages.iter().all(|stage| stage["capped"] == true
                    && (stage["remaining_volume_mm3"].as_f64().unwrap() - 24.).abs() < 0.001));
                checked_fixed_animation = true;
                continue;
            }
            for (i, stage) in stages.iter().enumerate() {
                assert_eq!(stage["z_top_mm"], (i + 1) as f64);
                assert!(
                    (stage["remaining_volume_mm3"].as_f64().unwrap() - 12. * (i + 1) as f64).abs()
                        < 0.001
                );
            }
            assert_eq!(stages[0]["capped"], true);
            checked_progression = true;
        } else if data["build_analysis"]["z_top_mm"] == 2. {
            assert_eq!(data["build_analysis"]["capped"], true);
            checked_partial = true;
        }
    }
    assert!(checked_progression && checked_partial && checked_fixed_animation);
    assert_eq!(fs::read(source).unwrap(), before);
    fs::write(dir.join("result.json"),serde_json::to_vec_pretty(&serde_json::json!({"png":png,"gif":gif,"mp4":mp4,"fixed_turntable":fixed,"solid_cap_in_final_outputs":true,"source_unchanged":true})).unwrap()).unwrap();
    println!("Solid encoded export proof: {}", dir.display());
}
