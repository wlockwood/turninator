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
