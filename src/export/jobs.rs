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
    pub transparent: bool,
    #[serde(default)]
    pub video_codec: VideoCodec,
    #[serde(default)]
    pub layout: ExportLayout,
    #[serde(default)]
    pub section: Option<SectionSpec>,
    #[serde(default)]
    pub animation: AnimationMode,
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
            transparent: false,
            video_codec: VideoCodec::H264,
            layout: ExportLayout::Single,
            section: None,
            animation: AnimationMode::Turntable,
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

pub fn run_export(
    worker: &mut WorkerClient,
    tools: &ToolPaths,
    request: &ExportRequest,
    cancel: &AtomicBool,
    mut callback: impl FnMut(JobEvent),
) -> Result<ExportResult> {
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
        && (request.section.is_some() || !matches!(request.kind, ExportKind::Gif | ExportKind::Mp4))
    {
        return Err(
            "build-up supports GIF/MP4 bead animations and cannot be combined with sections".into(),
        );
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
    let recipe = json!({"schema_version":2,"scene_hash":hash(&request.scene_path)?,"mesh_hash":hash(&mesh)?,"engine":request.engine,"size":request.size,"samples":request.samples,"frame_count":rendered_count,"transparent":request.transparent&&!animated,"section":request.section,"layout":request.layout,"animation":request.animation,"kind":if animated{"animation"}else if request.kind==ExportKind::Diagram{"diagram"}else{"still"},"worker_hash":hash(&crate::render::worker::renderer_script("worker.py"))?,"scene_script_hash":hash(&crate::render::worker::renderer_script("scene.py"))?,"blender_hash":hash(&tools.blender)?});
    let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&recipe)?));
    // Encoding changes must be recorded without invalidating compatible PNGs.
    let encoding_recipe = match request.kind {
        ExportKind::Gif | ExportKind::SectionTurntable => json!({
            "format":"gif","ffmpeg_hash":hash(&tools.ffmpeg)?,
            "options":encode::gif_encoding_recipe()
        }),
        ExportKind::Mp4 => json!({
            "format":"mp4","codec":request.video_codec,"fps":20,
            "ffmpeg_hash":hash(&tools.ffmpeg)?,"ffprobe_hash":hash(&tools.ffprobe)?
        }),
        _ => json!({"format":"png"}),
    };
    let render_dir = request.job_dir.join(&fingerprint[..20]);
    fs::create_dir_all(&render_dir)?;
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
                stage: "Rendering".into(),
            });
            let complete=worker.request(json!({"request_id":format!("export-{}",&fingerprint[..20]),"scene_revision":1,"kind":"render","scene_path":request.scene_path,"output_dir":frames,"mode":if animation{if request.animation==AnimationMode::BuildUp{"buildup"}else{"turntable"}}else{"still"},"engine":request.engine,"size":request.size,"samples":request.samples,"frame_count":count,"start_angle_degrees":0,"transparent":request.transparent&&!animation,"reuse_verified_frames":frames.join("render-manifest.json").exists()}),cancel,|event|{
            if event["kind"]=="frame"{let n=event["payload"]["index"].as_u64().unwrap_or(0) as u32+1;callback(JobEvent::Progress{completed:n,total:count,stage:if event["payload"]["reused"]==true{"Reusing verified frames"}else{"Rendering"}.into()});if let Some(path)=event["payload"]["path"].as_str(){callback(JobEvent::Frame(path.into()));}}
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
            ExportKind::Gif | ExportKind::SectionTurntable => encode::encode_gif_dimensions(
                &frames,
                &request.destination,
                tools,
                count,
                width,
                request.size,
                cancel,
            ),
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
                encode::encode_mp4_with_codec(
                    &frames,
                    &request.destination,
                    tools,
                    count,
                    width,
                    request.size,
                    request.video_codec,
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
        write_json(&scene_path, &spec)?;
        let frames = directory.join(panel);
        let complete=worker.request(json!({"request_id":format!("section-{panel}"),"scene_revision":1,"kind":"render","scene_path":scene_path,"output_dir":frames,"mode":if animated{"turntable"}else{"still"},"engine":request.engine,"size":request.size,"width":request.size,"height":panel_height,"samples":request.samples,"frame_count":count,"start_angle_degrees":0,"transparent":request.transparent&&!animated,"reuse_verified_frames":frames.join("render-manifest.json").exists()}),cancel,|event|{
            if event["kind"]=="frame"{
                let completed=panel_index as u32*count+event["payload"]["index"].as_u64().unwrap_or(0) as u32+1;
                callback(JobEvent::Progress{completed,total:count*panels.len() as u32,stage:format!("Rendering {panel} view")});
            }else if event["kind"]=="log" && let Some(message)=event["payload"]["message"].as_str(){callback(JobEvent::Log(message.into()));}
        })?;
        if *panel == "section" {
            let analysis = complete["payload"]["section_analysis"].clone();
            if analysis.is_null() {
                return Err("renderer omitted section analysis".into());
            }
            write_json(&directory.join("analysis.json"), &analysis)?;
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
    let directions = [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.], [1., -1., 1.]];
    let ups = [[0., 0., 1.], [0., 0., 1.], [0., 1., 0.], [0., 0., 1.]];
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
    span = (span * 1.1).max(0.001);
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
    let mut sheet = image::RgbaImage::from_pixel(
        request.size,
        request.size,
        image::Rgba([255, 255, 255, 255]),
    );
    let labels = ["FRONT", "RIGHT", "TOP", "ISOMETRIC"];
    for index in 0..4 {
        if cancel.load(Ordering::Acquire) {
            return Err("cancelled".into());
        }
        let mut panel_scene = scene.clone();
        panel_scene["display_rotation_degrees"] = json!([0., 0., 0.]);
        panel_scene["camera"] = cameras[index].clone();
        panel_scene["studio_preset"] = json!("diagram");
        if !scene["diagram_printed_surface"].as_bool().unwrap_or(false) {
            panel_scene["layer_appearance"] = json!(false);
            panel_scene["material_preset"] = json!("clay");
            panel_scene["color"] = json!([0.32, 0.35, 0.4]);
            panel_scene["roughness"] = json!(0.6);
        }
        let scene_path = directory.join(format!("diagram-{index}.json"));
        write_json(&scene_path, &panel_scene)?;
        let frames = directory.join(format!("panel-{index}"));
        callback(JobEvent::Progress {
            completed: index as u32,
            total: 4,
            stage: format!("Rendering {}", labels[index]),
        });
        worker.request(json!({"request_id":format!("diagram-{index}"),"scene_revision":1,"kind":"render","scene_path":scene_path,"output_dir":frames,"mode":"still","engine":request.engine,"size":request.size,"width":layout.panel_width,"height":layout.panel_height,"samples":request.samples,"frame_count":1,"transparent":true,"reuse_verified_frames":frames.join("render-manifest.json").exists()}),cancel,|e|{if e["kind"]=="log" && let Some(s)=e["payload"]["message"].as_str(){callback(JobEvent::Log(s.into()));}})?;
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
