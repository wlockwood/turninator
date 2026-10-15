//! Optional in-process renderer boundary. Loading is lazy and never runs tools.
#[cfg(all(windows, target_pointer_width = "64"))]
mod abi;
mod loader;
use super::{
    contract::{
        BackendIdentity, GeometryKind, PhysicalMaterial, RenderAppearance, RenderCamera,
        RenderLimits,
    },
    selection::{BackendId, BackendQuality, RenderSelection},
};
pub use loader::{EmbeddedContext, EmbeddedLibrary};
pub struct EmbeddedConfig {
    pub selection: RenderSelection,
    pub limits: RenderLimits,
    pub identity: BackendIdentity,
    /// Existing application-owned cache root. The host resolves and validates
    /// the directory but cannot establish its ownership. Storage is not pixel identity.
    pub cache_directory: Option<std::path::PathBuf>,
}
pub struct SceneUpload<'a> {
    pub revision: u64,
    pub positions_mm: &'a [[f32; 3]],
    pub normals: &'a [[f32; 3]],
    pub build_positions_mm: &'a [[f32; 3]],
    pub triangles: &'a [[u32; 3]],
    pub triangle_materials: &'a [u32],
    pub metadata: &'a EmbeddedSceneMetadata,
}
pub struct EmbeddedSceneMetadata {
    pub appearance: RenderAppearance,
    pub materials: Vec<PhysicalMaterial>,
    pub reference_bounds_mm: crate::mesh::Bounds,
    pub geometry_kind: GeometryKind,
    pub layer_filter: EmbeddedLayerFilter,
    pub color: EmbeddedColorConfig,
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct EmbeddedLayerFilter {
    pub enabled: bool,
    pub fade_start_pixels: f64,
    pub fade_end_pixels: f64,
    pub footprint_diagonal_factor: f64,
}
impl EmbeddedLayerFilter {
    pub fn standard(enabled: bool) -> Self {
        Self {
            enabled,
            fade_start_pixels: 2.,
            fade_end_pixels: 4.,
            footprint_diagonal_factor: std::f64::consts::SQRT_2,
        }
    }
}
#[derive(Clone, Debug)]
pub struct EmbeddedColorConfig {
    pub exposure_ev: f64,
    pub tone_map: String,
}
impl EmbeddedColorConfig {
    pub fn for_backend(backend: BackendId) -> crate::Result<Self> {
        let tone_map = match backend {
            BackendId::Filament => "filament-agx-v1",
            BackendId::LuxCore => "turninator-agx-v1",
            _ => return Err("backend does not use the native SDK color config".into()),
        };
        Ok(Self {
            exposure_ev: 0.,
            tone_map: tone_map.into(),
        })
    }
}
pub struct EmbeddedFrame {
    pub geometry_revision: u64,
    pub width: u32,
    pub height: u32,
    pub camera: RenderCamera,
    /// Row-major affine matrix; build millimeters to world millimeters.
    pub model_to_world: [f64; 16],
    pub quality: BackendQuality,
    pub transparent: bool,
    pub sample_seed: u32,
}
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeInfo {
    pub schema_version: u32,
    pub backend: String,
    pub sdk_version: String,
    pub device_id: String,
    pub device_name: String,
    pub api: String,
    pub driver: String,
    pub compiler: Option<RuntimeCompilerInfo>,
}
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCompilerInfo {
    pub name: String,
    pub version: String,
    pub sha256: String,
    pub path: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbeddedState {
    Idle,
    Rendering,
    Ready,
    Cancelling,
    Cancelled,
    Failed,
}
#[derive(Debug)]
pub struct EmbeddedPoll {
    pub state: EmbeddedState,
    pub completed_work: u64,
    pub total_work: Option<u64>,
}
