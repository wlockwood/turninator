use std::{fs, sync::Arc, time::Duration};
use turninator::{
    cache::CacheStore,
    export::ExportKind,
    mesh::Mesh,
    render::ToolPaths,
    session::{RenderEvent, RenderService, RenderTask, default_request},
};
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
