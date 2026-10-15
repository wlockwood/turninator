//! SDK-independent owner-thread orchestration. Native drivers own SDK policy and objects.
use super::*;
use crate::{
    Result,
    render::{contract::*, geometry, materials, prepared},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

/// Small synchronous owner interface; polling never calls Rust from an SDK thread.
/// Implementations must drain on cancellation and retain their module through destruction.
pub trait EmbeddedDriver {
    fn upload(&mut self, scene: &SceneUpload<'_>) -> Result<()>;
    fn begin(&mut self, frame: &EmbeddedFrame) -> Result<()>;
    fn poll(&mut self) -> Result<EmbeddedPoll>;
    fn read(&mut self, pixels: &mut [u8]) -> Result<(u32, u32)>;
    fn cancel(&mut self) -> Result<()>;
    fn runtime_info(&mut self) -> Result<RuntimeInfo>;
}
impl EmbeddedDriver for EmbeddedContext {
    fn upload(&mut self, scene: &SceneUpload<'_>) -> Result<()> {
        self.upload_scene(scene)
    }
    fn begin(&mut self, frame: &EmbeddedFrame) -> Result<()> {
        self.begin_frame(frame)
    }
    fn poll(&mut self) -> Result<EmbeddedPoll> {
        self.poll()
    }
    fn read(&mut self, pixels: &mut [u8]) -> Result<(u32, u32)> {
        self.read_pixels(pixels)
    }
    fn cancel(&mut self) -> Result<()> {
        self.cancel()
    }
    fn runtime_info(&mut self) -> Result<RuntimeInfo> {
        self.runtime_info()
    }
}

/// A CPU image ready for owner/UI transfer, with tags for rejecting obsolete results.
pub struct StudioFrame {
    pub job_id: u64,
    pub revision: u64,
    pub backend: BackendId,
    pub key: FrameKey,
    pub upload_generation: u64,
    pub image: RgbaFrame,
    pub section: Option<SectionAnalysis>,
    pub build: Option<BuildAnalysis>,
}

/// Construct and call on its owner thread. No manifest, encoder or UI texture ownership.
pub struct EmbeddedStudio<D: EmbeddedDriver = EmbeddedContext> {
    driver: D,
    selection: RenderSelection,
    identity: BackendIdentity,
    limits: RenderLimits,
    generation: u64,
    poisoned: bool,
    diagnostics: Option<RuntimeDiagnostics>,
    runtime_signature: String,
}
fn runtime_signature(info: &RuntimeInfo) -> Result<String> {
    // Descriptive observations never participate in immutable runtime identity.
    Ok(serde_json::to_string(
        &serde_json::json!({"schema":info.schema_version,
        "backend":info.backend,"sdk":info.sdk_version,"device":info.device_id,
        "name":info.device_name,"api":info.api,"driver":info.driver,
        "compiler":info.compiler.as_ref().map(|c|(&c.name,&c.version,&c.sha256,&c.path))}),
    )?)
}
#[derive(Debug)]
struct DrainFailure {
    original: Box<dyn std::error::Error>,
    drain: String,
}
impl std::fmt::Display for DrainFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}; failed to drain renderer: {}",
            self.original, self.drain
        )
    }
}
impl std::error::Error for DrainFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.original.as_ref())
    }
}
impl<D: EmbeddedDriver> EmbeddedStudio<D> {
    /// The SDK wrapper must attest the runtime/package and supply its actual identity.
    pub fn from_driver(
        mut driver: D,
        selection: RenderSelection,
        identity: BackendIdentity,
        limits: RenderLimits,
    ) -> Result<Self> {
        selection.validate()?;
        if selection.backend != identity.backend
            || selection
                .device
                .as_ref()
                .is_some_and(|d| d != &identity.device)
        {
            return Err("embedded selection does not match prepared identity".into());
        }
        let runtime = driver.runtime_info()?;
        if runtime.backend != identity.backend.as_str()
            || runtime.device_id != identity.device
            || runtime.api != identity.api
            || runtime.driver != identity.driver
        {
            return Err("embedded actual runtime differs from supplied identity".into());
        }
        let runtime_signature = runtime_signature(&runtime)?;
        Ok(Self {
            driver,
            selection,
            identity,
            limits,
            generation: 0,
            poisoned: false,
            diagnostics: runtime.diagnostics,
            runtime_signature,
        })
    }
    pub fn identity(&self) -> BackendIdentity {
        self.identity.clone()
    }
    pub fn diagnostics(&self) -> Option<&RuntimeDiagnostics> {
        self.diagnostics.as_ref()
    }
    pub fn validate_selection(&self, selection: &RenderSelection) -> Result<()> {
        selection.validate()?;
        if selection.backend != self.selection.backend
            || selection.quality != self.selection.quality
            || selection
                .device
                .as_ref()
                .is_some_and(|d| d != &self.identity.device)
        {
            return Err("selection changed; prepare a new embedded renderer context".into());
        }
        Ok(())
    }
    /// Holds the immutable snapshot for this job. Dropping it invalidates all geometry keys.
    pub fn begin_job<'a, 'c>(
        &'a mut self,
        scene: &'a RenderSceneSnapshot,
        context: &'a RenderContext<'c>,
        transparent: bool,
    ) -> Result<EmbeddedJob<'a, 'c, D>> {
        self.validate_selection(context.selection)?;
        if self.poisoned {
            return Err("embedded context failed to drain; recreate renderer".into());
        }
        if context.identity != &self.identity {
            return Err("embedded context identity mismatch".into());
        }
        if context.limits.host_bytes != self.limits.host_bytes
            || context.limits.gpu_bytes != self.limits.gpu_bytes
        {
            return Err("embedded native resource budgets changed; prepare a new context".into());
        }
        if context.cancel.load(Ordering::Relaxed) {
            return Err("render cancelled".into());
        }
        if scene.mesh.positions.len()
            > context
                .limits
                .max_source_vertices
                .min(self.limits.max_source_vertices)
            || scene.mesh.triangles.len()
                > context
                    .limits
                    .max_source_triangles
                    .min(self.limits.max_source_triangles)
        {
            return Err("embedded source geometry exceeds budget".into());
        }
        Ok(EmbeddedJob {
            studio: self,
            scene,
            context,
            transparent,
            cached: None,
            completed: BTreeSet::new(),
            analyses: BTreeMap::new(),
            build_analyses: BTreeMap::new(),
            timings: BTreeMap::new(),
            uploads: 0,
        })
    }
    fn drain(&mut self) -> Result<()> {
        self.poisoned = true;
        self.driver.cancel()?;
        loop {
            match self.driver.poll()?.state {
                EmbeddedState::Idle | EmbeddedState::Cancelled => {
                    self.poisoned = false;
                    return Ok(());
                }
                EmbeddedState::Failed => {
                    return Err("embedded worker failed; recreate renderer".into());
                }
                _ => std::thread::sleep(Duration::from_millis(2)),
            }
        }
    }
}
struct CachedGeometry {
    key: String,
    generation: u64,
    section: Option<SectionAnalysis>,
    build: Option<BuildAnalysis>,
}
pub struct EmbeddedJob<'a, 'c, D: EmbeddedDriver> {
    studio: &'a mut EmbeddedStudio<D>,
    scene: &'a RenderSceneSnapshot,
    context: &'a RenderContext<'c>,
    transparent: bool,
    // Only the currently uploaded mesh: bounded even for 10,000 distinct stages.
    cached: Option<CachedGeometry>,
    completed: BTreeSet<FrameKey>,
    analyses: BTreeMap<FrameKey, SectionAnalysis>,
    build_analyses: BTreeMap<FrameKey, BuildAnalysis>,
    timings: BTreeMap<String, Duration>,
    uploads: u64,
}
impl<D: EmbeddedDriver> EmbeddedJob<'_, '_, D> {
    pub fn upload_count(&self) -> u64 {
        self.uploads
    }
    fn cancelled(&self) -> Result<()> {
        if self.context.cancel.load(Ordering::Relaxed) {
            Err("render cancelled".into())
        } else {
            Ok(())
        }
    }
    fn timing(&mut self, phase: &str, start: Instant) {
        *self.timings.entry(phase.into()).or_default() += start.elapsed();
    }
    pub fn render_frame(&mut self, frame: &FrameSpec) -> Result<StudioFrame> {
        if self.studio.poisoned {
            return Err("embedded context failed to drain; recreate renderer".into());
        }
        self.cancelled()?;
        if self.completed.contains(&frame.key) {
            return Err("duplicate embedded frame key".into());
        }
        if self.completed.len()
            >= self
                .context
                .limits
                .max_frames
                .min(self.studio.limits.max_frames) as usize
        {
            return Err("embedded frame budget exceeded".into());
        }
        let bytes = u64::from(frame.width)
            .checked_mul(u64::from(frame.height))
            .and_then(|n| n.checked_mul(4))
            .ok_or("frame byte overflow")?;
        let limits = self.context.limits;
        if frame.width == 0
            || frame.height == 0
            || frame.width.max(frame.height)
                > limits
                    .max_output_side
                    .min(self.studio.limits.max_output_side)
            || bytes > limits.host_bytes.min(self.studio.limits.host_bytes)
            || bytes > limits.gpu_bytes.min(self.studio.limits.gpu_bytes)
        {
            return Err("embedded frame dimensions exceed budget".into());
        }
        let key = prepared::frame_geometry_key(frame, true)?;
        if self.cached.as_ref().is_none_or(|c| c.key != key) {
            self.cached = None;
            let start = Instant::now();
            let mut realized = geometry::realize_frame(self.scene, frame)?;
            let mut materials = materials::resolve_materials(self.scene)?;
            geometry::expand_design_edges(
                self.scene,
                frame,
                &mut realized,
                &mut materials,
                limits,
            )?;
            let estimate = (realized.mesh.positions.len() as u64)
                .checked_mul(64)
                .and_then(|n| {
                    (realized.mesh.triangles.len() as u64)
                        .checked_mul(32)
                        .and_then(|t| n.checked_add(t))
                })
                .and_then(|n| n.checked_add(bytes))
                .ok_or("geometry byte overflow")?;
            if estimate > limits.host_bytes.min(self.studio.limits.host_bytes) {
                return Err("realized embedded geometry exceeds host budget".into());
            }
            self.timing("realization", start);
            self.cancelled()?;
            let start = Instant::now();
            let positions: Vec<_> = realized
                .mesh
                .positions
                .iter()
                .map(|p| p.map(|v| v as f32))
                .collect();
            self.studio.generation = self
                .studio
                .generation
                .checked_add(1)
                .ok_or("upload generation exhausted")?;
            let generation = self.studio.generation;
            let metadata = EmbeddedSceneMetadata {
                appearance: self.scene.appearance.clone(),
                materials: materials.materials,
                reference_bounds_mm: realized.reference_bounds_mm,
                geometry_kind: self.scene.metadata.kind,
                layer_filter: EmbeddedLayerFilter::standard(
                    self.scene.legacy_scene["layer_filtering"]
                        .as_bool()
                        .unwrap_or(true),
                ),
                color: EmbeddedColorConfig::for_backend(self.studio.identity.backend)?,
            };
            self.studio.driver.upload(&SceneUpload {
                revision: generation,
                positions_mm: &positions,
                normals: &realized.normals,
                build_positions_mm: &realized.build_positions_mm,
                triangles: &realized.mesh.triangles,
                triangle_materials: &realized.triangle_materials,
                metadata: &metadata,
            })?;
            self.cached = Some(CachedGeometry {
                key,
                generation,
                section: realized.analysis,
                build: realized.build_analysis,
            });
            self.uploads += 1;
            self.timing("upload", start);
        }
        self.cancelled()?;
        let generation = self.cached.as_ref().unwrap().generation;
        let matrix = geometry::resolve_display_transform(self.scene, frame.angle_degrees)?;
        let native = EmbeddedFrame {
            geometry_revision: generation,
            width: frame.width,
            height: frame.height,
            camera: frame.camera.clone(),
            model_to_world: std::array::from_fn(|i| matrix[i / 4][i % 4]),
            quality: self.studio.selection.quality.clone(),
            transparent: self.transparent,
            sample_seed: 0,
        };
        let start = Instant::now();
        // Even a failed begin may have queued native work. Drain all transport failures.
        let result = (|| -> Result<RgbaFrame> {
            self.studio.driver.begin(&native)?;
            loop {
                self.cancelled()?;
                let poll = self.studio.driver.poll()?;
                (self.context.progress)(RenderProgress {
                    key: Some(frame.key),
                    completed_work: poll.completed_work,
                    total_work: poll.total_work,
                    stage: "Rendering".into(),
                    preview: None,
                });
                self.cancelled()?;
                match poll.state {
                    EmbeddedState::Ready => break,
                    EmbeddedState::Rendering => std::thread::sleep(Duration::from_millis(5)),
                    EmbeddedState::Cancelled | EmbeddedState::Cancelling => {
                        return Err("render cancelled".into());
                    }
                    _ => return Err("embedded renderer stopped without a completed frame".into()),
                }
            }
            self.timing("render", start);
            let read_start = Instant::now();
            let mut pixels = vec![0; usize::try_from(bytes)?];
            let (width, height) = self.studio.driver.read(&mut pixels)?;
            self.cancelled()?;
            if (width, height) != (frame.width, frame.height) {
                return Err("embedded readback dimensions mismatch".into());
            }
            // ABI1 already guarantees top-first straight, display-encoded RGBA8.
            // Do not transfer-encode or unpremultiply it again.
            let info = self.studio.driver.runtime_info()?;
            if runtime_signature(&info)? != self.studio.runtime_signature {
                return Err(
                    RenderReportRejected("embedded runtime identity changed".into()).into(),
                );
            }
            self.studio.diagnostics = info.diagnostics;
            self.timing("readback", read_start);
            Ok(RgbaFrame {
                width,
                height,
                bytes: pixels,
            })
        })();
        let image = match result {
            Ok(image) => image,
            Err(error) => {
                self.cached = None;
                if let Err(drain) = self.studio.drain() {
                    return Err(Box::new(DrainFailure {
                        original: error,
                        drain: drain.to_string(),
                    }));
                }
                return Err(error);
            }
        };
        let cached = self.cached.as_ref().unwrap();
        if let Some(a) = &cached.section {
            self.analyses.insert(frame.key, a.clone());
        }
        if let Some(a) = &cached.build {
            self.build_analyses.insert(frame.key, a.clone());
        }
        self.completed.insert(frame.key);
        Ok(StudioFrame {
            job_id: self.context.job_id,
            revision: self.context.revision,
            backend: self.studio.identity.backend,
            key: frame.key,
            upload_generation: generation,
            image,
            section: cached.section.clone(),
            build: cached.build.clone(),
        })
    }
    pub fn report(&self) -> RenderReport {
        RenderReport {
            frame_count: self.completed.len() as u32,
            warnings: self.scene.warnings.clone(),
            identity: self.studio.identity.clone(),
            quality: self.studio.selection.quality.clone(),
            analyses: self.analyses.clone(),
            build_analyses: self.build_analyses.clone(),
            phase_timings: self.timings.clone(),
        }
    }
}

/// Sequence transport keeps metadata ordered before corresponding durable pixels.
pub fn render_sequence<D: EmbeddedDriver>(
    studio: &mut EmbeddedStudio<D>,
    scene: &RenderSceneSnapshot,
    frames: &FramePlan,
    context: &RenderContext<'_>,
    emit: &mut dyn FnMut(FrameArtifact) -> Result<()>,
) -> Result<RenderReport> {
    let mut job = studio.begin_job(scene, context, frames.transparent)?;
    for frame in &frames.frames {
        if frames.repeat_of.contains_key(&frame.key) {
            continue;
        }
        let completed = job.render_frame(frame)?;
        if frame.section.is_some() || frame.visible_layers.is_some() {
            emit(FrameArtifact::Metadata {
                key: frame.key,
                section: completed.section,
                build: completed.build,
            })?;
        }
        emit(FrameArtifact::Image {
            key: frame.key,
            image: completed.image,
        })?;
    }
    Ok(job.report())
}
