//! Native studio. Import and rendering run independently of the UI thread.
mod build_orientation;
mod comparison;
mod controls;
mod help;
mod playback;
mod renderer_smoke;
pub mod renderer_view;
pub mod renderers;
mod sidebar;
mod workspace;
use crate::{
    Result,
    animation::{CameraKeyframe, Easing, Timeline, Transition, TurntableTiming},
    cache::{CacheStore, JobRecord},
    export::{
        AnimationMode, ExportKind, ExportLayout, ExportRequest, ExportResult, JobEvent,
        OutputFormat, SectionSpec, VideoCodec,
    },
    presentation::{
        ImportSettings, ImportedModel, RevisionGate, SceneSettings, group_color, group_color_index,
        load_model, model_scene,
    },
    project::{OutputPreferences, ProjectFile, SourceStatus, load_project, save_project},
    render::{BackendId, RenderSelection, RendererPreferences},
    session::{RenderEvent, RenderService, RenderTask, default_request},
    settings::{AppSettings, default_data_directory},
    viewport::{
        CanonicalView, InspectionMode, OrbitCamera, SectionPlane, Viewport, ViewportSettings,
    },
};
use eframe::egui;
use help::HelpResponse;
use playback::{DecodedOutput, LivePreview, Playback, decode_output};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct LaunchOptions {
    pub open: Option<PathBuf>,
    pub data_directory: Option<PathBuf>,
    pub smoke_output: Option<PathBuf>,
}

pub fn run(options: LaunchOptions) -> Result<()> {
    let native = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("Turninator — print rendering studio")
            .with_inner_size([1280., 850.])
            .with_min_inner_size([900., 650.]),
        ..Default::default()
    };
    eframe::run_native(
        "Turninator",
        native,
        Box::new(move |cc| {
            Ok(Box::new(
                Studio::new(cc, options).map_err(|error| error.to_string())?,
            ))
        }),
    )
    .map_err(|error| error.to_string().into())
}

