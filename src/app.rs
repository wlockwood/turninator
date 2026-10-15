//! Native studio. Import and rendering run independently of the UI thread.
use crate::{
    Result,
    animation::{CameraKeyframe, Easing, Timeline, Transition, TurntableTiming},
    cache::{CacheStore, JobRecord},
    export::{
        AnimationMode, ExportKind, ExportLayout, ExportRequest, ExportResult, JobEvent,
        SectionSpec, VideoCodec,
    },
    presentation::{
        ImportSettings, ImportedModel, RevisionGate, SceneSettings, group_color, load_model,
        model_scene,
    },
    project::{OutputPreferences, ProjectFile, SourceStatus, load_project, save_project},
    session::{RenderEvent, RenderService, RenderTask, default_request},
    settings::{AppSettings, default_data_directory},
    viewport::{
        CanonicalView, InspectionMode, OrbitCamera, SectionPlane, Viewport, ViewportSettings,
    },
};
use eframe::egui;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
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
        preview: bool,
        result: std::result::Result<Vec<PlaybackFrame>, String>,
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
struct PlaybackFrame {
    image: egui::ColorImage,
    duration: Duration,
}
struct Playback {
    frames: Vec<PlaybackFrame>,
    texture: egui::TextureHandle,
    index: usize,
    next: Instant,
    playing: bool,
}
impl Playback {
    fn new(ctx: &egui::Context, frames: Vec<PlaybackFrame>) -> Option<Self> {
        let first = frames.first()?;
        let texture = ctx.load_texture(
            "encoded-output",
            first.image.clone(),
            egui::TextureOptions::LINEAR,
        );
        let next = Instant::now() + first.duration;
        Some(Self {
            frames,
            texture,
            index: 0,
            next,
            playing: true,
        })
    }
    fn advance(&mut self, ctx: &egui::Context) {
        if self.playing && self.frames.len() > 1 {
            let now = Instant::now();
            // Catch up without accumulating drift when the window is briefly obstructed.
            let mut changed = false;
            for _ in 0..self.frames.len() {
                if now < self.next {
                    break;
                }
                self.index = (self.index + 1) % self.frames.len();
                self.next += self.frames[self.index].duration;
                changed = true;
            }
            if changed {
                self.texture.set(
                    self.frames[self.index].image.clone(),
                    egui::TextureOptions::LINEAR,
                );
            }
            ctx.request_repaint_after(self.next.saturating_duration_since(now));
        }
    }
}

