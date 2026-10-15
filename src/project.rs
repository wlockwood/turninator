use crate::export::{AnimationMode, ExportLayout, SectionSpec, VideoCodec};
use crate::render::{RenderSelection, RendererPreferences};
use crate::{Result, presentation::SceneSettings, settings::atomic_write, viewport::OrbitCamera};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "Value")]
pub struct ProjectFile {
    pub schema_version: u32,
    pub source_path: PathBuf,
    pub source_hash: String,
    pub scene: SceneSettings,
    pub camera: OrbitCamera,
    pub export_size: u32,
    pub renderers: RendererPreferences,
    #[serde(default)]
    pub output_options: OutputPreferences,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputPreferences {
    pub output_format: crate::export::OutputFormat,
    pub still_size: u32,
    pub frame_rate: u32,
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
            output_format: Default::default(),
            still_size: 4096,
            frame_rate: crate::animation::DEFAULT_FRAME_RATE,
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
// Versioned wire readers keep legacy renderer fields out of the canonical model.
#[derive(Deserialize)]
struct ProjectBody {
    source_path: PathBuf,
    source_hash: String,
    scene: SceneSettings,
    camera: OrbitCamera,
    export_size: u32,
    #[serde(default)]
    output_options: OutputPreferences,
}
#[derive(Deserialize)]
#[serde(default)]
struct LegacyOutputRenderers {
    still_engine: String,
    samples: u32,
    still_samples: u32,
}
impl Default for LegacyOutputRenderers {
    fn default() -> Self {
        Self {
            still_engine: "cycles".into(),
            samples: 32,
            still_samples: 64,
        }
    }
}
impl TryFrom<Value> for ProjectFile {
    type Error = String;
    fn try_from(value: Value) -> std::result::Result<Self, String> {
        let version = value["schema_version"]
            .as_u64()
            .ok_or("project schema_version required")?;
        let renderers = match version {
            1 => {
                let legacy: LegacyOutputRenderers = serde_json::from_value(
                    value
                        .get("output_options")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({})),
                )
                .map_err(|e| e.to_string())?;
                let animation = RenderSelection::from_legacy(
                    value["engine"]
                        .as_str()
                        .ok_or("legacy project engine required")?,
                    legacy.samples,
                )
                .map_err(|e| e.to_string())?;
                let still =
                    RenderSelection::from_legacy(&legacy.still_engine, legacy.still_samples)
                        .map_err(|e| e.to_string())?;
                let mut prefs = RendererPreferences {
                    animation,
                    still,
                    ..Default::default()
                };
                // Store migrated qualities in their corresponding saved profiles too.
                if let Some(profile) = prefs.profiles.get_mut(&prefs.animation.backend) {
                    profile.animation = prefs.animation.quality.clone();
                }
                if let Some(profile) = prefs.profiles.get_mut(&prefs.still.backend) {
                    profile.still = prefs.still.quality.clone();
                }
                prefs
            }
            2 => {
                if value.get("engine").is_some()
                    || ["still_engine", "samples", "still_samples"]
                        .iter()
                        .any(|key| value["output_options"].get(key).is_some())
                {
                    return Err(
                        "schema-2 project uses renderers, not legacy renderer fields".into(),
                    );
                }
                serde_json::from_value(value["renderers"].clone()).map_err(|e| e.to_string())?
            }
            _ => return Err(format!("unsupported project version: {version}")),
        };
        renderers.validate().map_err(|e| e.to_string())?;
        let b: ProjectBody = serde_json::from_value(value).map_err(|e| e.to_string())?;
        Ok(Self {
            schema_version: 2,
            source_path: b.source_path,
            source_hash: b.source_hash,
            scene: b.scene,
            camera: b.camera,
            export_size: b.export_size,
            renderers,
            output_options: b.output_options,
        })
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
    crate::animation::validate_frame_rate(output.frame_rate)?;
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
        || output
            .section
            .as_ref()
            .is_some_and(|s| s.axis > 2 || !s.position_mm.is_finite() || s.position_mm.abs() > 1e7)
    {
        return Err("invalid saved output preferences".into());
    }
    project.renderers.validate()?;
    if project.schema_version != 2 {
        return Err("unsupported project version".into());
    }
    if !(16..=8000).contains(&project.export_size)
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
