use serde_json::json;
use turninator::{
    export::jobs::{diagram_cameras, diagram_measurements},
    mesh::Mesh,
};

fn tetrahedron() -> Mesh {
    Mesh {
        positions: vec![[0., 0., 0.], [10., 0., 0.], [0., 10., 0.], [0., 0., 10.]],
        triangles: vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
    }
}

#[test]
fn dimensions_measure_actual_projected_geometry_and_choose_one_largest_extent() {
    let mut mesh = tetrahedron();
    // Unreferenced vertices affect framing, but cannot inflate a visible dimension.
    mesh.positions.push([50., 50., 50.]);
    let cameras = diagram_cameras([50.; 3], 400, 380);
    let measurements = diagram_measurements(&mesh, &cameras, None).unwrap();
    for index in [0, 2, 3] {
        assert!((measurements[index].unwrap().length_mm - 10.).abs() < 1e-9);
    }
    let iso = measurements[1].unwrap();
    assert_eq!(iso.axis, 1);
    assert!((iso.length_mm - 30. / 6.0_f64.sqrt()).abs() < 1e-9);
    assert!((iso.max_mm[0] - iso.min_mm[0] - 10. / 2.0_f64.sqrt()).abs() < 1e-9);
}

#[test]
fn dimensions_follow_selected_geometry_and_omit_an_empty_build() {
    let mesh = tetrahedron();
    let cameras = diagram_cameras([10.; 3], 400, 380);
    let bottom = diagram_measurements(&mesh, &cameras, Some(&[true, false, false, false])).unwrap();
    assert_eq!(bottom[2].unwrap().axis, 0);
    assert_eq!(bottom[2].unwrap().length_mm, 10.);
    assert_eq!(
        bottom[2].unwrap().max_mm[1] - bottom[2].unwrap().min_mm[1],
        0.
    );
    assert!(
        diagram_measurements(&mesh, &cameras, Some(&[false; 4]))
            .unwrap()
            .iter()
            .all(Option::is_none)
    );
    assert!(diagram_measurements(&mesh, &cameras, Some(&[true])).is_err());
    let mut invalid = cameras;
    invalid[0]["direction"] = json!([0., 0., 0.]);
    assert!(diagram_measurements(&mesh, &invalid, None).is_err());
}

