use std::{fs, sync::Arc, time::Duration};
use turninator::{
    cache::CacheStore,
    export::ExportKind,
    mesh::Mesh,
    render::ToolPaths,
    session::{RenderEvent, RenderService, RenderTask, default_request},
};
#[test]
fn snapshot_bytes_count_toward_quota_before_any_snapshot_write() {
    let root =
        std::env::temp_dir().join(format!("turninator-snapshot-quota-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let cache = CacheStore::new(root.join("jobs")).unwrap();
    let job = cache.create_job("snapshot-budget").unwrap();
    let output = root.join("keep.png");
    fs::write(&output, b"sentinel").unwrap();
    let mut positions = Vec::new();
    let mut triangles = Vec::new();
    for i in 0..10_000u32 {
        positions.extend([[0., 0., i as f64], [1., 0., i as f64], [0., 1., i as f64]]);
        triangles.push([i * 3, i * 3 + 1, i * 3 + 2]);
    }
    let tools = ToolPaths {
        blender: root.join("missing-blender.exe"),
        ffmpeg: root.join("missing-ffmpeg.exe"),
        ffprobe: root.join("missing-ffprobe.exe"),
    };
    let service = RenderService::new();
    service
        .submit(RenderTask {
            id: 43,
            revision: 9,
            mesh: Arc::new(Mesh {
                positions,
                triangles,
            }),
            scene: serde_json::json!({}),
            request: default_request(
                job.clone(),
                output.clone(),
                ExportKind::Still,
                "eevee".into(),
                64,
                16,
                false,
            ),
            preview: false,
            tools,
            // Old pixel-only reservation was 131072 bytes; mesh alone is 480020.
            cache_budget_bytes: 200_000,
        })
        .unwrap();
    match service.events.recv_timeout(Duration::from_secs(3)).unwrap() {
        RenderEvent::Failed { message, .. } => assert!(message.contains("budget"), "{message}"),
        _ => panic!("snapshot quota failure expected"),
    }
    assert!(!job.join("mesh.bin").exists());
    assert!(!job.join("scene.json").exists());
    assert_eq!(fs::read(output).unwrap(), b"sentinel");
    drop(service);
    cache.remove_job(&job, None).unwrap();
}
#[test]
fn render_actor_rejects_storage_budget_before_starting_a_worker_or_touching_output() {
    let root = std::env::temp_dir().join(format!("turninator-actor-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let cache = CacheStore::new(root.join("jobs")).unwrap();
    let job = cache.create_job("small-budget").unwrap();
    let output = root.join("keep.png");
    fs::write(&output, b"sentinel").unwrap();
    let service = RenderService::new();
    service
        .submit(RenderTask {
            id: 42,
            revision: 8,
            mesh: Arc::new(Mesh {
                positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
                triangles: vec![[0, 1, 2]],
            }),
            scene: serde_json::json!({}),
            request: default_request(
                job.clone(),
                output.clone(),
                ExportKind::Still,
                "eevee".into(),
                768,
                16,
                false,
            ),
            preview: false,
            tools: ToolPaths::default(),
            cache_budget_bytes: 1,
        })
        .unwrap();
    match service.events.recv_timeout(Duration::from_secs(2)).unwrap() {
        RenderEvent::Failed { id, message } => {
            assert_eq!(id, 42);
            assert!(message.contains("budget"));
        }
        _ => panic!("budget failure expected"),
    }
    assert!(!job.join("mesh.bin").exists());
    assert_eq!(fs::read(output).unwrap(), b"sentinel");
    drop(service);
    cache.remove_job(&job, None).unwrap();
}
