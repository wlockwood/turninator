//! External tools are requirements of an operation, not renderer availability.
use super::{BackendId, RenderSelection, ToolPaths};
use crate::{Result, export::ExportKind};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ToolRequirements {
    pub blender: bool,
    pub ffmpeg: bool,
    pub ffprobe: bool,
}

pub fn requirements_for(
    selection: &RenderSelection,
    kind: ExportKind,
    encode_only: bool,
) -> ToolRequirements {
    ToolRequirements {
        blender: !encode_only
            && matches!(
                selection.backend,
                BackendId::BlenderEevee | BackendId::BlenderCycles
            ),
        ffmpeg: matches!(
            kind,
            ExportKind::Gif | ExportKind::SectionTurntable | ExportKind::Mp4
        ),
        ffprobe: kind == ExportKind::Mp4,
    }
}

pub fn validate_requirements(tools: &ToolPaths, required: &ToolRequirements) -> Result<()> {
    for (needed, label, path) in [
        (required.blender, "Blender", &tools.blender),
        (required.ffmpeg, "FFmpeg", &tools.ffmpeg),
        (required.ffprobe, "ffprobe", &tools.ffprobe),
    ] {
        if needed && (!path.is_absolute() || !path.is_file()) {
            return Err(format!("{label} executable not found at {}", path.display()).into());
        }
    }
    Ok(())
}
