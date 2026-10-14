use crate::{
    Result,
    export::{ExportKind, ExportRequest, ExportResult, JobEvent, run_export},
    mesh::Mesh,
    render::{ToolPaths, WorkerClient},
};
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
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
pub enum RenderEvent {
    Progress {
        id: u64,
        event: JobEvent,
    },
    Completed {
        id: u64,
        revision: u64,
        preview: bool,
        result: ExportResult,
    },
    Failed {
        id: u64,
        message: String,
    },
}
pub struct RenderService {
    commands: Option<SyncSender<RenderTask>>,
    pub events: Receiver<RenderEvent>,
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Default for RenderService {
    fn default() -> Self {
        Self::new()
    }
}
impl RenderService {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::sync_channel::<RenderTask>(1);
        let (events, event_receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let actor_cancel = cancel.clone();
        let handle = thread::spawn(move || {
            let mut worker: Option<WorkerClient> = None;
            let mut last_tools: Option<ToolPaths> = None;
            while let Ok(task) = receiver.recv() {
                let result = (|| -> Result<ExportResult> {
                    if actor_cancel.load(Ordering::Acquire) {
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
                    let frames = if matches!(
                        task.request.kind,
                        ExportKind::Gif | ExportKind::Mp4 | ExportKind::SectionTurntable
                    ) {
                        if let Some(timeline) = &task.request.timeline {
                            timeline.frame_count()?
                        } else {
                            task.request.frame_count
                        }
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
                    scene["mesh_path"] = serde_json::json!(
                        fs::canonicalize(&task.request.job_dir)?.join("mesh.bin")
                    );
                    let scene_bytes = serde_json::to_vec(&scene)?;
                    if scene_bytes.len() > 1024 * 1024 {
                        return Err("render scene exceeds 1 MiB byte budget".into());
                    }
                    let timeline_bytes = if let Some(timeline) = &task.request.timeline {
                        let mut timeline = timeline.clone();
                        timeline.buildup |=
                            task.request.animation == crate::export::AnimationMode::BuildUp;
                        let bytes = serde_json::to_vec(&timeline.worker_frames()?)?;
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
                        .and_then(|n| {
                            n.checked_add((task.mesh.triangles.len() as u64).checked_mul(12)?)
                        })
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
                    cache.check_budget(task.cache_budget_bytes, reserve)?;
                    task.mesh.write_binary(&mesh_path)?;
                    fs::write(&task.request.scene_path, scene_bytes)?;
                    let changed = last_tools.as_ref().is_none_or(|p| {
                        p.blender != task.tools.blender
                            || p.ffmpeg != task.tools.ffmpeg
                            || p.ffprobe != task.tools.ffprobe
                    });
                    if changed || worker.as_mut().is_none_or(|w| !w.is_running()) {
                        worker = Some(WorkerClient::spawn(&task.tools)?);
                        last_tools = Some(task.tools.clone());
                    }
                    let event_sender = events.clone();
                    run_export(
                        worker.as_mut().ok_or("renderer unavailable")?,
                        &task.tools,
                        &task.request,
                        &actor_cancel,
                        |event| {
                            let _ = event_sender.send(RenderEvent::Progress { id: task.id, event });
                        },
                    )
                })();
                match result {
                    Ok(result) => {
                        let _ = events.send(RenderEvent::Completed {
                            id: task.id,
                            revision: task.revision,
                            preview: task.preview,
                            result,
                        });
                    }
                    Err(error) => {
                        let _ = events.send(RenderEvent::Failed {
                            id: task.id,
                            message: error.to_string(),
                        });
                        if actor_cancel.load(Ordering::Acquire) {
                            worker = None;
                        }
                    }
                }
            }
        });
        Self {
            commands: Some(sender),
            events: event_receiver,
            cancel,
            thread: Some(handle),
        }
    }
    pub fn submit(&self, task: RenderTask) -> std::result::Result<(), String> {
        self.cancel.store(false, Ordering::Release);
        self.commands
            .as_ref()
            .ok_or("renderer stopped")?
            .try_send(task)
            .map_err(|error| error.to_string())
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}
impl Drop for RenderService {
    fn drop(&mut self) {
        self.cancel();
        self.commands.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn default_request(
    job_dir: PathBuf,
    destination: PathBuf,
    kind: ExportKind,
    engine: String,
    size: u32,
    samples: u32,
    transparent: bool,
) -> ExportRequest {
    ExportRequest {
        scene_path: job_dir.join("scene.json"),
        job_dir,
        destination,
        kind,
        engine,
        size,
        samples,
        frame_count: 80,
        transparent,
        ..ExportRequest::default()
    }
}
