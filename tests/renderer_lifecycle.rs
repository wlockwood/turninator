use std::sync::atomic::Ordering;
use turninator::{render::BackendId, session::JobTicket};

#[test]
fn cancelled_job_does_not_cancel_successor() {
    let first = JobTicket::new(1, 4, BackendId::BlenderEevee);
    first.cancel.store(true, Ordering::Release);
    let second = JobTicket::new(2, 4, BackendId::BlenderEevee);
    assert!(first.cancel.load(Ordering::Acquire));
    assert!(!second.cancel.load(Ordering::Acquire));
    assert!(!first.accepts(1, 4, BackendId::BlenderEevee));
    assert!(second.accepts(2, 4, BackendId::BlenderEevee));
}

#[test]
fn late_completion_rejected_after_engine_or_model_switch() {
    let ticket = JobTicket::new(3, 7, BackendId::Wgpu);
    assert!(ticket.accepts(3, 7, BackendId::Wgpu));
    assert!(!ticket.accepts(2, 7, BackendId::Wgpu));
    assert!(!ticket.accepts(3, 6, BackendId::Wgpu));
    assert!(!ticket.accepts(3, 7, BackendId::BlenderEevee));
}

#[test]
fn actor_cancellation_replacement_and_owner_thread_destruction() {
    use std::{
        collections::BTreeMap,
        sync::{Arc, atomic::AtomicUsize, mpsc},
        time::Duration,
    };
    use turninator::{
        Result,
        export::{ExportKind, ExportRequest, encode::Scratch},
        mesh::Mesh,
        presentation::{SceneSettings, render_scene},
        render::{RenderSelection, ToolPaths, contract::*, registry::PreparedBackend},
        session::{RenderEvent, RenderService, RenderTask},
    };
    struct Blocking {
        identity: BackendIdentity,
        started: mpsc::Sender<u64>,
        dropped: Arc<AtomicUsize>,
        owner: std::thread::ThreadId,
    }
    impl Drop for Blocking {
        fn drop(&mut self) {
            assert_eq!(self.owner, std::thread::current().id());
            self.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl RenderBackend for Blocking {
        fn id(&self) -> BackendId {
            self.identity.backend
        }
        fn identity(&self) -> BackendIdentity {
            self.identity.clone()
        }
        fn assess(
            &self,
            _: &RenderSceneSnapshot,
            _: &ExportRequest,
        ) -> Vec<turninator::render::capabilities::FeatureSupport> {
            vec![]
        }
        fn render_sequence(
            &mut self,
            scene: &RenderSceneSnapshot,
            plan: &FramePlan,
            ctx: &RenderContext<'_>,
            emit: &mut dyn FnMut(FrameArtifact) -> Result<()>,
        ) -> Result<RenderReport> {
            assert_eq!(self.owner, std::thread::current().id());
            assert_eq!(scene.mesh.triangles.len(), 1);
            self.started.send(ctx.job_id).unwrap();
            if ctx.job_id == 1 {
                while !ctx.cancel.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                return Err("cancelled old job".into());
            }
            assert!(!ctx.cancel.load(Ordering::Acquire));
            for value in [1, 2] {
                (ctx.progress)(RenderProgress {
                    key: None,
                    completed_work: 0,
                    total_work: None,
                    stage: "partial".into(),
                    preview: Some(RgbaFrame {
                        width: 1,
                        height: 1,
                        bytes: vec![value, 0, 0, 255],
                    }),
                });
            }
            for f in &plan.frames {
                emit(FrameArtifact::Image {
                    key: f.key,
                    image: RgbaFrame {
                        width: f.width,
                        height: f.height,
                        bytes: vec![255; (f.width * f.height * 4) as usize],
                    },
                })?;
            }
            Ok(RenderReport {
                frame_count: plan.frames.len() as u32,
                warnings: vec![],
                identity: self.identity.clone(),
                quality: ctx.selection.quality.clone(),
                analyses: BTreeMap::new(),
                build_analyses: BTreeMap::new(),
                phase_timings: BTreeMap::new(),
            })
        }
    }
    let scratch = Scratch::new(&std::env::temp_dir()).unwrap();
    let cache = turninator::cache::CacheStore::new(scratch.0.join("jobs")).unwrap();
    let mesh = Arc::new(Mesh {
        positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 1.]],
        triangles: vec![[0, 1, 2]],
    });
    let (started, starts) = mpsc::channel();
    let dropped = Arc::new(AtomicUsize::new(0));
    let drop_counter = dropped.clone();
    let ui_thread = std::thread::current().id();
    let service = RenderService::with_backend_factory(move |selection, _, _, _, _, cancel| {
        assert_ne!(ui_thread, std::thread::current().id());
        assert!(!cancel.load(Ordering::Acquire));
        let identity = BackendIdentity {
            backend: selection.backend,
            adapter_recipe_version: 1,
            runtime_digest: "test".into(),
            bridge_digest: None,
            shader_digest: None,
            material_digest: None,
            color_digest: "test".into(),
            denoiser_digest: None,
            device: "test".into(),
            api: "test".into(),
            driver: "test".into(),
        };
        let mut selection = selection.clone();
        selection.device = Some(identity.device.clone());
        Ok(PreparedBackend {
            backend: Box::new(Blocking {
                identity: identity.clone(),
                started: started.clone(),
                dropped: drop_counter.clone(),
                owner: std::thread::current().id(),
            }),
            identity,
            effective_selection: selection,
            assessment: vec![],
            preparation_timings: BTreeMap::new(),
        })
    });
    let task = |id, preview, engine| {
        let job = cache.create_job(&format!("job{id}")).unwrap();
        RenderTask {
            id,
            revision: id + 10,
            mesh: mesh.clone(),
            scene: render_scene(
                &SceneSettings::default(),
                &job.join("mesh.bin"),
                "original",
                &[],
                None,
            )
            .unwrap(),
            request: ExportRequest {
                job_dir: job.clone(),
                scene_path: job.join("scene.json"),
                destination: scratch.0.join(format!("out{id}.png")),
                kind: ExportKind::Still,
                size: 16,
                selection: RenderSelection::for_backend(engine, true),
                ..Default::default()
            },
            preview,
            tools: ToolPaths {
                blender: "absent".into(),
                ffmpeg: "absent".into(),
                ffprobe: "absent".into(),
            },
            cache_budget_bytes: 100_000_000,
            memory: Default::default(),
            generation: 0,
        }
    };
    service.submit(task(1, true, BackendId::Wgpu)).unwrap();
    assert_eq!(starts.recv_timeout(Duration::from_secs(3)).unwrap(), 1);
    service.cancel();
    service.submit(task(2, false, BackendId::Filament)).unwrap();
    assert_eq!(starts.recv_timeout(Duration::from_secs(3)).unwrap(), 2);
    let mut cancelled = false;
    loop {
        match service.events.recv_timeout(Duration::from_secs(3)).unwrap() {
            RenderEvent::Failed {
                id: 1,
                revision: 11,
                engine: BackendId::Wgpu,
                message,
                generation: 0,
            } => {
                assert!(message.contains("cancelled"));
                cancelled = true;
            }
            RenderEvent::Completed {
                id: 2,
                revision: 12,
                engine: BackendId::Filament,
                result,
                ..
            } => {
                assert!(cancelled);
                assert!(result.path.exists());
                break;
            }
            RenderEvent::Completed { id: 1, .. } => panic!("cancelled completion escaped"),
            RenderEvent::Failed { message, .. } => panic!("successor failed: {message}"),
            _ => {}
        }
    }
    assert!(!scratch.0.join("out1.png").exists());
    assert_eq!(
        service.take_partial().unwrap().1.bytes[0],
        2,
        "partial slot must retain newest image even inside throttle interval"
    );
    assert_eq!(mesh.triangles, vec![[0, 1, 2]]);
    drop(service);
    assert_eq!(dropped.load(Ordering::SeqCst), 2);
}
