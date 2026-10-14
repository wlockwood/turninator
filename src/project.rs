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
    atomic_write(path, &serde_json::to_vec_pretty(&portable)?)
}
pub fn load_project(path: &Path) -> Result<(ProjectFile, SourceStatus)> {
    if fs::metadata(path)?.len() > 1024 * 1024 {
        return Err("project exceeds 1 MiB budget".into());
    }
    let mut project: ProjectFile = serde_json::from_slice(&fs::read(path)?)?;
    validate_project(&project)?;
    if !project.source_path.is_absolute() {
        project.source_path = path
            .parent()
            .unwrap_or(Path::new("."))
            .join(&project.source_path);
    }
    let status = if !project.source_path.is_file() {
        SourceStatus::Missing
    } else if format!("{:x}", Sha256::digest(fs::read(&project.source_path)?))
        == project.source_hash
    {
        SourceStatus::Current
    } else {
        SourceStatus::Changed
    };
    Ok((project, status))
}

fn validate_project(project: &ProjectFile) -> Result<()> {
    project.scene.validate()?;
    project.camera.validate()?;
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
