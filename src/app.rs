//! Native studio. Import and rendering run independently of the UI thread.
use crate::{
    Result,
    cache::{CacheStore, JobRecord},
    export::{
        ExportKind, ExportLayout, ExportRequest, ExportResult, JobEvent, SectionSpec, VideoCodec,
    },
    presentation::{ImportedModel, RevisionGate, SceneSettings, load_model, render_scene},
    project::{ProjectFile, SourceStatus, load_project, save_project},
    session::{RenderEvent, RenderService, RenderTask, default_request},
    settings::{AppSettings, default_data_directory},
    viewport::{CanonicalView, OrbitCamera, Viewport, ViewportSettings},
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
        result: std::result::Result<ImportedModel, String>,
    },
    OpenModel(Option<PathBuf>),
    OpenProject(Option<PathBuf>),
    SaveProject(Option<PathBuf>),
    ExportDestination {
        kind: ExportKind,
        path: Option<PathBuf>,
    },
    Image {
        revision: u64,
        preview: bool,
        result: std::result::Result<Vec<PlaybackFrame>, String>,
    },
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
    sender: Sender<Message>,
    messages: Receiver<Message>,
    gate: RevisionGate,
    importing: bool,
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
    output: Option<Playback>,
    show_output: bool,
    error: Option<String>,
    status: String,
    restore_camera: bool,
    unresolved_project: Option<(ProjectFile, SourceStatus)>,
    project_path: Option<PathBuf>,
    engine: String,
    size: u32,
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
    smoke_output: Option<PathBuf>,
    smoke_stage: u8,
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
            output: None,
            show_output: false,
            error,
            status: "Open an STL or G-code file to begin.".into(),
            restore_camera: false,
            unresolved_project: None,
            project_path: None,
            engine: "eevee".into(),
            size: 960,
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
            smoke_output: options.smoke_output,
            smoke_stage: 0,
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
            layer_appearance: self.scene.layer_appearance,
            layer_height_mm: self.scene.import.layer_height_mm as f32,
            first_layer_height_mm: self.scene.import.first_layer_height_mm as f32,
            groove_depth_mm: self.scene.groove_depth_mm as f32,
            display_rotation_degrees: self.scene.display_rotation_degrees.map(|v| v as f32),
            show_edges: self.show_edges,
            ..Default::default()
        }
    }
    fn changed(&mut self) {
        self.revision += 1;
        self.changed_at = Instant::now();
    }
    fn import(&mut self, path: PathBuf) {
        let generation = self.gate.advance();
        self.importing = true;
        self.status = format!("Loading {}…", path.display());
        let settings = self.scene.import.clone();
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = load_model(&path, &settings).map_err(|error| error.to_string());
            let _ = sender.send(Message::Imported { generation, result });
        });
    }
    fn fit(&mut self) {
        if let Some(bounds) = self.viewport.display_bounds(&self.viewport_settings()) {
            self.camera.fit(&bounds);
        }
    }
    fn save_prefs(&mut self) {
        if let Err(error) = self.prefs.save(&self.data_dir.join("settings.json")) {
            self.error = Some(format!("Could not save settings: {error}"));
        }
    }
    fn snapshot(&self, request: &ExportRequest) -> Result<serde_json::Value> {
        let model = self.model.as_ref().ok_or("open a model first")?;
        let mut scene = render_scene(
            &self.scene,
            &request.job_dir.join("mesh.bin"),
            &model.geometry_kind,
            &model.warnings,
            Some(self.camera.camera_spec()),
        )?;
        scene["source_path"] = serde_json::json!(model.path);
        Ok(scene)
    }
    fn start_export(&mut self, kind: ExportKind, destination: PathBuf, preview: bool) {
        if self.busy.is_some() || self.importing {
            return;
        }
        let result = (|| -> Result<()> {
            self.prefs.tools.validate()?;
            self.next_job += 1;
            let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
            let cache = CacheStore::new(self.data_dir.join("jobs"))?;
            let render_size = if preview { 768 } else { self.size };
            let reserve = u64::from(render_size).pow(2)
                * 8
                * if matches!(kind, ExportKind::Gif | ExportKind::Mp4) {
                    80
                } else {
                    4
                };
            cache.check_budget(self.prefs.cache_budget_bytes, reserve)?;
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
                self.engine.clone(),
                if preview { 768 } else { self.size },
                if preview { 16 } else { self.samples },
                self.transparent,
            );
            if !preview && self.section && kind != ExportKind::Diagram {
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
                })
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            self.busy = Some(self.next_job);
            self.progress = (0, 1, "Starting renderer".into());
            self.error = None;
            Ok(())
        })();
        if let Err(error) = result {
            self.error = Some(error.to_string());
        }
    }
    fn retry(&mut self) {
        if self.busy.is_some() {
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
            }) {
                Ok(()) => {
                    self.busy = Some(self.next_job);
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
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
        let sender = self.sender.clone();
        thread::spawn(move || {
            let extension = match kind {
                ExportKind::Gif | ExportKind::SectionTurntable => "gif",
                ExportKind::Mp4 => "mp4",
                _ => "png",
            };
            let path = rfd::FileDialog::new()
                .add_filter("Output", &[extension])
                .set_file_name(format!("turninator.{extension}"))
                .save_file();
            let _ = sender.send(Message::ExportDestination { kind, path });
        });
    }
    fn receive(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.messages.try_recv() {
            match message {
                Message::Imported { generation, result } if self.gate.accepts(generation) => {
                    self.importing = false;
                    match result {
                        Ok(model) => {
                            match self.viewport.set_mesh(
                                &model.mesh,
                                &model.geometry_kind,
                                generation,
                            ) {
                                Ok(()) => {
                                    self.status = format!(
                                        "{} • {} triangles",
                                        model.path.display(),
                                        model.mesh.triangles.len()
                                    );
                                    self.section_position = model
                                        .mesh
                                        .bounds()
                                        .map(|b| (b.min[2] + b.max[2]) / 2.)
                                        .unwrap_or(0.);
                                    self.prefs.remember(model.path.clone());
                                    self.model = Some(model);
                                    if !self.restore_camera {
                                        self.fit();
                                    }
                                    self.restore_camera = false;
                                    self.changed();
                                    self.save_prefs();
                                }
                                Err(error) => self.error = Some(error.to_string()),
                            }
                        }
                        Err(error) => {
                            self.status = "Import failed; previous model remains available.".into();
                            self.error = Some(error);
                        }
                    }
                }
                Message::Imported { .. } => {}
                Message::OpenModel(Some(path)) => {
                    self.restore_camera = false;
                    self.import(path);
                }
                Message::OpenProject(Some(path)) => match load_project(&path) {
                    Ok((project, SourceStatus::Current)) => {
                        self.project_path = Some(path);
                        self.restore_project(project);
                    }
                    Ok((project, status)) => {
                        self.project_path = Some(path);
                        self.unresolved_project = Some((project, status));
                    }
                    Err(error) => self.error = Some(error.to_string()),
                },
                Message::SaveProject(Some(path)) => {
                    if let Some(model) = &self.model {
                        let project = ProjectFile {
                            schema_version: 1,
                            source_path: model.path.clone(),
                            source_hash: model.source_hash.clone(),
                            scene: self.scene.clone(),
                            camera: self.camera.clone(),
                            export_size: self.size,
                            engine: self.engine.clone(),
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
                    path: Some(path),
                } => self.start_export(kind, path, false),
                Message::Image {
                    revision,
                    preview,
                    result,
                } => match result {
                    Ok(frames) => {
                        if preview {
                            if revision == self.revision {
                                self.preview = Playback::new(ctx, frames);
                                self.preview_revision = Some(revision);
                            }
                        } else {
                            self.output = Playback::new(ctx, frames);
                            self.show_output = true;
                        }
                    }
                    Err(error) => self.error = Some(error),
                },
                _ => {}
            }
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
                        continue;
                    }
                    let sender = self.sender.clone();
                    thread::spawn(move || {
                        let decoded =
                            decode_output(&result.path).map_err(|error| error.to_string());
                        let _ = sender.send(Message::Image {
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
        self.scene = project.scene;
        self.camera = project.camera;
        self.size = project.export_size;
        self.engine = project.engine;
        self.restore_camera = true;
        self.unresolved_project = None;
        self.import(project.source_path);
    }
    fn sidebar(&mut self, ui: &mut egui::Ui) {
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
        }
        ui.color_edit_button_rgb(&mut self.scene.color);
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
        });
        ui.collapsing("Presentation pose", |ui| {
            ui.label("Display rotation (keeps print layers)");
            rotation_editor(ui, &mut self.scene.display_rotation_degrees);
        });
        if serde_json::to_string(&self.scene).unwrap_or_default() != before {
            self.changed();
        }
        if serde_json::to_string(&self.scene.import).unwrap_or_default() != import_before
            && let Some(model) = &self.model
        {
            self.restore_camera = true;
            self.import(model.path.clone());
        }
        ui.separator();
        ui.heading("Camera");
        ui.horizontal(|ui| {
            if ui.button("Fit").clicked() {
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
            self.changed();
        }
        ui.checkbox(&mut self.show_edges, "Triangle edges");
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
            ui.label("Pixels");
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
            ui.checkbox(&mut self.section, "Enable section in exports");
            ui.horizontal(|ui| { ui.label("Build axis"); for (axis, label) in [(0, "X"), (1, "Y"), (2, "Z")] { ui.selectable_value(&mut self.section_axis, axis, label); } });
            ui.horizontal(|ui| { ui.label("Position (mm)"); ui.add(egui::DragValue::new(&mut self.section_position).speed(0.1)); });
            ui.checkbox(&mut self.section_positive, "Keep positive side");
            ui.checkbox(&mut self.section_cap, "Cap closed geometry");
            ui.checkbox(&mut self.side_by_side, "Full model beside section");
            ui.small("STL sections show the modeled solid. G-code sections preserve deposited beads and voids.");
        });
        ui.add_enabled_ui(
            self.model.is_some() && self.busy.is_none() && !self.importing,
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
    }
    fn tools_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_tools;
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
            if let Ok(cache) = CacheStore::new(self.data_dir.join("jobs")) {
                ui.label(format!("Retained jobs: {:.2} GiB", cache.usage().unwrap_or(0) as f64 / 1073741824.));
                if let Ok(records) = cache.records() {
                    for (path, record) in records.into_iter().take(20) {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(format!("{} — {}", record.label, record.state));
                            if ui.small_button("Folder").clicked() { open_path(&path); }
                            let active = self.last_task.as_ref().filter(|_| self.busy.is_some()).map(|t| t.0.job_dir.as_path());
                            if ui.add_enabled(active != Some(path.as_path()), egui::Button::new("Remove job")).clicked()
                                && let Err(error) = cache.remove_job(&path, active) { self.error = Some(error.to_string()); }
                        });
                    }
                }
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
                    self.unresolved_project = None;
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
            self.start_export(ExportKind::Still, directory.join("rendered.png"), true);
        }
        if self.smoke_stage == 1 && self.preview_revision == Some(self.revision) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                "rendered",
            )));
            self.smoke_stage = 2;
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
            if name == "rendered" {
                let _ = fs::write(directory.join("result.json"), serde_json::json!({"model":self.model.as_ref().map(|m| &m.path), "triangles":self.viewport.triangle_count(),"revision":self.revision,"preview_revision":self.preview_revision,"error":self.error}).to_string());
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}
impl eframe::App for Studio {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.receive(&ctx);
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
                    .add_enabled(self.model.is_some(), egui::Button::new("Save project…"))
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
                self.viewport
                    .show(&mut columns[0], &mut self.camera, &settings);
                if self.camera.camera_spec() != before {
                    self.changed();
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
                    if playback.frames.len() > 1 {
                        columns[1].checkbox(&mut playback.playing, "Play animation");
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
            let buffer = frame.into_buffer();
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
        let buffer = image::open(path)?.to_rgba8();
        Ok(vec![PlaybackFrame {
            image: egui::ColorImage::from_rgba_unmultiplied(
                [buffer.width() as usize, buffer.height() as usize],
                buffer.as_raw(),
            ),
            duration: Duration::from_secs(1),
        }])
    }
}
