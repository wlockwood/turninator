use crate::{
    Result,
    export::{ExportKind, ExportRequest, ExportResult, JobEvent},
    mesh::Mesh,
    render::{
        BackendId, ToolPaths,
        contract::*,
        registry::{BackendRegistry, PreparedBackend},
    },
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::Instant,
};

pub struct RenderTask {
    pub id: u64,
    pub revision: u64,
    pub mesh: Arc<Mesh>,
    pub scene: Value,
    pub request: ExportRequest,
    pub preview: bool,
    pub tools: ToolPaths,
    pub cache_budget_bytes: u64,
}
#[derive(Clone, Debug)]
pub struct JobTicket {
    pub id: u64,
    pub revision: u64,
    pub engine: BackendId,
    pub cancel: Arc<AtomicBool>,
}
impl JobTicket {
    pub fn new(id: u64, revision: u64, engine: BackendId) -> Self {
        Self {
            id,
            revision,
            engine,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn accepts(&self, id: u64, revision: u64, engine: BackendId) -> bool {
        self.id == id
            && self.revision == revision
            && self.engine == engine
            && !self.cancel.load(Ordering::Acquire)
    }
}
pub enum RenderEvent {
    Progress {
        id: u64,
        revision: u64,
        engine: BackendId,
        event: JobEvent,
    },
    Completed {
        id: u64,
        revision: u64,
        engine: BackendId,
        preview: bool,
        result: ExportResult,
    },
    Failed {
        id: u64,
        revision: u64,
        engine: BackendId,
        message: String,
    },
}
#[derive(Default)]
struct Queue {
    pending: Option<(RenderTask, JobTicket, bool)>,
    active: Option<(JobTicket, bool)>,
    shutdown: bool,
}
pub struct RenderService {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    pub events: Receiver<RenderEvent>,
    partial: Arc<Mutex<Option<(JobTicket, RgbaFrame)>>>,
    last_partial_delivery: Mutex<Option<Instant>>,
    thread: Option<JoinHandle<()>>,
}
impl Default for RenderService {
    fn default() -> Self {
        Self::new()
    }
}
type BackendFactory = dyn FnMut(
        &crate::render::RenderSelection,
        &RenderSceneSnapshot,
        &ExportRequest,
        &ToolPaths,
        &RenderLimits,
        &AtomicBool,
    ) -> Result<PreparedBackend>
    + Send;
impl RenderService {
    pub fn new() -> Self {
        Self::with_backend_factory(|selection, scene, request, tools, limits, cancel| {
            BackendRegistry::default().prepare(selection, scene, request, tools, limits, cancel)
        })
    }
    /// The factory crosses the thread boundary; its renderer is created and destroyed only on the actor.
    /// This also permits deterministic backend conformance tests without registry substitutes.
    pub fn with_backend_factory(
        factory: impl FnMut(
            &crate::render::RenderSelection,
            &RenderSceneSnapshot,
            &ExportRequest,
            &ToolPaths,
            &RenderLimits,
            &AtomicBool,
        ) -> Result<PreparedBackend>
        + Send
        + 'static,
    ) -> Self {
        let queue = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let actor_queue = queue.clone();
        let partial = Arc::new(Mutex::new(None));
        let actor_partial = partial.clone();
        let (events, event_receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let mut factory: Box<BackendFactory> = Box::new(factory);
            let mut prepared: Option<(String, PreparedBackend)> = None;
            loop {
                let (task, ticket, recovery) = {
                    let (lock, wake) = &*actor_queue;
                    let mut queue = lock.lock().unwrap();
                    while queue.pending.is_none() && !queue.shutdown {
                        queue = wake.wait(queue).unwrap();
                    }
                    if queue.shutdown {
                        break;
                    }
                    let (task, ticket, recovery) = queue.pending.take().unwrap();
                    queue.active = Some((ticket.clone(), task.preview));
                    (task, ticket, recovery)
                };
                let result = run_task(
                    &task,
                    &ticket,
                    recovery,
                    factory.as_mut(),
                    &mut prepared,
                    &events,
                    &actor_partial,
                );
                actor_queue.0.lock().unwrap().active = None;
                match result {
                    Ok(result) if !ticket.cancel.load(Ordering::Acquire) => {
                        let _ = events.send(RenderEvent::Completed {
                            id: ticket.id,
                            revision: ticket.revision,
                            engine: ticket.engine,
                            preview: task.preview,
                            result,
                        });
                    }
                    Ok(_) => {
                        let _ = events.send(RenderEvent::Failed {
                            id: ticket.id,
                            revision: ticket.revision,
                            engine: ticket.engine,
                            message: "cancelled".into(),
                        });
                    }
                    Err(error) => {
                        let _ = events.send(RenderEvent::Failed {
                            id: ticket.id,
                            revision: ticket.revision,
                            engine: ticket.engine,
                            message: error.to_string(),
                        });
                        prepared = None;
                    }
                }
            }
        });
        Self {
            queue,
            events: event_receiver,
            partial,
            last_partial_delivery: Mutex::new(None),
            thread: Some(handle),
        }
    }
    pub fn submit(&self, task: RenderTask) -> std::result::Result<(), String> {
        self.submit_inner(task, false)
    }
    pub fn submit_recovery(&self, task: RenderTask) -> std::result::Result<(), String> {
        self.submit_inner(task, true)
    }
    fn submit_inner(&self, task: RenderTask, recovery: bool) -> std::result::Result<(), String> {
        let mut queue = self.queue.0.lock().unwrap();
        if queue.shutdown {
            return Err("renderer stopped".into());
        }
        if queue.active.as_ref().is_some_and(|(_, preview)| !*preview)
            || queue
                .pending
                .as_ref()
                .is_some_and(|(task, _, _)| !task.preview)
        {
            return Err("an export is already active".into());
        }
        if let Some((ticket, _)) = &queue.active {
            ticket.cancel.store(true, Ordering::Release);
        }
        if let Some((_, ticket, _)) = queue.pending.take() {
            ticket.cancel.store(true, Ordering::Release);
        }
        let ticket = JobTicket::new(task.id, task.revision, task.request.selection.backend);
        queue.pending = Some((task, ticket, recovery));
        *self.partial.lock().unwrap() = None;
        self.queue.1.notify_one();
        Ok(())
    }
    pub fn cancel(&self) {
        let queue = self.queue.0.lock().unwrap();
        if let Some((ticket, _)) = &queue.active {
            ticket.cancel.store(true, Ordering::Release);
        }
        if let Some((_, ticket, _)) = &queue.pending {
            ticket.cancel.store(true, Ordering::Release);
        }
        *self.partial.lock().unwrap() = None;
    }
    pub fn cancel_preview(&self) {
        let queue = self.queue.0.lock().unwrap();
        if let Some((ticket, true)) = &queue.active {
            ticket.cancel.store(true, Ordering::Release);
        }
        if let Some((task, ticket, _)) = &queue.pending
            && task.preview
        {
            ticket.cancel.store(true, Ordering::Release);
        }
        *self.partial.lock().unwrap() = None;
    }
    pub fn take_partial(&self) -> Option<(JobTicket, RgbaFrame)> {
        let mut delivered = self.last_partial_delivery.lock().unwrap();
        if delivered.is_some_and(|time| time.elapsed().as_millis() < 200) {
            return None;
        }
        let frame = self
            .partial
            .lock()
            .unwrap()
            .take()
            .filter(|(ticket, _)| !ticket.cancel.load(Ordering::Acquire));
        if frame.is_some() {
            *delivered = Some(Instant::now());
        }
        frame
    }
}
fn runtime_inputs(task: &RenderTask) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(&task.request.selection)?);
    if matches!(
        task.request.selection.backend,
        BackendId::BlenderEevee | BackendId::BlenderCycles
    ) {
        for path in [
            task.tools.blender.clone(),
            crate::render::worker::renderer_script("worker.py"),
            crate::render::worker::renderer_script("scene.py"),
            crate::render::worker::renderer_script("frame_pipeline.py"),
        ] {
            let meta = fs::metadata(&path)?;
            hasher.update(path.to_string_lossy().as_bytes());
            hasher.update(meta.len().to_le_bytes());
            hasher.update(format!("{:?}", meta.modified()?).as_bytes());
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}
fn run_task(
    task: &RenderTask,
    ticket: &JobTicket,
    recovery: bool,
    factory: &mut BackendFactory,
    prepared: &mut Option<(String, PreparedBackend)>,
    events: &mpsc::Sender<RenderEvent>,
    partial: &Mutex<Option<(JobTicket, RgbaFrame)>>,
) -> Result<ExportResult> {
    if ticket.cancel.load(Ordering::Acquire) {
        return Err("cancelled".into());
    }
    let cache = crate::cache::CacheStore::new(
        task.request
            .job_dir
            .parent()
            .ok_or("job parent missing")?
            .to_path_buf(),
    )?;
    cache.owned_job(&task.request.job_dir)?;
    let _job_lock = cache.lock_job(&task.request.job_dir)?;
    let frames = if matches!(
        task.request.kind,
        ExportKind::Gif | ExportKind::Mp4 | ExportKind::SectionTurntable
    ) {
        task.request.animation_frame_count()?
    } else {
        4
    };
    let panels = if task.request.layout == crate::export::ExportLayout::SideBySide {
        4
    } else {
        1
    };
    let mesh_path = task.request.job_dir.join("mesh.bin");
    let mut scene = task.scene.clone();
    // Resolve the owned parent before creating either immutable snapshot.
    scene["mesh_path"] =
        serde_json::json!(fs::canonicalize(&task.request.job_dir)?.join("mesh.bin"));
    let scene_bytes = serde_json::to_vec(&scene)?;
    if scene_bytes.len() > 1024 * 1024 {
        return Err("render scene exceeds 1 MiB byte budget".into());
    }
    let timeline_bytes = if let Some(timeline) = &task.request.timeline {
        let mut timeline = timeline.clone();
        timeline.buildup |= task.request.animation == crate::export::AnimationMode::BuildUp;
        let bytes = serde_json::to_vec(&timeline.worker_frames_at_rate(task.request.frame_rate)?)?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("timeline states exceed 8 MiB budget".into());
        }
        bytes.len() as u64
    } else {
        0
    };
    let mesh_bytes = 20u64
        .checked_add(
            (task.mesh.positions.len() as u64)
                .checked_mul(12)
                .ok_or("snapshot size overflow")?,
        )
        .and_then(|n| n.checked_add((task.mesh.triangles.len() as u64).checked_mul(12)?))
        .ok_or("snapshot size overflow")?;
    let reserve = u64::from(task.request.size)
        .checked_pow(2)
        .and_then(|n| n.checked_mul(8))
        .and_then(|n| n.checked_mul(u64::from(frames)))
        .and_then(|n| n.checked_mul(panels))
        .and_then(|n| n.checked_add(mesh_bytes))
        .and_then(|n| n.checked_add(scene_bytes.len() as u64))
        .and_then(|n| n.checked_add(timeline_bytes))
        .and_then(|n| n.checked_add(1024 * 1024))
        .ok_or("snapshot reservation overflow")?;
    let snapshots_match = snapshot_matches(&mesh_path, &task.mesh)?
        && crate::settings::read_bounded(&task.request.scene_path, 1024 * 1024)
            .is_ok_and(|bytes| bytes == scene_bytes);
    if !snapshots_match {
        cache.check_budget(task.cache_budget_bytes, reserve)?;
    }
    if !snapshots_match && (mesh_path.exists() || task.request.scene_path.exists()) {
        return Err("immutable retained job snapshots differ".into());
    }
    if !snapshots_match {
        task.mesh.write_binary(&mesh_path)?;
        crate::settings::atomic_write(&task.request.scene_path, &scene_bytes)?;
    }
    if !recovery || !task.request.job_dir.join("request.json").exists() {
        crate::settings::atomic_write(
            &task.request.job_dir.join("request.json"),
            &serde_json::to_vec_pretty(&task.request)?,
        )?;
    }
    cache.save_record(
        &task.request.job_dir,
        &crate::cache::JobRecord {
            schema_version: 1,
            label: format!("{:?}", task.request.kind),
            state: "running".into(),
            destination: task.request.destination.clone(),
            error: None,
        },
    )?;

    let callback = |event| {
        if !ticket.cancel.load(Ordering::Acquire) {
            let _ = events.send(RenderEvent::Progress {
                id: ticket.id,
                revision: ticket.revision,
                engine: ticket.engine,
                event,
            });
        }
    };
    if prepared
        .as_ref()
        .is_some_and(|(_, p)| p.backend.id() != task.request.selection.backend)
    {
        *prepared = None;
    }
    // Recovery is deliberately before any renderer requirement or GPU preparation.
    if recovery
        && let Some(result) = crate::export::pipeline::encode_only_locked(
            &task.tools,
            &task.request,
            &ticket.cancel,
            &mut { callback },
        )?
    {
        return Ok(result);
    }
    if recovery {
        crate::export::pipeline::validate_recovery_recipe(&task.request.job_dir, None)?;
    }
    let request = crate::render::sequence::normalized_request(&task.request)?;
    let snapshot = snapshot_scene(task.mesh.clone(), scene)?;
    let limits = RenderLimits::default();
    let inputs = runtime_inputs(task)?;
    if prepared
        .as_ref()
        .is_some_and(|(saved, p)| saved != &inputs || p.identity != p.backend.identity())
    {
        *prepared = None;
    }
    if prepared.is_none() {
        // Drop the old context first; never overlap engine/device ownership.
        *prepared = None;
        let next = factory(
            &task.request.selection,
            &snapshot,
            &request,
            &task.tools,
            &limits,
            &ticket.cancel,
        )?;
        callback(JobEvent::Log(format!(
            "Renderer preparation: {:?}",
            next.preparation_timings
        )));
        *prepared = Some((inputs, next));
    }
    let prepared = &mut prepared.as_mut().ok_or("renderer unavailable")?.1;
    prepared.backend.configure(&request)?;
    let key = crate::export::identity::pixel_recipe_key(
        &snapshot.legacy_scene,
        &task.mesh.snapshot_hash()?,
        &request,
        &prepared.identity,
    )?;
    if recovery {
        crate::export::pipeline::validate_recovery_recipe(&task.request.job_dir, Some(&key))?;
    }
    let progress = |mut p: RenderProgress| {
        if ticket.cancel.load(Ordering::Acquire) {
            return;
        }
        if let Some(image) = p.preview.take() {
            *partial.lock().unwrap() = Some((ticket.clone(), image));
        }
        callback(JobEvent::Progress {
            completed: p.completed_work.min(u64::from(u32::MAX)) as u32,
            total: p.total_work.unwrap_or(0).min(u64::from(u32::MAX)) as u32,
            stage: p.stage,
        });
    };
    let context = RenderContext {
        cancel: &ticket.cancel,
        limits: &limits,
        scratch_dir: &task.request.job_dir,
        selection: &prepared.effective_selection,
        identity: &prepared.identity,
        pixel_recipe_key: &key,
        job_id: ticket.id,
        revision: ticket.revision,
        progress: &progress,
    };
    crate::export::pipeline::run_export_with_backend_budget(
        prepared.backend.as_mut(),
        &task.tools,
        &task.request,
        &context,
        Some(task.cache_budget_bytes),
        &mut { callback },
    )
}
fn snapshot_matches(path: &std::path::Path, mesh: &Mesh) -> Result<bool> {
    let Ok(mut file) = fs::File::open(path) else {
        return Ok(false);
    };
    let expected_len = 20 + mesh.positions.len() as u64 * 12 + mesh.triangles.len() as u64 * 12;
    if file.metadata()?.len() != expected_len {
        return Ok(false);
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()) == mesh.snapshot_hash()?)
}
impl Drop for RenderService {
    fn drop(&mut self) {
        self.cancel();
        {
            let mut queue = self.queue.0.lock().unwrap();
            queue.shutdown = true;
            queue.pending = None;
        }
        self.queue.1.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn default_request(
    job_dir: PathBuf,
    destination: PathBuf,
    kind: ExportKind,
    selection: crate::render::RenderSelection,
    size: u32,
    transparent: bool,
) -> ExportRequest {
    ExportRequest {
        scene_path: job_dir.join("scene.json"),
        job_dir,
        destination,
        kind,
        selection,
        size,
        frame_count: 80,
        transparent,
        ..ExportRequest::default()
    }
}
