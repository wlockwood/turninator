//! Immutable frame identity and bounded interactive scheduling.
use crate::render::{BackendId, selection::ViewportRenderer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameIdentity {
    pub job: u64,
    pub revision: u64,
    pub backend: BackendId,
    pub generation: u64,
}
pub struct LatestFrameQueue<T> {
    current: Option<FrameIdentity>,
    active: Option<FrameIdentity>,
    pending: Option<(FrameIdentity, T)>,
    paused: bool,
}
impl<T> Default for LatestFrameQueue<T> {
    fn default() -> Self {
        Self {
            current: None,
            active: None,
            pending: None,
            paused: false,
        }
    }
}
impl<T> LatestFrameQueue<T> {
    pub fn submit(&mut self, identity: FrameIdentity, request: T) {
        self.current = Some(identity);
        self.pending = Some((identity, request));
    }
    pub fn take_pending(&mut self) -> Option<(FrameIdentity, T)> {
        if self.active.is_some() || self.paused {
            return None;
        }
        let next = self.pending.take()?;
        self.active = Some(next.0);
        Some(next)
    }
    pub fn finish(&mut self, identity: FrameIdentity) {
        if self.active == Some(identity) {
            self.active = None;
        }
    }
    pub fn accepts(&self, identity: FrameIdentity) -> bool {
        self.current == Some(identity)
    }
    pub fn invalidate(&mut self) {
        self.current = None;
        self.pending = None;
    }
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }
}