#[test]
#[ignore = "requires Blender/GPU; actual annotated four-view export and retained dimension/camera checks"]
fn annotated_sheet_renders_requested_quadrants_at_shared_scale() {
    use std::{fs, path::Path, sync::atomic::AtomicBool};
    use turninator::{
        export::{ExportKind, ExportRequest, run_export},
        presentation::{SceneSettings, render_scene},
        render::{ToolPaths, WorkerClient},
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("artifacts/annotated-sheet-proof");
    fs::create_dir_all(&dir).unwrap();
    let mesh = Mesh {
        positions: vec![
            [0., 0., 0.],
            [20., 0., 0.],
            [20., 12., 0.],
            [0., 12., 0.],
            [0., 0., 80.],
            [20., 0., 80.],
            [20., 12., 80.],
            [0., 12., 80.],
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
    let mesh_path = dir.join("mesh.bin");
    mesh.write_binary(&mesh_path).unwrap();
    let original = fs::read(&mesh_path).unwrap();
    let scene = render_scene(&SceneSettings::default(), &mesh_path, "original", &[], None).unwrap();
    let scene_path = dir.join("scene.json");
    fs::write(&scene_path, serde_json::to_vec(&scene).unwrap()).unwrap();
    let mut tools = ToolPaths::default();
    let maintained = root.join("artifacts/runtime-tools/blender-4.5.14-windows-x64/blender.exe");
    if maintained.is_file() {
        tools.blender = maintained;
    }
    let mut worker = WorkerClient::spawn(&tools).unwrap();
    let request = ExportRequest {
        job_dir: dir.join("jobs"),
        scene_path,
        destination: dir.join("annotated-sheet.png"),
        kind: ExportKind::Diagram,
        size: 1024,
        samples: 8,
        ..Default::default()
    };
    let result = run_export(
        &mut worker,
        &tools,
        &request,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!((result.width, result.height), (1024, 1024));
    let job = fs::read_dir(&request.job_dir)
        .unwrap()
        .map(|p| p.unwrap().path())
        .find(|p| p.join("four-view-dimensions.json").is_file())
        .unwrap();
    let dimensions: serde_json::Value =
        serde_json::from_slice(&fs::read(job.join("four-view-dimensions.json")).unwrap()).unwrap();
    assert_eq!(dimensions["views"], json!(["TOP", "ISO", "FRONT", "RIGHT"]));
    assert_eq!(dimensions["units"], "mm");
    for (index, expected) in [(0, 20.), (1, 192. / 6.0_f64.sqrt()), (2, 80.), (3, 80.)] {
        assert!(
            (dimensions["measurements"][index]["length_mm"]
                .as_f64()
                .unwrap()
                - expected)
                .abs()
                < 1e-8
        );
    }
    let mut span = None;
    for index in 0..4 {
        let scene: serde_json::Value =
            serde_json::from_slice(&fs::read(job.join(format!("diagram-{index}.json"))).unwrap())
                .unwrap();
        let current = scene["camera"]["ortho_span_mm"].as_f64().unwrap();
        if let Some(expected) = span {
            assert_eq!(current, expected);
        } else {
            span = Some(current);
        }
    }
    let recipe: serde_json::Value =
        serde_json::from_slice(&fs::read(job.join("job.json")).unwrap()).unwrap();
    assert_eq!(recipe["recipe"]["diagram_recipe"]["version"], 2);
    assert_eq!(fs::read(mesh_path).unwrap(), original);
    fs::write(
        dir.join("result.json"),
        serde_json::to_vec_pretty(
            &json!({"export":result.path,"dimensions":dimensions,"shared_span_mm":span}),
        )
        .unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "requires Blender/GPU and supplied spiral egg; actual 4096 annotated sheet"]
fn supplied_model_exports_an_annotated_4k_sheet() {
    use std::{fs, path::Path, sync::atomic::AtomicBool};
    use turninator::{
        export::{ExportKind, ExportRequest, run_export},
        presentation::{ImportSettings, SceneSettings, load_model, model_scene},
        render::{ToolPaths, WorkerClient},
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("artifacts/annotated-sheet-proof/spiral-egg");
    fs::create_dir_all(&dir).unwrap();
    let model = load_model(
        &root.join("test-models/spiral egg - A.stl"),
        &ImportSettings::default(),
    )
    .unwrap();
    model.mesh.write_binary(&dir.join("mesh.bin")).unwrap();
    let scene = model_scene(
        &model,
        &SceneSettings::default(),
        &dir.join("mesh.bin"),
        None,
    )
    .unwrap();
    let scene_path = dir.join("scene.json");
    fs::write(&scene_path, serde_json::to_vec(&scene).unwrap()).unwrap();
    let mut tools = ToolPaths::default();
    let maintained = root.join("artifacts/runtime-tools/blender-4.5.14-windows-x64/blender.exe");
    if maintained.is_file() {
        tools.blender = maintained;
    }
    let mut worker = WorkerClient::spawn(&tools).unwrap();
    let request = ExportRequest {
        job_dir: dir.join("jobs"),
        scene_path,
        destination: dir.join("four-view.png"),
        kind: ExportKind::Diagram,
        size: 4096,
        samples: 8,
        ..Default::default()
    };
    let result = run_export(
        &mut worker,
        &tools,
        &request,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!((result.width, result.height), (4096, 4096));
    let job = fs::read_dir(&request.job_dir)
        .unwrap()
        .map(|p| p.unwrap().path())
        .find(|p| p.join("four-view-dimensions.json").is_file())
        .unwrap();
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(job.join("four-view-dimensions.json")).unwrap()).unwrap();
    assert!(
        metadata["measurements"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["length_mm"].as_f64().is_some_and(|v| v > 0.))
    );
    fs::write(
        dir.join("result.json"),
        serde_json::to_vec_pretty(
            &json!({"export":result.path,"source_hash":model.source_hash,"dimensions":metadata}),
        )
        .unwrap(),
    )
    .unwrap();
}