enum Message {
    Imported {
        generation: u64,
        result: std::result::Result<Box<ImportedModel>, String>,
    },
    OpenModel(Option<PathBuf>),
    OpenProject(Option<PathBuf>),
    LoadedProject {
        generation: u64,
        path: PathBuf,
        result: std::result::Result<Box<(ProjectFile, SourceStatus)>, String>,
    },
    SaveProject(Option<PathBuf>),
    CacheUpdated(std::result::Result<(u64, Vec<(PathBuf, JobRecord)>), String>),
    RemovedJob(std::result::Result<(), String>),
    RecoveredJob(
        std::result::Result<
            Box<(
                ExportRequest,
                serde_json::Value,
                std::sync::Arc<crate::mesh::Mesh>,
            )>,
            String,
        >,
    ),
    ExportDestination {
        kind: ExportKind,
        generation: u64,
        path: Option<PathBuf>,
    },
    Image {
        id: u64,
        revision: u64,
        engine: BackendId,
        generation: u64,
        preview: bool,
        result: std::result::Result<Box<DecodedOutput>, String>,
    },
}
#[derive(Clone)]
struct UiSender {
    channel: Sender<Message>,
    context: egui::Context,
}
impl UiSender {
    fn send(&self, message: Message) -> std::result::Result<(), mpsc::SendError<Message>> {
        let result = self.channel.send(message);
        self.context.request_repaint();
        result
    }
}
pub struct Studio {
    active_tab: workspace::Tab,
    viewport: Viewport,
    camera: OrbitCamera,
    scene: SceneSettings,
    prefs: AppSettings,
    data_dir: PathBuf,
    sender: UiSender,
    messages: Receiver<Message>,
    gate: RevisionGate,
    importing: bool,
    import_active: bool,
    pending_import: Option<(u64, PathBuf, ImportSettings)>,
    pending_build_orientation: Option<build_orientation::PendingBuildOrientation>,
    debounced_import: Option<(PathBuf, Instant)>,
    selected_source: Option<PathBuf>,
    model: Option<ImportedModel>,
    revision: u64,
    view_generation: u64,
    preview_identity: Option<renderer_view::FrameIdentity>,
    output_identity: Option<renderer_view::FrameIdentity>,
    partial_image: Option<(renderer_view::FrameIdentity, egui::TextureHandle)>,
    changed_at: Instant,
    render: RenderService,
    interactive: renderer_view::LatestFrameWorker<
        renderer_view::InteractiveRequest,
        renderer_view::InteractiveFrame,
    >,
    interactive_signature: Option<String>,
    interactive_generation: u64,
    interactive_requested: Option<renderer_view::FrameIdentity>,
    interactive_displayed: Option<renderer_view::FrameIdentity>,
    interactive_texture: Option<egui::TextureHandle>,
    interactive_error: Option<String>,
    next_job: u64,
    busy: Option<u64>,
    queued_render: Option<(RenderTask, bool)>,
    progress: (u32, u32, String),
    last_task: Option<(
        ExportRequest,
        serde_json::Value,
        std::sync::Arc<crate::mesh::Mesh>,
        bool,
        u64,
    )>,
    last_output: Option<ExportResult>,
    preview: Option<Playback>,
    preview_revision: Option<u64>,
    latest_preview_job: u64,
    latest_output_job: u64,
    display_job: u64,
    preview_decode_pending: Option<u64>,
    output: Option<Playback>,
    live_preview: LivePreview,
    show_output: bool,
    error: Option<String>,
    status: String,
    restore_camera: bool,
    unresolved_project: Option<(ProjectFile, SourceStatus)>,
    project_path: Option<PathBuf>,
    renderers: RendererPreferences,
    renderer_statuses: Vec<crate::render::capabilities::BackendStatus>,
    renderer_identities:
        std::collections::BTreeMap<BackendId, crate::render::contract::BackendIdentity>,
    renderer_warnings: Vec<String>,
    size: u32,
    still_size: u32,
    transparent: bool,
    show_edges: bool,
    show_tools: bool,
    codec: VideoCodec,
    output_format: OutputFormat,
    section: bool,
    section_axis: u8,
    section_position: f64,
    section_positive: bool,
    section_cap: bool,
    side_by_side: bool,
    analysis: Option<serde_json::Value>,
    color_signature: String,
    inspection: InspectionMode,
    opacity: f32,
    buildup: bool,
    buildup_height: f32,
    restored_stage: Option<u32>,
    buildup_playing: bool,
    last_tick: Instant,
    export_buildup: bool,
    settle_face: bool,
    native_rect: Option<egui::Rect>,
    timeline: Timeline,
    turntable_timing: TurntableTiming,
    frame_rate: u32,
    timeline_enabled: bool,
    timeline_playing: bool,
    timeline_time: f64,
    timeline_angle: f32,
    adaptive_text: String,
    cache_records: Vec<(PathBuf, JobRecord)>,
    cache_usage: u64,
    cache_query_pending: bool,
    cache_action_pending: bool,
    cache_refreshed: Instant,
    smoke_output: Option<PathBuf>,
    smoke_stage: u8,
    smoke_checks: serde_json::Value,
    started: Instant,
}
impl Studio {
    fn new(cc: &eframe::CreationContext<'_>, options: LaunchOptions) -> Result<Self> {
        let data_dir = options
            .data_directory
            .unwrap_or_else(default_data_directory);
        fs::create_dir_all(&data_dir)?;
        let data_dir = fs::canonicalize(data_dir)?;
        let (prefs, error) = match AppSettings::load(&data_dir.join("settings.json")) {
            Ok(prefs) => (prefs, None),
            Err(error) => (
                AppSettings::default(),
                Some(format!("Settings could not be loaded: {error}")),
            ),
        };
        let (sender, messages) = mpsc::channel();
        let sender = UiSender {
            channel: sender,
            context: cc.egui_ctx.clone(),
        };
        workspace::configure(&cc.egui_ctx);
        let mut studio = Self {
            active_tab: workspace::Tab::default(),
            viewport: Viewport::new(
                cc.wgpu_render_state
                    .as_ref()
                    .ok_or("wgpu renderer unavailable")?,
            ),
            camera: OrbitCamera::default(),
            scene: SceneSettings::default(),
            renderers: prefs.renderers.clone(),
            renderer_statuses: BackendId::ALL
                .into_iter()
                .map(|id| crate::render::registry::BackendRegistry::default().status(id))
                .collect(),
            renderer_identities: Default::default(),
            renderer_warnings: Vec::new(),
            prefs,
            data_dir,
            sender,
            messages,
            gate: RevisionGate::default(),
            importing: false,
            import_active: false,
            pending_import: None,
            pending_build_orientation: None,
            debounced_import: None,
            selected_source: None,
            model: None,
            revision: 0,
            view_generation: 0,
            preview_identity: None,
            output_identity: None,
            partial_image: None,
            changed_at: Instant::now(),
            render: RenderService::new(),
            interactive: renderer_view::interactive_worker(),
            interactive_signature: None,
            interactive_generation: 0,
            interactive_requested: None,
            interactive_displayed: None,
            interactive_texture: None,
            interactive_error: None,
            next_job: 0,
            busy: None,
            queued_render: None,
            progress: (0, 1, String::new()),
            last_task: None,
            last_output: None,
            preview: None,
            preview_revision: None,
            latest_preview_job: 0,
            latest_output_job: 0,
            display_job: 0,
            preview_decode_pending: None,
            output: None,
            live_preview: LivePreview::new(cc.egui_ctx.clone()),
            show_output: false,
            error,
            status: "Open an STL or G-code file to begin.".into(),
            restore_camera: false,
            unresolved_project: None,
            project_path: None,
            size: 960,
            still_size: 4096,
            transparent: false,
            show_edges: false,
            show_tools: false,
            codec: VideoCodec::H264,
            output_format: OutputFormat::default(),
            section: false,
            section_axis: 2,
            section_position: 0.,
            section_positive: false,
            section_cap: true,
            side_by_side: true,
            analysis: None,
            color_signature: String::new(),
            inspection: InspectionMode::Opaque,
            opacity: 0.3,
            buildup: false,
            buildup_height: 0.,
            restored_stage: None,
            buildup_playing: false,
            last_tick: Instant::now(),
            export_buildup: false,
            settle_face: false,
            native_rect: None,
            timeline: Timeline::default(),
            turntable_timing: TurntableTiming::default(),
            frame_rate: crate::animation::DEFAULT_FRAME_RATE,
            timeline_enabled: false,
            timeline_playing: false,
            timeline_time: 0.,
            timeline_angle: 0.,
            adaptive_text: String::new(),
            cache_records: Vec::new(),
            cache_usage: 0,
            cache_query_pending: false,
            cache_action_pending: false,
            cache_refreshed: Instant::now() - Duration::from_secs(60),
            smoke_output: options.smoke_output,
            smoke_stage: 0,
            smoke_checks: serde_json::json!({}),
            started: Instant::now(),
        };
        if let Some(path) = options.open {
            studio.import(path);
        }
        Ok(studio)
    }
    fn viewport_settings(&self) -> ViewportSettings {
        let sheet = self.output_format == OutputFormat::FourView;
        let beads = self
            .model
            .as_ref()
            .is_some_and(|model| model.geometry_kind == "beads");
        ViewportSettings {
            color: if sheet {
                [0.32, 0.35, 0.4]
            } else {
                self.scene.color
            },
            roughness: if sheet { 0.6 } else { self.scene.roughness },
            layer_appearance: !sheet
                && !beads
                && self.scene.layer_appearance
                && !self.scene.import.stepped_envelope,
            layer_height_mm: self.scene.import.layer_height_mm as f32,
            first_layer_height_mm: self.scene.import.first_layer_height_mm as f32,
            groove_depth_mm: self.scene.groove_depth_mm as f32,
            display_rotation_degrees: if sheet {
                [0.; 3]
            } else {
                self.scene.display_rotation_degrees.map(|v| v as f32)
            },
            show_edges: self.show_edges,
            section: if self.section && self.output_format.supports_section() {
                Some(SectionPlane {
                    axis: self.section_axis,
                    position_mm: self.section_position as f32,
                    keep_positive: self.section_positive,
                })
            } else {
                None
            },
            buildup_height_mm: if self.buildup {
                Some(self.model.as_ref().map_or(self.buildup_height, |model| {
                    crate::presentation::visible_layer_height(model, self.buildup_height as f64)
                        as f32
                }))
            } else {
                None
            },
            buildup_complete_layers: self
                .model
                .as_ref()
                .is_none_or(|model| model.layer_tops_mm.is_empty()),
            inspection_mode: self.inspection,
            opacity: self.opacity,
            turntable_angle_degrees: if sheet { 0. } else { self.timeline_angle },
        }
    }
    fn render_event_matches(
        &self,
        id: u64,
        revision: u64,
        engine: crate::render::BackendId,
        generation: u64,
    ) -> bool {
        self.busy == Some(id)
            && revision == self.revision
            && generation == self.view_generation
            && self
                .last_task
                .as_ref()
                .is_some_and(|(request, _, _, _, _)| request.selection.backend == engine)
    }
    fn changed(&mut self) {
        self.render.cancel_preview();
        if self
            .queued_render
            .as_ref()
            .is_some_and(|(task, _)| task.preview)
        {
            self.queued_render = None;
            self.busy = None;
        }
        self.live_preview.stop();
        self.revision += 1;
        self.view_generation += 1;
        self.renderer_warnings.clear();
        self.changed_at = Instant::now();
    }
    fn import(&mut self, path: PathBuf) {
        self.live_preview.stop();
        self.settle_face = false;
        self.timeline_playing = false;
        if !self.restore_camera {
            self.restored_stage = None;
        }
        self.debounced_import = None;
        self.unresolved_project = None;
        self.selected_source = Some(path.clone());
        let generation = self.gate.advance();
        self.importing = true;
        self.status = format!("Loading {}…", path.display());
        self.pending_import = Some((generation, path, self.scene.import.clone()));
        self.begin_pending_import();
    }
    fn begin_pending_import(&mut self) {
        if self.import_active {
            return;
        }
        let Some((generation, path, settings)) = self.pending_import.take() else {
            return;
        };
        self.import_active = true;
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = load_model(&path, &settings)
                .map(Box::new)
                .map_err(|error| error.to_string());
            let _ = sender.send(Message::Imported { generation, result });
        });
    }
    fn fit(&mut self) {
        if let Some(bounds) = self.viewport.display_bounds(&self.viewport_settings()) {
            self.camera.fit(&bounds);
        }
    }
    fn settle_hit(&mut self, hit: crate::viewport::FaceHit) -> Result<()> {
        self.scene.display_rotation_degrees = crate::model::orientation::settled_display_rotation(
            self.scene.display_rotation_degrees,
            hit.normal_mm,
        )?;
        self.settle_face = false;
        self.timeline_playing = false;
        self.timeline_angle = 0.;
        self.fit();
        self.changed();
        if self.scene.import.stepped_envelope {
            self.prepare_stepped_down()?;
            self.restore_camera = true;
            let path = self
                .model
                .as_ref()
                .ok_or("open a model first")?
                .path
                .clone();
            self.import(path);
            self.status = "Rebuilding stepped layers from the selected face down…".into();
        } else {
            self.status = if self
                .model
                .as_ref()
                .is_some_and(|model| hit.triangle_index >= model.mesh.triangles.len())
            {
                "Build-up cut face now points down. Print layers retain their build orientation."
                    .into()
            } else {
                format!(
                    "Face {} now points down. Print layers retain their build orientation.",
                    hit.triangle_index + 1
                )
            };
        }
        Ok(())
    }
    fn prepare_stepped_down(&mut self) -> Result<()> {
        if self.pending_build_orientation.is_some() {
            return Err("Wait for the current build-orientation import to finish.".into());
        }
        self.pending_build_orientation = Some(build_orientation::PendingBuildOrientation::prepare(
            &mut self.scene,
        )?);
        self.timeline_playing = false;
        self.settle_face = false;
        Ok(())
    }
    fn settle_pointer(
        &mut self,
        rect: egui::Rect,
        pointer: egui::Pos2,
    ) -> Option<crate::viewport::FaceHit> {
        if self.importing {
            return None;
        }
        let settings = self.viewport_settings();
        let model = self.model.as_ref()?;
        let hit = self
            .viewport
            .pick_at(&model.picking, &self.camera, &settings, rect, pointer)?;
        if let Err(error) = self.settle_hit(hit) {
            self.error = Some(error.to_string());
            return None;
        }
        Some(hit)
    }
    fn sample_timeline(&mut self) {
        let mut timeline = self.timeline.clone();
        timeline.buildup |= self.export_buildup;
        match timeline.sample(self.timeline_time) {
            Ok(sample) => {
                self.camera = sample.camera;
                self.timeline_angle = sample.turntable_degrees as f32;
                if let Some(fraction) = sample.buildup_fraction
                    && let Some(model) = &self.model
                    && !model.layer_tops_mm.is_empty()
                {
                    let count = ((fraction * model.layer_tops_mm.len() as f64).floor() as usize
                        + 1)
                    .min(model.layer_tops_mm.len());
                    self.buildup_height = count
                        .checked_sub(1)
                        .and_then(|i| model.layer_tops_mm.get(i))
                        .copied()
                        .unwrap_or(0.) as f32;
                    self.buildup = true;
                    self.buildup_playing = false;
                }
                self.changed();
            }
            Err(error) => {
                self.timeline_playing = false;
                self.error = Some(error.to_string());
            }
        }
    }
    fn save_prefs(&mut self) {
        self.prefs.renderers = self.renderers.clone();
        if let Err(error) = self.prefs.save(&self.data_dir.join("settings.json")) {
            self.error = Some(format!("Could not save settings: {error}"));
        }
    }
    fn snapshot(&self, request: &ExportRequest) -> Result<serde_json::Value> {
        let model = self.model.as_ref().ok_or("open a model first")?;
        let scene = model_scene(
            model,
            &self.scene,
            &request.job_dir.join("mesh.bin"),
            Some(self.camera.camera_spec()),
        )?;
        Ok(scene)
    }
    fn preview_selection(&self) -> &RenderSelection {
        renderers::preview_selection(&self.renderers, self.output_format)
    }
    fn preview_kind(&self) -> ExportKind {
        if self.output_format == OutputFormat::FourView {
            ExportKind::Diagram
        } else {
            ExportKind::Still
        }
    }
    fn start_export(&mut self, kind: ExportKind, destination: PathBuf, preview: bool) {
        if self.busy.is_some() || self.importing || self.cache_action_pending {
            return;
        }
        let kind = if preview { self.preview_kind() } else { kind };
        let result = (|| -> Result<()> {
            self.next_job += 1;
            let cache = CacheStore::new(self.data_dir.join("jobs"))?;
            let render_size = if preview {
                768
            } else if matches!(kind, ExportKind::Still | ExportKind::Diagram) {
                self.still_size
            } else {
                self.size
            };
            let timeline = if self.timeline_enabled
                && !preview
                && matches!(kind, ExportKind::Gif | ExportKind::Mp4)
            {
                let mut timeline = self.timeline.clone();
                timeline.buildup |= self.export_buildup;
                Some(timeline)
            } else {
                None
            };
            let turntable_timing = (!preview
                && timeline.is_none()
                && !self.export_buildup
                && matches!(kind, ExportKind::Gif | ExportKind::Mp4))
            .then_some(self.turntable_timing);
            let frames = if let Some(timeline) = &timeline {
                timeline.frame_count_at_rate(self.frame_rate)?
            } else if let Some(timing) = turntable_timing {
                timing.frame_count_at_rate(self.frame_rate)?
            } else {
                crate::animation::frame_count_for_duration(4., self.frame_rate)?
            };
            // Resolve the scene before choosing its stable, format-independent cache job.
            let job = self.data_dir.join("jobs").join("pending-render");
            let destination = if destination.is_absolute() {
                destination
            } else {
                std::env::current_dir()?.join(destination)
            };
            let mut request = default_request(
                job,
                destination,
                kind,
                {
                    let mut selection = if preview {
                        self.preview_selection().clone()
                    } else if matches!(kind, ExportKind::Still | ExportKind::Diagram) {
                        self.renderers.still.clone()
                    } else {
                        self.renderers.animation.clone()
                    };
                    if preview && let Some(samples) = selection.quality.samples_mut() {
                        *samples = (*samples).min(16);
                    }
                    selection
                },
                render_size,
                self.transparent
                    && kind == ExportKind::Still
                    && (!preview || self.output_format.supports_transparency()),
            );
            crate::render::requirements::validate_requirements(
                &self.prefs.tools,
                &crate::render::requirements::requirements_for(&request.selection, kind, false),
            )?;
            request.selection.validate()?;
            request.timeline = timeline;
            request.turntable_timing = turntable_timing;
            request.frame_count = frames;
            request.frame_rate =
                if preview || matches!(kind, ExportKind::Still | ExportKind::Diagram) {
                    crate::animation::DEFAULT_FRAME_RATE
                } else {
                    self.frame_rate
                };
            if self.section && kind != ExportKind::Diagram {
                request.section = Some(SectionSpec {
                    axis: self.section_axis,
                    position_mm: self.section_position,
                    keep_positive: self.section_positive,
                    cap: self.section_cap,
                });
                request.layout = if self.side_by_side {
                    ExportLayout::SideBySide
                } else {
                    ExportLayout::Single
                };
                if kind == ExportKind::Gif {
                    request.kind = ExportKind::SectionTurntable;
                }
                if kind == ExportKind::Still {
                    request.kind = ExportKind::SectionStill;
                }
            }
            request.video_codec = self.codec;
            request.animation = if self.export_buildup
                && !preview
                && matches!(kind, ExportKind::Gif | ExportKind::Mp4)
            {
                AnimationMode::BuildUp
            } else {
                AnimationMode::Turntable
            };
            let animated = !preview && matches!(kind, ExportKind::Gif | ExportKind::Mp4);
            if !animated {
                request.start_angle_degrees = f64::from(self.timeline_angle);
            }
            let progressive = animated
                && (request.animation == AnimationMode::BuildUp
                    || request.timeline.as_ref().is_some_and(|t| t.buildup));
            if self.buildup
                && !progressive
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
            let mut scene = self.snapshot(&request)?;
            let model = self.model.as_ref().ok_or("open a model first")?;
            let assessment = renderers::assess_selection(
                &request.selection,
                &crate::render::contract::snapshot_scene(model.mesh.clone(), scene.clone())?,
                &request,
                &self.renderer_statuses,
            );
            if !assessment.errors.is_empty() {
                return Err(assessment.errors.join("; ").into());
            }
            self.renderer_warnings = assessment.warnings;
            let key = crate::export::identity::retained_render_key(
                &scene,
                &model.snapshot_hash,
                &request,
            )?;
            request.job_dir = cache.render_job(&key)?;
            request.scene_path = request.job_dir.join("scene.json");
            scene["mesh_path"] = serde_json::json!(request.job_dir.join("mesh.bin"));
            let mesh = self
                .model
                .as_ref()
                .ok_or("open a model first")?
                .mesh
                .clone();
            self.last_task = Some((
                request.clone(),
                scene.clone(),
                mesh.clone(),
                preview,
                self.revision,
            ));
            self.submit_when_viewport_released(
                RenderTask {
                    id: self.next_job,
                    revision: self.revision,
                    mesh,
                    scene,
                    request,
                    preview,
                    tools: self.prefs.tools.clone(),
                    cache_budget_bytes: self.prefs.cache_budget_bytes,
                    memory: self.prefs.render_memory,
                    generation: self.view_generation,
                },
                false,
            )
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            self.busy = Some(self.next_job);
            self.display_job = self.next_job;
            self.live_preview.start(self.next_job, self.revision);
            if preview {
                self.show_output = false;
            }
            if preview {
                self.latest_preview_job = self.next_job;
            } else {
                self.latest_output_job = self.next_job;
            }
            self.progress = (0, 1, "Starting renderer".into());
            self.error = None;
            Ok(())
        })();
        if let Err(error) = result {
            self.error = Some(error.to_string());
        }
    }
    fn retry(&mut self) {
        if self.busy.is_some() || self.cache_action_pending {
            return;
        }
        if let Some((request, scene, mesh, preview, revision)) = self.last_task.clone() {
            self.next_job += 1;
            match self.submit_when_viewport_released(
                RenderTask {
                    id: self.next_job,
                    revision,
                    mesh,
                    scene,
                    request,
                    preview,
                    tools: self.prefs.tools.clone(),
                    cache_budget_bytes: self.prefs.cache_budget_bytes,
                    memory: self.prefs.render_memory,
                    generation: self.view_generation,
                },
                true,
            ) {
                Ok(()) => {
                    self.busy = Some(self.next_job);
                    self.display_job = self.next_job;
                    self.live_preview.start(self.next_job, revision);
                    if preview {
                        self.latest_preview_job = self.next_job;
                    } else {
                        self.latest_output_job = self.next_job;
                    }
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
    }
    fn refresh_cache(&mut self) {
        if self.cache_query_pending {
            return;
        }
        self.cache_query_pending = true;
        let sender = self.sender.clone();
        let root = self.data_dir.join("jobs");
        thread::spawn(move || {
            let result = (|| -> Result<_> {
                let cache = CacheStore::new(root)?;
                Ok((cache.usage()?, cache.records()?))
            })();
            let _ = sender.send(Message::CacheUpdated(
                result.map_err(|error| error.to_string()),
            ));
        });
    }
    fn recover_job(&mut self, path: PathBuf) {
        if self.busy.is_some() || self.cache_action_pending {
            return;
        }
        self.cache_action_pending = true;
        let root = self.data_dir.join("jobs");
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = (|| -> Result<_> {
                let cache = CacheStore::new(root)?;
                let path = cache.owned_job(&path)?;
                let request_bytes =
                    crate::settings::read_bounded(&path.join("request.json"), 1024 * 1024)?;
                let scene_bytes =
                    crate::settings::read_bounded(&path.join("scene.json"), 1024 * 1024)?;
                if request_bytes.len() > 1024 * 1024 || scene_bytes.len() > 1024 * 1024 {
                    return Err("Recovery metadata exceeds budget".into());
                }
                let request: ExportRequest = serde_json::from_slice(&request_bytes)?;
                if fs::canonicalize(&request.job_dir)? != path
                    || fs::canonicalize(&request.scene_path)? != path.join("scene.json")
                {
                    return Err("Recovery request references a different job directory".into());
                }
                let scene: serde_json::Value = serde_json::from_slice(&scene_bytes)?;
                let mesh =
                    std::sync::Arc::new(crate::mesh::Mesh::read_binary(&path.join("mesh.bin"))?);
                Ok(Box::new((request, scene, mesh)))
            })();
            let _ = sender.send(Message::RecoveredJob(
                result.map_err(|error| error.to_string()),
            ));
        });
    }
    fn record_result(&mut self, state: &str, error: Option<String>) {
        if let Some((request, _, _, _, _)) = &self.last_task
            && let Ok(cache) = CacheStore::new(self.data_dir.join("jobs"))
            && let Ok(_guard) = cache.lock_job(&request.job_dir)
        {
            let _ = cache.save_record(
                &request.job_dir,
                &JobRecord {
                    schema_version: 1,
                    label: format!("{:?}", request.kind),
                    state: state.into(),
                    destination: request.destination.clone(),
                    error,
                },
            );
        }
    }
    fn pick_model(&self) {
        let sender = self.sender.clone();
        thread::spawn(move || {
            let _ = sender.send(Message::OpenModel(
                rfd::FileDialog::new()
                    .add_filter("Models", &["stl", "gcode", "gco"])
                    .pick_file(),
            ));
        });
    }
    fn pick_project(&self) {
        let sender = self.sender.clone();
        thread::spawn(move || {
            let _ = sender.send(Message::OpenProject(
                rfd::FileDialog::new()
                    .add_filter("Turninator project", &["turninator"])
                    .pick_file(),
            ));
        });
    }
    fn pick_save_project(&self) {
        let sender = self.sender.clone();
        thread::spawn(move || {
            let _ = sender.send(Message::SaveProject(
                rfd::FileDialog::new()
                    .add_filter("Turninator project", &["turninator"])
                    .set_file_name("scene.turninator")
                    .save_file(),
            ));
        });
    }
    fn pick_export(&self, kind: ExportKind) {
        let Some(model) = &self.model else {
            return;
        };
        let file_name = kind.suggested_file_name(&model.path);
        let generation = self.gate.current();
        let sender = self.sender.clone();
        thread::spawn(move || {
            let extension = kind.extension();
            let path = rfd::FileDialog::new()
                .add_filter("Output", &[extension])
                .set_file_name(file_name)
                .save_file();
            let _ = sender.send(Message::ExportDestination {
                kind,
                generation,
                path,
            });
        });
    }
    fn receive(&mut self, ctx: &egui::Context) {
        if self.queued_render.is_some() && self.interactive.is_released() {
            let (task, recovery) = self.queued_render.take().unwrap();
            let result = if recovery {
                self.render.submit_recovery(task)
            } else {
                self.render.submit(task)
            };
            if let Err(error) = result {
                self.busy = None;
                self.error = Some(error);
            }
        }
        while let Ok(message) = self.messages.try_recv() {
            if matches!(&message, Message::Imported { .. }) {
                self.import_active = false;
            }
            match message {
                Message::CacheUpdated(result) => {
                    self.cache_query_pending = false;
                    self.cache_refreshed = Instant::now();
                    match result {
                        Ok((usage, records)) => {
                            self.cache_usage = usage;
                            self.cache_records = records;
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                Message::RemovedJob(result) => {
                    self.cache_action_pending = false;
                    if let Err(error) = result {
                        self.error = Some(error);
                    }
                    self.refresh_cache();
                }
                Message::RecoveredJob(result) => {
                    self.cache_action_pending = false;
                    if self.busy.is_some() {
                        self.error=Some("Recovery was superseded by an active export; retry recovery after it finishes.".into());
                        continue;
                    }
                    match result {
                        Ok(snapshot) => {
                            let (request, scene, mesh) = *snapshot;
                            self.last_task = Some((request, scene, mesh, false, self.revision));
                            self.retry();
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                Message::Imported { generation, result } if self.gate.accepts(generation) => {
                    self.importing = false;
                    match result {
                        Ok(model) => {
                            let model = *model;
                            if model.import_settings != self.scene.import {
                                self.import(model.path.clone());
                                continue;
                            }
                            let accepted_display = match self
                                .pending_build_orientation
                                .as_ref()
                                .map(|pending| {
                                    pending.accepted_display_rotation(
                                        self.scene.display_rotation_degrees,
                                    )
                                })
                                .transpose()
                            {
                                Ok(rotation) => rotation,
                                Err(error) => {
                                    self.restore_accepted_import();
                                    self.error = Some(error.to_string());
                                    continue;
                                }
                            };
                            match self.viewport.set_mesh(
                                &model.mesh,
                                &model.geometry_kind,
                                generation,
                            ) {
                                Ok(()) => {
                                    if let Some(rotation) = accepted_display {
                                        self.scene.display_rotation_degrees = rotation;
                                        self.pending_build_orientation = None;
                                    }
                                    if !self.restore_camera {
                                        self.buildup = false;
                                        self.buildup_playing = false;
                                        self.timeline_enabled = false;
                                        self.timeline = Timeline::default();
                                        self.timeline_angle = 0.;
                                    }
                                    self.status = format!(
                                        "{} • {} triangles",
                                        model.path.display(),
                                        model.mesh.triangles.len()
                                    );
                                    if !self.restore_camera {
                                        let axis = self.section_axis as usize;
                                        self.section_position = (model.bounds_mm.min[axis]
                                            + model.bounds_mm.max[axis])
                                            / 2.;
                                    }
                                    self.prefs.remember(model.path.clone());
                                    let mut tops = Vec::new();
                                    if !model.groups.is_empty() {
                                        tops = vec![0.; model.mesh.triangles.len()];
                                        for group in &model.groups {
                                            tops[group.start_triangle..group.end_triangle]
                                                .fill(group.z_top_mm as f32);
                                        }
                                    }
                                    if let Err(error) = self.viewport.set_triangle_layer_tops(&tops)
                                    {
                                        self.buildup = false;
                                        self.error = Some(format!(
                                            "Layer inspection unavailable for this mesh: {error}"
                                        ));
                                    }
                                    self.model = Some(model);
                                    if self
                                        .model
                                        .as_ref()
                                        .is_none_or(|model| model.layer_tops_mm.is_empty())
                                    {
                                        self.export_buildup = false;
                                        self.timeline.buildup = false;
                                        self.buildup = false;
                                    }
                                    self.color_signature.clear();
                                    self.buildup_height = self
                                        .model
                                        .as_ref()
                                        .map_or(0., |m| m.bounds_mm.max[2] as f32);
                                    if let Some(count) = self.restored_stage.take() {
                                        self.buildup_height = self
                                            .model
                                            .as_ref()
                                            .and_then(|m| {
                                                (count as usize)
                                                    .min(m.layer_tops_mm.len())
                                                    .checked_sub(1)
                                                    .map(|i| m.layer_tops_mm[i] as f32)
                                            })
                                            .unwrap_or(0.);
                                    }
                                    if !self.restore_camera {
                                        self.fit();
                                    }
                                    self.restore_camera = false;
                                    self.changed();
                                    self.save_prefs();
                                }
                                Err(error) => {
                                    self.restore_accepted_import();
                                    self.error = Some(error.to_string());
                                }
                            }
                        }
                        Err(error) => {
                            self.status = "Import failed; previous model remains available.".into();
                            self.restore_accepted_import();
                            self.error = Some(error);
                        }
                    }
                }
                Message::Imported { .. } => {}
                Message::OpenModel(Some(path)) => {
                    self.restore_camera = false;
                    self.import(path);
                }
                Message::OpenProject(Some(path)) => {
                    self.pending_import = None;
                    self.debounced_import = None;
                    self.unresolved_project = None;
                    self.restore_accepted_import();
                    let generation = self.gate.advance();
                    self.importing = true;
                    self.status = format!("Loading project {}…", path.display());
                    let sender = self.sender.clone();
                    thread::spawn(move || {
                        let result = load_project(&path)
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        let _ = sender.send(Message::LoadedProject {
                            generation,
                            path,
                            result,
                        });
                    });
                }
                Message::LoadedProject {
                    generation,
                    path,
                    result,
                } if self.gate.accepts(generation) => match result.map(|project| *project) {
                    Ok((project, SourceStatus::Current)) => {
                        self.project_path = Some(path);
                        self.restore_project(project);
                    }
                    Ok((project, status)) => {
                        self.importing = false;
                        self.project_path = Some(path);
                        self.unresolved_project = Some((project, status));
                    }
                    Err(error) => {
                        self.importing = false;
                        self.error = Some(error);
                    }
                },
                Message::SaveProject(Some(path)) => {
                    if self.importing
                        || self
                            .model
                            .as_ref()
                            .is_some_and(|model| model.import_settings != self.scene.import)
                    {
                        self.error = Some(
                            "Finish a successful model import before saving the project.".into(),
                        );
                        continue;
                    }
                    if let Some(model) = &self.model {
                        let project = ProjectFile {
                            schema_version: 2,
                            source_path: model.path.clone(),
                            source_hash: model.source_hash.clone(),
                            scene: self.scene.clone(),
                            camera: self.camera.clone(),
                            export_size: self.size,
                            renderers: self.renderers.clone(),
                            output_options: self.output_preferences(),
                        };
                        match save_project(&path, &project) {
                            Ok(()) => {
                                self.status = format!("Saved project {}", path.display());
                                self.project_path = Some(path);
                            }
                            Err(error) => self.error = Some(error.to_string()),
                        }
                    }
                }
                Message::ExportDestination {
                    kind,
                    generation,
                    path: Some(path),
                } => {
                    if self.gate.accepts(generation) && !self.importing {
                        self.start_export(kind, path, false);
                    } else {
                        self.status =
                            "Model changed while the save dialog was open. Choose an export again."
                                .into();
                    }
                }
                Message::Image {
                    id,
                    revision,
                    engine,
                    generation,
                    preview,
                    result,
                } => {
                    if preview && self.preview_decode_pending == Some(id) {
                        self.preview_decode_pending = None;
                    }
                    if generation != self.view_generation
                        || revision != self.revision
                        || (preview
                            && (engine != self.preview_selection().backend
                                || id != self.latest_preview_job))
                        || (!preview && id != self.latest_output_job)
                    {
                        continue;
                    }
                    if id == self.display_job {
                        self.live_preview.stop();
                    }
                    match result {
                        Ok(decoded) => {
                            if preview {
                                if revision == self.revision {
                                    self.preview = Some(Playback::new(ctx, *decoded));
                                    self.preview_revision = Some(revision);
                                    self.preview_identity = Some(renderer_view::FrameIdentity {
                                        job: id,
                                        revision,
                                        backend: engine,
                                        generation,
                                    });
                                }
                            } else {
                                self.output = Some(Playback::new(ctx, *decoded));
                                self.output_identity = Some(renderer_view::FrameIdentity {
                                    job: id,
                                    revision,
                                    backend: engine,
                                    generation,
                                });
                                if id == self.display_job {
                                    self.show_output = true;
                                }
                            }
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                _ => {}
            }
            self.begin_pending_import();
        }
        while let Ok(event) = self.render.events.try_recv() {
            match event {
                RenderEvent::Prepared {
                    id,
                    revision,
                    generation,
                    engine,
                    identity,
                    assessment,
                } if self.render_event_matches(id, revision, engine, generation) => {
                    self.renderer_identities.insert(engine, identity);
                    self.renderer_warnings = assessment
                        .into_iter()
                        .filter_map(|support| match support {
                            crate::render::capabilities::FeatureSupport::Approximate { reason } => {
                                Some(reason)
                            }
                            _ => None,
                        })
                        .collect();
                    if let Some(status) = self
                        .renderer_statuses
                        .iter_mut()
                        .find(|s| s.backend == engine)
                    {
                        status.availability =
                            crate::render::capabilities::BackendAvailability::Available;
                    }
                }
                RenderEvent::Progress {
                    id,
                    revision,
                    event,
                    engine,
                    generation,
                } if self.render_event_matches(id, revision, engine, generation) => match event {
                    JobEvent::Progress {
                        completed,
                        total,
                        stage,
                    } => self.progress = (completed, total, stage),
                    JobEvent::Log(line) => self.status = line,
                    JobEvent::Analysis(value) => self.analysis = Some(value),
                    JobEvent::Frame(path) => {
                        self.live_preview.submit(id, self.revision, path);
                    }
                },
                RenderEvent::Completed {
                    id,
                    revision,
                    preview,
                    result,
                    engine,
                    generation,
                } if self.render_event_matches(id, revision, engine, generation) => {
                    self.busy = None;
                    self.record_result("completed", None);
                    self.status = format!(
                        "Saved {} ({} × {}, {:.1} MiB)",
                        result.path.display(),
                        result.width,
                        result.height,
                        result.bytes as f64 / 1048576.
                    );
                    if !preview {
                        self.last_output = Some(result.clone());
                    }
                    if result.path.extension().is_some_and(|v| v == "mp4") {
                        self.live_preview.stop();
                        self.output = None;
                        self.show_output = false;
                        continue;
                    }
                    let sender = self.sender.clone();
                    if preview {
                        self.preview_decode_pending = Some(id);
                    }
                    thread::spawn(move || {
                        let decoded = decode_output(&result.path, sender.context.clone())
                            .map(Box::new)
                            .map_err(|error| error.to_string());
                        let _ = sender.send(Message::Image {
                            id,
                            revision,
                            engine,
                            generation,
                            preview,
                            result: decoded,
                        });
                    });
                }
                RenderEvent::Failed {
                    id,
                    message,
                    revision,
                    engine,
                    generation,
                } if self.render_event_matches(id, revision, engine, generation) => {
                    self.busy = None;
                    self.live_preview.stop();
                    self.record_result("failed", Some(message.clone()));
                    self.error = Some(message);
                }
                RenderEvent::Completed { id, .. } | RenderEvent::Failed { id, .. }
                    if self.busy == Some(id) =>
                {
                    self.busy = None;
                }
                _ => {}
            }
        }
        if let Some((ticket, image)) = self.render.take_partial()
            && self.render_event_matches(
                ticket.id,
                ticket.revision,
                ticket.engine,
                ticket.generation,
            )
            && image.width > 0
            && image.height > 0
            && image.width <= 16000
            && image.height <= 8000
            && image.bytes.len() as u64 == u64::from(image.width) * u64::from(image.height) * 4
        {
            let identity = renderer_view::FrameIdentity {
                job: ticket.id,
                revision: ticket.revision,
                backend: ticket.engine,
                generation: ticket.generation,
            };
            let pixels = egui::ColorImage::from_rgba_unmultiplied(
                [image.width as usize, image.height as usize],
                &image.bytes,
            );
            if let Some((tag, texture)) = &mut self.partial_image {
                texture.set(pixels, egui::TextureOptions::LINEAR);
                *tag = identity;
            } else {
                self.partial_image = Some((
                    identity,
                    ctx.load_texture("render-progress", pixels, egui::TextureOptions::LINEAR),
                ));
            }
        }
        match self.live_preview.receive(ctx, self.revision) {
            Ok(true) if self.smoke_output.is_some() => {
                let count = self.smoke_checks["live_render_images_displayed"]
                    .as_u64()
                    .unwrap_or(0)
                    + 1;
                self.smoke_checks["live_render_images_displayed"] = serde_json::json!(count);
                if count == 1 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                        "live-render",
                    )));
                }
            }
            Err(error) => self.status = error,
            _ => {}
        }
    }
    fn restore_project(&mut self, project: ProjectFile) {
        self.pending_build_orientation = None;
        let options = project.output_options;
        self.still_size = options.still_size;
        self.transparent = options.transparent;
        self.codec = options.video_codec;
        self.output_format = options.output_format;
        self.export_buildup = options.animation == AnimationMode::BuildUp;
        self.turntable_timing = options.turntable_timing;
        self.frame_rate = options.frame_rate;
        self.timeline_enabled = options.timeline.is_some();
        self.timeline = options.timeline.unwrap_or_default();
        self.timeline_playing = false;
        self.timeline_time = 0.;
        self.timeline_angle = options.start_angle_degrees as f32;
        self.buildup = options.build_stage.is_some();
        self.buildup_playing = false;
        self.restored_stage = options.build_stage.map(|stage| stage.visible_layers);
        self.side_by_side = options.layout == ExportLayout::SideBySide;
        self.section = options.section.is_some();
        if let Some(section) = options.section {
            self.section_axis = section.axis;
            self.section_position = section.position_mm;
            self.section_positive = section.keep_positive;
            self.section_cap = section.cap;
        }
        self.scene = project.scene;
        self.adaptive_text = self
            .scene
            .import
            .layer_boundaries_mm
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        self.camera = project.camera;
        self.size = project.export_size;
        self.renderers = project.renderers;
        self.restore_camera = true;
        self.unresolved_project = None;
        self.import(project.source_path);
    }
    fn restore_accepted_import(&mut self) {
        self.pending_build_orientation = None;
        self.restored_stage = None;
        self.restore_camera = false;
        if let Some(model) = &self.model {
            self.scene.import = model.import_settings.clone();
            self.selected_source = Some(model.path.clone());
            self.changed();
        }
    }
    fn dismiss_project_recovery(&mut self) {
        self.unresolved_project = None;
        self.importing = false;
        self.pending_import = None;
        self.debounced_import = None;
        self.gate.advance();
        self.restore_accepted_import();
    }
    fn output_preferences(&self) -> OutputPreferences {
        OutputPreferences {
            output_format: self.output_format,
            still_size: self.still_size,
            frame_rate: self.frame_rate,
            transparent: self.transparent,
            video_codec: self.codec,
            animation: if self.export_buildup {
                AnimationMode::BuildUp
            } else {
                AnimationMode::Turntable
            },
            section: self.section.then_some(SectionSpec {
                axis: self.section_axis,
                position_mm: self.section_position,
                keep_positive: self.section_positive,
                cap: self.section_cap,
            }),
            layout: if self.side_by_side {
                ExportLayout::SideBySide
            } else {
                ExportLayout::Single
            },
            timeline: self.timeline_enabled.then(|| self.timeline.clone()),
            turntable_timing: self.turntable_timing,
            build_stage: self
                .model
                .as_ref()
                .filter(|m| self.buildup && !m.layer_tops_mm.is_empty())
                .map(|model| crate::export::BuildStage {
                    visible_layers: crate::presentation::visible_layer_count(
                        model,
                        f64::from(self.buildup_height),
                    ) as u32,
                }),
            start_angle_degrees: f64::from(self.timeline_angle),
        }
    }
    fn timeline_controls(&mut self, ui: &mut egui::Ui) {
        workspace::section(ui, "Animation motion");
        if ui.checkbox(&mut self.timeline_enabled, "Use camera keyframes").help("Animate the camera between recorded views instead of using the basic turntable controls. The timeline sets clip duration and can also rotate the model or reveal deposited layers.").changed() {
                self.timeline_playing = false;
                self.timeline_angle = 0.;
                if self.timeline_enabled && self.timeline.keyframes.len() == 1 {
                    self.timeline.keyframes[0].camera = self.camera.clone();
                }
                self.changed();
            }
        if !self.timeline_enabled {
            return;
        }
        ui.add(egui::DragValue::new(&mut self.timeline.duration_seconds).range(0.05..=500.).speed(0.1).suffix(" seconds")).help("Total camera-animation duration in seconds, rounded to the nearest frame at the selected export framerate. All key times must fit within this duration.");
        ui.checkbox(&mut self.timeline.looping, "Close camera path into a loop").help("Return the camera to its first recorded view before the clip repeats. Leave time after the last key for a closing transition, or finish with a matching camera; incompatible endpoints are rejected.");
        ui.checkbox(&mut self.timeline.turntable, "Rotate the model too").help("Add one full model rotation over the camera timeline while lights stay fixed. Camera motion and deposited-layer build-up can run at the same time.");
        let deposited = self
            .model
            .as_ref()
            .is_some_and(|m| !m.layer_tops_mm.is_empty());
        ui.small("Set a time, orbit/pan to compose a view, then record it. Export uses the selected framerate. Transition settings belong to the segment leaving each key.");
        let scrub = ui.add(egui::Slider::new(&mut self.timeline_time, 0.0..=self.timeline.duration_seconds).text("Time (s)")).help("Scrub this camera path in seconds to preview the camera and deposited stage in native inspection. Scrubbing pauses playback; it does not change clip length or key times.");
        let mut sample = scrub.changed();
        if sample {
            self.timeline_playing = false;
        }
        ui.horizontal(|ui| {
                if ui.button("Record this view").help("Store the current camera at the selected time, including pan, zoom and projection. Recording at an existing key time replaces that view. Transition and easing belong to the segment leaving a key.").clicked() {
                    let key = CameraKeyframe { time_seconds: self.timeline_time, camera: self.camera.clone(), ..Default::default() };
                    if let Some(existing) = self.timeline.keyframes.iter_mut().find(|k| (k.time_seconds - self.timeline_time).abs() < 1e-6) {
                        existing.camera = key.camera;
                    } else if self.timeline.keyframes.len() < 256 {
                        self.timeline.keyframes.push(key);
                        self.timeline.keyframes.sort_by(|a,b| a.time_seconds.total_cmp(&b.time_seconds));
                    }
                }
                if ui.button(if self.timeline_playing { "Pause" } else { "Play" }).help("Play or pause this camera path in native inspection. This previews camera motion without rendering a file; exported GIF and MP4 clips use the same sampled path.").clicked()
                    && self.timeline.validate().is_ok() {
                        self.timeline_playing = !self.timeline_playing;
                        self.buildup_playing = false;
                        if self.timeline_time >= self.timeline.duration_seconds { self.timeline_time = 0.; }
                        sample = self.timeline_playing;
                }
                if ui.button("Reset motion").help("Replace the authored camera path with one key at the current view and the default duration. This removes the existing keys and their transition settings.").clicked() {
                    self.timeline = Timeline { keyframes: vec![CameraKeyframe {camera:self.camera.clone(), ..Default::default()}], ..Default::default() };
                    self.timeline_time = 0.;
                    self.timeline_angle = 0.;
                    self.timeline_playing = false;
                    self.changed();
                }
            });
        let mut remove = None;
        let mut recall = None;
        let key_count = self.timeline.keyframes.len();
        for (index, key) in self.timeline.keyframes.iter_mut().enumerate() {
            ui.push_id(index, |ui| {
                    ui.collapsing(format!("View {} • {:.2} s", index+1,key.time_seconds), |ui| {
                        if index > 0 { ui.add(egui::DragValue::new(&mut key.time_seconds).range(0.001..=self.timeline.duration_seconds).speed(0.05).suffix(" s")).help("Time when the camera reaches this recorded view, in seconds. Keep key times strictly increasing and within the clip duration. The first key stays at zero; changing this value moves the key, not the total clip length."); }
                        ui.small(format!("Yaw {:.1}° • elevation {:.1}° • target {:.1?}",key.camera.yaw_degrees,key.camera.pitch_degrees,key.camera.target_mm));
                        if index + 1 < key_count || self.timeline.looping && key.time_seconds < self.timeline.duration_seconds {
                        egui::ComboBox::from_label("Outgoing motion").selected_text(format!("{:?}",key.transition)).show_ui(ui,|ui|{
                            ui.selectable_value(&mut key.transition,Transition::Orbit,"Orbit / pan").help("Take the shortest yaw path to the next camera while interpolating elevation, pan and zoom. Use Spiral to add complete camera revolutions.");
                            ui.selectable_value(&mut key.transition,Transition::Spiral,"Spiral").help("Add signed whole camera revolutions while moving toward the next view. The turn count applies to the segment leaving this key and can approach a top or bottom view.");
                        }).help("Choose how the camera leaves this key for the next one. Orbit/pan takes the shortest yaw path; Spiral adds whole revolutions while changing view.");
                        if key.transition == Transition::Spiral { ui.add(egui::DragValue::new(&mut key.spiral_turns).range(-100.0..=100.).speed(1.).max_decimals(0).suffix(" whole turns")).help("Extra whole camera revolutions before the next view, from -100 to 100. Positive and negative values reverse the spiral direction. Whole turns let the camera meet its recorded endpoint."); }
                        egui::ComboBox::from_label("Easing").selected_text(format!("{:?}",key.easing)).show_ui(ui,|ui|{
                            ui.selectable_value(&mut key.easing,Easing::Linear,"Linear").help("Move at a constant rate through this transition, without slowing at its ends.");
                            ui.selectable_value(&mut key.easing,Easing::SmoothStep,"Ease in / out").help("Start and finish this transition gently, moving faster through the middle. Duration stays the same.");
                        }).help("Control how quickly this camera transition starts and ends. Linear has no easing; Ease in/out slows at both ends.");
                        }
                        ui.horizontal(|ui| {
                            if ui.small_button("Load view").help("Move the native camera to this recorded key and set the time slider to its time. This does not overwrite the stored key.").clicked() { recall = Some((key.camera.clone(),key.time_seconds)); }
                            if ui.small_button("Replace with current view").help("Overwrite this key with the camera currently shown in native inspection, keeping its time and outgoing motion settings.").clicked() { key.camera=self.camera.clone(); }
                            if index>0 && ui.small_button("Remove").help("Remove this recorded camera key. The first key stays at time zero and cannot be removed.").clicked() { remove=Some(index); }
                        });
                    }).help("Inspect or edit this recorded camera key. Times must increase, with the first key at zero; outgoing settings affect the transition to the following view.");
                });
        }
        if let Some(index) = remove {
            self.timeline.keyframes.remove(index);
        }
        if let Some((camera, time)) = recall {
            self.camera = camera;
            self.timeline_time = time;
            self.timeline_angle = 0.;
            self.timeline_playing = false;
            self.changed();
        }
        if let Err(error) = self.timeline.validate() {
            ui.colored_label(egui::Color32::YELLOW, error.to_string());
        } else {
            match self.timeline.frame_count_at_rate(self.frame_rate) {
                Ok(frames) => {
                    ui.small(format!(
                        "{frames} frames • {:.2} s at {} fps",
                        f64::from(frames) / f64::from(self.frame_rate),
                        self.frame_rate
                    ));
                }
                Err(error) => {
                    ui.colored_label(egui::Color32::YELLOW, error.to_string());
                }
            }
        }
        if sample {
            self.sample_timeline();
        }
        if deposited && (self.timeline.buildup || self.export_buildup) {
            ui.small("Build-up reveals whole print layers. STL cuts show the modeled solid; G-code retains deposited paths and gaps. This does not reproduce printer time or nozzle travel.");
        }
    }
    fn tools_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_tools;
        if open && self.cache_refreshed.elapsed() > Duration::from_secs(5) {
            self.refresh_cache();
        }
        egui::Window::new("Settings").open(&mut open).default_width(620.).default_height(540.).resizable(true).vscroll(true).show(ctx, |ui| {
            ui.heading("Rendering tools");
            ui.label("Native renderers work without Blender. Blender is used by Eevee and Cycles; FFmpeg encodes video and GIF output.");
            for (label, path) in [("Blender", &mut self.prefs.tools.blender), ("FFmpeg", &mut self.prefs.tools.ffmpeg), ("FFprobe", &mut self.prefs.tools.ffprobe)] {
                let mut value = path.to_string_lossy().into_owned();
                let label_response = ui.label(label); if ui.add(egui::TextEdit::singleline(&mut value).desired_width(f32::INFINITY)).labelled_by(label_response.id).help(format!("Full path to the {label} executable on this machine. Blender renders images; FFmpeg encodes animations/videos; FFprobe verifies video output. Save settings to keep this path.")).changed() { *path = PathBuf::from(value); }
            }
            ui.separator();
            ui.heading("Render memory maximum");
            ui.small("Maximum per render, not installed or free memory. These limits do not reserve memory. Larger renders can fail when the device cannot allocate the requested resources.");
            ui.add(egui::DragValue::new(&mut self.prefs.render_memory.host_gib).range(1..=64).suffix(" GiB host maximum per render"));
            ui.add(egui::DragValue::new(&mut self.prefs.render_memory.gpu_gib).range(1..=48).suffix(" GiB GPU maximum per render"));
            ui.heading("Renderer details");
            for backend in BackendId::ALL {
                ui.collapsing(renderers::label(backend), |ui| {
                    if let Some(identity) = self.renderer_identities.get(&backend) {
                        if backend == BackendId::Filament { ui.label(format!("SDK: {}", crate::render::backends::filament::SDK_VERSION)); }
                        ui.label(format!("Device: {}", identity.device));
                        ui.label(format!("API: {}", identity.api));
                        ui.label(format!("Driver: {}", identity.driver));
                        ui.label(format!("Runtime: {}", identity.runtime_digest));
                    } else if let Some(status) = self.renderer_statuses.iter().find(|s| s.backend == backend) {
                        match &status.availability {
                            crate::render::capabilities::BackendAvailability::Unavailable { reason } => { ui.label(reason); }
                            _ => { ui.label("Details appear after the first successful render."); }
                        }
                    }
                    if ui.button("Retry discovery").clicked()
                        && let Some(status) = self.renderer_statuses.iter_mut().find(|s| s.backend == backend)
                    {
                        *status = crate::render::registry::BackendRegistry::default().status(backend);
                    }
                });
            }
            ui.separator();
            ui.heading("Retained frames and storage");
            ui.small("Matching exports reuse these frames across formats and app restarts.");
            ui.add(egui::Label::new(format!("Location: {}", self.data_dir.display())).truncate()).help(self.data_dir.display().to_string());
            let mut gb = self.prefs.cache_budget_bytes as f64 / 1073741824.;
            if ui.add(egui::DragValue::new(&mut gb).range(1.0..=1000.).suffix(" GiB job budget")).help("Maximum retained-job storage in GiB. New jobs are refused when their reserved space exceeds this budget; existing files are not automatically deleted. Remove inactive jobs below to free space.").changed() { self.prefs.cache_budget_bytes = (gb * 1073741824.) as u64; }
            if ui.button("Save settings").help("Save external-tool paths, preview behavior and storage budget on this machine. These preferences are separate from portable model projects.").clicked() { self.save_prefs(); self.status = "Settings saved.".into(); }
            ui.small("Failed job frames are retained for Retry. Native inspection works without external render tools.");
                ui.label(format!("Retained jobs: {:.2} GiB", self.cache_usage as f64 / 1073741824.));
                if self.cache_query_pending || self.cache_action_pending {ui.spinner();}
                    for (path, record) in self.cache_records.clone().into_iter().take(20) {
                        ui.group(|ui| {
                            let state=if record.state=="running" && self.busy.is_none() {"Interrupted"} else {&record.state};
                            ui.label(format!("{} — {}", record.label, state));
                            ui.horizontal_wrapped(|ui| {
                            if ui.small_button("Folder").help("Open this retained job folder in the file explorer to inspect its snapshots, rendered frames and diagnostics.").clicked() { open_path(&path); }
                            let active = self.last_task.as_ref().filter(|_| self.busy.is_some()).map(|t| t.0.job_dir.clone());
                            if ui.add_enabled(self.busy.is_none() && !self.cache_action_pending,egui::Button::new("Recover / encode")).help("Restore this job from its saved geometry and settings, validate retained frames, and finish rendering or encoding. It uses the saved snapshot rather than the currently loaded model. Wait for active jobs or storage actions to finish.").clicked() {self.recover_job(path.clone());}
                            if ui.add_enabled(active.as_ref()!=Some(&path) && !self.cache_action_pending,egui::Button::new("Remove job")).help("Delete this inactive job folder and its retained files to free storage. Files outside the job folder stay untouched. A running job cannot be removed.").clicked() {
                                self.cache_action_pending=true;let root=self.data_dir.join("jobs");let sender=self.sender.clone();let target=path.clone();
                                thread::spawn(move||{let result=CacheStore::new(root).and_then(|cache|cache.remove_job(&target,active.as_deref())).map_err(|error|error.to_string());
                                    let _=sender.send(Message::RemovedJob(result));});
                            }
                            });
                        });
                    }
        });
        self.show_tools = open;
    }
    fn recovery_window(&mut self, ctx: &egui::Context) {
        let Some((project, status)) = self.unresolved_project.clone() else {
            return;
        };
        egui::Window::new("Project source needs attention")
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(if status == SourceStatus::Missing {
                    "The saved source could not be found."
                } else {
                    "The source differs from the saved project."
                });
                ui.label(project.source_path.display().to_string());
                if status == SourceStatus::Changed
                    && ui
                        .button("Reimport current source with saved settings").help("Use the changed file at its saved location with the project settings. Geometry may differ from the version the project originally referenced.")
                        .clicked()
                {
                    self.restore_project(project.clone());
                }
                if ui.button("Locate replacement source…").help("Choose the replacement STL or G-code file for this project, then import it with the saved settings. The selected source file is read without being rewritten.").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Models", &["stl", "gcode", "gco"])
                        .pick_file()
                {
                    let mut relocated = project.clone();
                    relocated.source_path = path;
                    self.restore_project(relocated);
                }
                if ui.button("Dismiss").help("Close source recovery and keep the previously accepted model available. The missing or changed source is not replaced.").clicked() {
                    self.dismiss_project_recovery();
                }
            });
    }
    fn smoke(&mut self, ctx: &egui::Context) {
        let Some(directory) = self.smoke_output.clone() else {
            return;
        };
        if directory.join("renderer-cases.json").is_file() {
            self.renderer_smoke(ctx, &directory);
            return;
        }
        if self.started.elapsed() > Duration::from_secs(180) {
            let _ = fs::create_dir_all(&directory);
            let _ = fs::write(
                directory.join("failed.txt"),
                self.error.as_deref().unwrap_or("smoke timeout"),
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if self.smoke_stage == 0
            && self.model.is_none()
            && self.selected_source.is_none()
            && !self.importing
        {
            let _ = fs::create_dir_all(&directory);
            self.smoke_checks["empty_workspace_available"] = serde_json::json!(true);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                "complete",
            )));
            self.smoke_stage = 9;
        }
        if self.smoke_stage == 0 && self.model.is_some() && !self.importing {
            let _ = fs::create_dir_all(&directory);
            let solid_stage = self
                .model
                .as_ref()
                .filter(|model| model.geometry_kind == "original" && model.layer_tops_mm.len() >= 2)
                .map(|model| {
                    let count = model.layer_tops_mm.len() / 2;
                    (count, model.layer_tops_mm[count - 1])
                });
            if let Some((_, height)) = solid_stage {
                self.buildup = true;
                self.buildup_height = height as f32;
                self.changed();
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                "native",
            )));
            self.smoke_stage = 1;
            self.start_export(ExportKind::Still, directory.join("stale-preview.png"), true);
            if let Some((count, _)) = solid_stage {
                self.smoke_checks["solid_build_stage_reaches_preview"] =
                    serde_json::json!(self.last_task.as_ref().is_some_and(
                        |(request, scene, _, _, _)| {
                            request
                                .build_stage
                                .is_some_and(|stage| stage.visible_layers == count as u32)
                                && scene["solid_layer_tops_mm"]
                                    .as_array()
                                    .is_some_and(|tops| tops.len() > count)
                        }
                    ));
            }
            self.scene.import.layer_height_mm = 0.3;
            self.scene.import.first_layer_height_mm = 0.25;
            self.scene.print_settings_edited = true;
            self.scene.display_rotation_degrees = [20., 0., 35.];
            self.changed();
            let path = self.model.as_ref().expect("smoke model").path.clone();
            self.smoke_checks["initial_output_name_matches_model"] = serde_json::json!(
                ExportKind::Still.suggested_file_name(&path)
                    == format!("{}.png", path.file_stem().unwrap().to_string_lossy())
            );
            let renamed = directory.join(format!(
                "renamed-output-model.2.{}",
                path.extension().unwrap().to_string_lossy()
            ));
            if let Err(error) = fs::copy(&path, &renamed) {
                self.error = Some(error.to_string());
                return;
            }
            self.import(directory.join("missing-stale-import.stl"));
            self.import(path.clone());
            self.import(directory.join("another-missing-stale-import.stl"));
            self.import(renamed);
        }
        if self.smoke_stage == 1 && self.busy.is_none() && !self.importing {
            self.smoke_checks["stale_preview_rejected"] =
                serde_json::json!(self.preview_revision != Some(self.revision));
            self.smoke_checks["latest_import_selected"] =
                serde_json::json!(self.error.is_none() && self.model.is_some());
            self.smoke_checks["output_name_follows_new_model"] = serde_json::json!(
                self.model
                    .as_ref()
                    .is_some_and(|m| ExportKind::Still.suggested_file_name(&m.path)
                        == "renamed-output-model.2.png")
            );
            self.renderers
                .select_backend(false, BackendId::BlenderCycles);
            self.start_export(
                ExportKind::Still,
                directory.join("cancelled-preview.png"),
                true,
            );
            self.smoke_stage = 2;
        }
        if self.smoke_stage == 2 && self.busy.is_some() && self.progress.2.starts_with("Rendering")
        {
            self.render.cancel();
            self.smoke_stage = 3;
        }
        if self.smoke_stage == 3 && self.busy.is_none() {
            self.smoke_checks["cancel_received"] = serde_json::json!(
                self.error
                    .as_ref()
                    .is_some_and(|e| e.to_lowercase().contains("cancel"))
            );
            self.error = None;
            self.scene.import.millimeters_per_unit = 0.;
            self.import(self.model.as_ref().expect("smoke model").path.clone());
            self.smoke_stage = 31;
        }
        if self.smoke_stage == 31 && !self.importing {
            self.smoke_checks["failed_reimport_preserves_settings"] = serde_json::json!(
                self.error.is_some()
                    && self
                        .model
                        .as_ref()
                        .is_some_and(|model| model.import_settings == self.scene.import)
            );
            self.error = None;
            self.renderers
                .select_backend(false, BackendId::BlenderEevee);
            self.fit();
            self.changed();
            self.size = 128;
            if let Some(samples) = self.renderers.animation.quality.samples_mut() {
                *samples = 4;
            }
            self.turntable_timing = TurntableTiming {
                seconds_per_turn: 0.2,
                duration_seconds: 0.6,
            };
            self.start_export(
                ExportKind::Gif,
                directory.join("basic-three-turns.gif"),
                false,
            );
            self.smoke_stage = 32;
        }
        if self.smoke_stage == 32
            && self.busy.is_none()
            && self.output.as_ref().is_some_and(Playback::animated)
            && self
                .last_output
                .as_ref()
                .is_some_and(|o| o.frame_count == 12)
        {
            self.smoke_checks["basic_turntable_timing_exported"] = serde_json::json!(
                self.error.is_none()
                    && self
                        .turntable_timing
                        .total_turns()
                        .is_ok_and(|turns| (turns - 3.).abs() < 1e-12)
                    && self
                        .last_task
                        .as_ref()
                        .is_some_and(|task| task.0.frame_count == 12
                            && task.0.turntable_timing == Some(self.turntable_timing)
                            && task.0.timeline.is_none())
            );
            let details = self
                .last_task
                .as_ref()
                .and_then(|task| fs::read_dir(&task.0.job_dir).ok())
                .and_then(|entries| {
                    entries.filter_map(|entry| entry.ok()).find_map(|entry| {
                        fs::read(entry.path().join("frames/render-result.json")).ok()
                    })
                })
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
            self.smoke_checks["basic_turntable_reuses_two_rotations"] = serde_json::json!(
                details
                    .is_some_and(|stats| stats["rendered_frame_count"] == 4
                        && stats["copied_frame_count"] == 8)
            );
            self.turntable_timing = TurntableTiming {
                seconds_per_turn: 4.,
                duration_seconds: 12.,
            };
            self.show_output = false;
            self.start_export(ExportKind::Still, directory.join("rendered.png"), true);
            self.smoke_stage = 4;
        }
        if self.smoke_stage == 4 && self.preview_revision == Some(self.revision) {
            if let Some(rect) = self.native_rect {
                let old_import = self.scene.import.clone();
                let mut picked = None;
                for y in 2..=8 {
                    for x in 2..=8 {
                        let point =
                            rect.min + rect.size() * egui::vec2(x as f32 / 10., y as f32 / 10.);
                        if let Some(hit) = self.settle_pointer(rect, point) {
                            picked = Some(hit);
                            break;
                        }
                    }
                    if picked.is_some() {
                        break;
                    }
                }
                let down = picked.and_then(|hit| {
                    crate::model::orientation::rotation_matrix(self.scene.display_rotation_degrees)
                        .ok()
                        .map(|matrix| crate::model::orientation::apply(matrix, hit.normal_mm))
                });
                self.smoke_checks["native_pick_and_settle"] = serde_json::json!(
                    down.is_some_and(|n| n[2] < -0.999999) && self.scene.import == old_import
                );
            }
            let first = self.camera.clone();
            let mut last = first.clone();
            last.canonical(CanonicalView::Top);
            last.target_mm[0] += 0.5;
            self.timeline = Timeline {
                duration_seconds: 0.2,
                keyframes: vec![
                    CameraKeyframe {
                        camera: first,
                        transition: Transition::Spiral,
                        spiral_turns: 1.,
                        easing: Easing::SmoothStep,
                        ..Default::default()
                    },
                    CameraKeyframe {
                        time_seconds: 0.2,
                        camera: last,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            };
            self.timeline_enabled = true;
            self.output_format = OutputFormat::Mp4;
            self.section = true;
            self.section_axis = 0;
            self.section_position = 0.75;
            if let Some(model) = &self.model {
                let project = ProjectFile {
                    schema_version: 2,
                    source_path: model.path.clone(),
                    source_hash: model.source_hash.clone(),
                    scene: self.scene.clone(),
                    camera: self.camera.clone(),
                    export_size: self.size,
                    renderers: self.renderers.clone(),
                    output_options: self.output_preferences(),
                };
                let project_path = directory.join("smoke.turninator");
                let check = (|| -> Result<bool> {
                    save_project(&project_path, &project)?;
                    let (loaded, status) = load_project(&project_path)?;
                    Ok(status == SourceStatus::Current
                        && loaded.scene.import.layer_height_mm == 0.3
                        && loaded.scene.print_settings_edited
                        && loaded.camera.camera_spec() == self.camera.camera_spec())
                })();
                self.smoke_checks["project_roundtrip"] = serde_json::json!(check.unwrap_or(false));
                self.debounced_import = Some((
                    directory.join("stale-project-intent.stl"),
                    Instant::now() - Duration::from_secs(1),
                ));
                self.smoke_checks["save_choice_expected_job_count"] =
                    serde_json::json!(self.next_job);
                let _ = self.sender.send(Message::ExportDestination {
                    kind: ExportKind::Still,
                    generation: self.gate.current().saturating_sub(1),
                    path: Some(directory.join("stale-save-choice.png")),
                });
                let _ = self.sender.send(Message::OpenProject(Some(project_path)));
            }
            self.smoke_stage = 5;
        }
        if self.smoke_stage == 5
            && !self.importing
            && self.project_path.as_ref() == Some(&directory.join("smoke.turninator"))
        {
            self.smoke_checks["stale_save_choice_rejected"] = serde_json::json!(
                self.smoke_checks["save_choice_expected_job_count"] == self.next_job
                    && !directory.join("stale-save-choice.png").exists()
            );
            self.smoke_checks["saved_section_preserved"] = serde_json::json!(
                self.section && self.section_axis == 0 && self.section_position == 0.75
            );
            self.smoke_checks["project_intent_supersedes_debounce"] =
                serde_json::json!(self.error.is_none() && self.debounced_import.is_none());
            self.smoke_checks["timeline_project_restored"] = serde_json::json!(
                self.timeline_enabled
                    && self.timeline.frame_count_at_rate(self.frame_rate).ok() == Some(4)
                    && self.timeline.keyframes.len() == 2
                    && self.timeline.keyframes[0].spiral_turns == 1.
            );
            self.smoke_checks["basic_turntable_project_restored"] = serde_json::json!(
                self.turntable_timing.seconds_per_turn == 4.
                    && self.turntable_timing.duration_seconds == 12.
            );
            self.smoke_checks["selected_encoding_project_restored"] =
                serde_json::json!(self.output_format == OutputFormat::Mp4);
            if let Ok((mut project, _)) = load_project(&directory.join("smoke.turninator")) {
                project.source_path = directory.join("missing-project-source.stl");
                let path = directory.join("missing.turninator");
                if save_project(&path, &project).is_ok() {
                    let _ = self.sender.send(Message::OpenProject(Some(path)));
                }
            }
            self.smoke_stage = 6;
        }
        if self.smoke_stage == 6 && self.unresolved_project.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                "missing-project",
            )));
            self.dismiss_project_recovery();
            self.smoke_checks["dismiss_unblocks_previous_model"] = serde_json::json!(
                !self.importing
                    && self
                        .model
                        .as_ref()
                        .is_some_and(|model| model.import_settings == self.scene.import)
            );
            if let Some((request, _, _, _, _)) = &self.last_task {
                let job = request.job_dir.clone();
                let before = self.next_job;
                self.recover_job(job.clone());
                self.start_export(
                    ExportKind::Still,
                    directory.join("blocked-during-recovery.png"),
                    true,
                );
                self.smoke_checks["recovery_blocks_new_exports"] =
                    serde_json::json!(self.next_job == before && self.busy.is_none());
                self.smoke_checks["recovery_job"] = serde_json::json!(job);
            }
            self.smoke_stage = 7;
        }
        if self.smoke_stage == 7 && !self.cache_action_pending && self.busy.is_none() {
            self.smoke_checks["recovered_snapshot_keeps_job_identity"] = serde_json::json!(
                self.error.is_none()
                    && self
                        .last_task
                        .as_ref()
                        .is_some_and(|task| serde_json::json!(task.0.job_dir)
                            == self.smoke_checks["recovery_job"])
            );
            self.section = false;
            self.show_output = false;
            self.changed();
            self.start_export(
                ExportKind::Still,
                directory.join("final-rendered.png"),
                true,
            );
            self.smoke_stage = 8;
        }
        if self.smoke_stage == 8 && self.preview_revision == Some(self.revision) {
            self.smoke_checks["preview_display_intent_kept"] = serde_json::json!(!self.show_output);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                "rendered",
            )));
            self.size = 128;
            if let Some(samples) = self.renderers.animation.quality.samples_mut() {
                *samples = 4;
            }
            if let Some(model) = self.model.as_ref().filter(|m| !m.groups.is_empty()) {
                self.section_axis = 0;
                self.section_position = (model.bounds_mm.min[0] + model.bounds_mm.max[0]) / 2.;
                self.section = true;
                self.side_by_side = true;
                self.timeline.buildup = false;
                self.export_buildup = true;
                self.scene.select_material("resin");
                self.scene.color_mode = "tool".into();
                self.scene.studio_preset = "dark".into();
                self.renderers
                    .select_backend(false, BackendId::BlenderCycles);
                self.changed();
            }
            self.start_export(
                ExportKind::Gif,
                directory.join("camera-timeline.gif"),
                false,
            );
            self.smoke_stage = 81;
        }
        if self.smoke_stage == 81
            && self.busy.is_none()
            && self.output.as_ref().is_some_and(Playback::animated)
            && self
                .last_output
                .as_ref()
                .is_some_and(|o| o.frame_count == 4)
        {
            self.smoke_checks["timeline_export_decoded"] = serde_json::json!(
                self.last_task.as_ref().is_some_and(|t| t.0.frame_count == 4
                    && t.0
                        .timeline
                        .as_ref()
                        .is_some_and(|t| t.keyframes.len() == 2))
                    && self.error.is_none()
            );
            self.timeline.turntable = true;
            self.timeline_time = 0.1;
            self.sample_timeline();
            self.smoke_checks["native_timeline_sample"] = serde_json::json!(
                (self.timeline_angle - 180.).abs() < 0.01
                    && self.camera.pitch_degrees > self.timeline.keyframes[0].camera.pitch_degrees
                    && self.camera.pitch_degrees < 90.
            );
            self.start_export(
                ExportKind::Still,
                directory.join("timeline-stage-preview.png"),
                true,
            );
            self.smoke_stage = 82;
        }
        if self.smoke_stage == 82 && self.preview_revision == Some(self.revision) {
            self.show_output = false;
            if self.model.as_ref().is_some_and(|m| !m.groups.is_empty()) {
                self.smoke_checks["composed_resin_buildup_section_stage"] = serde_json::json!(
                    self.last_task
                        .as_ref()
                        .is_some_and(|t| t.0.section.is_some()
                            && t.0.layout == ExportLayout::SideBySide
                            && t.0
                                .build_stage
                                .is_some_and(|stage| stage.visible_layers == 3)
                            && t.1["material_preset"] == "resin")
                        && self.error.is_none()
                );
            }
            self.smoke_checks["timeline_sample_preview_matches_request"] = serde_json::json!(
                self.last_task
                    .as_ref()
                    .is_some_and(|t| t.0.timeline.is_none()
                        && (t.0.start_angle_degrees - 180.).abs() < 0.01
                        && t.1["camera"] == self.camera.camera_spec())
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                "timeline",
            )));
            self.smoke_stage = 90;
        }
        if self.smoke_stage == 90 && self.busy.is_none() {
            self.output_format = OutputFormat::Mp4;
            self.changed();
            self.start_export(
                ExportKind::Still,
                directory.join("format-mp4-preview.png"),
                true,
            );
            self.smoke_stage = 91;
        }
        if self.smoke_stage == 91 && self.preview_revision == Some(self.revision) {
            self.smoke_checks["mp4_preview_uses_animation_renderer"] = serde_json::json!(
                self.last_task
                    .as_ref()
                    .is_some_and(|task| task.0.selection.backend
                        == self.renderers.animation.backend
                        && !task.0.transparent)
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                "format-mp4",
            )));
            self.output_format = OutputFormat::Png;
            self.renderers.select_backend(true, BackendId::BlenderEevee);
            self.transparent = true;
            self.changed();
            self.start_export(
                ExportKind::Still,
                directory.join("format-png-preview.png"),
                true,
            );
            self.smoke_stage = 92;
        }
        if self.smoke_stage == 92 && self.preview_revision == Some(self.revision) {
            self.smoke_checks["png_preview_uses_png_renderer_and_alpha"] =
                serde_json::json!(self.last_task.as_ref().is_some_and(|task| {
                    task.0.selection.backend == self.renderers.still.backend && task.0.transparent
                }));
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                "format-png",
            )));
            self.output_format = OutputFormat::FourView;
            self.changed();
            self.start_export(
                ExportKind::Still,
                directory.join("format-sheet-preview.png"),
                true,
            );
            self.smoke_stage = 93;
        }
        if self.smoke_stage == 93 && self.preview_revision == Some(self.revision) {
            self.smoke_checks["sheet_preview_uses_sheet_renderer_and_views"] = serde_json::json!(
                self.last_task
                    .as_ref()
                    .is_some_and(|task| task.0.kind == ExportKind::Diagram
                        && task.0.selection.backend == self.renderers.still.backend
                        && task.0.section.is_none()
                        && !task.0.transparent)
                    && self.viewport_settings().section.is_none()
                    && self.viewport_settings().display_rotation_degrees == [0.; 3]
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                "format-sheet",
            )));
            self.still_size = 512;
            if let Some(samples) = self.renderers.still.quality.samples_mut() {
                *samples = 4;
            }
            self.start_export(
                ExportKind::Diagram,
                directory.join("selected-sheet.png"),
                false,
            );
            self.smoke_stage = 95;
        }
        if self.smoke_stage == 95
            && self.busy.is_none()
            && self
                .last_output
                .as_ref()
                .is_some_and(|output| output.path == directory.join("selected-sheet.png"))
            && self
                .output
                .as_ref()
                .is_some_and(|output| !output.animated())
        {
            self.smoke_checks["sheet_export_uses_visible_quality"] =
                serde_json::json!(self.last_task.as_ref().is_some_and(|task| {
                    task.0.selection.backend == self.renderers.still.backend
                        && task.0.size == 512
                        && task.0.selection.quality.samples() == Some(4)
                        && task.0.kind == ExportKind::Diagram
                }));
            self.output_format = OutputFormat::Gif;
            self.show_output = false;
            self.changed();
            self.start_export(
                ExportKind::Still,
                directory.join("format-gif-preview.png"),
                true,
            );
            self.smoke_stage = 94;
        }
        if self.smoke_stage == 94 && self.preview_revision == Some(self.revision) {
            self.smoke_checks["gif_restores_hidden_settings_and_opaque_preview"] = serde_json::json!(
                self.transparent
                    && self
                        .last_task
                        .as_ref()
                        .is_some_and(|task| !task.0.transparent
                            && task.0.selection.backend == self.renderers.animation.backend)
                    && self.output_format.supports_section()
                    && self.timeline_enabled
            );
            self.smoke_checks["camera_before_layout_resize"] = self.camera.camera_spec();
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(940., 700.)));
            self.smoke_stage = 96;
        }
        if self.smoke_stage == 96 && (ctx.viewport_rect().width() - 940.).abs() < 1. {
            self.smoke_checks["narrow_comparison_rects"] =
                self.smoke_checks["comparison_rects"].clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                "comparison-narrow",
            )));
            self.smoke_stage = 98;
        }
        if self.smoke_stage == 97 && (ctx.viewport_rect().width() - 1420.).abs() < 1. {
            self.smoke_checks["wide_comparison_rects"] =
                self.smoke_checks["comparison_rects"].clone();
            self.smoke_checks["resize_preserves_camera"] = serde_json::json!(
                self.smoke_checks["camera_before_layout_resize"] == self.camera.camera_spec()
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                "comparison-wide",
            )));
            if self
                .model
                .as_ref()
                .is_some_and(|m| m.geometry_kind == "original")
            {
                self.scene.display_rotation_degrees = [0., 90., 0.];
                let model = self.model.as_ref().unwrap();
                let expected = crate::presentation::make_build_mesh(&model.mesh, [0., 90., 0.])
                    .unwrap()
                    .bounds()
                    .unwrap()
                    .max[2];
                self.smoke_checks["current_down_expected_height"] = serde_json::json!(expected);
                let path = model.path.clone();
                self.buildup = false;
                self.section = false;
                self.show_output = false;
                self.prepare_stepped_down().unwrap();
                self.restore_camera = true;
                self.import(path);
                self.smoke_stage = 101;
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                    "complete",
                )));
                self.smoke_stage = 9;
            }
        }

        if self.smoke_stage == 101 && !self.importing {
            self.smoke_checks["stepped_enable_uses_current_down"] = serde_json::json!(
                self.pending_build_orientation.is_none()
                    && self.scene.display_rotation_degrees == [0.; 3]
                    && self
                        .model
                        .as_ref()
                        .is_some_and(|m| m.geometry_kind == "stepped"
                            && (m.bounds_mm.max[2]
                                - self.smoke_checks["current_down_expected_height"]
                                    .as_f64()
                                    .unwrap())
                            .abs()
                                < 1e-4)
            );
            self.start_export(
                ExportKind::Still,
                directory.join("current-down-preview.png"),
                true,
            );
            self.smoke_stage = 102;
        }
        if self.smoke_stage == 102
            && self.preview_revision == Some(self.revision)
            && self.busy.is_none()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                "stepped-current-down",
            )));
            self.smoke_checks["before_stepped_settle_build"] =
                serde_json::json!(self.scene.import.build_rotation_degrees);
            let mut picked = None;
            if let Some(rect) = self.native_rect {
                'search: for y in 2..=8 {
                    for x in 2..=8 {
                        let point =
                            rect.min + rect.size() * egui::vec2(x as f32 / 10., y as f32 / 10.);
                        if let Some(hit) = self.settle_pointer(rect, point) {
                            picked = Some(hit);
                            break 'search;
                        }
                    }
                }
            }
            self.smoke_checks["stepped_face_triggers_rebuild"] = serde_json::json!(
                picked.is_some() && self.importing && self.pending_build_orientation.is_some()
            );
            self.smoke_checks["stepped_selected_normal"] =
                serde_json::json!(picked.map(|h| h.normal_mm));
            self.smoke_stage = 103;
        }
        if self.smoke_stage == 103 && !self.importing {
            let before: [f64; 3] =
                serde_json::from_value(self.smoke_checks["before_stepped_settle_build"].clone())
                    .unwrap();
            let aligned = self.smoke_checks["stepped_selected_normal"]
                .as_array()
                .and_then(|_| {
                    let normal: [f64; 3] = serde_json::from_value(
                        self.smoke_checks["stepped_selected_normal"].clone(),
                    )
                    .ok()?;
                    let relative = crate::model::orientation::relative_rotation(
                        self.scene.import.build_rotation_degrees,
                        before,
                    )
                    .ok()?;
                    let matrix = crate::model::orientation::rotation_matrix(relative).ok()?;
                    Some(crate::model::orientation::apply(matrix, normal))
                });
            self.smoke_checks["stepped_selected_face_becomes_build_down"] = serde_json::json!(
                self.scene.display_rotation_degrees == [0.; 3]
                    && self.pending_build_orientation.is_none()
                    && aligned.is_some_and(|n| n[0].abs() < 1e-8
                        && n[1].abs() < 1e-8
                        && (n[2] + 1.).abs() < 1e-8)
            );
            self.start_export(
                ExportKind::Still,
                directory.join("resettled-stepped-preview.png"),
                true,
            );
            self.smoke_stage = 104;
        }
        if self.smoke_stage == 104
            && self.preview_revision == Some(self.revision)
            && self.busy.is_none()
        {
            self.smoke_checks["before_task_navigation"] = serde_json::json!({"revision": self.revision, "camera": self.camera.camera_spec(), "scene": self.scene});
            self.active_tab = workspace::Tab::Model;
            self.smoke_stage = 109;
        }
        for (stage, tag) in [
            (110, "workspace-model"),
            (112, "workspace-appearance"),
            (114, "workspace-view"),
            (116, "workspace-export"),
            (118, "workspace-settings"),
            (120, "workspace-narrow-export"),
        ] {
            if self.smoke_stage == stage {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(tag)));
                self.smoke_stage += 1;
            }
        }
        if self.smoke_stage == 119 && (ctx.viewport_rect().width() - 940.).abs() < 1. {
            self.smoke_stage = 120;
        }
        if self.smoke_stage == 109 {
            self.smoke_stage = 110;
        }
        let screenshots = ctx.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| {
                    if let egui::Event::Screenshot {
                        image, user_data, ..
                    } = event
                    {
                        Some((image.clone(), user_data.clone()))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        });
        for (image, tag) in screenshots {
            let name = tag
                .data
                .as_ref()
                .and_then(|data| data.downcast_ref::<&str>())
                .copied()
                .unwrap_or("screenshot");
            let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            let _ = image::save_buffer(
                directory.join(format!("{name}-ui.png")),
                &rgba,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ColorType::Rgba8,
            );
            if name == "comparison-narrow" {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1420., 1020.)));
                self.smoke_stage = 97;
            }
            if name == "workspace-model" {
                self.active_tab = workspace::Tab::Appearance;
                self.smoke_stage = 112;
            }
            if name == "workspace-appearance" {
                self.active_tab = workspace::Tab::View;
                self.smoke_stage = 114;
            }
            if name == "workspace-view" {
                self.active_tab = workspace::Tab::Export;
                self.smoke_stage = 116;
            }
            if name == "workspace-export" {
                self.smoke_checks["after_task_navigation"] = serde_json::json!({"revision":self.revision,"camera":self.camera.camera_spec(),"scene":self.scene});
                let before = &self.smoke_checks["before_task_navigation"];
                self.smoke_checks["task_navigation_preserves_render_state"] = serde_json::json!(
                    before["revision"].as_u64() == Some(self.revision)
                        && before["camera"]
                            == serde_json::to_value(self.camera.camera_spec()).unwrap()
                        && before["scene"] == serde_json::to_value(&self.scene).unwrap()
                );
                self.show_tools = true;
                self.smoke_stage = 118;
            }
            if name == "workspace-settings" {
                self.show_tools = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(940., 700.)));
                self.smoke_stage = 119;
            }
            if name == "workspace-narrow-export" {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                    "complete",
                )));
                self.smoke_stage = 9;
            }
            if name == "complete" {
                let _ = fs::write(directory.join("result.json"), serde_json::json!({"model":self.model.as_ref().map(|m| &m.path), "triangles":self.viewport.triangle_count(),"revision":self.revision,"preview_revision":self.preview_revision,"error":self.error,"checks":self.smoke_checks}).to_string());
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}
impl eframe::App for Studio {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.receive(&ctx);
        let elapsed = self.last_tick.elapsed().as_secs_f32();
        self.last_tick = Instant::now();
        if self.timeline_enabled && self.timeline_playing {
            self.timeline_time += f64::from(elapsed);
            if self.timeline_time >= self.timeline.duration_seconds {
                if self.timeline.looping {
                    self.timeline_time = self
                        .timeline_time
                        .rem_euclid(self.timeline.duration_seconds);
                } else {
                    self.timeline_time = self.timeline.duration_seconds;
                    self.timeline_playing = false;
                }
            }
            self.sample_timeline();
            ctx.request_repaint_after(Duration::from_millis(33));
        }
        if self.buildup && self.buildup_playing {
            let before = self.viewport_settings().buildup_height_mm;
            let top = self
                .model
                .as_ref()
                .map_or(1., |m| m.bounds_mm.max[2] as f32);
            self.buildup_height = (self.buildup_height + top * elapsed / 4.)
                % (top + self.scene.import.layer_height_mm as f32);
            if self.viewport_settings().buildup_height_mm != before {
                self.changed();
            }
            ctx.request_repaint_after(Duration::from_millis(33));
        }
        let color_signature = serde_json::to_string(&(
            self.output_format,
            &self.scene.color_mode,
            &self.scene.tool_colors,
        ))
        .unwrap_or_default();
        if self.color_signature != color_signature {
            if let Some(model) = &self.model {
                let colors = if self.output_format == OutputFormat::FourView
                    || self.scene.color_mode == "uniform"
                    || model.geometry_kind != "beads"
                    || model.groups.is_empty()
                {
                    Vec::new()
                } else {
                    let mut colors = vec![self.scene.color; model.mesh.triangles.len()];
                    for group in &model.groups {
                        colors[group.start_triangle..group.end_triangle]
                            .fill(group_color(group, &self.scene));
                    }
                    colors
                };
                if let Err(error) = self.viewport.set_triangle_colors(&colors) {
                    self.error = Some(error.to_string());
                }
            }
            self.color_signature = color_signature;
        }
        if self
            .debounced_import
            .as_ref()
            .is_some_and(|(_, when)| when.elapsed() > Duration::from_millis(350))
            && let Some((path, _)) = self.debounced_import.take()
        {
            self.import(path);
        }
        let dropped = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect::<Vec<_>>()
        });
        if let Some(path) = dropped.into_iter().next() {
            self.restore_camera = false;
            self.import(path);
        }
        match workspace::file_shortcut(&ctx) {
            Some(workspace::FileCommand::OpenModel) => self.pick_model(),
            Some(workspace::FileCommand::OpenProject) => self.pick_project(),
            Some(workspace::FileCommand::SaveProject)
                if self.model.is_some() && !self.importing =>
            {
                self.pick_save_project()
            }
            _ => (),
        }
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong(egui::RichText::new("Turninator").size(18.));
                ui.add_space(8.);
                ui.menu_button("File", |ui| {
                    if ui.add(egui::Button::new("Open model…").shortcut_text("Ctrl+O")).help("Open an STL or supported G-code file. Drag and drop also works. Source files are read without being rewritten.").clicked() { self.pick_model(); ui.close(); }
                    if ui.add(egui::Button::new("Open project…").shortcut_text("Ctrl+Shift+O")).help("Restore a .turninator project with its source reference, print settings, camera and output preferences.").clicked() { self.pick_project(); ui.close(); }
                    if ui.add_enabled(self.model.is_some() && !self.importing, egui::Button::new("Save project…").shortcut_text("Ctrl+S")).help("Save the current source reference and scene settings as a portable project. The model file is referenced separately. Finish importing first.").clicked() { self.pick_save_project(); ui.close(); }
                    ui.separator();
                    ui.menu_button("Recent models", |ui| {
                        if self.prefs.recent_files.is_empty() { ui.label("No recent models"); }
                        for path in self.prefs.recent_files.clone() {
                            if ui.button(path.file_name().unwrap_or_default().to_string_lossy()).help(path.display().to_string()).clicked() {
                                self.restore_camera = false;
                                self.import(path);
                                ui.close();
                            }
                        }
                    }).help("Reopen a recently imported model. Hover a filename to see its full path.");
                }).help("Open models and projects, save the current project, or revisit recent models. Keyboard shortcuts are listed in the menu.");
                if ui.button("Open model…").help("Open an STL or supported G-code file (Ctrl+O), or drag one into the workspace.").clicked() { self.pick_model(); }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Settings").help("Configure external rendering tools and retained-frame storage. These application settings are separate from model projects.").clicked() { self.show_tools = true; }
                    if self.importing { ui.label("Rebuilding model…"); ui.spinner(); }
                });
            });
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            if self.busy.is_some() {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::ProgressBar::new(
                            self.progress.0 as f32 / self.progress.1.max(1) as f32,
                        )
                        .text(&self.progress.2),
                    ).help("Progress through the current job. Reused frames advance the count without rendering again; encoding and validation happen after rendering.");
                    if ui.button("Cancel").help("Stop the active render or encode job. Verified frames already written are retained for Retry; existing published output is preserved on failure.").clicked() {
                        self.cancel_render();
                        self.progress.2 = "Cancelling owned render job…".into();
                    }
                });
            }
            if let Some(error) = self.error.clone() {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                    if self.busy.is_none()
                        && self.last_task.is_some()
                        && ui.button("Retry retained job").help("Retry the last failed job using its saved model and settings. Matching verified frames are reused. To render newer scene settings, start a new export instead.").clicked()
                    {
                        self.retry();
                    }
                    if ui.small_button("Dismiss").help("Hide this error message without deleting the retained job. Automatic previews can resume if they are enabled.").clicked() {
                        self.error = None;
                    }
                });
            }
            ui.add(egui::Label::new(&self.status).truncate()).help(self.status.clone());
            if let Some(output) = &self.last_output {
                let path = output.path.clone();
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Open saved output").help("Open the last exported image, animation or video in its associated application. MP4 playback uses your external video player.").clicked() {
                        open_path(&path);
                    }
                    if ui.button("Open output folder").help("Open the folder containing the last exported file in the file explorer.").clicked()
                        && let Some(parent) = path.parent()
                    {
                        open_path(parent);
                    }
                    if self.output.is_some() {
                        ui.checkbox(&mut self.show_output, "Show encoded output").help("Switch the right panel between the rendered preview and the last decoded image or GIF output. This changes the display only, not the scene or export settings.");
                    }
                });
            }
        });
        egui::Panel::left("controls")
            .default_size(328.)
            .size_range(300.0..=460.0)
            .resizable(true)
            .show(ui, |ui| {
                if let Some(model) = &self.model {
                    let filename = model.path.file_name().unwrap_or_default().to_string_lossy();
                    ui.add(egui::Label::new(egui::RichText::new(filename).strong()).truncate())
                        .help(model.path.display().to_string());
                    let b = model.bounds_mm;
                    ui.small(format!(
                        "{:.1} × {:.1} × {:.1} mm",
                        b.max[0] - b.min[0],
                        b.max[1] - b.min[1],
                        b.max[2] - b.min[2]
                    ));
                    if !model.layer_tops_mm.is_empty() {
                        ui.small(format!("{} print layers", model.layer_tops_mm.len()));
                    }
                } else {
                    ui.strong("Print workspace");
                }
                workspace::navigation(ui, &mut self.active_tab);
                ui.separator();
                let action_height = 154.;
                let body_height = (ui.available_height() - action_height).max(40.);
                ui.allocate_ui(egui::vec2(ui.available_width(), body_height), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt(("settings", self.active_tab.label()))
                        .auto_shrink([false, false])
                        .max_height(body_height)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            self.sidebar(ui);
                        });
                });
                ui.separator();
                let actions = ui.scope(|ui| self.primary_actions(ui)).response.rect;
                if self.smoke_output.is_some() {
                    let visible = ctx.viewport_rect().contains_rect(actions);
                    self.smoke_checks["primary_actions_always_visible"] = serde_json::json!(
                        visible
                            && self.smoke_checks["primary_actions_always_visible"]
                                .as_bool()
                                .unwrap_or(true)
                    );
                }
            });
        egui::CentralPanel::default().show(ui, |ui| {
            let route = self.interactive_route();
            if let Some(badge) = route.badge { ui.label(badge); }
            let displayed_identity = if self.show_output { self.output_identity } else { self.preview_identity };
            if let Some(identity) = displayed_identity {
                ui.small(format!("{} · job {} · scene {}{}", renderers::label(identity.backend), identity.job, identity.revision,
                    if identity.revision != self.revision || identity.generation != self.view_generation { " · previous result" } else { " · completed" }));
            }
            let rendering = self.live_preview.current(self.revision).is_some();
            let display = self.live_preview.current(self.revision)
                .or(if self.show_output { self.output.as_ref() } else { self.preview.as_ref() });
            let partial = self.partial_image.as_ref().filter(|(tag,_)| self.render_event_matches(tag.job,tag.revision,tag.backend,tag.generation)).map(|(_,texture)|texture.clone());
            let image_size = partial.as_ref().map(|texture| texture.size_vec2()).unwrap_or_else(|| display.as_ref().map_or(egui::vec2(1.,1.),|p| p.texture.size_vec2()));
            let animated = display.as_ref().is_some_and(|p| p.animated());
            let status = (!rendering && partial.is_none() && !self.show_output && self.model.is_some() && self.preview_revision != Some(self.revision))
                .then_some(if self.busy.is_some() { "Updating…" } else if self.preview.is_none() { "Not rendered" } else { "Needs refresh" });
            let panes = comparison::layout_renderer(ui, if route.backend == BackendId::Filament { "Filament viewport" } else { "Native inspection" },if rendering { if self.busy.is_some() { "Rendering — latest completed image" } else { "Loading saved output…" } } else if self.show_output { "Encoded output" } else { "Rendered preview" },status,image_size,animated);
            let before = self.camera.camera_spec();
            let settings = self.viewport_settings();
            let mut native_ui = ui.new_child(egui::UiBuilder::new()
                .id_salt("native_canvas").max_rect(panes.native));
            let response = (if route.backend == BackendId::Filament {
                crate::viewport::gpu::viewport_interaction(&mut native_ui, &mut self.camera)
            } else { self.viewport.show(&mut native_ui,&mut self.camera,&settings) })
                .help(if self.settle_face {
                    "Click a visible triangle to align that face downward while keeping the print layers attached. Use Cancel face selection to leave this mode. This aligns the selected face; it does not calculate a stable resting position."
                } else { help::NATIVE_VIEW });
            self.native_rect = Some(response.rect);
            if self.camera.camera_spec() != before {
                self.timeline_playing = false;
                self.changed();
            }
            self.update_interactive(&ctx, response.rect);
            if route.backend == BackendId::Filament {
                if let Some(texture) = &self.interactive_texture {
                    native_ui.painter().image(texture.id(), response.rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.,1.)), egui::Color32::WHITE);
                }
                let message = self.interactive_error.as_deref().or_else(|| {
                    if self.busy.is_some() { Some("Viewport paused during render") }
                    else if self.interactive_requested != self.interactive_displayed { Some("Updating Filament · previous frame retained") }
                    else { None }
                });
                if let Some(message) = message {
                    native_ui.painter().text(response.rect.left_top() + egui::vec2(8., 8.), egui::Align2::LEFT_TOP, message, egui::FontId::proportional(13.), egui::Color32::YELLOW);
                }
            }
            if self.settle_face && !self.importing && response.clicked()
                && let Some(pointer) = response.interact_pointer_pos()
                && self.settle_pointer(response.rect,pointer).is_none()
            {
                self.status = "No visible face under that point; click the model surface.".into();
            }
            let display = if rendering { self.live_preview.current_mut(self.revision) } else if self.show_output { self.output.as_mut() } else { self.preview.as_mut() };
            if let Some(texture) = partial {
                ui.put(panes.preview,egui::Image::new(&texture).fit_to_exact_size(panes.preview.size()));
                ui.painter().text(panes.preview.left_top()+egui::vec2(8.,8.),egui::Align2::LEFT_TOP,"Rendering · partial image",egui::FontId::proportional(13.),egui::Color32::YELLOW);
            } else if let Some(playback) = display {
                if let Err(error) = playback.advance(&ctx) { self.error = Some(error); }
                let image = ui.put(panes.preview,egui::Image::new(&playback.texture)
                    .fit_to_exact_size(panes.preview.size()))
                    .help("The latest rendered preview or decoded image/GIF output. Both panels scale together to this image's aspect ratio; resizing does not change the render camera or output resolution. Open saved output shows the full exported file. A stale preview needs refreshing after edits.");
                if let Some(footer) = panes.playback
                    && ui.put(footer,egui::Checkbox::new(&mut playback.playing,"Play animation"))
                        .help("Play or pause the decoded animation in this panel using its saved frame delays. This does not change export speed or camera keys.")
                        .changed() && playback.playing
                {
                    playback.resume();
                }
                if self.smoke_output.is_some() {
                    let matched = (response.rect.size()-image.rect.size()).length()<0.01
                        && (response.rect.top()-image.rect.top()).abs()<0.01;
                    self.smoke_checks["comparison_canvases_match"] = serde_json::json!(matched
                        && self.smoke_checks["comparison_canvases_match"].as_bool().unwrap_or(true));
                    self.smoke_checks["comparison_rects"] = serde_json::json!({
                        "native":[response.rect.left(),response.rect.top(),response.rect.width(),response.rect.height()],
                        "preview":[image.rect.left(),image.rect.top(),image.rect.width(),image.rect.height()]
                    });
                }
            } else {
                let mut empty = ui.new_child(egui::UiBuilder::new().max_rect(panes.preview).layout(egui::Layout::top_down(egui::Align::Center)));
                empty.add_space((panes.preview.height()*0.38).max(0.));
                empty.heading("Your rendered preview");
                empty.label(if self.model.is_some() { "Choose Render preview to check materials and lighting." } else { "Open a model to get started." });
                empty.small("Both views use the same scale and camera.");
            }
        });
        self.tools_window(&ctx);
        self.recovery_window(&ctx);
        self.smoke(&ctx);
        if self.prefs.auto_preview
            && self.model.is_some()
            && !self.importing
            && self.busy.is_none()
            && self.preview_decode_pending.is_none()
            && self.preview_revision != Some(self.revision)
            && self.changed_at.elapsed() > Duration::from_millis(450)
            && self.error.is_none()
        {
            self.start_export(
                ExportKind::Still,
                self.data_dir.join(format!("preview-{}.png", self.revision)),
                true,
            );
        }
        if self.busy.is_some()
            || self.importing
            || self.smoke_output.is_some()
            || self.prefs.auto_preview
        {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }
}
fn material_name(value: &str) -> &str {
    match value {
        "matte_pla" => "Matte PLA",
        "silk_pla" => "Silk PLA",
        "glossy_plastic" => "Glossy plastic",
        "clay" => "Clay",
        "resin" => "Resin",
        _ => value,
    }
}
fn rotation_editor(ui: &mut egui::Ui, rotation: &mut [f64; 3], purpose: &str) {
    ui.columns(3, |columns| {
        for ((label, value), column) in ["X", "Y", "Z"].into_iter().zip(rotation).zip(columns) {
            column
                .add_sized(
                    [column.available_width(), column.spacing().interact_size.y],
                    egui::DragValue::new(value)
                        .speed(1.)
                        .prefix(format!("{label} "))
                        .suffix("°"),
                )
                .help(format!("{label}-axis rotation in degrees. {purpose}"));
        }
    });
}

fn open_path(path: &Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}
#[cfg(test)]
mod playback_regression_tests {
    use super::*;

    #[test]
    fn playback_starts_when_total_decoded_gif_exceeds_512_mib() {
        let scratch = crate::export::encode::Scratch::new(&std::env::temp_dir()).unwrap();
        let path = scratch.0.join("large-decoded.gif");
        {
            let mut encoder = gif::Encoder::new(
                fs::File::create(&path).unwrap(),
                960,
                960,
                &[255, 0, 0, 0, 255, 0],
            )
            .unwrap();
            encoder.set_repeat(gif::Repeat::Infinite).unwrap();
            let mut frame = gif::Frame {
                width: 960,
                height: 960,
                delay: 5,
                buffer: vec![0; 960 * 960].into(),
                ..Default::default()
            };
            for index in 0..160 {
                frame.buffer.to_mut().fill(index % 2);
                encoder.write_frame(&frame).unwrap();
            }
        }
        // This small compressed file expands to 562.5 MiB across all 160 frames.
        assert!(fs::metadata(&path).unwrap().len() < 1024 * 1024);
        let result = decode_output(&path, egui::Context::default());
        assert!(result.is_ok(), "playback should stream: {:?}", result.err());
    }
}