use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};
type WorkResult<U> = std::result::Result<U, String>;
type Executor<T, U> = dyn FnMut(T, &AtomicBool) -> WorkResult<U>;
struct WorkerState<T, U> {
    queue: LatestFrameQueue<T>,
    active_cancel: Option<Arc<AtomicBool>>,
    completed: Option<(FrameIdentity, WorkResult<U>)>,
    release: bool,
    released: bool,
    shutdown: bool,
}
/// Only the factory crosses threads. The executor and all SDK objects it owns
/// are constructed, used, released and destroyed on this dedicated owner.
pub struct LatestFrameWorker<T, U> {
    state: Arc<(Mutex<WorkerState<T, U>>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl<T: Send + 'static, U: Send + 'static> LatestFrameWorker<T, U> {
    pub fn new(factory: impl Fn() -> Box<Executor<T, U>> + Send + 'static) -> Self {
        let state = Arc::new((
            Mutex::new(WorkerState {
                queue: LatestFrameQueue::default(),
                active_cancel: None,
                completed: None,
                release: false,
                released: true,
                shutdown: false,
            }),
            Condvar::new(),
        ));
        let owner = state.clone();
        let thread = std::thread::spawn(move || {
            let mut execute: Option<Box<Executor<T, U>>> = None;
            loop {
                let (identity, request, cancel) = {
                    let mut state = owner.0.lock().unwrap();
                    loop {
                        if state.shutdown {
                            return;
                        }
                        if state.release {
                            state.release = false;
                            drop(state);
                            execute = None;
                            state = owner.0.lock().unwrap();
                            state.released = true;
                            continue;
                        }
                        if let Some((identity, request)) = state.queue.take_pending() {
                            let cancel = Arc::new(AtomicBool::new(false));
                            state.active_cancel = Some(cancel.clone());
                            state.released = false;
                            break (identity, request, cancel);
                        }
                        state = owner.1.wait(state).unwrap();
                    }
                };
                let executor = execute.get_or_insert_with(&factory);
                let result = executor(request, &cancel);
                let mut state = owner.0.lock().unwrap();
                state.queue.finish(identity);
                state.active_cancel = None;
                if !cancel.load(Ordering::Acquire) && state.queue.accepts(identity) {
                    state.completed = Some((identity, result));
                }
            }
        });
        Self {
            state,
            thread: Some(thread),
        }
    }
    pub fn submit(&self, identity: FrameIdentity, request: T) {
        let mut state = self.state.0.lock().unwrap();
        if let Some(cancel) = &state.active_cancel {
            cancel.store(true, Ordering::Release);
        }
        state.completed = None;
        state.queue.submit(identity, request);
        self.state.1.notify_one();
    }
    pub fn take_completed(&self) -> Option<(FrameIdentity, WorkResult<U>)> {
        let mut state = self.state.0.lock().unwrap();
        state
            .completed
            .take()
            .filter(|(identity, _)| state.queue.accepts(*identity))
    }
    pub fn set_paused(&self, paused: bool) {
        let mut state = self.state.0.lock().unwrap();
        state.queue.set_paused(paused);
        if paused && let Some(cancel) = &state.active_cancel {
            cancel.store(true, Ordering::Release);
        }
        self.state.1.notify_one();
    }
    pub fn release(&self) {
        let mut state = self.state.0.lock().unwrap();
        if let Some(cancel) = &state.active_cancel {
            cancel.store(true, Ordering::Release);
        }
        state.queue.invalidate();
        state.completed = None;
        state.release = true;
        self.state.1.notify_one();
    }
    pub fn is_released(&self) -> bool {
        let state = self.state.0.lock().unwrap();
        state.released && state.active_cancel.is_none() && state.queue.pending.is_none()
    }
}
impl<T, U> Drop for LatestFrameWorker<T, U> {
    fn drop(&mut self) {
        {
            let mut state = self.state.0.lock().unwrap();
            state.shutdown = true;
            if let Some(cancel) = &state.active_cancel {
                cancel.store(true, Ordering::Release);
            }
            self.state.1.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) struct InteractiveRequest {
    pub task: crate::session::RenderTask,
    pub dimensions: [u32; 2],
}
pub(crate) struct InteractiveFrame {
    pub image: crate::render::contract::RgbaFrame,
    pub identity: crate::render::contract::BackendIdentity,
}
pub(crate) fn interactive_worker() -> LatestFrameWorker<InteractiveRequest, InteractiveFrame> {
    LatestFrameWorker::new(|| {
        let registry = crate::render::registry::BackendRegistry::default();
        let mut prepared: Option<(String, crate::render::registry::PreparedBackend)> = None;
        Box::new(move |input: InteractiveRequest, cancel| {
            let result = render_interactive(input, cancel, &registry, &mut prepared);
            if result.is_err() {
                prepared = None;
            }
            result.map_err(|error| error.to_string())
        })
    })
}
fn render_interactive(
    input: InteractiveRequest,
    cancel: &AtomicBool,
    registry: &crate::render::registry::BackendRegistry,
    prepared: &mut Option<(String, crate::render::registry::PreparedBackend)>,
) -> crate::Result<InteractiveFrame> {
    use crate::render::contract::*;
    let task = input.task;
    if cancel.load(Ordering::Acquire) {
        return Err("cancelled".into());
    }
    let scene = snapshot_scene(task.mesh.clone(), task.scene.clone())?;
    let request = crate::render::sequence::normalized_request(&task.request)?;
    let limits = task.memory.limits()?;
    let fingerprint = crate::session::runtime_inputs(&task)?;
    if prepared
        .as_ref()
        .is_some_and(|(old, _)| *old != fingerprint)
    {
        *prepared = None;
    }
    if prepared.is_none() {
        *prepared = Some((
            fingerprint,
            registry.prepare(
                &request.selection,
                &scene,
                &request,
                &task.tools,
                &limits,
                cancel,
            )?,
        ));
    }
    let backend = &mut prepared.as_mut().ok_or("viewport renderer unavailable")?.1;
    backend.backend.configure(&request)?;
    let mut frames =
        crate::render::sequence::plan_frames_for_identity(&scene, &request, &backend.identity)?;
    if frames.frames.len() != 1 {
        return Err("interactive viewport requires one frame".into());
    }
    frames.frames[0].width = input.dimensions[0];
    frames.frames[0].height = input.dimensions[1];
    let key = crate::export::identity::pixel_recipe_key(
        &scene.legacy_scene,
        &task.mesh.snapshot_hash()?,
        &request,
        &backend.identity,
    )?;
    let context = RenderContext {
        cancel,
        limits: &limits,
        scratch_dir: &request.job_dir,
        selection: &backend.effective_selection,
        identity: &backend.identity,
        pixel_recipe_key: &key,
        job_id: task.id,
        revision: task.revision,
        progress: &|_| {},
    };
    let mut image = None;
    let report = backend
        .backend
        .render_sequence(&scene, &frames, &context, &mut |artifact| {
            if cancel.load(Ordering::Acquire) {
                return Err("cancelled".into());
            }
            if let FrameArtifact::Image { key, image: pixels } = artifact {
                if key != frames.frames[0].key
                    || image.is_some()
                    || pixels.width != input.dimensions[0]
                    || pixels.height != input.dimensions[1]
                    || pixels.bytes.len() as u64
                        != u64::from(pixels.width) * u64::from(pixels.height) * 4
                {
                    return Err("invalid interactive renderer frame".into());
                }
                image = Some(pixels);
            }
            Ok(())
        })?;
    if report.frame_count != 1
        || report.identity != backend.identity
        || report.quality != request.selection.quality
    {
        return Err("interactive renderer report differs from requested identity/quality".into());
    }
    Ok(InteractiveFrame {
        image: image.ok_or("viewport renderer produced no image")?,
        identity: backend.identity.clone(),
    })
}

impl super::Studio {
    pub(super) fn submit_when_viewport_released(
        &mut self,
        task: crate::session::RenderTask,
        recovery: bool,
    ) -> std::result::Result<(), String> {
        self.interactive.release();
        self.interactive_requested = None;
        self.interactive_signature = None;
        if self.interactive.is_released() {
            if recovery {
                self.render.submit_recovery(task)
            } else {
                self.render.submit(task)
            }
        } else {
            self.queued_render = Some((task, recovery));
            Ok(())
        }
    }
    pub(super) fn cancel_render(&mut self) {
        if self.queued_render.take().is_some() {
            self.busy = None;
            self.error = Some("cancelled before renderer preparation".into());
        }
        self.render.cancel();
    }
    pub(super) fn interactive_route(&self) -> ViewportRoute {
        viewport_route(
            self.renderers.viewport,
            InspectionFeatures {
                edges: self.show_edges,
                ghost_or_xray: self.inspection != crate::viewport::InspectionMode::Opaque,
                face_selection: self.settle_face,
                cut: self.section,
            },
        )
    }
    pub(super) fn update_interactive(
        &mut self,
        ctx: &eframe::egui::Context,
        rect: eframe::egui::Rect,
    ) {
        if self.interactive_route().backend != BackendId::Filament
            || self.model.is_none()
            || self.importing
            || self.busy.is_some()
        {
            if self.interactive_signature.take().is_some() {
                self.interactive.release();
                self.interactive_requested = None;
            }
            return;
        }
        let pixels = (rect.size() * ctx.pixels_per_point()).ceil();
        let dimensions = [pixels.x.max(16.) as u32, pixels.y.max(16.) as u32];
        let signature = serde_json::to_string(&(
            self.revision,
            self.camera.camera_spec(),
            dimensions,
            &self.renderers.profiles.get(&BackendId::Filament),
            self.prefs.render_memory,
        ))
        .unwrap_or_default();
        if self.interactive_signature.as_ref() != Some(&signature) {
            self.interactive_generation += 1;
            let identity = FrameIdentity {
                job: self.interactive_generation,
                revision: self.revision,
                backend: BackendId::Filament,
                generation: self.interactive_generation,
            };
            let result = (|| -> crate::Result<InteractiveRequest> {
                let mut selection =
                    crate::render::RenderSelection::for_backend(BackendId::Filament, false);
                if self.renderers.animation.backend == BackendId::Filament {
                    selection = self.renderers.animation.clone();
                } else if let Some(profile) = self.renderers.profiles.get(&BackendId::Filament) {
                    selection.quality = profile.animation.clone();
                }
                let scratch = self.data_dir.join("interactive");
                std::fs::create_dir_all(&scratch)?;
                let mut request = crate::session::default_request(
                    scratch.clone(),
                    scratch.join("viewport.png"),
                    crate::export::ExportKind::Still,
                    selection,
                    dimensions[0].max(dimensions[1]),
                    false,
                );
                request.start_angle_degrees = f64::from(self.timeline_angle);
                if self.buildup
                    && let Some(model) = &self.model
                    && !model.layer_tops_mm.is_empty()
                {
                    request.build_stage = Some(crate::export::BuildStage {
                        visible_layers: crate::presentation::visible_layer_count(
                            model,
                            f64::from(self.buildup_height),
                        ) as u32,
                    });
                }
                let scene = self.snapshot(&request)?;
                let mesh = self
                    .model
                    .as_ref()
                    .ok_or("open a model first")?
                    .mesh
                    .clone();
                Ok(InteractiveRequest {
                    task: crate::session::RenderTask {
                        id: identity.job,
                        revision: identity.revision,
                        generation: identity.generation,
                        mesh,
                        scene,
                        request,
                        preview: true,
                        tools: self.prefs.tools.clone(),
                        memory: self.prefs.render_memory,
                        cache_budget_bytes: self.prefs.cache_budget_bytes,
                    },
                    dimensions,
                })
            })();
            self.interactive_requested = Some(identity);
            self.interactive_signature = Some(signature);
            self.interactive_error = None;
            match result {
                Ok(request) => self.interactive.submit(identity, request),
                Err(error) => self.interactive_error = Some(error.to_string()),
            }
        }
        if let Some((identity, result)) = self.interactive.take_completed()
            && self.interactive_requested == Some(identity)
            && identity.revision == self.revision
        {
            match result {
                Ok(frame) => {
                    self.renderer_identities
                        .insert(BackendId::Filament, frame.identity);
                    let image = eframe::egui::ColorImage::from_rgba_unmultiplied(
                        [frame.image.width as usize, frame.image.height as usize],
                        &frame.image.bytes,
                    );
                    if let Some(texture) = &mut self.interactive_texture {
                        texture.set(image, eframe::egui::TextureOptions::LINEAR);
                    } else {
                        self.interactive_texture = Some(ctx.load_texture(
                            "filament-viewport",
                            image,
                            eframe::egui::TextureOptions::LINEAR,
                        ));
                    }
                    self.interactive_displayed = Some(identity);
                }
                Err(error) => self.interactive_error = Some(error),
            }
        }
        if self.interactive_requested != self.interactive_displayed
            && self.interactive_error.is_none()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}
#[derive(Default)]
pub struct InspectionFeatures {
    pub edges: bool,
    pub ghost_or_xray: bool,
    pub face_selection: bool,
    pub cut: bool,
}
pub struct ViewportRoute {
    pub backend: BackendId,
    pub badge: Option<&'static str>,
}
pub fn viewport_route(preferred: ViewportRenderer, features: InspectionFeatures) -> ViewportRoute {
    let inspection =
        features.edges || features.ghost_or_xray || features.face_selection || features.cut;
    ViewportRoute {
        backend: if preferred == ViewportRenderer::Filament && !inspection {
            BackendId::Filament
        } else {
            BackendId::Wgpu
        },
        badge: (preferred == ViewportRenderer::Filament && inspection)
            .then_some("Inspection uses Native wgpu"),
    }
}
