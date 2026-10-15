use crate::export::{AnimationMode, ExportLayout, SectionSpec, VideoCodec};
use crate::{Result, presentation::SceneSettings, settings::atomic_write, viewport::OrbitCamera};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectFile {
    pub schema_version: u32,
    pub source_path: PathBuf,
    pub source_hash: String,
    pub scene: SceneSettings,
    pub camera: OrbitCamera,
    pub export_size: u32,
    pub engine: String,
    #[serde(default)]
    pub output_options: OutputPreferences,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputPreferences {
    pub still_size: u32,
    pub still_engine: String,
    pub still_samples: u32,
    pub samples: u32,
    pub transparent: bool,
    pub video_codec: VideoCodec,
    pub animation: AnimationMode,
    pub section: Option<SectionSpec>,
    pub layout: ExportLayout,
    pub timeline: Option<crate::animation::Timeline>,
    pub turntable_timing: crate::animation::TurntableTiming,
    pub build_stage: Option<crate::export::BuildStage>,
    pub start_angle_degrees: f64,
}
impl Default for OutputPreferences {
    fn default() -> Self {
        Self {
            still_size: 4096,
            still_engine: "cycles".into(),
            still_samples: 64,
            samples: 32,
            transparent: false,
            video_codec: VideoCodec::H264,
            animation: AnimationMode::Turntable,
            section: None,
            layout: ExportLayout::Single,
            timeline: None,
            turntable_timing: Default::default(),
            build_stage: None,
            start_angle_degrees: 0.,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceStatus {
    Current,
    Missing,
    Changed,
}

pub fn save_project(path: &Path, project: &ProjectFile) -> Result<()> {
    validate_project(project)?;
    if path.exists() && fs::canonicalize(path).ok() == fs::canonicalize(&project.source_path).ok() {
        return Err("project save cannot replace the source model".into());
    }
    let mut portable = project.clone();
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    if let Ok(base) = fs::canonicalize(parent)
        && let Ok(source) = fs::canonicalize(&portable.source_path)
    {
        portable.source_path = relative_source(&source, &base).unwrap_or(source);
    }
    let bytes = serde_json::to_vec_pretty(&portable)?;
    if bytes.len() > 1024 * 1024 {
        return Err(
            "project exceeds 1 MiB reload budget; reduce explicit boundaries before saving".into(),
        );
    }
    atomic_write(path, &bytes)
}
pub fn load_project(path: &Path) -> Result<(ProjectFile, SourceStatus)> {
    if fs::metadata(path)?.len() > 1024 * 1024 {
        return Err("project exceeds 1 MiB budget".into());
    }
    let mut project: ProjectFile =
        serde_json::from_slice(&crate::settings::read_bounded(path, 1024 * 1024)?)?;
    validate_project(&project)?;
    if !project.source_path.is_absolute() {
        project.source_path = path
            .parent()
            .unwrap_or(Path::new("."))
            .join(&project.source_path);
    }
    let status = if !project.source_path.is_file() {
        SourceStatus::Missing
    } else if file_hash(&project.source_path)? == project.source_hash {
        SourceStatus::Current
    } else {
        SourceStatus::Changed
    };
    Ok((project, status))
}
fn file_hash(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn validate_project(project: &ProjectFile) -> Result<()> {
    project.scene.validate()?;
    project.camera.validate()?;
    let output = &project.output_options;
    output.turntable_timing.validate()?;
    if let Some(timeline) = &output.timeline {
        timeline.validate()?;
    }
    if !output.start_angle_degrees.is_finite()
        || output.start_angle_degrees.abs() > 1e9
        || output
            .build_stage
            .is_some_and(|stage| stage.visible_layers > 50_000)
    {
        return Err("invalid saved build stage or sampled angle".into());
    }
    if !(64..=8000).contains(&output.still_size)
        || !(1..=4096).contains(&output.samples)
        || !(1..=4096).contains(&output.still_samples)
        || !["eevee", "cycles"].contains(&output.still_engine.as_str())
        || output
            .section
            .as_ref()
            .is_some_and(|s| s.axis > 2 || !s.position_mm.is_finite() || s.position_mm.abs() > 1e7)
    {
        return Err("invalid saved output preferences".into());
    }
    if project.schema_version != 1 {
        return Err("unsupported project version".into());
    }
    if !(16..=8000).contains(&project.export_size)
        || !["eevee", "cycles"].contains(&project.engine.as_str())
        || project.source_hash.len() != 64
        || !project.source_hash.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err("invalid project output settings or source hash".into());
    }
    Ok(())
}

fn relative_source(source: &Path, base: &Path) -> Option<PathBuf> {
    let source_parts: Vec<_> = source.components().collect();
    let base_parts: Vec<_> = base.components().collect();
    let shared = source_parts
        .iter()
        .zip(&base_parts)
        .take_while(|(a, b)| a == b)
        .count();
    if shared == 0 {
        return None;
    }
    let mut path = PathBuf::new();
    for _ in shared..base_parts.len() {
        path.push("..");
    }
    for part in &source_parts[shared..] {
        path.push(part.as_os_str());
    }
    Some(path)
}
