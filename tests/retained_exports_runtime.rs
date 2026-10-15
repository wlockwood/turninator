//! Production actor acceptance: separate exports share retained render frames.
#[test]
#[ignore = "requires Blender/GPU and FFmpeg; writes artifacts/retained-frames-proof"]
fn separate_exports_reuse_frames_across_formats_and_restart() {
    use serde_json::{Value, json};
    use std::{
        fs,
        path::PathBuf,
        sync::Arc,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    use turninator::{
        cache::CacheStore,
        export::{ExportKind, ExportRequest, JobEvent, identity::retained_render_key},
        mesh::Mesh,
        render::ToolPaths,
        session::{RenderEvent, RenderService, RenderTask},
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts/retained-frames-proof")
        .join(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis()
                .to_string(),
        );
    fs::create_dir_all(&root).unwrap();
    let cache = CacheStore::new(root.join("jobs")).unwrap();
    let mesh = Arc::new(Mesh {
        positions: vec![[0., 0., 0.], [20., 0., 0.], [0., 12., 0.], [0., 0., 10.]],
        triangles: vec![[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]],
    });
    let mesh_hash = mesh.snapshot_hash().unwrap();
    let mut scene = json!({"schema_version":1,"mesh_path":root.join("pending.bin"),"geometry_kind":"original",
        "display_rotation_degrees":[0,0,0],"color":[0.04,0.2,0.65],"roughness":0.4,
        "layer_height_mm":0.2,"studio_preset":"dark","warnings":[]});
    let mut request = ExportRequest {
        kind: ExportKind::Gif,
        size: 64,
        selection: turninator::render::RenderSelection::from_legacy("eevee", 4).unwrap(),
        frame_count: 4,
        ..Default::default()
    };
    let mut tools = ToolPaths::default();
    if let Some(path) = std::env::var_os("TURNINATOR_TEST_BLENDER") {
        tools.blender = path.into();
    }
    let mut service = RenderService::new();
    let mut results = Vec::new();
    let mut submit = |service: &RenderService,
                      id: u64,
                      scene: &Value,
                      request: &mut ExportRequest,
                      budget: u64| {
        let key = retained_render_key(scene, &mesh_hash, request).unwrap();
        request.job_dir = cache.render_job(&key).unwrap();
        request.scene_path = request.job_dir.join("scene.json");
        let started = Instant::now();
        service
            .submit(RenderTask {
                id,
                revision: 1,
                mesh: mesh.clone(),
                scene: scene.clone(),
                request: request.clone(),
                preview: false,
                tools: tools.clone(),
                cache_budget_bytes: budget,
                memory: Default::default(),
                generation: 0,
            })
            .unwrap();
        let mut reused = 0;
        loop {
            match service
                .events
                .recv_timeout(Duration::from_secs(180))
                .unwrap()
            {
                RenderEvent::Completed { result, .. } => {
                    assert!(result.path.exists());
                    break;
                }
                RenderEvent::Failed { message, .. } => panic!("export {id}: {message}"),
                RenderEvent::Progress {
                    event:
                        JobEvent::Progress {
                            stage, completed, ..
                        },
                    ..
                } if stage.contains("Reusing") => reused = reused.max(completed),
                _ => {}
            }
        }
        results.push(json!({"id":id,"seconds":started.elapsed().as_secs_f64(),"reused":reused,"job":request.job_dir}));
        reused
    };
    request.destination = root.join("first.gif");
    submit(&service, 1, &scene, &mut request, 1_000_000_000);
    let first_job = request.job_dir.clone();
    let render_dir = fs::read_dir(&first_job)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.join("job.json").exists())
        .unwrap();
    let frame = render_dir.join("host-frames/Single-0000.png");
    let bytes = fs::read(&frame).unwrap();
    let modified = fs::metadata(&frame).unwrap().modified().unwrap();
    let details: Value = serde_json::from_slice::<Value>(
        &fs::read(render_dir.join("job.json")).unwrap(),
    )
    .unwrap()["report"]
        .clone();
    request.kind = ExportKind::Mp4;
    request.destination = root.join("second.mp4");
    assert_eq!(submit(&service, 2, &scene, &mut request, 1), 4); // Re-encoding needs no new-frame quota.
    assert_eq!(request.job_dir, first_job);
    assert_eq!(fs::metadata(&frame).unwrap().modified().unwrap(), modified);
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(render_dir.join("job.json")).unwrap()).unwrap()["report"],
        details
    );
    drop(service);
    service = RenderService::new();
    request.kind = ExportKind::Gif;
    request.destination = root.join("after-restart.gif");
    assert_eq!(submit(&service, 3, &scene, &mut request, 1), 4);
    assert_eq!(fs::read(&frame).unwrap(), bytes);
    fs::write(render_dir.join("host-frames/Single-0001.png"), b"broken").unwrap();
    request.destination = root.join("repaired.gif");
    submit(&service, 4, &scene, &mut request, 1_000_000_000);
    let repaired: Value =
        serde_json::from_slice(&fs::read(render_dir.join("job.json")).unwrap()).unwrap();
    assert_eq!(repaired["report"]["rendered_frame_count"], 1);
    assert_eq!(fs::metadata(&frame).unwrap().modified().unwrap(), modified);
    scene["color"] = json!([0.7, 0.1, 0.05]);
    request.destination = root.join("changed-material.gif");
    assert_eq!(submit(&service, 5, &scene, &mut request, 1_000_000_000), 0);
    assert_ne!(request.job_dir, first_job);
    request.kind = ExportKind::Still;
    request.destination = root.join("first-still.png");
    submit(&service, 6, &scene, &mut request, 1_000_000_000);
    let still_job = request.job_dir.clone();
    let still_dir = fs::read_dir(&still_job)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.join("host-frames/Single-0000.png").exists())
        .unwrap();
    let still_modified = fs::metadata(still_dir.join("host-frames/Single-0000.png"))
        .unwrap()
        .modified()
        .unwrap();
    request.destination = root.join("repeat-still.png");
    assert_eq!(submit(&service, 7, &scene, &mut request, 1), 1);
    assert_eq!(request.job_dir, still_job);
    assert_eq!(
        fs::metadata(still_dir.join("host-frames/Single-0000.png"))
            .unwrap()
            .modified()
            .unwrap(),
        still_modified
    );
    assert_eq!(
        fs::read(root.join("first-still.png")).unwrap(),
        fs::read(root.join("repeat-still.png")).unwrap()
    );
    fs::write(
        root.join("acceptance.json"),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();
    eprintln!("Retained frame acceptance: {}", root.display());
}
