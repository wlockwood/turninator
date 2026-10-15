use super::encode;
use crate::{
    Result,
    render::{ToolPaths, WorkerClient},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExportKind {
    Gif,
    Still,
    Diagram,
    Mp4,
    SectionTurntable,
    SectionStill,
}
/// User-facing output selection; section variants are derived from scene options.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum OutputFormat {
    #[default]
    Gif,
    Mp4,
    Png,
    FourView,
}
impl OutputFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Gif => "GIF animation",
            Self::Mp4 => "MP4 video",
            Self::Png => "PNG image",
            Self::FourView => "4-view sheet",
        }
    }
    pub fn kind(self) -> ExportKind {
        match self {
            Self::Gif => ExportKind::Gif,
            Self::Mp4 => ExportKind::Mp4,
            Self::Png => ExportKind::Still,
            Self::FourView => ExportKind::Diagram,
        }
    }
    pub fn animated(self) -> bool {
        matches!(self, Self::Gif | Self::Mp4)
    }
    pub fn supports_transparency(self) -> bool {
        self == Self::Png
    }
    pub fn supports_section(self) -> bool {
        self != Self::FourView
    }
}
impl ExportKind {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Gif | Self::SectionTurntable => "gif",
            Self::Mp4 => "mp4",
            _ => "png",
        }
    }

    /// Derive suggestions from the accepted model, without retaining a previous model's name.
    pub fn suggested_file_name(self, source: &Path) -> String {
        let stem = source
            .file_stem()
            .filter(|s| !s.is_empty())
            .unwrap_or(std::ffi::OsStr::new("turninator"))
            .to_string_lossy();
        format!("{stem}.{}", self.extension())
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum VideoCodec {
    #[default]
    H264,
    Av1,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum AnimationMode {
    #[default]
    Turntable,
    BuildUp,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExportLayout {
    #[default]
    Single,
    SideBySide,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SectionSpec {
    pub axis: u8,
    pub position_mm: f64,
    pub keep_positive: bool,
    pub cap: bool,
}
/// Number of print layers to retain: STL solid slices or G-code deposited layers.
/// Zero is an empty build plate; the full count shows the complete source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildStage {
    pub visible_layers: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportRequest {
    pub job_dir: PathBuf,
    pub scene_path: PathBuf,
    pub destination: PathBuf,
    pub kind: ExportKind,
    pub engine: String,
    pub size: u32,
    pub samples: u32,
    pub frame_count: u32,
    #[serde(default = "crate::animation::default_frame_rate")]
    pub frame_rate: u32,
    pub transparent: bool,
    #[serde(default)]
    pub video_codec: VideoCodec,
    #[serde(default)]
    pub layout: ExportLayout,
    #[serde(default)]
    pub section: Option<SectionSpec>,
    #[serde(default)]
    pub animation: AnimationMode,
    #[serde(default)]
    pub timeline: Option<crate::animation::Timeline>,
    #[serde(default)]
    pub turntable_timing: Option<crate::animation::TurntableTiming>,
    #[serde(default)]
    pub build_stage: Option<BuildStage>,
    #[serde(default)]
    pub start_angle_degrees: f64,
}
impl Default for ExportRequest {
    fn default() -> Self {
        Self {
            job_dir: PathBuf::new(),
            scene_path: PathBuf::new(),
            destination: PathBuf::new(),
            kind: ExportKind::Gif,
            engine: "eevee".into(),
            size: 960,
            samples: 32,
            frame_count: 80,
            frame_rate: crate::animation::DEFAULT_FRAME_RATE,
            transparent: false,
            video_codec: VideoCodec::H264,
            layout: ExportLayout::Single,
            section: None,
            animation: AnimationMode::Turntable,
            timeline: None,
            turntable_timing: None,
            build_stage: None,
            start_angle_degrees: 0.0,
        }
    }
}
impl ExportRequest {
    pub fn animation_frame_count(&self) -> Result<u32> {
        crate::animation::validate_frame_rate(self.frame_rate)?;
        if let Some(timing) = self.turntable_timing {
            if self.timeline.is_some() || self.animation != AnimationMode::Turntable {
                return Err(
                    "basic turntable timing cannot be combined with a camera timeline or build-up"
                        .into(),
                );
            }
            timing.frame_count_at_rate(self.frame_rate)
        } else if let Some(timeline) = &self.timeline {
            timeline.frame_count_at_rate(self.frame_rate)
        } else {
            Ok(self.frame_count)
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportResult {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub frame_count: u32,
    pub duration_ms: u64,
}
#[derive(Clone, Debug)]
pub enum JobEvent {
    Progress {
        completed: u32,
        total: u32,
        stage: String,
    },
    Log(String),
    Frame(PathBuf),
    Analysis(Value),
}

fn hash(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut f = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let n = f.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        digest.update(&bytes[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn write_json(path: &Path, value: &Value) -> Result<()> {
    let partial = path.with_extension("partial.json");
    fs::write(&partial, serde_json::to_vec_pretty(value)?)?;
    encode::publish(&partial, path)
}
fn write_scene_json(path: &Path, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > 1024 * 1024 {
        return Err("derived render scene exceeds 1 MiB byte budget".into());
    }
    crate::settings::atomic_write(path, &bytes)
}

#[cfg(test)]
mod diagram_style_tests {
    use super::*;
    #[test]
    fn fixed_diagram_ignores_hidden_material_pose_and_palette() {
        let source = json!({"face_groups":[{"color":[1.,0.,0.],"start_triangle":0,"end_triangle":1}],
            "material_preset":"resin","layer_appearance":true,"display_rotation_degrees":[10,20,30]});
        let camera = json!({"projection":"orthographic"});
        let panel = diagram_panel_scene(&source, &camera);
        assert_eq!(panel["face_groups"], json!([]));
        assert_eq!(panel["material_preset"], "clay");
        assert_eq!(panel["layer_appearance"], false);
        assert_eq!(panel["display_rotation_degrees"], json!([0., 0., 0.]));
        assert_eq!(panel["camera"], camera);
        assert_eq!(source["face_groups"].as_array().unwrap().len(), 1);
        let printed = diagram_panel_scene(
            &json!({"diagram_printed_surface":true,"face_groups":source["face_groups"],"material_preset":"resin"}),
            &camera,
        );
        assert_eq!(printed["face_groups"], source["face_groups"]);
        assert_eq!(printed["material_preset"], "resin");
    }
}

#[cfg(test)]
mod scene_budget_tests {
    use super::*;

    #[test]
    fn derived_scenes_keep_compact_budget_and_reject_growth_before_overwrite() {
        let path = std::env::temp_dir().join(format!(
            "turninator-derived-scene-{}.json",
            std::process::id()
        ));
        let group = json!({"start_triangle":123456,"end_triangle":123520,"layer_index":1234});
        let scene = json!({"layer_face_ranges":vec![group; 12000],"section":{"axis":0,"position_mm":4,"keep_positive":true,"cap":true}});
        assert!(serde_json::to_vec(&scene).unwrap().len() < 1024 * 1024);
        assert!(serde_json::to_vec_pretty(&scene).unwrap().len() > 1024 * 1024);
        write_scene_json(&path, &scene).unwrap();
        let original = fs::read(&path).unwrap();
        assert!(original.len() <= 1024 * 1024);
        assert_eq!(serde_json::from_slice::<Value>(&original).unwrap(), scene);
        let oversized = json!({"warnings":["x".repeat(1024*1024)]});
        assert!(write_scene_json(&path, &oversized).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::remove_file(path).unwrap();
    }
}

pub fn run_export(
    worker: &mut WorkerClient,
    tools: &ToolPaths,
    request: &ExportRequest,
    cancel: &AtomicBool,
    mut callback: impl FnMut(JobEvent),
) -> Result<ExportResult> {
    crate::animation::validate_frame_rate(request.frame_rate)?;
    if !request.start_angle_degrees.is_finite() || request.start_angle_degrees.abs() > 1e9 {
        return Err("start angle must be finite and within supported range".into());
    }
    let mut effective_request = request.clone();
    if effective_request.turntable_timing.is_some() || effective_request.timeline.is_some() {
        effective_request.frame_count = effective_request.animation_frame_count()?;
        if !matches!(
            request.kind,
            ExportKind::Gif | ExportKind::Mp4 | ExportKind::SectionTurntable
        ) {
            return Err("animation timing requires GIF or MP4 output".into());
        }
    }
    if effective_request.timeline.is_some() || effective_request.kind == ExportKind::Diagram {
        effective_request.start_angle_degrees = 0.;
    }
    let request = &effective_request;
    if let Some(stage) = request.build_stage {
        if stage.visible_layers > 50_000 {
            return Err("static build stage exceeds 50000 layers".into());
        }
        if request.animation == AnimationMode::BuildUp
            || request
                .timeline
                .as_ref()
                .is_some_and(|timeline| timeline.buildup)
        {
            return Err("choose a fixed build stage or progressive build-up, not both".into());
        }
    }
    if !(16..=8000).contains(&request.size)
        || !(1..=4096).contains(&request.samples)
        || !(1..=10000).contains(&request.frame_count)
        || !matches!(request.engine.as_str(), "eevee" | "cycles")
    {
        return Err("invalid export settings".into());
    }
    if !request.job_dir.is_absolute()
        || !request.scene_path.is_absolute()
        || !request.destination.is_absolute()
    {
        return Err("export paths must be absolute".into());
    }
    if cancel.load(Ordering::Acquire) {
        return Err("cancelled".into());
    }
    if request.animation == AnimationMode::BuildUp
        && !matches!(
            request.kind,
            ExportKind::Gif | ExportKind::Mp4 | ExportKind::SectionTurntable
        )
    {
        return Err("build-up supports GIF/MP4 bead animations".into());
    }
    if let Some(section) = &request.section {
        if section.axis > 2 || !section.position_mm.is_finite() {
            return Err("section requires a finite position and axis 0, 1 or 2".into());
        }
        if request.kind == ExportKind::Diagram {
            return Err("choose a section still or turntable for section exports".into());
        }
    } else if request.layout == ExportLayout::SideBySide
        || matches!(
            request.kind,
            ExportKind::SectionStill | ExportKind::SectionTurntable
        )
    {
        return Err("section export requires section settings".into());
    }
    if request.layout == ExportLayout::SideBySide && u64::from(request.size).pow(2) * 2 > 64_000_000
    {
        return Err("side-by-side output exceeds 64 million pixels; reduce panel size".into());
    }
    fs::create_dir_all(&request.job_dir)?;
    let scene: Value = serde_json::from_slice(&crate::settings::read_bounded(
        &request.scene_path,
        1024 * 1024,
    )?)?;
    let mesh = PathBuf::from(
        scene["mesh_path"]
            .as_str()
            .ok_or("scene mesh_path required")?,
    );
    let mut protected = vec![request.scene_path.clone(), mesh.clone()];
    if let Some(source) = scene["source_path"].as_str() {
        protected.push(PathBuf::from(source));
    }
    if request.destination.exists() {
        let destination = fs::canonicalize(&request.destination)?
            .to_string_lossy()
            .to_lowercase();
        for path in protected {
            if path.exists()
                && fs::canonicalize(path)?.to_string_lossy().to_lowercase() == destination
            {
                return Err("export destination would replace a source or job input".into());
            }
        }
    }
    let animated = matches!(
        request.kind,
        ExportKind::Gif | ExportKind::Mp4 | ExportKind::SectionTurntable
    );
    let rendered_count = if animated {
        request.frame_count
    } else if request.kind == ExportKind::Diagram {
        4
    } else {
        1
    };
    preflight_storage(
        &request.job_dir,
        request.size as u64
            * request.size as u64
            * 5
            * rendered_count as u64
            * if request.layout == ExportLayout::SideBySide {
                4
            } else {
                1
            },
    )?;
    let mut recipe = json!({"schema_version":2,"scene_hash":hash(&request.scene_path)?,"mesh_hash":hash(&mesh)?,"engine":request.engine,"size":request.size,"samples":request.samples,"frame_count":rendered_count,"frame_rate":if animated{Some(request.frame_rate)}else{None},"transparent":request.transparent&&!animated,"section":request.section,"layout":request.layout,"animation":request.animation,"timeline":request.timeline,"turntable_timing":request.turntable_timing,"build_stage":request.build_stage,"start_angle_degrees":request.start_angle_degrees,"kind":if animated{"animation"}else if request.kind==ExportKind::Diagram{"diagram"}else{"still"},"worker_hash":hash(&crate::render::worker::renderer_script("worker.py"))?,"scene_script_hash":hash(&crate::render::worker::renderer_script("scene.py"))?,"frame_pipeline_hash":hash(&crate::render::worker::renderer_script("frame_pipeline.py"))?,"blender_hash":hash(&tools.blender)?});
    if request.kind == ExportKind::Diagram {
        recipe["diagram_recipe"] = json!({"version":2,"views":["TOP","ISO","FRONT","RIGHT"],"dimension":"largest_visible_projected_extent_mm","framing_padding":1.4});
    }
    let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&recipe)?));
    // Encoding changes must be recorded without invalidating compatible PNGs.
    let encoding_recipe = match request.kind {
        ExportKind::Gif | ExportKind::SectionTurntable => json!({
            "format":"gif","ffmpeg_hash":hash(&tools.ffmpeg)?,
            "options":encode::gif_encoding_recipe_at_rate(request.frame_rate)
        }),
        ExportKind::Mp4 => json!({
            "format":"mp4","codec":request.video_codec,"fps":request.frame_rate,
            "ffmpeg_hash":hash(&tools.ffmpeg)?,"ffprobe_hash":hash(&tools.ffprobe)?
        }),
        _ => json!({"format":"png"}),
    };
    let render_dir = request.job_dir.join(&fingerprint[..20]);
    fs::create_dir_all(&render_dir)?;
    let frame_states_path = if let Some(timeline) = &request.timeline {
        let mut timeline = timeline.clone();
        timeline.buildup |= request.animation == AnimationMode::BuildUp;
        let path = render_dir.join("frame-states.json");
        let states = serde_json::to_value(timeline.worker_frames_at_rate(request.frame_rate)?)?;
        let bytes = serde_json::to_vec(&states)?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("timeline states exceed 8 MiB budget".into());
        }
        crate::settings::atomic_write(&path, &bytes)?;
        Some(path)
    } else {
        None
    };
    let manifest = render_dir.join("job.json");
    if manifest.exists() {
        let previous: Value =
            serde_json::from_slice(&crate::settings::read_bounded(&manifest, 2 * 1024 * 1024)?)?;
        if previous["recipe"] != recipe {
            return Err("existing job recipe differs".into());
        }
    }
    write_json(
        &manifest,
        &json!({"recipe":recipe,"encoding_recipe":encoding_recipe,"stage":"rendering","destination":request.destination}),
    )?;
    let operation = (|| -> Result<ExportResult> {
        if request.kind == ExportKind::Diagram {
            return render_diagram(
                worker,
                request,
                &scene,
                &mesh,
                &render_dir,
                cancel,
                &mut callback,
            );
        }
        let animation = animated;
        let count = if animation { request.frame_count } else { 1 };
        let frames = if request.section.is_some() {
            render_section(worker, request, &scene, &render_dir, cancel, &mut callback)?
        } else {
            render_dir.join("frames")
        };
        if request.section.is_none() {
            callback(JobEvent::Progress {
                completed: 0,
                total: count,
                stage: format!(
                    "Rendering with {}",
                    if request.engine == "eevee" {
                        "Eevee"
                    } else {
                        "Cycles / OptiX"
                    }
                ),
            });
            let complete=worker.request(json!({"request_id":format!("export-{}",&fingerprint[..20]),"scene_revision":1,"kind":"render","scene_path":request.scene_path,"output_dir":frames,"mode":if animation{if request.animation==AnimationMode::BuildUp{"buildup"}else{"turntable"}}else{"still"},"engine":request.engine,"size":request.size,"samples":request.samples,"frame_count":count,"frame_rate":request.frame_rate,"seconds_per_turn":request.turntable_timing.map(|timing| timing.seconds_per_turn),"build_stage":request.build_stage,"frame_states_path":frame_states_path,"start_angle_degrees":request.start_angle_degrees,"transparent":request.transparent&&!animation,"reuse_verified_frames":frames.join("render-manifest.json").exists()}),cancel,|event|{
            if event["kind"]=="frame"{let n=event["payload"]["index"].as_u64().unwrap_or(0) as u32+1;callback(JobEvent::Progress{completed:n,total:count,stage:if event["payload"]["reuse_kind"]=="identical"{"Reusing identical frames"}else if event["payload"]["reused"]==true{"Reusing verified frames"}else{if request.engine == "eevee" { "Rendering with Eevee" } else { "Rendering with Cycles / OptiX" }}.into()});if let Some(path)=event["payload"]["path"].as_str(){callback(JobEvent::Frame(path.into()));}}
            else if event["kind"]=="log" && let Some(line)=event["payload"]["message"].as_str(){callback(JobEvent::Log(line.to_owned()));}
        })?;
            if let Some(warnings) = complete["payload"]["warnings"].as_array() {
                for warning in warnings {
                    if let Some(message) = warning.as_str() {
                        callback(JobEvent::Log(format!("Renderer: {message}")));
                    }
                }
            }
        }
        if hash(&mesh)? != recipe["mesh_hash"].as_str().unwrap_or("")
            || hash(&request.scene_path)? != recipe["scene_hash"].as_str().unwrap_or("")
        {
            return Err("job source changed during render".into());
        }
        write_json(
            &manifest,
            &json!({"recipe":recipe,"encoding_recipe":encoding_recipe,"stage":"encoding","destination":request.destination}),
        )?;
        callback(JobEvent::Progress {
            completed: count,
            total: count,
            stage: "Encoding and validating".into(),
        });
        let width = request.size
            * if request.layout == ExportLayout::SideBySide {
                2
            } else {
                1
            };
        match request.kind {
            ExportKind::Gif | ExportKind::SectionTurntable => {
                encode::encode_gif_dimensions_at_rate(
                    &frames,
                    &request.destination,
                    tools,
                    count,
                    width,
                    request.size,
                    request.frame_rate,
                    cancel,
                )
            }
            ExportKind::Mp4 => {
                let encoder = encode::video_encoder(tools, request.video_codec, cancel)?;
                callback(JobEvent::Log(format!(
                    "Encoding {:?} with {} ({})",
                    request.video_codec,
                    encoder,
                    if encoder.ends_with("_nvenc") {
                        "hardware"
                    } else {
                        "software"
                    }
                )));
                encode::encode_mp4_with_codec_at_rate(
                    &frames,
                    &request.destination,
                    tools,
                    count,
                    width,
                    request.size,
                    request.video_codec,
                    request.frame_rate,
                    cancel,
                )
            }
            ExportKind::Still | ExportKind::SectionStill => encode::publish_png(
                &frames.join("still.png"),
                &request.destination,
                width,
                request.size,
                cancel,
            ),
            ExportKind::Diagram => unreachable!(),
        }
    })();
    match &operation {
        Ok(output) => {
            write_json(
                &manifest,
                &json!({"recipe":recipe,"encoding_recipe":encoding_recipe,"stage":"complete","result":output}),
            )?;
            callback(JobEvent::Progress {
                completed: 1,
                total: 1,
                stage: "Complete".into(),
            });
        }
        Err(error) => {
            let _ = write_json(
                &manifest,
                &json!({"recipe":recipe,"encoding_recipe":encoding_recipe,"stage":if cancel.load(Ordering::Acquire){"cancelled"}else{"failed"},"error":error.to_string()}),
            );
        }
    }
    operation
}

fn render_section(
    worker: &mut WorkerClient,
    request: &ExportRequest,
    scene: &Value,
    directory: &Path,
    cancel: &AtomicBool,
    callback: &mut impl FnMut(JobEvent),
) -> Result<PathBuf> {
    let animated = matches!(
        request.kind,
        ExportKind::Gif | ExportKind::Mp4 | ExportKind::SectionTurntable
    );
    let count = if animated { request.frame_count } else { 1 };
    let paired = request.layout == ExportLayout::SideBySide;
    let bar = (request.size / 24).max(11).min(request.size / 4);
    let panel_height = request.size - if paired { bar } else { 0 };
    let panels: &[&str] = if paired {
        &["full", "section"]
    } else {
        &["section"]
    };
    for (panel_index, panel) in panels.iter().enumerate() {
        let mut spec = scene.clone();
        spec["section"] = if *panel == "section" {
            serde_json::to_value(&request.section)?
        } else {
            Value::Null
        };
        // Both scenes keep the complete source mesh bounds for their camera,
        // lighting and floor. Only the disposable section mesh is cut.
        let scene_path = directory.join(format!("{panel}-scene.json"));
        write_scene_json(&scene_path, &spec)?;
        let frames = directory.join(panel);
        let frame_states_path = request
            .timeline
            .as_ref()
            .map(|_| directory.join("frame-states.json"));
        let complete=worker.request(json!({"request_id":format!("section-{panel}"),"scene_revision":1,"kind":"render","scene_path":scene_path,"output_dir":frames,"mode":if animated{if request.animation==AnimationMode::BuildUp{"buildup"}else{"turntable"}}else{"still"},"engine":request.engine,"size":request.size,"width":request.size,"height":panel_height,"samples":request.samples,"frame_count":count,"frame_rate":request.frame_rate,"seconds_per_turn":request.turntable_timing.map(|timing| timing.seconds_per_turn),"build_stage":request.build_stage,"frame_states_path":frame_states_path,"start_angle_degrees":request.start_angle_degrees,"transparent":request.transparent&&!animated,"reuse_verified_frames":frames.join("render-manifest.json").exists()}),cancel,|event|{
            if event["kind"]=="frame"{
                let completed=panel_index as u32*count+event["payload"]["index"].as_u64().unwrap_or(0) as u32+1;
                callback(JobEvent::Progress{completed,total:count*panels.len() as u32,stage:format!("{} {panel} view",if event["payload"]["reuse_kind"]=="identical"{"Reusing identical frames for"}else if event["payload"]["reused"]==true{"Reusing verified frames for"}else{"Rendering"})});
            }else if event["kind"]=="log" && let Some(message)=event["payload"]["message"].as_str(){callback(JobEvent::Log(message.into()));}
        })?;
        if *panel == "section" {
            let analysis = complete["payload"]["section_analysis"].clone();
            if analysis.is_null() {
                return Err("renderer omitted section analysis".into());
            }
            write_json(&directory.join("analysis.json"), &analysis)?;
            if let Some(per_frame) = complete["payload"]["frame_analysis"].as_array() {
                write_json(&directory.join("frame-analysis.json"), &json!(per_frame))?;
            } else {
                let path = frames.join("frame-analysis.json");
                let per_frame: Value = serde_json::from_slice(&crate::settings::read_bounded(
                    &path,
                    16 * 1024 * 1024,
                )?)?;
                if per_frame
                    .as_array()
                    .is_none_or(|items| items.len() != count as usize)
                {
                    return Err("section frame analysis count differs from animation".into());
                }
                write_json(&directory.join("frame-analysis.json"), &per_frame)?;
            }
            callback(JobEvent::Analysis(analysis));
        }
    }
    if !paired {
        return Ok(directory.join("section"));
    }
    let combined = directory.join("frames");
    fs::create_dir_all(&combined)?;
    for index in 0..count {
        if cancel.load(Ordering::Acquire) {
            return Err("cancelled".into());
        }
        let name = if animated {
            format!("frame_{index:04}.png")
        } else {
            "still.png".into()
        };
        let mut sheet = image::RgbaImage::new(request.size * 2, request.size);
        for (panel_index, panel) in panels.iter().enumerate() {
            let path = directory.join(panel).join(&name);
            encode::validate_png(&path, request.size, panel_height)?;
            let pixels = image::open(path)?.to_rgba8();
            image::imageops::replace(
                &mut sheet,
                &pixels,
                (panel_index as u32 * request.size) as i64,
                0,
            );
        }
        // Render panels exclude the caption strip; labels never cover geometry.
        for y in request.size - bar..request.size {
            for x in 0..request.size * 2 {
                sheet[(x, y)] = image::Rgba([255, 255, 255, 255]);
            }
        }
        draw_label(&mut sheet, "FULL", 0, request.size - bar, request.size, bar);
        draw_label(
            &mut sheet,
            "SECTION",
            request.size,
            request.size - bar,
            request.size,
            bar,
        );
        let partial = combined.join(format!("{name}.partial.png"));
        sheet.save(&partial)?;
        encode::publish(&partial, &combined.join(&name))?;
        callback(JobEvent::Frame(combined.join(&name)));
    }
    Ok(combined)
}

fn preflight_storage(directory: &Path, estimated: u64) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let path: Vec<u16> = directory.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut available = 0u64;
        if unsafe {
            GetDiskFreeSpaceExW(
                path.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        if estimated > available {
            return Err(format!(
                "not enough disk space: estimated {} MiB of frames, {} MiB available",
                estimated / 1048576,
                available / 1048576
            )
            .into());
        }
    }
    #[cfg(not(windows))]
    let _ = (directory, estimated);
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub struct DiagramLayout {
    pub margin: u32,
    pub panel_width: u32,
    pub panel_height: u32,
    pub label_height: u32,
}
pub fn diagram_layout(size: u32) -> Result<DiagramLayout> {
    if !(256..=8000).contains(&size) {
        return Err("four-view size must be 256..8000".into());
    }
    let margin = size / 64;
    let label_height = size * 3 / 128;
    let panel_width = (size - 3 * margin) / 2;
    let panel_height = panel_width - label_height;
    Ok(DiagramLayout {
        margin,
        panel_width,
        panel_height,
        label_height,
    })
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: [f64; 3]) -> [f64; 3] {
    let d = dot(a, a).sqrt();
    a.map(|v| v / d)
}
pub fn diagram_cameras(dimensions: [f64; 3], width: u32, height: u32) -> [Value; 4] {
    // Row-major sheet: TOP / ISO, FRONT / RIGHT.
    let directions = [[0., 0., 1.], [1., -1., 1.], [0., -1., 0.], [1., 0., 0.]];
    let ups = [[0., 1., 0.], [0., 0., 1.], [0., 0., 1.], [0., 0., 1.]];
    let mut span = 0.0_f64;
    for i in 0..4 {
        let direction = unit(directions[i]);
        let right = unit(cross(ups[i], direction));
        let up = cross(direction, right);
        let projected_width = right
            .iter()
            .zip(dimensions)
            .map(|(a, b)| a.abs() * b)
            .sum::<f64>();
        let projected_height = up
            .iter()
            .zip(dimensions)
            .map(|(a, b)| a.abs() * b)
            .sum::<f64>();
        span = span
            .max(projected_height)
            .max(projected_width * height as f64 / width as f64);
    }
    // Reserve the same annotation clearance in every view while retaining shared scale.
    span = (span * 1.4).max(0.001);
    let distance = dimensions
        .iter()
        .map(|d| d * d)
        .sum::<f64>()
        .sqrt()
        .max(0.001)
        * 3.;
    std::array::from_fn(
        |i| json!({"target_mm":[0.,0.,dimensions[2]*0.5],"direction":directions[i],"up":ups[i],"projection":"orthographic","vertical_fov_degrees":35.,"ortho_span_mm":span,"distance_mm":distance}),
    )
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct DiagramDimension {
    pub min_mm: [f64; 2],
    pub max_mm: [f64; 2],
    /// 0 is horizontal; 1 is vertical in the view's camera plane.
    pub axis: usize,
    pub length_mm: f64,
}

/// Measure referenced vertices, not projected corners of the 3D bounding box.
/// Coordinates are relative to the full-source camera target, even for a partial build.
pub fn diagram_measurements(
    mesh: &crate::mesh::Mesh,
    cameras: &[Value; 4],
    visible_triangles: Option<&[bool]>,
) -> Result<[Option<DiagramDimension>; 4]> {
    let bounds = mesh.bounds()?;
    if visible_triangles.is_some_and(|mask| mask.len() != mesh.triangles.len()) {
        return Err("dimension visibility must match source triangles".into());
    }
    let center = std::array::from_fn::<_, 3, _>(|i| (bounds.min[i] + bounds.max[i]) * 0.5);
    let mut referenced = vec![false; mesh.positions.len()];
    for (index, triangle) in mesh.triangles.iter().enumerate() {
        if visible_triangles.is_none_or(|mask| mask[index]) {
            for &vertex in triangle {
                referenced[vertex as usize] = true;
            }
        }
    }
    let points: Vec<_> = mesh
        .positions
        .iter()
        .zip(referenced)
        .filter_map(|(point, used)| used.then_some(*point))
        .collect();
    diagram_point_measurements(&points, center, cameras)
}

/// Project the actual clipped solid, including triangle/plane intersections.
/// Retain the full source origin so labels align with the unchanged framing.
pub fn diagram_solid_measurements(
    mesh: &crate::mesh::Mesh,
    cameras: &[Value; 4],
    height_mm: f64,
) -> Result<[Option<DiagramDimension>; 4]> {
    let bounds = mesh.bounds()?;
    if !height_mm.is_finite() || height_mm < 0. {
        return Err("solid build height must be finite and nonnegative".into());
    }
    if height_mm <= bounds.min[2] {
        return Ok([None; 4]);
    }
    let mut points = Vec::new();
    for triangle in &mesh.triangles {
        for i in 0..3 {
            let a = mesh.positions[triangle[i] as usize];
            let b = mesh.positions[triangle[(i + 1) % 3] as usize];
            if a[2] <= height_mm {
                points.push(a);
            }
            if (a[2] < height_mm && b[2] > height_mm) || (b[2] < height_mm && a[2] > height_mm) {
                let t = (height_mm - a[2]) / (b[2] - a[2]);
                points.push([
                    a[0] + t * (b[0] - a[0]),
                    a[1] + t * (b[1] - a[1]),
                    height_mm,
                ]);
            }
        }
    }
    let center = std::array::from_fn(|i| (bounds.min[i] + bounds.max[i]) * 0.5);
    diagram_point_measurements(&points, center, cameras)
}

fn diagram_point_measurements(
    points: &[[f64; 3]],
    center: [f64; 3],
    cameras: &[Value; 4],
) -> Result<[Option<DiagramDimension>; 4]> {
    let mut output = [None; 4];
    for (index, camera) in cameras.iter().enumerate() {
        let direction: [f64; 3] = serde_json::from_value(camera["direction"].clone())?;
        let up: [f64; 3] = serde_json::from_value(camera["up"].clone())?;
        if direction.iter().chain(&up).any(|v| !v.is_finite())
            || dot(direction, direction) == 0.
            || dot(cross(up, direction), cross(up, direction)) == 0.
        {
            return Err("dimension camera needs finite independent direction/up vectors".into());
        }
        let direction = unit(direction);
        let right = unit(cross(up, direction));
        let up = cross(direction, right);
        let mut min_mm = [f64::INFINITY; 2];
        let mut max_mm = [f64::NEG_INFINITY; 2];
        for point in points {
            let offset = std::array::from_fn(|i| point[i] - center[i]);
            for (axis, value) in [dot(offset, right), dot(offset, up)]
                .into_iter()
                .enumerate()
            {
                min_mm[axis] = min_mm[axis].min(value);
                max_mm[axis] = max_mm[axis].max(value);
            }
        }
        if min_mm[0].is_finite() {
            let axis = usize::from(max_mm[1] - min_mm[1] > max_mm[0] - min_mm[0]);
            output[index] = Some(DiagramDimension {
                min_mm,
                max_mm,
                axis,
                length_mm: max_mm[axis] - min_mm[axis],
            });
        }
    }
    Ok(output)
}

fn diagram_stage_visibility(
    mesh: &crate::mesh::Mesh,
    scene: &Value,
    stage: Option<BuildStage>,
) -> Result<Option<Vec<bool>>> {
    let Some(stage) = stage else {
        return Ok(None);
    };
    let ranges = scene["layer_face_ranges"]
        .as_array()
        .ok_or("build-stage dimensions require layer faces")?;
    let layer_ids = ranges
        .iter()
        .map(|r| {
            r["layer_index"]
                .as_u64()
                .ok_or("invalid dimension layer index")
        })
        .collect::<std::result::Result<std::collections::BTreeSet<_>, _>>()?;
    if stage.visible_layers as usize > layer_ids.len() {
        return Err("build stage exceeds deposited layers".into());
    }
    let cutoff = stage
        .visible_layers
        .checked_sub(1)
        .and_then(|i| layer_ids.iter().nth(i as usize))
        .copied();
    let mut visible = vec![false; mesh.triangles.len()];
    for range in ranges {
        let start = range["start_triangle"]
            .as_u64()
            .ok_or("invalid dimension face start")? as usize;
        let end = range["end_triangle"]
            .as_u64()
            .ok_or("invalid dimension face end")? as usize;
        if start >= end || end > visible.len() {
            return Err("dimension layer range exceeds source mesh".into());
        }
        if cutoff.is_some_and(|last| range["layer_index"].as_u64().is_some_and(|id| id <= last)) {
            visible[start..end].fill(true);
        }
    }
    Ok(Some(visible))
}

fn diagram_panel_scene(scene: &Value, camera: &Value) -> Value {
    let mut panel = scene.clone();
    panel["display_rotation_degrees"] = json!([0., 0., 0.]);
    panel["camera"] = camera.clone();
    panel["studio_preset"] = json!("diagram");
    if !scene["diagram_printed_surface"].as_bool().unwrap_or(false) {
        panel["layer_appearance"] = json!(false);
        panel["material_preset"] = json!("clay");
        panel["color"] = json!([0.32, 0.35, 0.4]);
        panel["roughness"] = json!(0.6);
        panel["face_groups"] = json!([]);
    }
    panel
}

fn render_diagram(
    worker: &mut WorkerClient,
    request: &ExportRequest,
    scene: &Value,
    mesh: &Path,
    directory: &Path,
    cancel: &AtomicBool,
    callback: &mut impl FnMut(JobEvent),
) -> Result<ExportResult> {
    let expected_mesh_hash = hash(mesh)?;
    let layout = diagram_layout(request.size)?;
    let data = crate::mesh::Mesh::read_binary(mesh)?;
    let bounds = data.bounds()?;
    let dimensions = std::array::from_fn(|i| bounds.max[i] - bounds.min[i]);
    let cameras = diagram_cameras(dimensions, layout.panel_width, layout.panel_height);
    let measurements = if scene["geometry_kind"] == "original"
        && let Some(stage) = request.build_stage
    {
        let tops = scene["solid_layer_tops_mm"]
            .as_array()
            .ok_or("solid build stage requires layer heights")?;
        let count = stage.visible_layers as usize;
        if count > tops.len() {
            return Err("solid build stage exceeds available layers".into());
        }
        let height = if count == 0 {
            0.
        } else {
            tops[count - 1]
                .as_f64()
                .filter(|height| height.is_finite() && *height > 0.)
                .ok_or("invalid solid layer top")?
        };
        diagram_solid_measurements(&data, &cameras, height)?
    } else {
        let visibility = diagram_stage_visibility(&data, scene, request.build_stage)?;
        diagram_measurements(&data, &cameras, visibility.as_deref())?
    };
    let mut sheet = image::RgbaImage::from_pixel(
        request.size,
        request.size,
        image::Rgba([255, 255, 255, 255]),
    );
    let labels = ["TOP", "ISO", "FRONT", "RIGHT"];
    write_json(
        &directory.join("four-view-dimensions.json"),
        &json!({"units":"mm","views":labels,"measurements":measurements}),
    )?;
    for index in 0..4 {
        if cancel.load(Ordering::Acquire) {
            return Err("cancelled".into());
        }
        let panel_scene = diagram_panel_scene(scene, &cameras[index]);
        let scene_path = directory.join(format!("diagram-{index}.json"));
        write_scene_json(&scene_path, &panel_scene)?;
        let frames = directory.join(format!("panel-{index}"));
        callback(JobEvent::Progress {
            completed: index as u32,
            total: 4,
            stage: format!("Rendering {}", labels[index]),
        });
        worker.request(json!({"request_id":format!("diagram-{index}"),"scene_revision":1,"kind":"render","scene_path":scene_path,"output_dir":frames,"mode":"still","engine":request.engine,"size":request.size,"width":layout.panel_width,"height":layout.panel_height,"samples":request.samples,"frame_count":1,"build_stage":request.build_stage,"transparent":true,"reuse_verified_frames":frames.join("render-manifest.json").exists()}),cancel,|e|{if e["kind"]=="log" && let Some(s)=e["payload"]["message"].as_str(){callback(JobEvent::Log(s.into()));}})?;
        let path = frames.join("still.png");
        encode::validate_png(&path, layout.panel_width, layout.panel_height)?;
        let mut panel = image::open(&path)?.to_rgba8();
        // Alpha outlines are geometric silhouettes, independent of bumped normals.
        let alpha = panel.clone();
        for y in 1..panel.height() - 1 {
            for x in 1..panel.width() - 1 {
                if alpha[(x, y)][3] > 128
                    && [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]
                        .iter()
                        .any(|&(a, b)| alpha[(a, b)][3] < 64)
                {
                    panel[(x, y)] = image::Rgba([40, 45, 52, 255]);
                }
            }
        }
        let x = layout.margin + (index as u32 % 2) * (layout.panel_width + layout.margin);
        let y = layout.margin
            + (index as u32 / 2) * (layout.panel_height + layout.label_height + layout.margin);
        if let Some(dimension) = measurements[index] {
            draw_dimension(
                &mut panel,
                dimension,
                cameras[index]["ortho_span_mm"]
                    .as_f64()
                    .ok_or("diagram span missing")?,
            );
        }
        image::imageops::overlay(&mut sheet, &panel, x as i64, y as i64);
        draw_label(
            &mut sheet,
            labels[index],
            x,
            y + layout.panel_height,
            layout.panel_width,
            layout.label_height,
        );
        callback(JobEvent::Frame(path));
    }
    let assembled = directory.join("four-view.png");
    if hash(mesh)? != expected_mesh_hash {
        return Err("geometry changed during four-view rendering".into());
    }
    sheet.save(&assembled)?;
    encode::publish_png(
        &assembled,
        &request.destination,
        request.size,
        request.size,
        cancel,
    )
}

fn dimension_line(image: &mut image::RgbaImage, a: [i32; 2], b: [i32; 2]) {
    let steps = (b[0] - a[0]).abs().max((b[1] - a[1]).abs()).max(1);
    let thickness = (image.height() / 1000).max(1) as i32;
    for step in 0..=steps {
        let x = a[0] + ((b[0] - a[0]) as f64 * f64::from(step) / f64::from(steps)).round() as i32;
        let y = a[1] + ((b[1] - a[1]) as f64 * f64::from(step) / f64::from(steps)).round() as i32;
        for dy in 0..thickness {
            for dx in 0..thickness {
                if x + dx >= 0
                    && y + dy >= 0
                    && x + dx < image.width() as i32
                    && y + dy < image.height() as i32
                {
                    image[((x + dx) as u32, (y + dy) as u32)] = image::Rgba([35, 40, 48, 255]);
                }
            }
        }
    }
}

fn draw_dimension(panel: &mut image::RgbaImage, dimension: DiagramDimension, span_mm: f64) {
    let pixels_per_mm = panel.height() as f64 / span_mm;
    let left = (panel.width() as f64 * 0.5 + dimension.min_mm[0] * pixels_per_mm).round() as i32;
    let right = (panel.width() as f64 * 0.5 + dimension.max_mm[0] * pixels_per_mm).round() as i32;
    let top = (panel.height() as f64 * 0.5 - dimension.max_mm[1] * pixels_per_mm).round() as i32;
    let bottom = (panel.height() as f64 * 0.5 - dimension.min_mm[1] * pixels_per_mm).round() as i32;
    let value = dimension.length_mm;
    let text = if value >= 0.01 {
        format!("{value:.2} mm")
    } else if value >= 0.000001 {
        format!("{value:.6} mm")
    } else {
        format!("{value:.2e}").replace('e', "E") + " mm"
    };
    let scale = (panel.height() / 180).max(1);
    let mut label = image::RgbaImage::from_pixel(
        text.len() as u32 * 6 * scale,
        11 * scale,
        image::Rgba([255; 4]),
    );
    let (label_width, label_height) = label.dimensions();
    draw_label(&mut label, &text, 0, 0, label_width, label_height);
    let gap = (panel.height() / 180).max(2) as i32;
    let arrow = (panel.height() / 100).max(3) as i32;
    let (label, x, y) = if dimension.axis == 0 {
        let line_y = (bottom + gap + label_height as i32 + 2).min(panel.height() as i32 - 2);
        dimension_line(panel, [left, line_y], [right, line_y]);
        for (end, direction) in [(left, 1), (right, -1)] {
            dimension_line(panel, [end, bottom + gap], [end, line_y + arrow / 2]);
            for sign in [-1, 1] {
                dimension_line(
                    panel,
                    [end, line_y],
                    [end + direction * arrow, line_y + sign * arrow / 2],
                );
            }
        }
        (
            label,
            (left + right - label_width as i32) / 2,
            line_y - label_height as i32 - 2,
        )
    } else {
        let line_x = (right + gap + label_height as i32 + 2).min(panel.width() as i32 - 2);
        dimension_line(panel, [line_x, top], [line_x, bottom]);
        for (end, direction) in [(top, 1), (bottom, -1)] {
            dimension_line(panel, [right + gap, end], [line_x + arrow / 2, end]);
            for sign in [-1, 1] {
                dimension_line(
                    panel,
                    [line_x, end],
                    [line_x + sign * arrow / 2, end + direction * arrow],
                );
            }
        }
        (
            image::imageops::rotate270(&label),
            line_x - label_height as i32 - 2,
            (top + bottom - label_width as i32) / 2,
        )
    };
    let x = x.clamp(0, panel.width().saturating_sub(label.width()) as i32);
    let y = y.clamp(0, panel.height().saturating_sub(label.height()) as i32);
    image::imageops::overlay(panel, &label, i64::from(x), i64::from(y));
}

fn draw_label(image: &mut image::RgbaImage, text: &str, x: u32, y: u32, width: u32, height: u32) {
    let scale = (height / 11).max(1);
    let text_width = text.len() as u32 * 6 * scale;
    let left = x + (width.saturating_sub(text_width)) / 2;
    let top = y + (height.saturating_sub(7 * scale)) / 2;
    for (i, c) in text.chars().enumerate() {
        let rows: [u8; 7] = match c {
            'F' => [31, 16, 16, 30, 16, 16, 16],
            'U' => [17, 17, 17, 17, 17, 17, 14],
            'L' => [16, 16, 16, 16, 16, 16, 31],
            'R' => [30, 17, 17, 30, 20, 18, 17],
            'O' => [14, 17, 17, 17, 17, 17, 14],
            'N' => [17, 25, 25, 21, 19, 19, 17],
            'T' => [31, 4, 4, 4, 4, 4, 4],
            'I' => [14, 4, 4, 4, 4, 4, 14],
            'G' => [14, 17, 16, 23, 17, 17, 15],
            'H' => [17, 17, 17, 31, 17, 17, 17],
            'P' => [30, 17, 17, 30, 16, 16, 16],
            'S' => [15, 16, 16, 14, 1, 1, 30],
            'M' => [17, 27, 21, 21, 17, 17, 17],
            'E' => [31, 16, 16, 30, 16, 16, 31],
            'C' => [14, 17, 16, 16, 16, 17, 14],
            '0' => [14, 17, 19, 21, 25, 17, 14],
            '1' => [4, 12, 4, 4, 4, 4, 14],
            '2' => [14, 17, 1, 2, 4, 8, 31],
            '3' => [30, 1, 1, 14, 1, 1, 30],
            '4' => [2, 6, 10, 18, 31, 2, 2],
            '5' => [31, 16, 16, 30, 1, 1, 30],
            '6' => [14, 16, 16, 30, 17, 17, 14],
            '7' => [31, 1, 2, 4, 8, 8, 8],
            '8' => [14, 17, 17, 14, 17, 17, 14],
            '9' => [14, 17, 17, 15, 1, 1, 14],
            '.' => [0, 0, 0, 0, 0, 12, 12],
            'm' => [0, 0, 26, 21, 21, 21, 21],
            '-' => [0, 0, 0, 31, 0, 0, 0],
            '+' => [0, 4, 4, 31, 4, 4, 0],
            _ => [0; 7],
        };
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let px = left + i as u32 * 6 * scale + col * scale + dx;
                            let py = top + row as u32 * scale + dy;
                            if px < image.width() && py < image.height() {
                                image[(px, py)] = image::Rgba([35, 40, 48, 255]);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod dimension_tests {
    #[test]
    fn dimension_labels_render_digits_decimal_and_millimetres() {
        for character in "0123456789.m".chars() {
            let mut image = image::RgbaImage::from_pixel(40, 30, image::Rgba([255; 4]));
            super::draw_label(&mut image, &character.to_string(), 0, 0, 40, 30);
            assert!(
                image.pixels().any(|p| p[0] < 100),
                "missing dimension glyph {character}"
            );
        }
    }
}
