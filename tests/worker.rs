use turninator::render::worker::{decode_event, quote_windows_argument};

#[test]
fn windows_arguments_preserve_spaces_quotes_and_trailing_slashes() {
    assert_eq!(quote_windows_argument("hello"), "\"hello\"");
    assert_eq!(quote_windows_argument("a b\\"), "\"a b\\\\\"");
    assert_eq!(quote_windows_argument("say\"hi"), "\"say\\\"hi\"");
}

#[test]
fn malformed_protocol_fails_before_request_filtering() {
    for input in ["[]", "{", r#"{"protocol_version":2}"#] {
        assert!(decode_event(input).is_err());
    }
    let valid = r#"{"protocol_version":1,"request_id":"x","scene_revision":2,"kind":"complete","payload":{}}"#;
    assert_eq!(decode_event(valid).unwrap()["scene_revision"], 2);
}

#[test]
#[ignore = "requires installed Blender and GPU; writes artifacts/native-backend-probe"]
fn native_worker_exports_diagram_and_survives_request_errors() {
    use serde_json::json;
    use std::{
        fs,
        path::Path,
        sync::atomic::{AtomicBool, Ordering},
    };
    use turninator::{
        export::{ExportKind, ExportRequest, run_export},
        render::{ToolPaths, WorkerClient},
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("artifacts/native-backend-probe");
    fs::create_dir_all(&dir).unwrap();
    let scene = root.join("artifacts/verified-models/model-0/scene.json");
    assert!(scene.exists());
    let tools = ToolPaths::default();
    let cancel = AtomicBool::new(false);
    let mut worker = WorkerClient::spawn(&tools).unwrap();
    let probe = worker
        .request(
            json!({"request_id":"probe","scene_revision":1,"kind":"probe"}),
            &cancel,
            |_| {},
        )
        .unwrap();
    assert!(probe["payload"]["blender_version"].as_str().is_some());
    assert!(
        worker
            .request(
                json!({"request_id":"bad","scene_revision":1,"kind":"bad"}),
                &cancel,
                |_| {}
            )
            .is_err()
    );
    let request = ExportRequest {
        job_dir: dir.join("jobs"),
        scene_path: scene,
        destination: dir.join("four-view.png"),
        kind: ExportKind::Diagram,
        size: 2048,
        samples: 8,
        ..ExportRequest::default()
    };
    let result = run_export(&mut worker, &tools, &request, &cancel, |event| {
        eprintln!("{event:?}")
    })
    .unwrap();
    assert_eq!((result.width, result.height), (2048, 2048));
    worker
        .request(
            json!({"request_id":"after","scene_revision":2,"kind":"probe"}),
            &cancel,
            |_| {},
        )
        .unwrap();
    cancel.store(true, Ordering::Release);
    assert!(
        worker
            .request(
                json!({"request_id":"cancel","scene_revision":3,"kind":"probe"}),
                &cancel,
                |_| {}
            )
            .is_err()
    );
    assert!(!worker.is_running());
}

#[test]
#[ignore = "requires installed Blender/OptiX; writes artifacts/native-material-probe"]
fn native_materials_alpha_mp4_and_busy_cancellation() {
    use serde_json::json;
    use std::{
        fs,
        path::Path,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::Duration,
    };
    use turninator::{
        export::{ExportKind, ExportRequest, run_export},
        render::{ToolPaths, WorkerClient},
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("artifacts/native-material-probe");
    fs::create_dir_all(&dir).unwrap();
    let scene: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("artifacts/verified-models/model-0/scene.json")).unwrap(),
    )
    .unwrap();
    let tools = ToolPaths::default();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut worker = WorkerClient::spawn(&tools).unwrap();
    worker
        .request(
            json!({"request_id":"probe","scene_revision":1,"kind":"probe"}),
            &cancel,
            |_| {},
        )
        .unwrap();
    for preset in ["matte_pla", "clay", "silk_pla", "glossy_plastic", "resin"] {
        let mut spec = scene.clone();
        spec["material_preset"] = json!(preset);
        spec["studio_preset"] = json!("dark");
        let scene_path = dir.join(format!("{preset}.json"));
        fs::write(&scene_path, serde_json::to_vec_pretty(&spec).unwrap()).unwrap();
        let request = ExportRequest {
            job_dir: dir.join("jobs"),
            scene_path,
            destination: dir.join(format!("{preset}.png")),
            kind: ExportKind::Still,
            size: 128,
            samples: 8,
            engine: if preset == "resin" { "cycles" } else { "eevee" }.into(),
            transparent: true,
            ..ExportRequest::default()
        };
        run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
        let image = image::open(&request.destination).unwrap().to_rgba8();
        assert!(image.pixels().any(|p| p[3] == 0));
        assert!(image.pixels().any(|p| p[3] > 128));
    }
    let request = ExportRequest {
        job_dir: dir.join("jobs"),
        scene_path: dir.join("matte_pla.json"),
        destination: dir.join("turntable.mp4"),
        kind: ExportKind::Mp4,
        size: 128,
        samples: 8,
        frame_count: 4,
        ..ExportRequest::default()
    };
    let video = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    assert_eq!((video.frame_count, video.duration_ms), (4, 200));
    let epoch = worker.epoch();
    let signal = cancel.clone();
    let timer = thread::spawn(move || {
        thread::sleep(Duration::from_secs(2));
        signal.store(true, Ordering::Release);
    });
    let result=worker.request(json!({"request_id":"cancel-busy","scene_revision":2,"kind":"render","scene_path":request.scene_path,"output_dir":dir.join("cancelled"),"mode":"turntable","engine":"cycles","size":4096,"samples":4096,"frame_count":80}),&cancel,|_|{});
    timer.join().unwrap();
    assert!(result.is_err());
    assert!(!worker.is_running());
    cancel.store(false, Ordering::Release);
    let mut replacement = WorkerClient::spawn(&tools).unwrap();
    assert_ne!(replacement.epoch(), epoch);
    replacement
        .request(
            json!({"request_id":"restart","scene_revision":3,"kind":"probe"}),
            &cancel,
            |_| {},
        )
        .unwrap();
}
#[test]
#[ignore = "requires installed Blender; verifies actual capped section metrics and matched panels"]
fn native_section_cube_has_exact_area_volume_and_matched_panels() {
    use serde_json::json;
    use std::{fs, path::Path, sync::atomic::AtomicBool};
    use turninator::{
        export::{ExportKind, ExportLayout, ExportRequest, JobEvent, SectionSpec, run_export},
        mesh::Mesh,
        render::{ToolPaths, WorkerClient},
    };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/native-section-probe");
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
    mesh.write_binary(&dir.join("cube.meshbin")).unwrap();
    let scene = json!({"schema_version":1,"mesh_path":dir.join("cube.meshbin"),"geometry_kind":"original","layer_height_mm":0.2,"color":[0.04,0.2,0.65],"roughness":0.4,"display_rotation_degrees":[20,0,30],"warnings":[]});
    fs::write(
        dir.join("cube.json"),
        serde_json::to_vec_pretty(&scene).unwrap(),
    )
    .unwrap();
    let tools = ToolPaths::default();
    let mut worker = WorkerClient::spawn(&tools).unwrap();
    let request = ExportRequest {
        job_dir: dir.join("jobs"),
        scene_path: dir.join("cube.json"),
        destination: dir.join("section.gif"),
        kind: ExportKind::SectionTurntable,
        layout: ExportLayout::SideBySide,
        section: Some(SectionSpec {
            axis: 0,
            position_mm: 10.,
            keep_positive: true,
            cap: true,
        }),
        size: 128,
        samples: 8,
        frame_count: 4,
        ..Default::default()
    };
    let mut analysis = None;
    let result = run_export(
        &mut worker,
        &tools,
        &request,
        &AtomicBool::new(false),
        |event| {
            if let JobEvent::Analysis(value) = event {
                analysis = Some(value)
            }
        },
    )
    .unwrap();
    assert_eq!(
        (
            result.width,
            result.height,
            result.frame_count,
            result.duration_ms
        ),
        (256, 128, 4, 200)
    );
    let analysis = analysis.expect("section analysis must reach caller");
    assert!(
        (analysis["cross_section_area_mm2"].as_f64().unwrap() - 120.).abs() < 0.02,
        "{analysis}"
    );
    assert!(
        (analysis["original_volume_mm3"].as_f64().unwrap() - 2400.).abs() < 0.1,
        "{analysis}"
    );
    assert!(
        (analysis["remaining_volume_mm3"].as_f64().unwrap() - 1200.).abs() < 0.1,
        "{analysis}"
    );
}
#[test]
#[ignore = "requires installed Blender; verifies physical bead gaps and uncapped open input"]
fn native_section_bead_voids_and_open_input_are_honest() {
    use serde_json::json;
    use std::{fs, path::Path, sync::atomic::AtomicBool};
    use turninator::{
        export::{ExportKind, ExportRequest, JobEvent, SectionSpec, run_export},
        mesh::Mesh,
        print::beads::{BeadSegment, build_bead_mesh},
        render::{ToolPaths, WorkerClient},
    };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/native-section-beads");
    fs::create_dir_all(&dir).unwrap();
    let segments: Vec<_> = [-3., 3.]
        .into_iter()
        .map(|y| BeadSegment {
            start: [0., y, 0.2],
            end: [20., y, 0.2],
            width_mm: 0.44,
            height_mm: 0.2,
            volume_mm3: 1.6,
            source_line: 1,
            feature: "infill".into(),
            tool: 0,
            layer_index: 0,
            width_source: "test".into(),
            height_source: "test".into(),
        })
        .collect();
    let beads = build_bead_mesh(&segments).unwrap();
    let open = Mesh {
        positions: vec![[0., 0., 0.], [20., 0., 0.], [0., 12., 10.]],
        triangles: vec![[0, 1, 2]],
    };
    let tools = ToolPaths::default();
    let mut worker = WorkerClient::spawn(&tools).unwrap();
    for (name, mesh, kind) in [("beads", beads, "beads"), ("open", open, "original")] {
        mesh.write_binary(&dir.join(format!("{name}.meshbin")))
            .unwrap();
        let scene = json!({"schema_version":1,"mesh_path":dir.join(format!("{name}.meshbin")),"geometry_kind":kind,"layer_height_mm":0.2,"color":[0.04,0.2,0.65],"roughness":0.4,"display_rotation_degrees":[0,0,0],"warnings":[]});
        let scene_path = dir.join(format!("{name}.json"));
        fs::write(&scene_path, serde_json::to_vec_pretty(&scene).unwrap()).unwrap();
        let request = ExportRequest {
            job_dir: dir.join("jobs"),
            scene_path,
            destination: dir.join(format!("{name}.png")),
            kind: ExportKind::SectionStill,
            section: Some(SectionSpec {
                axis: 0,
                position_mm: 10.,
                keep_positive: true,
                cap: true,
            }),
            size: 256,
            samples: 8,
            ..Default::default()
        };
        let mut analysis = None;
        run_export(
            &mut worker,
            &tools,
            &request,
            &AtomicBool::new(false),
            |event| {
                if let JobEvent::Analysis(value) = event {
                    analysis = Some(value)
                }
            },
        )
        .unwrap();
        let analysis = analysis.unwrap();
        if name == "beads" {
            let area = analysis["cross_section_area_mm2"].as_f64().unwrap();
            // Two stadiums: twice (straight middle + circular ends). The
            // polygon approximation is slightly smaller than the analytic area.
            let exact = 2. * (0.2 * (0.44 - 0.2) + std::f64::consts::PI * 0.1f64.powi(2));
            assert!((area - exact).abs() < 0.001, "{analysis}");
            assert!(
                area < 0.2,
                "the 6 mm empty gap must not be filled: {analysis}"
            );
            let remaining = analysis["remaining_volume_mm3"].as_f64().unwrap();
            let original = analysis["original_volume_mm3"].as_f64().unwrap();
            assert!((remaining - original * 0.5).abs() < 0.001, "{analysis}");
        } else {
            assert_eq!(analysis["capped"], false);
            assert!(analysis["cross_section_area_mm2"].is_null());
            assert!(analysis["remaining_volume_mm3"].is_null());
            assert!(!analysis["warnings"].as_array().unwrap().is_empty());
        }
    }
}
#[test]
#[ignore = "requires Blender/OptiX; actual joined multi-tool build-up and AV1 frame reuse"]
fn native_buildup_maps_tools_reveals_geometry_and_reuses_frames_for_av1() {
    use serde_json::json;
    use std::{fs, path::Path, sync::atomic::AtomicBool};
    use turninator::{
        export::{AnimationMode, ExportKind, ExportRequest, JobEvent, VideoCodec, run_export},
        print::beads::{BeadSegment, build_joined_bead_mesh_with_report},
        render::{ToolPaths, WorkerClient},
    };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/native-buildup-probe");
    fs::create_dir_all(&dir).unwrap();
    let segments: Vec<_> = (0..8)
        .map(|layer| BeadSegment {
            start: [0., 0., 0.5 * (layer + 1) as f64],
            end: [8., 0., 0.5 * (layer + 1) as f64],
            width_mm: 1.,
            height_mm: 0.5,
            volume_mm3: 4.,
            source_line: layer + 1,
            feature: "wall".into(),
            tool: if layer < 4 { 0 } else { 1 },
            layer_index: layer as u32,
            width_source: "test".into(),
            height_source: "test".into(),
        })
        .collect();
    let joined = build_joined_bead_mesh_with_report(&segments).unwrap();
    joined
        .mesh
        .write_binary(&dir.join("joined.meshbin"))
        .unwrap();
    let groups:Vec<_>=(0..8).map(|i|json!({"start_triangle":i*128,"end_triangle":(i+1)*128,"tool":if i<4{0}else{1},"color":if i<4{[0.7,0.01,0.01]}else{[0.01,0.08,0.7]},"label":if i<4{"Red"}else{"Blue"}})).collect();
    let layers:Vec<_>=(0..8).map(|i|json!({"start_triangle":i*128,"end_triangle":(i+1)*128,"layer_index":i,"z_top_mm":0.5*(i+1) as f64})).collect();
    let mut scene = json!({"schema_version":1,"mesh_path":dir.join("joined.meshbin"),"geometry_kind":"beads","bead_layout":"joined_stadium_v1","bead_side_face_ranges":joined.smooth_face_ranges,"face_groups":groups,"layer_face_ranges":layers,"layer_height_mm":0.5,"color":[0.04,0.2,0.65],"roughness":0.4,"display_rotation_degrees":[0,0,0],"warnings":[]});
    let scene_path = dir.join("joined.json");
    fs::write(&scene_path, serde_json::to_vec_pretty(&scene).unwrap()).unwrap();
    let tools = ToolPaths::default();
    let mut worker = WorkerClient::spawn(&tools).unwrap();
    let cancel = AtomicBool::new(false);
    let mut request = ExportRequest {
        job_dir: dir.join("jobs"),
        scene_path,
        destination: dir.join("buildup.gif"),
        kind: ExportKind::Gif,
        animation: AnimationMode::BuildUp,
        size: 256,
        samples: 8,
        frame_count: 8,
        ..Default::default()
    };
    let mut frames = Vec::new();
    let output = run_export(&mut worker, &tools, &request, &cancel, |event| {
        if let JobEvent::Frame(path) = event {
            frames.push(path)
        }
    })
    .unwrap();
    assert_eq!((output.frame_count, output.duration_ms), (8, 400));
    let first = image::open(&frames[0]).unwrap().to_rgb8();
    let last = image::open(&frames[7]).unwrap().to_rgb8();
    let blue = |pixels: &image::RgbImage| {
        pixels
            .pixels()
            .filter(|p| p[2] as u32 > p[0] as u32 * 2 && p[2] > 80)
            .count()
    };
    let red = |pixels: &image::RgbImage| {
        pixels
            .pixels()
            .filter(|p| p[0] as u32 > p[2] as u32 * 2 && p[0] > 80)
            .count()
    };
    assert!(
        red(&first) > 100,
        "first deposited layer must use red tool color"
    );
    assert!(
        blue(&first) < 10,
        "future blue layers must be absent from geometry"
    );
    assert!(
        blue(&last) > 100 && red(&last) > 100,
        "completed geometry must contain both tool materials"
    );
    let original_times: Vec<_> = frames
        .iter()
        .map(|p| fs::metadata(p).unwrap().modified().unwrap())
        .collect();
    request.kind = ExportKind::Mp4;
    request.video_codec = VideoCodec::Av1;
    request.destination = dir.join("buildup-av1.mp4");
    let mut reused = 0;
    run_export(&mut worker, &tools, &request, &cancel, |event| {
        if let JobEvent::Progress { stage, .. } = event
            && stage == "Reusing verified frames"
        {
            reused += 1
        }
    })
    .unwrap();
    assert_eq!(reused, 8);
    for (path, time) in frames.iter().zip(original_times) {
        assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), time);
    }
    scene["material_preset"] = json!("resin");
    let resin_path = dir.join("resin.json");
    fs::write(&resin_path, serde_json::to_vec_pretty(&scene).unwrap()).unwrap();
    let complete=worker.request(json!({"request_id":"resin-joined","scene_revision":1,"kind":"render","scene_path":resin_path,"output_dir":dir.join("resin"),"mode":"still","engine":"cycles","size":256,"samples":16,"transparent":true,"reuse_verified_frames":dir.join("resin/render-manifest.json").exists()}),&cancel,|_|{}).unwrap();
    assert!(
        complete["payload"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w
                .as_str()
                .unwrap_or("")
                .contains("internal/overlapping interfaces"))
    );
    let resin = image::open(dir.join("resin/still.png")).unwrap().to_rgba8();
    assert!(resin.pixels().any(|p| p[3] > 128));
    assert!(resin.pixels().any(|p| p[3] == 0));
    // Deliberately overlap a second complete set of shells. The renderer must
    // preserve those interfaces and disclose them, not silently call it fused.
    let mut overlapping = turninator::mesh::Mesh::read_binary(&dir.join("joined.meshbin")).unwrap();
    let vertex_offset = overlapping.positions.len() as u32;
    let triangle_offset = overlapping.triangles.len();
    let extra_positions: Vec<_> = overlapping
        .positions
        .iter()
        .map(|p| [p[0], p[1] + 0.45, p[2]])
        .collect();
    let extra_triangles: Vec<_> = overlapping
        .triangles
        .iter()
        .map(|t| t.map(|i| i + vertex_offset))
        .collect();
    overlapping.positions.extend(extra_positions);
    overlapping.triangles.extend(extra_triangles);
    overlapping
        .write_binary(&dir.join("overlapping.meshbin"))
        .unwrap();
    scene["mesh_path"] = json!(dir.join("overlapping.meshbin"));
    scene.as_object_mut().unwrap().remove("layer_face_ranges");
    let mut ranges = scene["bead_side_face_ranges"].as_array().unwrap().clone();
    ranges.extend(ranges.clone().into_iter().map(|range| {
        json!([
            range[0].as_u64().unwrap() + triangle_offset as u64,
            range[1].as_u64().unwrap() + triangle_offset as u64
        ])
    }));
    scene["bead_side_face_ranges"] = json!(ranges);
    let mut groups = scene["face_groups"].as_array().unwrap().clone();
    groups.extend(groups.clone().into_iter().map(|mut group| {
        group["start_triangle"] =
            json!(group["start_triangle"].as_u64().unwrap() + triangle_offset as u64);
        group["end_triangle"] =
            json!(group["end_triangle"].as_u64().unwrap() + triangle_offset as u64);
        group
    }));
    scene["face_groups"] = json!(groups);
    let overlap_scene = dir.join("resin-overlap.json");
    fs::write(&overlap_scene, serde_json::to_vec_pretty(&scene).unwrap()).unwrap();
    let complete=worker.request(json!({"request_id":"resin-overlap","scene_revision":2,"kind":"render","scene_path":overlap_scene,"output_dir":dir.join("resin-overlap"),"mode":"still","engine":"cycles","size":256,"samples":16,"transparent":true,"reuse_verified_frames":dir.join("resin-overlap/render-manifest.json").exists()}),&cancel,|_|{}).unwrap();
    assert_eq!(complete["payload"]["triangles"], triangle_offset * 2);
    assert!(
        complete["payload"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w
                .as_str()
                .unwrap_or("")
                .contains("not a fused-resin simulation"))
    );
    let overlap_image = image::open(dir.join("resin-overlap/still.png"))
        .unwrap()
        .to_rgba8();
    assert_ne!(resin.as_raw(), overlap_image.as_raw());
}
#[test]
#[ignore = "large GPU acceptance: actual supplied STL 4096 sheet and 80-frame 1920x960 section GIF"]
fn native_large_sheet_and_complete_section_animation() {
    use serde_json::json;
    use std::{fs, path::Path, sync::atomic::AtomicBool, time::Instant};
    use turninator::{
        export::{ExportKind, ExportLayout, ExportRequest, SectionSpec, run_export},
        mesh::Mesh,
        render::{ToolPaths, WorkerClient},
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("artifacts/final-large-outputs");
    fs::create_dir_all(&dir).unwrap();
    let mut scene: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("artifacts/verified-models/model-0/scene.json")).unwrap(),
    )
    .unwrap();
    scene["display_rotation_degrees"] = json!([20., 15., 0.]);
    scene["layer_filtering"] = json!(true);
    let scene_path = dir.join("supplied-model.json");
    fs::write(&scene_path, serde_json::to_vec_pretty(&scene).unwrap()).unwrap();
    let bounds = Mesh::read_binary(Path::new(scene["mesh_path"].as_str().unwrap()))
        .unwrap()
        .bounds()
        .unwrap();
    let tools = ToolPaths::default();
    let mut worker = WorkerClient::spawn(&tools).unwrap();
    let cancel = AtomicBool::new(false);
    let mut request = ExportRequest {
        job_dir: dir.join("jobs"),
        scene_path,
        destination: dir.join("four-view-4096.png"),
        kind: ExportKind::Diagram,
        size: 4096,
        samples: 16,
        ..Default::default()
    };
    let started = Instant::now();
    let sheet = run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    let sheet_seconds = started.elapsed().as_secs_f64();
    assert_eq!((sheet.width, sheet.height), (4096, 4096));
    request.kind = ExportKind::SectionTurntable;
    request.size = 960;
    request.frame_count = 80;
    request.layout = ExportLayout::SideBySide;
    request.section = Some(SectionSpec {
        axis: 0,
        position_mm: (bounds.min[0] + bounds.max[0]) * 0.5,
        keep_positive: true,
        cap: true,
    });
    request.destination = dir.join("section-80x1920x960.gif");
    let started = Instant::now();
    let animation = run_export(&mut worker, &tools, &request, &cancel, |event| {
        if let turninator::export::JobEvent::Progress {
            completed,
            total,
            stage,
        } = event
            && completed % 20 == 0
        {
            eprintln!("{stage}: {completed}/{total}")
        }
    })
    .unwrap();
    let animation_seconds = started.elapsed().as_secs_f64();
    assert_eq!(
        (
            animation.width,
            animation.height,
            animation.frame_count,
            animation.duration_ms
        ),
        (1920, 960, 80, 4000)
    );
    fs::write(dir.join("results.json"),serde_json::to_vec_pretty(&json!({"sheet":sheet,"sheet_seconds":sheet_seconds,"section":animation,"section_seconds":animation_seconds,"display_rotation_degrees":[20,15,0],"layer_height_mm":scene["layer_height_mm"]})).unwrap()).unwrap();
}
#[test]
#[ignore = "GPU appearance acceptance: three complete 80-frame loops and temporal residual measurement"]
fn native_layer_filter_reduces_subpixel_temporal_residuals() {
    use serde_json::json;
    use std::{fs, path::Path, sync::atomic::AtomicBool, time::Instant};
    use turninator::{
        export::{ExportKind, ExportRequest, JobEvent, run_export},
        render::{ToolPaths, WorkerClient},
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("artifacts/layer-filter-proof");
    fs::create_dir_all(&dir).unwrap();
    let source = root.join("artifacts/native-section-probe/cube.meshbin");
    assert!(source.exists(), "section cube fixture must exist");
    let tools = ToolPaths::default();
    let mut worker = WorkerClient::spawn(&tools).unwrap();
    let cancel = AtomicBool::new(false);
    let mut clips = Vec::new();
    let mut timings = Vec::new();
    for name in ["unfiltered", "filtered", "reference"] {
        let scene = json!({"schema_version":1,"mesh_path":source,"geometry_kind":"original","layer_height_mm":0.02,"first_layer_height_mm":0.02,"color":[0.035,0.22,0.52],"roughness":0.42,"display_rotation_degrees":[23,17,0],"layer_appearance":name!="reference","layer_filtering":name!="unfiltered","warnings":[]});
        let scene_path = dir.join(format!("{name}.json"));
        fs::write(&scene_path, serde_json::to_vec_pretty(&scene).unwrap()).unwrap();
        let request = ExportRequest {
            job_dir: dir.join("jobs"),
            scene_path,
            destination: dir.join(format!("{name}.gif")),
            kind: ExportKind::Gif,
            size: 256,
            samples: 16,
            frame_count: 80,
            ..Default::default()
        };
        let mut paths = Vec::new();
        let started = Instant::now();
        run_export(&mut worker, &tools, &request, &cancel, |event| {
            if let JobEvent::Frame(path) = event {
                paths.push(path)
            } else if let JobEvent::Progress {
                completed, stage, ..
            } = event
                && completed % 20 == 0
            {
                eprintln!("{name}: {stage} {completed}")
            }
        })
        .unwrap();
        timings.push(started.elapsed().as_secs_f64());
        clips.push(
            paths
                .iter()
                .map(|path| image::open(path).unwrap().to_rgb8())
                .collect::<Vec<_>>(),
        );
    }
    let residual = |clip: &Vec<image::RgbImage>| -> f64 {
        let mut total = 0u64;
        for i in 0..80 {
            let previous = (i + 79) % 80;
            for ((now, base), (before, oldbase)) in
                clip[i].as_raw().iter().zip(clips[2][i].as_raw()).zip(
                    clip[previous]
                        .as_raw()
                        .iter()
                        .zip(clips[2][previous].as_raw()),
                )
            {
                total += ((*now as i32 - *base as i32) - (*before as i32 - *oldbase as i32))
                    .unsigned_abs() as u64;
            }
        }
        total as f64 / (80. * 256. * 256. * 3.)
    };
    let unfiltered = residual(&clips[0]);
    let filtered = residual(&clips[1]);
    fs::write(dir.join("measurements.json"),serde_json::to_vec_pretty(&json!({"frames":80,"size":256,"pitch_mm":0.02,"display_rotation_degrees":[23,17,0],"metric":"Mean absolute cyclic temporal difference of RGB residual relative to matched smooth reference, 0..255 channel units","unfiltered":unfiltered,"filtered":filtered,"elapsed_seconds":timings})).unwrap()).unwrap();
    assert!(
        unfiltered > 0.01,
        "fixture must expose unresolved bump variation: {unfiltered}"
    );
    assert!(
        filtered < unfiltered * 0.25,
        "filter must reduce temporal residual materially: {filtered} vs {unfiltered}"
    );
    let mut resolved: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("filtered.json")).unwrap()).unwrap();
    resolved["layer_height_mm"] = json!(2.);
    resolved["first_layer_height_mm"] = json!(2.);
    let resolved_path = dir.join("resolved.json");
    fs::write(
        &resolved_path,
        serde_json::to_vec_pretty(&resolved).unwrap(),
    )
    .unwrap();
    let request = ExportRequest {
        job_dir: dir.join("jobs"),
        scene_path: resolved_path,
        destination: dir.join("resolved-pitch.png"),
        kind: ExportKind::Still,
        size: 256,
        samples: 16,
        ..Default::default()
    };
    run_export(&mut worker, &tools, &request, &cancel, |_| {}).unwrap();
    let resolved_pixels = image::open(&request.destination).unwrap().to_rgb8();
    let difference: usize = resolved_pixels
        .as_raw()
        .iter()
        .zip(clips[2][0].as_raw())
        .map(|(a, b)| (*a as i32 - *b as i32).unsigned_abs() as usize)
        .sum();
    assert!(
        difference as f64 / (256. * 256. * 3.) > 0.1,
        "resolved layer detail must remain visible"
    );
}