pub struct Studio {
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
    debounced_import: Option<(PathBuf, Instant)>,
    selected_source: Option<PathBuf>,
    model: Option<ImportedModel>,
    revision: u64,
    changed_at: Instant,
    render: RenderService,
    next_job: u64,
    busy: Option<u64>,
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
    show_output: bool,
    error: Option<String>,
    status: String,
    restore_camera: bool,
    unresolved_project: Option<(ProjectFile, SourceStatus)>,
    project_path: Option<PathBuf>,
    engine: String,
    size: u32,
    still_size: u32,
    still_engine: String,
    still_samples: u32,
    samples: u32,
    transparent: bool,
    show_edges: bool,
    show_tools: bool,
    codec: VideoCodec,
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
        let mut studio = Self {
            viewport: Viewport::new(
                cc.wgpu_render_state
                    .as_ref()
                    .ok_or("wgpu renderer unavailable")?,
            ),
            camera: OrbitCamera::default(),
            scene: SceneSettings::default(),
            prefs,
            data_dir,
            sender,
            messages,
            gate: RevisionGate::default(),
            importing: false,
            import_active: false,
            pending_import: None,
            debounced_import: None,
            selected_source: None,
            model: None,
            revision: 0,
            changed_at: Instant::now(),
            render: RenderService::new(),
            next_job: 0,
            busy: None,
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
            show_output: false,
            error,
            status: "Open an STL or G-code file to begin.".into(),
            restore_camera: false,
            unresolved_project: None,
            project_path: None,
            engine: "eevee".into(),
            size: 960,
            still_size: 4096,
            still_engine: "cycles".into(),
            still_samples: 64,
            samples: 32,
            transparent: false,
            show_edges: false,
            show_tools: false,
            codec: VideoCodec::H264,
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
        ViewportSettings {
            color: self.scene.color,
            roughness: self.scene.roughness,
            layer_appearance: self.scene.layer_appearance && !self.scene.import.stepped_envelope,
            layer_height_mm: self.scene.import.layer_height_mm as f32,
            first_layer_height_mm: self.scene.import.first_layer_height_mm as f32,
            groove_depth_mm: self.scene.groove_depth_mm as f32,
            display_rotation_degrees: self.scene.display_rotation_degrees.map(|v| v as f32),
            show_edges: self.show_edges,
            section: if self.section {
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
            turntable_angle_degrees: self.timeline_angle,
        }
    }
    fn changed(&mut self) {
        self.revision += 1;
        self.changed_at = Instant::now();
    }
    fn import(&mut self, path: PathBuf) {
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
        self.status = format!(
            "Face {} now points down. Print layers retain their build orientation.",
            hit.triangle_index + 1
        );
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
        match self.timeline.sample(self.timeline_time) {
            Ok(sample) => {
                self.camera = sample.camera;
                self.timeline_angle = sample.turntable_degrees as f32;
                if let Some(fraction) = sample.buildup_fraction
                    && let Some(model) = &self.model
                    && !model.groups.is_empty()
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
    fn start_export(&mut self, kind: ExportKind, destination: PathBuf, preview: bool) {
        if self.busy.is_some() || self.importing || self.cache_action_pending {
            return;
        }
        let result = (|| -> Result<()> {
            self.prefs.tools.validate()?;
            self.next_job += 1;
            let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
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
                timeline.frame_count()?
            } else if let Some(timing) = turntable_timing {
                timing.frame_count()?
            } else {
                80
            };
            let reserve = u64::from(render_size).pow(2)
                * 8
                * if matches!(kind, ExportKind::Gif | ExportKind::Mp4) {
                    u64::from(frames)
                } else {
                    4
                };
            if self.cache_usage.saturating_add(reserve) > self.prefs.cache_budget_bytes {
                return Err("Job storage budget exceeded; review Tools / storage.".into());
            }
            let job = cache.create_job(&format!("job-{stamp}-{}", self.next_job))?;
            let destination = if destination.is_absolute() {
                destination
            } else {
                std::env::current_dir()?.join(destination)
            };
            let mut request = default_request(
                job,
                destination,
                kind,
                if !preview && kind == ExportKind::Still {
                    self.still_engine.clone()
                } else {
                    self.engine.clone()
                },
                render_size,
                if preview {
                    16
                } else if kind == ExportKind::Still {
                    self.still_samples
                } else {
                    self.samples
                },
                self.transparent,
            );
            request.timeline = timeline;
            request.turntable_timing = turntable_timing;
            request.frame_count = frames;
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
                && !model.groups.is_empty()
            {
                request.build_stage = Some(crate::export::BuildStage {
                    visible_layers: crate::presentation::visible_layer_count(
                        model,
                        f64::from(self.buildup_height),
                    ) as u32,
                });
            }
            fs::write(
                request.job_dir.join("request.json"),
                serde_json::to_vec_pretty(&request)?,
            )?;
            cache.save_record(
                &request.job_dir,
                &JobRecord {
                    schema_version: 1,
                    label: format!("{:?}", request.kind),
                    state: "running".into(),
                    destination: request.destination.clone(),
                    error: None,
                },
            )?;
            let scene = self.snapshot(&request)?;
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
            self.render
                .submit(RenderTask {
                    id: self.next_job,
                    revision: self.revision,
                    mesh,
                    scene,
                    request,
                    preview,
                    tools: self.prefs.tools.clone(),
                    cache_budget_bytes: self.prefs.cache_budget_bytes,
                })
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            self.busy = Some(self.next_job);
            self.display_job = self.next_job;
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
            match self.render.submit(RenderTask {
                id: self.next_job,
                revision,
                mesh,
                scene,
                request,
                preview,
                tools: self.prefs.tools.clone(),
                cache_budget_bytes: self.prefs.cache_budget_bytes,
            }) {
                Ok(()) => {
                    self.busy = Some(self.next_job);
                    self.display_job = self.next_job;
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
                            match self.viewport.set_mesh(
                                &model.mesh,
                                &model.geometry_kind,
                                generation,
                            ) {
                                Ok(()) => {
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
                                        .is_none_or(|model| model.groups.is_empty())
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
                            schema_version: 1,
                            source_path: model.path.clone(),
                            source_hash: model.source_hash.clone(),
                            scene: self.scene.clone(),
                            camera: self.camera.clone(),
                            export_size: self.size,
                            engine: self.engine.clone(),
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
                    preview,
                    result,
                } => {
                    if preview && self.preview_decode_pending == Some(id) {
                        self.preview_decode_pending = None;
                    }
                    if (preview && (revision != self.revision || id != self.latest_preview_job))
                        || (!preview && id != self.latest_output_job)
                    {
                        continue;
                    }
                    match result {
                        Ok(frames) => {
                            if preview {
                                if revision == self.revision {
                                    self.preview = Playback::new(ctx, frames);
                                    self.preview_revision = Some(revision);
                                }
                            } else {
                                self.output = Playback::new(ctx, frames);
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
                RenderEvent::Progress { id, event } if self.busy == Some(id) => match event {
                    JobEvent::Progress {
                        completed,
                        total,
                        stage,
                    } => self.progress = (completed, total, stage),
                    JobEvent::Log(line) => self.status = line,
                    JobEvent::Analysis(value) => self.analysis = Some(value),
                    JobEvent::Frame(_) => {}
                },
                RenderEvent::Completed {
                    id,
                    revision,
                    preview,
                    result,
                } if self.busy == Some(id) => {
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
                        self.output = None;
                        self.show_output = false;
                        continue;
                    }
                    let sender = self.sender.clone();
                    if preview {
                        self.preview_decode_pending = Some(id);
                    }
                    thread::spawn(move || {
                        let decoded =
                            decode_output(&result.path).map_err(|error| error.to_string());
                        let _ = sender.send(Message::Image {
                            id,
                            revision,
                            preview,
                            result: decoded,
                        });
                    });
                }
                RenderEvent::Failed { id, message } if self.busy == Some(id) => {
                    self.busy = None;
                    self.record_result("failed", Some(message.clone()));
                    self.error = Some(message);
                }
                _ => {}
            }
        }
    }
    fn restore_project(&mut self, project: ProjectFile) {
        let options = project.output_options;
        self.still_size = options.still_size;
        self.still_engine = options.still_engine;
        self.still_samples = options.still_samples;
        self.samples = options.samples;
        self.transparent = options.transparent;
        self.codec = options.video_codec;
        self.export_buildup = options.animation == AnimationMode::BuildUp;
        self.turntable_timing = options.turntable_timing;
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
        self.engine = project.engine;
        self.restore_camera = true;
        self.unresolved_project = None;
        self.import(project.source_path);
    }
    fn restore_accepted_import(&mut self) {
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
            still_size: self.still_size,
            still_engine: self.still_engine.clone(),
            still_samples: self.still_samples,
            samples: self.samples,
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
                .filter(|m| self.buildup && !m.groups.is_empty())
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
        ui.collapsing("Camera keyframes and motion", |ui| {
            if ui.checkbox(&mut self.timeline_enabled, "Use timeline for GIF / MP4").changed() {
                self.timeline_playing = false;
                self.timeline_angle = 0.;
                if self.timeline_enabled && self.timeline.keyframes.len() == 1 {
                    self.timeline.keyframes[0].camera = self.camera.clone();
                }
                self.changed();
            }
            if !self.timeline_enabled { return; }
            ui.add(egui::DragValue::new(&mut self.timeline.duration_seconds).range(0.05..=500.).speed(0.1).suffix(" seconds"));
            ui.checkbox(&mut self.timeline.looping, "Close camera path into a loop");
            ui.checkbox(&mut self.timeline.turntable, "Rotate the model too");
            let deposited = self.model.as_ref().is_some_and(|m| !m.groups.is_empty());
            ui.add_enabled(deposited, egui::Checkbox::new(&mut self.timeline.buildup, "Build deposited layers during motion"));
            ui.small("20 fps. Set a time, orbit/pan to compose a view, then record it. Transition settings belong to the segment leaving each key.");
            let scrub = ui.add(egui::Slider::new(&mut self.timeline_time, 0.0..=self.timeline.duration_seconds).text("Time (s)"));
            let mut sample = scrub.changed();
            if sample { self.timeline_playing = false; }
            ui.horizontal(|ui| {
                if ui.button("Record this view").clicked() {
                    let key = CameraKeyframe { time_seconds: self.timeline_time, camera: self.camera.clone(), ..Default::default() };
                    if let Some(existing) = self.timeline.keyframes.iter_mut().find(|k| (k.time_seconds - self.timeline_time).abs() < 1e-6) {
                        existing.camera = key.camera;
                    } else if self.timeline.keyframes.len() < 256 {
                        self.timeline.keyframes.push(key);
                        self.timeline.keyframes.sort_by(|a,b| a.time_seconds.total_cmp(&b.time_seconds));
                    }
                }
                if ui.button(if self.timeline_playing { "Pause" } else { "Play" }).clicked()
                    && self.timeline.validate().is_ok() {
                        self.timeline_playing = !self.timeline_playing;
                        self.buildup_playing = false;
                        if self.timeline_time >= self.timeline.duration_seconds { self.timeline_time = 0.; }
                        sample = self.timeline_playing;
                }
                if ui.button("Reset motion").clicked() {
                    self.timeline = Timeline { keyframes: vec![CameraKeyframe {camera:self.camera.clone(), ..Default::default()}], ..Default::default() };
                    self.timeline_time = 0.;
                    self.timeline_angle = 0.;
                    self.timeline_playing = false;
                    self.changed();
                }
            });
            let mut remove = None;
            let mut recall = None;
            for (index,key) in self.timeline.keyframes.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    ui.collapsing(format!("View {} • {:.2} s", index+1,key.time_seconds), |ui| {
                        if index > 0 { ui.add(egui::DragValue::new(&mut key.time_seconds).range(0.001..=self.timeline.duration_seconds).speed(0.05).suffix(" s")); }
                        ui.small(format!("Yaw {:.1}° • elevation {:.1}° • target {:.1?}",key.camera.yaw_degrees,key.camera.pitch_degrees,key.camera.target_mm));
                        egui::ComboBox::from_label("Outgoing motion").selected_text(format!("{:?}",key.transition)).show_ui(ui,|ui|{
                            ui.selectable_value(&mut key.transition,Transition::Orbit,"Orbit / pan");
                            ui.selectable_value(&mut key.transition,Transition::Spiral,"Spiral");
                        });
                        if key.transition == Transition::Spiral { ui.add(egui::DragValue::new(&mut key.spiral_turns).range(-100.0..=100.).speed(1.).max_decimals(0).suffix(" whole turns")); }
                        egui::ComboBox::from_label("Easing").selected_text(format!("{:?}",key.easing)).show_ui(ui,|ui|{
                            ui.selectable_value(&mut key.easing,Easing::Linear,"Linear");
                            ui.selectable_value(&mut key.easing,Easing::SmoothStep,"Ease in / out");
                        });
                        ui.horizontal(|ui| {
                            if ui.small_button("Load view").clicked() { recall = Some((key.camera.clone(),key.time_seconds)); }
                            if ui.small_button("Replace with current view").clicked() { key.camera=self.camera.clone(); }
                            if index>0 && ui.small_button("Remove").clicked() { remove=Some(index); }
                        });
                    });
                });
            }
            if let Some(index)=remove { self.timeline.keyframes.remove(index); }
            if let Some((camera,time))=recall {
                self.camera=camera;
                self.timeline_time=time;
                self.timeline_angle=0.;
                self.timeline_playing=false;
                self.changed();
            }
            if let Err(error)=self.timeline.validate() { ui.colored_label(egui::Color32::YELLOW,error.to_string()); }
            else if let Ok(frames)=self.timeline.frame_count() { ui.small(format!("{frames} frames • {:.2} s",self.timeline.duration_seconds)); }
            if sample { self.sample_timeline(); }
            ui.small("Build-up reveals complete deposited layers; it does not reproduce machine time, nozzle motion, or material flow.");
        });
    }
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let preview_options_before = (self.engine.clone(), self.transparent);
        if let Some(model) = &self.model {
            let b = model.bounds_mm;
            ui.small(format!(
                "{:.2} × {:.2} × {:.2} mm",
                b.max[0] - b.min[0],
                b.max[1] - b.min[1],
                b.max[2] - b.min[2]
            ));
            if !model.groups.is_empty() {
                ui.small(format!(
                    "{} paths • {} deposited layers",
                    model.segment_count,
                    model.layer_tops_mm.len()
                ));
            }
            ui.collapsing("Source assumptions and features", |ui| {
                for warning in &model.warnings {
                    ui.label(warning);
                }
                let mut features = std::collections::BTreeSet::new();
                let mut tools = std::collections::BTreeSet::new();
                for group in &model.groups {
                    features.insert(&group.feature);
                    tools.insert(group.tool);
                }
                if !tools.is_empty() {
                    ui.label(format!("Tools: {tools:?}"));
                }
                for feature in features {
                    ui.label(feature);
                }
            });
        }
        ui.heading("Print appearance");
        let before = serde_json::to_string(&self.scene).unwrap_or_default();
        let import_before = serde_json::to_string(&self.scene.import).unwrap_or_default();
        let mut material = self.scene.material_preset.clone();
        egui::ComboBox::from_label("Material")
            .selected_text(material_name(&material))
            .show_ui(ui, |ui| {
                for value in ["matte_pla", "silk_pla", "glossy_plastic", "clay", "resin"] {
                    ui.selectable_value(&mut material, value.into(), material_name(value));
                }
            });
        if material != self.scene.material_preset {
            self.scene.select_material(&material);
            if material == "resin" {
                self.engine = "cycles".into();
                self.still_engine = "cycles".into();
            }
        }
        ui.color_edit_button_rgb(&mut self.scene.color);
        if self.model.as_ref().is_some_and(|m| !m.groups.is_empty()) {
            egui::ComboBox::from_label("Color mapping")
                .selected_text(&self.scene.color_mode)
                .show_ui(ui, |ui| {
                    for mode in ["uniform", "tool", "feature"] {
                        ui.selectable_value(&mut self.scene.color_mode, mode.into(), mode);
                    }
                });
            if self.scene.color_mode != "uniform" {
                ui.collapsing("Print palette", |ui| {
                    for (index, color) in self.scene.tool_colors.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!(
                                "{} {index}",
                                if self.scene.color_mode == "tool" {
                                    "Tool"
                                } else {
                                    "Palette"
                                }
                            ));
                            ui.color_edit_button_rgb(color);
                        });
                    }
                });
            }
        }
        ui.add(egui::Slider::new(&mut self.scene.roughness, 0.02..=1.).text("Roughness"));
        ui.checkbox(&mut self.scene.layer_appearance, "Show layer appearance");
        let mut print_edited = false;
        ui.horizontal(|ui| {
            ui.label("Layer (mm)");
            print_edited |= ui
                .add(
                    egui::DragValue::new(&mut self.scene.import.layer_height_mm)
                        .speed(0.01)
                        .range(0.02..=2.),
                )
                .changed();
        });
        ui.horizontal(|ui| {
            ui.label("First layer (mm)");
            print_edited |= ui
                .add(
                    egui::DragValue::new(&mut self.scene.import.first_layer_height_mm)
                        .speed(0.01)
                        .range(0.02..=2.),
                )
                .changed();
        });
        let max_groove = 0.25
            * self
                .scene
                .import
                .layer_height_mm
                .min(self.scene.import.first_layer_height_mm);
        self.scene.groove_depth_mm = self.scene.groove_depth_mm.min(max_groove);
        print_edited |= ui
            .add(
                egui::Slider::new(&mut self.scene.groove_depth_mm, 0.0..=max_groove)
                    .text("Groove depth (mm)"),
            )
            .changed();
        if print_edited {
            self.scene.print_settings_edited = true;
        }
        if ui.button("Reset print defaults").clicked() {
            self.scene.reset_print_settings();
        }
        if self.scene.material_preset == "resin" {
            ui.add(egui::Slider::new(&mut self.scene.resin_clarity, 0.0..=1.).text("Clarity"));
            ui.add(
                egui::Slider::new(&mut self.scene.resin_cloudiness, 0.0..=1.).text("Cloudiness"),
            );
            ui.small("Resin appearance is approximate; internal voids require real geometry.");
        }
        egui::ComboBox::from_label("Studio")
            .selected_text(&self.scene.studio_preset)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.scene.studio_preset, "light".into(), "Light");
                ui.selectable_value(&mut self.scene.studio_preset, "dark".into(), "Dark");
            });
        ui.separator();
        ui.collapsing("Physical model settings", |ui| {
            ui.horizontal(|ui| {
                ui.label("mm per source unit");
                ui.add(
                    egui::DragValue::new(&mut self.scene.import.millimeters_per_unit)
                        .speed(0.1)
                        .range(0.001..=1000.),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Filament (mm)");
                ui.add(
                    egui::DragValue::new(&mut self.scene.import.filament_diameter_mm)
                        .speed(0.05)
                        .range(0.1..=10.),
                );
            });
            ui.label("Build rotation (changes print layers)");
            rotation_editor(ui, &mut self.scene.import.build_rotation_degrees);
            ui.checkbox(&mut self.scene.import.stepped_envelope,"Experimental STL stepped solid");
            ui.small("Mid-layer approximation changes the silhouette; closed solids only. Features between samples may be missed.");
            ui.collapsing("G-code interpretation",|ui| {
                egui::ComboBox::from_label("Firmware").selected_text(&self.scene.import.firmware).show_ui(ui,|ui|{
                    for mode in ["marlin","klipper"] {ui.selectable_value(&mut self.scene.import.firmware,mode.into(),mode);}
                });
                ui.checkbox(&mut self.scene.import.joined_beads,"Join contiguous bead paths");
                ui.checkbox(&mut self.scene.import.allow_arcs,"Allow planar G2/G3 arcs");
                ui.add(egui::DragValue::new(&mut self.scene.import.chord_error_mm).range(0.001..=1.).speed(0.001).suffix(" mm arc error"));
                ui.checkbox(&mut self.scene.import.allow_tool_changes,"Normalized tool changes");
                ui.small("Tool changes assume shared XYZ / global E coordinates. Printer macros and implicit offsets require preprocessing.");
                let mut homing = self.scene.import.homing_position_mm.is_some();
                if ui.checkbox(&mut homing,"Explicit G28 home position").changed() {self.scene.import.homing_position_mm=homing.then_some([0.;3]);}
                if let Some(home) = &mut self.scene.import.homing_position_mm {ui.horizontal(|ui|{for (axis,value) in ["X","Y","Z"].into_iter().zip(home) {ui.add(egui::DragValue::new(value).prefix(format!("{axis} ")).speed(0.1));}});}
            });
            ui.collapsing("Explicit adaptive layer boundaries",|ui| {
                ui.label("G-code / stepped STL: mm boundaries, starting at 0");
                ui.text_edit_multiline(&mut self.adaptive_text);
                if ui.button("Apply boundaries").clicked() {
                    let parsed: std::result::Result<Vec<f64>,_> = self.adaptive_text.split(',').map(|v|v.trim().parse::<f64>()).collect();
                    match parsed {
                        Ok(values) => match crate::print::schedule::LayerSchedule::explicit(values.clone()) {
                            Ok(_) => {self.scene.import.layer_boundaries_mm=values;self.scene.print_settings_edited=true;},
                            Err(error) => self.error=Some(error.to_string()),
                        },
                        Err(_) => self.error=Some("Enter numeric comma-separated layer boundaries.".into()),
                    }
                }
                if ui.button("Use uniform layers / file metadata").clicked() {self.scene.import.layer_boundaries_mm.clear();self.adaptive_text.clear();}
                ui.small(format!("{} explicit boundaries",self.scene.import.layer_boundaries_mm.len()));
            });
        });
        ui.collapsing("Presentation pose", |ui| {
            ui.label("Display rotation (keeps print layers)");
            rotation_editor(ui, &mut self.scene.display_rotation_degrees);
            if ui
                .add_enabled(
                    self.model.is_some() && !self.importing,
                    egui::Button::new(if self.settle_face {
                        "Cancel face selection"
                    } else {
                        "Settle on face…"
                    }),
                )
                .clicked()
            {
                self.settle_face = !self.settle_face;
                self.timeline_playing = false;
                if self.timeline_angle != 0. {
                    self.timeline_angle = 0.;
                    self.changed();
                }
            }
            if self.settle_face {
                ui.label("Click a visible face in Native inspection to point it down.");
            }
            ui.small(
                "Uses the chosen triangle's normal; this does not simulate physical stability.",
            );
        });
        if serde_json::to_string(&self.scene).unwrap_or_default() != before {
            self.changed();
        }
        if serde_json::to_string(&self.scene.import).unwrap_or_default() != import_before
            && let Some(path) = &self.selected_source
        {
            self.restore_camera = true;
            self.debounced_import = Some((path.clone(), Instant::now()));
            self.importing = true;
            self.gate.advance();
        }
        ui.separator();
        ui.heading("Camera");
        ui.horizontal(|ui| {
            if ui.button("Fit").clicked() {
                self.timeline_playing = false;
                self.fit();
                self.changed();
            }
            for (label, view) in [
                ("Front", CanonicalView::Front),
                ("Right", CanonicalView::Right),
                ("Top", CanonicalView::Top),
                ("Iso", CanonicalView::Iso),
            ] {
                if ui.button(label).clicked() {
                    self.timeline_playing = false;
                    self.camera.canonical(view);
                    self.fit();
                    self.changed();
                }
            }
        });
        if ui
            .checkbox(&mut self.camera.orthographic, "Orthographic")
            .changed()
        {
            self.timeline_playing = false;
            self.changed();
        }
        self.timeline_controls(ui);
        ui.checkbox(&mut self.show_edges, "Triangle edges");
        egui::ComboBox::from_label("Inspection")
            .selected_text(format!("{:?}", self.inspection))
            .show_ui(ui, |ui| {
                for mode in [
                    InspectionMode::Opaque,
                    InspectionMode::Ghost,
                    InspectionMode::Xray,
                ] {
                    ui.selectable_value(&mut self.inspection, mode, format!("{mode:?}"));
                }
            });
        if self.inspection != InspectionMode::Opaque {
            ui.add(egui::Slider::new(&mut self.opacity, 0.02..=0.95).text("Inspection opacity"));
        }
        if ui
            .checkbox(&mut self.buildup, "Show printed build-up")
            .changed()
        {
            self.changed();
        }
        if self.buildup {
            let top = self
                .model
                .as_ref()
                .map_or(1., |m| m.bounds_mm.max[2] as f32);
            if ui
                .add(
                    egui::Slider::new(&mut self.buildup_height, 0.0..=top)
                        .text("Printed height (mm)"),
                )
                .changed()
            {
                self.timeline_playing = false;
                self.changed();
            }
            ui.checkbox(&mut self.buildup_playing, "Play build-up");
        }
        ui.small("Drag to orbit • right-drag to pan • wheel to zoom");
        ui.separator();
        ui.heading("Export");
        egui::ComboBox::from_label("Renderer")
            .selected_text(&self.engine)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.engine, "eevee".into(), "EEVEE (fast)");
                ui.selectable_value(&mut self.engine, "cycles".into(), "Cycles / OptiX");
            });
        ui.horizontal(|ui| {
            ui.label("GIF/video pixels");
            ui.add(
                egui::DragValue::new(&mut self.size)
                    .range(64..=8000)
                    .speed(64),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Samples");
            ui.add(egui::DragValue::new(&mut self.samples).range(1..=4096));
        });
        ui.checkbox(&mut self.transparent, "Transparent PNG background");
        ui.collapsing("PNG and sheet quality", |ui| {
            ui.add(
                egui::DragValue::new(&mut self.still_size)
                    .range(64..=8000)
                    .speed(64)
                    .suffix(" px PNG/sheet"),
            );
            egui::ComboBox::from_label("PNG renderer")
                .selected_text(&self.still_engine)
                .show_ui(ui, |ui| {
                    for engine in ["cycles", "eevee"] {
                        ui.selectable_value(&mut self.still_engine, engine.into(), engine);
                    }
                });
            ui.add(
                egui::DragValue::new(&mut self.still_samples)
                    .range(1..=4096)
                    .suffix(" PNG samples"),
            );
        });
        ui.add_enabled(
            self.model.as_ref().is_some_and(|m| !m.groups.is_empty()),
            egui::Checkbox::new(&mut self.export_buildup, "Export build-up animation"),
        );
        if !self.timeline_enabled && !self.export_buildup {
            ui.group(|ui| {
                ui.label("Basic turntable");
                ui.horizontal(|ui| {
                    ui.label("Seconds per turn");
                    ui.add(egui::DragValue::new(&mut self.turntable_timing.seconds_per_turn)
                        .range(0.05..=500.).speed(0.05).max_decimals(3));
                });
                ui.horizontal(|ui| {
                    ui.label("Animation length (s)");
                    ui.add(egui::DragValue::new(&mut self.turntable_timing.duration_seconds)
                        .range(0.05..=500.).speed(0.05).max_decimals(2));
                });
                match (self.turntable_timing.total_turns(), self.turntable_timing.frame_count()) {
                    (Ok(turns), Ok(frames)) => {
                        ui.label(format!("{turns:.3} total turns"));
                        ui.small(format!("{frames} frames • {:.2} s at 20 fps", f64::from(frames)/20.));
                    }
                    (Err(error), _) | (_, Err(error)) => {
                        ui.colored_label(egui::Color32::YELLOW, error.to_string());
                    }
                }
                ui.small("Matching frames are rendered once and reused. Length rounds to the nearest frame.");
            });
        }
        egui::ComboBox::from_label("MP4 codec")
            .selected_text(if self.codec == VideoCodec::Av1 {
                "AV1"
            } else {
                "H.264"
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.codec, VideoCodec::H264, "H.264");
                ui.selectable_value(&mut self.codec, VideoCodec::Av1, "AV1");
            });
        ui.collapsing("Section analysis", |ui| {
            let before = (self.section,self.section_axis,self.section_position,self.section_positive,self.section_cap,self.side_by_side);
            ui.checkbox(&mut self.section, "Show section / export cutaway");
            ui.horizontal(|ui| { ui.label("Build axis"); for (axis, label) in [(0, "X"), (1, "Y"), (2, "Z")] { ui.selectable_value(&mut self.section_axis, axis, label); } });
            ui.horizontal(|ui| { ui.label("Position (mm)"); ui.add(egui::DragValue::new(&mut self.section_position).speed(0.1)); });
            ui.checkbox(&mut self.section_positive, "Keep positive side");
            ui.checkbox(&mut self.section_cap, "Cap closed geometry");
            ui.checkbox(&mut self.side_by_side, "Full model beside section");
            ui.small("STL sections show the modeled solid. G-code sections preserve deposited beads and voids.");
            ui.small("Native cutaway is uncapped. Reliable caps and area/volume metrics are computed during export.");
            if let Some(analysis) = &self.analysis {ui.label(serde_json::to_string_pretty(analysis).unwrap_or_default());}
            if before != (self.section,self.section_axis,self.section_position,self.section_positive,self.section_cap,self.side_by_side) { self.changed(); }
        });
        ui.add_enabled_ui(
            self.model.is_some()
                && self.busy.is_none()
                && !self.importing
                && !self.cache_action_pending,
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (label, kind) in [
                        ("GIF", ExportKind::Gif),
                        ("PNG", ExportKind::Still),
                        ("4-view sheet", ExportKind::Diagram),
                        ("MP4", ExportKind::Mp4),
                    ] {
                        if ui.button(label).clicked() {
                            self.pick_export(kind);
                        }
                    }
                });
                if ui.button("Render preview").clicked() {
                    self.start_export(
                        ExportKind::Still,
                        self.data_dir.join(format!("preview-{}.png", self.revision)),
                        true,
                    );
                }
            },
        );
        if ui
            .checkbox(&mut self.prefs.auto_preview, "Auto-update rendered preview")
            .changed()
        {
            self.save_prefs();
        }
        if (self.engine.clone(), self.transparent) != preview_options_before {
            self.changed();
        }
    }
    fn tools_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_tools;
        if open && self.cache_refreshed.elapsed() > Duration::from_secs(5) {
            self.refresh_cache();
        }
        egui::Window::new("Tools and storage").open(&mut open).show(ctx, |ui| {
            for (label, path) in [("Blender", &mut self.prefs.tools.blender), ("FFmpeg", &mut self.prefs.tools.ffmpeg), ("FFprobe", &mut self.prefs.tools.ffprobe)] {
                let mut value = path.to_string_lossy().into_owned();
                ui.label(label); if ui.text_edit_singleline(&mut value).changed() { *path = PathBuf::from(value); }
            }
            ui.label(format!("Job storage: {}", self.data_dir.display()));
            let mut gb = self.prefs.cache_budget_bytes as f64 / 1073741824.;
            if ui.add(egui::DragValue::new(&mut gb).range(1.0..=1000.).suffix(" GiB job budget")).changed() { self.prefs.cache_budget_bytes = (gb * 1073741824.) as u64; }
            if ui.button("Save settings").clicked() { self.save_prefs(); self.status = "Settings saved.".into(); }
            ui.small("Failed job frames are retained for Retry. Native inspection works without external render tools.");
                ui.label(format!("Retained jobs: {:.2} GiB", self.cache_usage as f64 / 1073741824.));
                if self.cache_query_pending || self.cache_action_pending {ui.spinner();}
                    for (path, record) in self.cache_records.clone().into_iter().take(20) {
                        ui.horizontal_wrapped(|ui| {
                            let state=if record.state=="running" && self.busy.is_none() {"Interrupted"} else {&record.state};
                            ui.label(format!("{} — {}", record.label, state));
                            if ui.small_button("Folder").clicked() { open_path(&path); }
                            let active = self.last_task.as_ref().filter(|_| self.busy.is_some()).map(|t| t.0.job_dir.clone());
                            if ui.add_enabled(self.busy.is_none() && !self.cache_action_pending,egui::Button::new("Recover / encode")).clicked() {self.recover_job(path.clone());}
                            if ui.add_enabled(active.as_ref()!=Some(&path) && !self.cache_action_pending,egui::Button::new("Remove job")).clicked() {
                                self.cache_action_pending=true;let root=self.data_dir.join("jobs");let sender=self.sender.clone();let target=path.clone();
                                thread::spawn(move||{let result=CacheStore::new(root).and_then(|cache|cache.remove_job(&target,active.as_deref())).map_err(|error|error.to_string());
                                    let _=sender.send(Message::RemovedJob(result));});
                            }
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
                        .button("Reimport current source with saved settings")
                        .clicked()
                {
                    self.restore_project(project.clone());
                }
                if ui.button("Locate replacement source…").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Models", &["stl", "gcode", "gco"])
                        .pick_file()
                {
                    let mut relocated = project.clone();
                    relocated.source_path = path;
                    self.restore_project(relocated);
                }
                if ui.button("Dismiss").clicked() {
                    self.dismiss_project_recovery();
                }
            });
    }
    fn smoke(&mut self, ctx: &egui::Context) {
        let Some(directory) = self.smoke_output.clone() else {
            return;
        };
        if self.started.elapsed() > Duration::from_secs(120) {
            let _ = fs::create_dir_all(&directory);
            let _ = fs::write(
                directory.join("failed.txt"),
                self.error.as_deref().unwrap_or("smoke timeout"),
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if self.smoke_stage == 0 && self.model.is_some() && !self.importing {
            let _ = fs::create_dir_all(&directory);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                "native",
            )));
            self.smoke_stage = 1;
            self.start_export(ExportKind::Still, directory.join("stale-preview.png"), true);
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
            self.engine = "cycles".into();
            self.start_export(
                ExportKind::Still,
                directory.join("cancelled-preview.png"),
                true,
            );
            self.smoke_stage = 2;
        }
        if self.smoke_stage == 2 && self.busy.is_some() && self.progress.2 == "Rendering" {
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
            self.engine = "eevee".into();
            self.fit();
            self.changed();
            self.size = 128;
            self.samples = 4;
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
            && self.output.as_ref().is_some_and(|p| p.frames.len() == 12)
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
            self.section = true;
            self.section_axis = 0;
            self.section_position = 0.75;
            if let Some(model) = &self.model {
                let project = ProjectFile {
                    schema_version: 1,
                    source_path: model.path.clone(),
                    source_hash: model.source_hash.clone(),
                    scene: self.scene.clone(),
                    camera: self.camera.clone(),
                    export_size: self.size,
                    engine: self.engine.clone(),
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
                    && self.timeline.frame_count().ok() == Some(4)
                    && self.timeline.keyframes.len() == 2
                    && self.timeline.keyframes[0].spiral_turns == 1.
            );
            self.smoke_checks["basic_turntable_project_restored"] = serde_json::json!(
                self.turntable_timing.seconds_per_turn == 4.
                    && self.turntable_timing.duration_seconds == 12.
            );
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
            self.samples = 4;
            if let Some(model) = self.model.as_ref().filter(|m| !m.groups.is_empty()) {
                self.section_axis = 0;
                self.section_position = (model.bounds_mm.min[0] + model.bounds_mm.max[0]) / 2.;
                self.section = true;
                self.side_by_side = true;
                self.timeline.buildup = true;
                self.export_buildup = true;
                self.scene.select_material("resin");
                self.scene.color_mode = "tool".into();
                self.scene.studio_preset = "dark".into();
                self.engine = "cycles".into();
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
            && self.output.as_ref().is_some_and(|p| p.frames.len() == 4)
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
            self.smoke_stage = 9;
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
            if name == "timeline" {
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
        let color_signature =
            serde_json::to_string(&(&self.scene.color_mode, &self.scene.tool_colors))
                .unwrap_or_default();
        if self.color_signature != color_signature {
            if let Some(model) = &self.model {
                let colors = if self.scene.color_mode == "uniform" || model.groups.is_empty() {
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
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Turninator");
                if ui.button("Open model…").clicked() {
                    self.pick_model();
                }
                if ui.button("Open project…").clicked() {
                    let sender = self.sender.clone();
                    thread::spawn(move || {
                        let _ = sender.send(Message::OpenProject(
                            rfd::FileDialog::new()
                                .add_filter("Turninator project", &["turninator"])
                                .pick_file(),
                        ));
                    });
                }
                if ui
                    .add_enabled(
                        self.model.is_some() && !self.importing,
                        egui::Button::new("Save project…"),
                    )
                    .clicked()
                {
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
                ui.menu_button("Recent", |ui| {
                    for path in self.prefs.recent_files.clone() {
                        if ui.button(path.display().to_string()).clicked() {
                            self.restore_camera = false;
                            self.import(path);
                            ui.close();
                        }
                    }
                });
                if ui.button("Tools / storage").clicked() {
                    self.show_tools = true;
                }
                if self.importing {
                    ui.spinner();
                    ui.label("Importing…");
                }
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
                    );
                    if ui.button("Cancel").clicked() {
                        self.render.cancel();
                        self.progress.2 = "Cancelling owned render job…".into();
                    }
                });
            }
            if let Some(error) = self.error.clone() {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                    if self.busy.is_none()
                        && self.last_task.is_some()
                        && ui.button("Retry retained job").clicked()
                    {
                        self.retry();
                    }
                    if ui.small_button("Dismiss").clicked() {
                        self.error = None;
                    }
                });
            }
            ui.label(&self.status);
            if let Some(output) = &self.last_output {
                let path = output.path.clone();
                ui.horizontal(|ui| {
                    if ui.button("Open saved output").clicked() {
                        open_path(&path);
                    }
                    if ui.button("Open output folder").clicked()
                        && let Some(parent) = path.parent()
                    {
                        open_path(parent);
                    }
                    if self.output.is_some() {
                        ui.checkbox(&mut self.show_output, "Show encoded output");
                    }
                });
            }
        });
        egui::Panel::left("controls")
            .default_size(285.)
            .resizable(true)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.sidebar(ui);
                });
            });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.columns(2, |columns| {
                columns[0].heading("Native inspection");
                let before = self.camera.camera_spec();
                let settings = self.viewport_settings();
                let response = self
                    .viewport
                    .show(&mut columns[0], &mut self.camera, &settings);
                self.native_rect = Some(response.rect);
                if self.camera.camera_spec() != before {
                    self.timeline_playing = false;
                    self.changed();
                }
                if self.settle_face
                    && !self.importing
                    && response.clicked()
                    && let Some(pointer) = response.interact_pointer_pos()
                    && self.settle_pointer(response.rect, pointer).is_none()
                {
                    self.status =
                        "No visible face under that point; click the model surface.".into();
                }
                columns[1].horizontal(|ui| {
                    ui.heading(if self.show_output {
                        "Encoded output"
                    } else {
                        "Rendered preview"
                    });
                    if !self.show_output && self.preview_revision != Some(self.revision) {
                        ui.label(if self.busy.is_some() {
                            "Updating…"
                        } else {
                            "Stale"
                        });
                    }
                });
                let display = if self.show_output {
                    &mut self.output
                } else {
                    &mut self.preview
                };
                if let Some(playback) = display {
                    playback.advance(&ctx);
                    if playback.frames.len() > 1
                        && columns[1]
                            .checkbox(&mut playback.playing, "Play animation")
                            .changed()
                        && playback.playing
                    {
                        playback.next = Instant::now() + playback.frames[playback.index].duration;
                    }
                    columns[1].add(egui::Image::new(&playback.texture).fit_to_exact_size(
                        fit_image(playback.texture.size_vec2(), columns[1].available_size()),
                    ));
                } else {
                    columns[1].label(
                        "Render a preview to see the selected material and studio lighting.",
                    );
                }
            });
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
fn rotation_editor(ui: &mut egui::Ui, rotation: &mut [f64; 3]) {
    ui.horizontal(|ui| {
        for (label, value) in ["X", "Y", "Z"].into_iter().zip(rotation) {
            ui.add(
                egui::DragValue::new(value)
                    .speed(1.)
                    .prefix(format!("{label} "))
                    .suffix("°"),
            );
        }
    });
}
fn fit_image(image: egui::Vec2, available: egui::Vec2) -> egui::Vec2 {
    image * (available.x / image.x).min(available.y / image.y).max(0.01)
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
fn decode_output(path: &Path) -> Result<Vec<PlaybackFrame>> {
    use image::AnimationDecoder;
    if path.extension().is_some_and(|v| v == "gif") {
        let decoder =
            image::codecs::gif::GifDecoder::new(std::io::BufReader::new(fs::File::open(path)?))?;
        let mut frames = Vec::new();
        let mut bytes = 0usize;
        for frame in decoder.into_frames() {
            let frame = frame?;
            let (n, d) = frame.delay().numer_denom_ms();
            let buffer = display_buffer(frame.into_buffer());
            bytes = bytes
                .checked_add(buffer.len())
                .ok_or("playback memory overflow")?;
            if bytes > 512 * 1024 * 1024 {
                return Err(
                    "Animation exceeds 512 MiB playback budget; use Open saved output.".into(),
                );
            }
            frames.push(PlaybackFrame {
                image: egui::ColorImage::from_rgba_unmultiplied(
                    [buffer.width() as usize, buffer.height() as usize],
                    buffer.as_raw(),
                ),
                duration: Duration::from_secs_f64((n as f64 / d.max(1) as f64 / 1000.).max(0.01)),
            });
        }
        Ok(frames)
    } else {
        let buffer = display_buffer(image::open(path)?.to_rgba8());
        Ok(vec![PlaybackFrame {
            image: egui::ColorImage::from_rgba_unmultiplied(
                [buffer.width() as usize, buffer.height() as usize],
                buffer.as_raw(),
            ),
            duration: Duration::from_secs(1),
        }])
    }
}
fn display_buffer(buffer: image::RgbaImage) -> image::RgbaImage {
    let largest = buffer.width().max(buffer.height());
    if largest <= 1280 {
        buffer
    } else {
        let scale = 1280. / largest as f64;
        image::imageops::resize(
            &buffer,
            (buffer.width() as f64 * scale).round().max(1.) as u32,
            (buffer.height() as f64 * scale).round().max(1.) as u32,
            image::imageops::FilterType::Triangle,
        )
    }
}
