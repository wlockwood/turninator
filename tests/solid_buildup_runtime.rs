//! Opt-in test of the published outputs, beyond the geometry/renderer helpers.
#[test]
#[ignore = "requires installed Blender/GPU and FFmpeg; verifies solid stages in final PNG/GIF/MP4/sheet"]
fn solid_stages_reach_final_encoded_outputs() {
    prove_stages(false);
}

#[test]
#[ignore = "requires installed Blender/GPU and FFmpeg; verifies stepped STL in final PNG/GIF/MP4/sheet"]
fn stepped_stages_reach_final_encoded_outputs() {
    prove_stages(true);
}

fn prove_stages(stepped: bool) {
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
        stepped_envelope: stepped,
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
        selection: turninator::render::RenderSelection::from_legacy("eevee", 4).unwrap(),
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
    let images = decoded_gif_frames(&gif.path);
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
    if stepped {
        let fixed_images = decoded_gif_frames(&fixed.path);
        assert_eq!(fixed_images.len(), 4);
        request.kind = ExportKind::Still;
        request.animation = AnimationMode::Turntable;
        for i in 0..4 {
            for (label, count, angle, encoded) in [
                ("growth", i + 1, 0., &images[i]),
                ("fixed", 2, i as f64 * 90., &fixed_images[i]),
            ] {
                request.build_stage = Some(BuildStage {
                    visible_layers: count as u32,
                });
                request.start_angle_degrees = angle;
                request.destination = dir.join(format!("{label}-reference-{i}.png"));
                let reference = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
                let expected = image::open(reference.path).unwrap().to_rgba8().into_raw();
                let error = expected
                    .iter()
                    .zip(encoded)
                    .map(|(a, b)| a.abs_diff(*b) as f64)
                    .sum::<f64>()
                    / expected.len() as f64;
                assert!(
                    error < 3.,
                    "{label} frame {i} differs from independent fixed-stage PNG by {error}"
                );
            }
        }
    }
    let mut checked_progression = false;
    let mut checked_partial = false;
    let mut checked_fixed_animation = false;
    for entry in fs::read_dir(&request.job_dir).unwrap() {
        let path = entry.unwrap().path().join("frames/render-result.json");
        if !path.is_file() {
            continue;
        }
        let data: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        if stepped {
            assert_eq!(data["build_layer_count"], 4);
            continue;
        }
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
    if !stepped {
        assert!(checked_progression && checked_partial && checked_fixed_animation);
    }
    assert_eq!(fs::read(source).unwrap(), before);
    fs::write(dir.join("result.json"),serde_json::to_vec_pretty(&serde_json::json!({"stepped":stepped,"png":png,"gif":gif,"mp4":mp4,"fixed_turntable":fixed,"solid_cap_in_final_outputs":true,"source_unchanged":true})).unwrap()).unwrap();
    println!("Solid encoded export proof: {}", dir.display());
}

fn decoded_gif_frames(path: &std::path::Path) -> Vec<Vec<u8>> {
    use image::AnimationDecoder;
    let decoder = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(
        std::fs::File::open(path).unwrap(),
    ))
    .unwrap();
    decoder
        .into_frames()
        .collect_frames()
        .unwrap()
        .into_iter()
        .map(|frame| frame.into_buffer().into_raw())
        .collect()
}
