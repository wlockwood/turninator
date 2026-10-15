//! Rust-owned renderer boundary. Coordinates are build-space millimeters, right-handed Z up.
use super::{
    capabilities::FeatureSupport,
    selection::{BackendId, BackendQuality, RenderSelection},
};
use crate::{
    Result,
    export::{ExportRequest, SectionSpec},
    mesh::{Bounds, Mesh},
    viewport::OrbitCamera,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendIdentity {
    pub backend: BackendId,
    pub adapter_recipe_version: u32,
    pub runtime_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bridge_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shader_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material_digest: Option<String>,
    pub color_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub denoiser_digest: Option<String>,
    pub device: String,
    pub api: String,
    pub driver: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RenderProjection {
    Perspective { vertical_fov_degrees: f64 },
    Orthographic { vertical_span_mm: f64 },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderCamera {
    pub position_mm: [f64; 3],
    pub target_mm: [f64; 3],
    pub up: [f64; 3],
    pub projection: RenderProjection,
}
impl RenderCamera {
    /// The sole legacy camera-spec converter; timeline cameras use this same path.
    pub fn from_legacy(value: &Value) -> Result<Self> {
        #[derive(Deserialize)]
        struct Legacy {
            target_mm: [f64; 3],
            direction: [f64; 3],
            up: [f64; 3],
            projection: String,
            distance_mm: f64,
            vertical_fov_degrees: f64,
            ortho_span_mm: f64,
        }
        let c: Legacy = serde_json::from_value(value.clone())?;
        if c.target_mm
            .iter()
            .chain(&c.direction)
            .chain(&c.up)
            .any(|n| !n.is_finite())
            || !c.distance_mm.is_finite()
            || c.distance_mm < 0.000001
            || !c.ortho_span_mm.is_finite()
            || c.ortho_span_mm < 0.000001
            || !c.vertical_fov_degrees.is_finite()
            || !(1.0..=150.).contains(&c.vertical_fov_degrees)
        {
            return Err("invalid physical camera specification".into());
        }
        let norm = c.direction.iter().map(|n| n * n).sum::<f64>().sqrt();
        let cross = [
            c.direction[1] * c.up[2] - c.direction[2] * c.up[1],
            c.direction[2] * c.up[0] - c.direction[0] * c.up[2],
            c.direction[0] * c.up[1] - c.direction[1] * c.up[0],
        ];
        if !norm.is_finite() || norm == 0. || cross.iter().map(|n| n * n).sum::<f64>() < 1e-15 {
            return Err("camera up and direction must be independent".into());
        }
        let projection = match c.projection.as_str() {
            "perspective" => RenderProjection::Perspective {
                vertical_fov_degrees: c.vertical_fov_degrees,
            },
            "orthographic" => RenderProjection::Orthographic {
                vertical_span_mm: c.ortho_span_mm,
            },
            _ => return Err("unsupported camera projection".into()),
        };
        let up_norm = c.up.iter().map(|n| n * n).sum::<f64>().sqrt();
        if !up_norm.is_finite() || up_norm == 0. {
            return Err("invalid camera up vector".into());
        }
        let direction = c.direction.map(|n| n / norm);
        let up_unit = c.up.map(|n| n / up_norm);
        let dot = (0..3).map(|i| up_unit[i] * direction[i]).sum::<f64>();
        let up_plane: [f64; 3] = std::array::from_fn(|i| up_unit[i] - dot * direction[i]);
        let length = up_plane.iter().map(|n| n * n).sum::<f64>().sqrt();
        if !length.is_finite() || length < 1e-12 {
            return Err("camera axes are collinear".into());
        }
        let up = up_plane.map(|n| n / length);
        let camera = Self {
            position_mm: std::array::from_fn(|i| {
                c.target_mm[i] + c.direction[i] / norm * c.distance_mm
            }),
            target_mm: c.target_mm,
            up,
            projection,
        };
        if camera.position_mm.iter().any(|n| !n.is_finite()) {
            return Err("camera position overflow".into());
        }
        Ok(camera)
    }
    pub fn from_orbit(camera: &OrbitCamera) -> Result<Self> {
        camera.validate()?;
        Self::from_legacy(&camera.camera_spec())
    }
    /// Match the current viewport's physical near/far convention.
    pub fn clip_planes_mm(&self) -> (f64, f64) {
        let d = (0..3)
            .map(|i| (self.position_mm[i] - self.target_mm[i]).powi(2))
            .sum::<f64>()
            .sqrt();
        ((d * 0.0001).max(1e-5), d * 100. + 100.)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometryKind {
    Original,
    Beads,
    Stepped,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FaceGroup {
    pub start_triangle: u32,
    pub end_triangle: u32,
    pub tool: u32,
    pub label: String,
    pub color: [f32; 3],
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LayerRange {
    pub start_triangle: u32,
    pub end_triangle: u32,
    pub layer_index: u32,
    pub z_top_mm: f64,
}
#[derive(Clone, Debug)]
pub struct GeometryMetadata {
    pub kind: GeometryKind,
    pub bead_layout: Option<String>,
    pub bead_ring_vertices: Option<u32>,
    pub bead_side_face_ranges: Vec<[u32; 2]>,
    pub face_groups: Vec<FaceGroup>,
    pub layer_ranges: Vec<LayerRange>,
    pub solid_layer_tops_mm: Vec<f64>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RenderAppearance {
    pub color: [f32; 3],
    pub roughness: f32,
    pub material_preset: String,
    pub studio_preset: String,
    pub resin_clarity: f32,
    pub resin_cloudiness: f32,
    pub layer_appearance: bool,
    pub first_layer_height_mm: f64,
    pub layer_height_mm: f64,
    pub groove_depth_mm: f64,
    pub display_rotation_degrees: [f64; 3],
}
#[derive(Clone, Debug)]
pub struct RenderSceneSnapshot {
    pub mesh: Arc<Mesh>,
    pub legacy_scene: Value,
    pub appearance: RenderAppearance,
    pub metadata: GeometryMetadata,
    pub camera: RenderCamera,
    pub reference_bounds_mm: Bounds,
    pub warnings: Vec<String>,
}

/// Validate emitted model_scene metadata without rebuilding import settings or touching mesh coordinates.
pub fn snapshot_scene(mesh: Arc<Mesh>, legacy_scene: Value) -> Result<RenderSceneSnapshot> {
    if serde_json::to_vec(&legacy_scene)?.len() > 1024 * 1024 {
        return Err("render scene exceeds 1 MiB metadata budget".into());
    }
    if legacy_scene["schema_version"] != 1 {
        return Err("unsupported scene version".into());
    }
    let reference_bounds_mm = mesh.bounds()?;
    if legacy_scene["mesh_path"]
        .as_str()
        .is_none_or(|s| !Path::new(s).is_absolute())
    {
        return Err("render mesh path must be absolute".into());
    }
    if legacy_scene
        .get("source_path")
        .is_some_and(|v| !v.is_string())
    {
        return Err("source path must be a string".into());
    }
    // Apply only the legacy renderer defaults to a disposable appearance view.
    // The original JSON remains byte-semantically unchanged for recipe/provenance.
    let mut appearance_wire = legacy_scene.clone();
    let layer_height = legacy_scene["layer_height_mm"]
        .as_f64()
        .ok_or("layer_height_mm required")?;
    let first = legacy_scene
        .get("first_layer_height_mm")
        .map_or(Some(layer_height), Value::as_f64)
        .ok_or("invalid first_layer_height_mm")?;
    let defaults = serde_json::json!({
        "layer_appearance":false,
        "first_layer_height_mm":first,
        "groove_depth_mm":(0.05*layer_height).min(0.25*first.min(layer_height)),
        "material_preset":"matte_pla", "studio_preset":"light",
        "resin_clarity":0.8,"resin_cloudiness":0.1
    });
    for (key, value) in defaults.as_object().unwrap() {
        appearance_wire
            .as_object_mut()
            .ok_or("scene must be object")?
            .entry(key)
            .or_insert_with(|| value.clone());
    }
    let appearance: RenderAppearance = serde_json::from_value(appearance_wire)?;
    if appearance
        .color
        .iter()
        .chain(
            [
                appearance.roughness,
                appearance.resin_clarity,
                appearance.resin_cloudiness,
            ]
            .iter(),
        )
        .any(|n| !n.is_finite() || !(0.0..=1.0).contains(n))
        || !["matte_pla", "silk_pla", "glossy_plastic", "clay", "resin"]
            .contains(&appearance.material_preset.as_str())
        || !["light", "dark", "diagram"].contains(&appearance.studio_preset.as_str())
        || appearance
            .display_rotation_degrees
            .iter()
            .any(|n| !n.is_finite())
        || [appearance.layer_height_mm, appearance.first_layer_height_mm]
            .iter()
            .any(|n| !n.is_finite() || !(0.02..=2.).contains(n))
        || !appearance.groove_depth_mm.is_finite()
        || !(0.0..=0.25
            * appearance
                .layer_height_mm
                .min(appearance.first_layer_height_mm))
            .contains(&appearance.groove_depth_mm)
    {
        return Err("invalid render appearance".into());
    }
    let kind: GeometryKind = serde_json::from_value(legacy_scene["geometry_kind"].clone())?;
    if appearance.layer_appearance && kind != GeometryKind::Original {
        return Err("layer appearance requires original geometry".into());
    }
    if legacy_scene
        .get("layer_filtering")
        .is_some_and(|v| !v.is_boolean())
    {
        return Err("layer filtering must be boolean".into());
    }
    fn field<T: serde::de::DeserializeOwned + Default>(v: &Value, name: &str) -> Result<T> {
        match v.get(name) {
            Some(value) => Ok(serde_json::from_value(value.clone())?),
            None => Ok(T::default()),
        }
    }
    let metadata = GeometryMetadata {
        kind,
        bead_layout: field(&legacy_scene, "bead_layout")?,
        bead_ring_vertices: field(&legacy_scene, "bead_ring_vertices")?,
        bead_side_face_ranges: field(&legacy_scene, "bead_side_face_ranges")?,
        face_groups: field(&legacy_scene, "face_groups")?,
        layer_ranges: field(&legacy_scene, "layer_face_ranges")?,
        solid_layer_tops_mm: field(&legacy_scene, "solid_layer_tops_mm")?,
    };
    if let Some(layout) = metadata.bead_layout.as_deref() {
        if kind != GeometryKind::Beads
            || ![
                "independent_stadium_v1",
                "joined_stadium_v1",
                "joined_stadium_v2",
                "joined_stadium_v3",
            ]
            .contains(&layout)
        {
            return Err("unsupported bead layout".into());
        }
        if matches!(layout, "joined_stadium_v2" | "joined_stadium_v3")
            && metadata.bead_ring_vertices.is_none_or(|n| {
                !([8, 12, 16, 24].contains(&n) || layout == "joined_stadium_v3" && n == 32)
            })
        {
            return Err("unsupported bead ring size".into());
        }
        if layout.starts_with("joined_stadium") && metadata.bead_side_face_ranges.is_empty() {
            return Err("joined bead layout requires side ranges".into());
        }
    }
    let count = mesh.triangles.len() as u32;
    fn ranges(values: impl Iterator<Item = (u32, u32)>, count: u32) -> Result<()> {
        let mut previous = 0;
        for (start, end) in values {
            if start < previous || end <= start || end > count {
                return Err(
                    "face ranges must be ordered, disjoint and within triangle count".into(),
                );
            }
            previous = end;
        }
        Ok(())
    }
    ranges(
        metadata.bead_side_face_ranges.iter().map(|r| (r[0], r[1])),
        count,
    )?;
    ranges(
        metadata
            .face_groups
            .iter()
            .map(|r| (r.start_triangle, r.end_triangle)),
        count,
    )?;

    if metadata.face_groups.len() > 100_000
        || metadata.face_groups.iter().any(|r| {
            r.tool > 255
                || r.label.chars().count() > 256
                || r.color
                    .iter()
                    .any(|c| !c.is_finite() || !(0.0..=1.).contains(c))
        })
    {
        return Err("invalid face-group material metadata".into());
    }
    let mut material_keys = std::collections::BTreeSet::new();
    for group in &metadata.face_groups {
        material_keys.insert((group.tool, group.color.map(f32::to_bits)));
    }
    if material_keys.len() > 256 {
        return Err("material mapping exceeds 256 unique tool/color combinations".into());
    }
    if (metadata.bead_ring_vertices.is_some() || !metadata.bead_side_face_ranges.is_empty())
        && kind != GeometryKind::Beads
    {
        return Err("bead metadata requires bead geometry".into());
    }
    if let Some(layout) = metadata.bead_layout.as_deref()
        && matches!(layout, "joined_stadium_v1" | "joined_stadium_v2")
    {
        let ring = metadata.bead_ring_vertices.unwrap_or(32);
        if layout == "joined_stadium_v1" && ring != 32
            || metadata
                .bead_side_face_ranges
                .iter()
                .any(|r| (r[1] - r[0]) % (ring * 2) != 0)
        {
            return Err("joined side ranges must contain complete bead rings".into());
        }
    }
    if legacy_scene.get("layer_face_ranges").is_some() && metadata.layer_ranges.is_empty()
        || metadata.layer_ranges.len() > 100_000
    {
        return Err("layer ranges must be a bounded nonempty list".into());
    }
    let mut previous = None;
    let mut previous_end = 0;
    for layer in &metadata.layer_ranges {
        if layer.start_triangle != previous_end
            || layer.end_triangle < layer.start_triangle
            || layer.end_triangle > count
            || layer.start_triangle == layer.end_triangle && kind != GeometryKind::Stepped
            || layer.layer_index > 1_000_000
            || !layer.z_top_mm.is_finite()
            || previous.is_some_and(|(i, z)| {
                layer.layer_index < i
                    || layer.z_top_mm < z
                    || layer.layer_index == i && layer.z_top_mm != z
            })
        {
            return Err("layer ranges must cover every face in increasing build order".into());
        }
        previous = Some((layer.layer_index, layer.z_top_mm));
        previous_end = layer.end_triangle;
    }
    if !metadata.layer_ranges.is_empty() && previous_end != count {
        return Err("layer ranges must cover every face".into());
    }
    if !metadata.solid_layer_tops_mm.is_empty() && kind != GeometryKind::Original {
        return Err("solid layer tops require original geometry".into());
    }
    if legacy_scene.get("solid_layer_tops_mm").is_some() && metadata.solid_layer_tops_mm.is_empty()
    {
        return Err("solid layer tops must be nonempty".into());
    }
    if metadata.solid_layer_tops_mm.len() > 50_000
        || metadata
            .solid_layer_tops_mm
            .iter()
            .any(|n| !n.is_finite() || *n <= 0.)
        || metadata
            .solid_layer_tops_mm
            .windows(2)
            .any(|w| w[0] >= w[1])
    {
        return Err("invalid solid layer tops".into());
    }
    let warnings: Vec<String> = serde_json::from_value(legacy_scene["warnings"].clone())?;
    let camera = if let Some(value) = legacy_scene.get("camera").filter(|v| !v.is_null()) {
        RenderCamera::from_legacy(value)?
    } else {
        let mut orbit = OrbitCamera::default();
        orbit.fit(&reference_bounds_mm);
        RenderCamera::from_orbit(&orbit)?
    };
    if let Some(section) = legacy_scene.get("section").filter(|v| !v.is_null()) {
        let s: SectionSpec = serde_json::from_value(section.clone())?;
        if s.axis > 2 || !s.position_mm.is_finite() {
            return Err("invalid section specification".into());
        }
    }
    Ok(RenderSceneSnapshot {
        mesh,
        legacy_scene,
        appearance,
        metadata,
        camera,
        reference_bounds_mm,
        warnings,
    })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RenderPanel {
    Single,
    Full,
    Section,
    Top,
    Iso,
    Front,
    Right,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FrameKey {
    pub index: u32,
    pub panel: RenderPanel,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrameSpec {
    pub key: FrameKey,
    pub camera: RenderCamera,
    pub angle_degrees: f64,
    pub visible_layers: Option<u32>,
    pub section: Option<SectionSpec>,
    pub width: u32,
    pub height: u32,
    pub time_seconds: f64,
}
#[derive(Clone, Debug, Default)]
pub struct FramePlan {
    pub frames: Vec<FrameSpec>,
    pub repeat_of: BTreeMap<FrameKey, FrameKey>,
    /// Whether completed producer pixels may contain nonopaque alpha.
    pub transparent: bool,
}
#[derive(Clone, Debug)]
pub struct SectionAnalysis {
    pub legacy: Value,
    pub cap_supported: bool,
}
impl SectionAnalysis {
    pub fn from_legacy(legacy: Value, cap_supported: bool) -> Result<Self> {
        let measurements = [
            "cross_section_area_mm2",
            "original_volume_mm3",
            "remaining_volume_mm3",
        ];
        // The historical empty-stage producer has no volume_basis: no geometry
        // was analyzed. Accept its exact shape without adding provenance fields.
        let empty_stage = legacy.get("volume_basis").is_none()
            && legacy["output_triangles"].as_u64() == Some(0)
            && legacy["visible_source_triangles"].as_u64() == Some(0)
            && legacy["visible_layer"]
                .as_i64()
                .is_some_and(|layer| (-1..=1_000_000).contains(&layer))
            && legacy["capped"] == false
            && legacy["closed_input"] == false
            && measurements
                .iter()
                .all(|key| legacy[key].as_f64() == Some(0.))
            && legacy["warnings"].as_array().is_some_and(Vec::is_empty)
            && legacy["meaning"] == "Empty build stage: no deposited geometry is visible.";
        if legacy["axis"].as_u64().is_none_or(|n| n > 2)
            || legacy["position_mm"]
                .as_f64()
                .is_none_or(|n| !n.is_finite())
            || ["keep_positive", "capped", "closed_input"]
                .iter()
                .any(|k| !legacy[k].is_boolean())
            || legacy["output_triangles"].as_u64().is_none()
            || !legacy["meaning"].is_string()
            || !legacy["volume_basis"].is_string() && !empty_stage
        {
            return Err("invalid legacy section analysis".into());
        }
        for key in measurements {
            let value = legacy.get(key).ok_or("missing section measurement")?;
            if !value.is_null() && value.as_f64().is_none_or(|n| !n.is_finite() || n < 0.) {
                return Err("invalid section measurement".into());
            }
        }
        let _: Vec<String> = serde_json::from_value(legacy["warnings"].clone())?;
        Ok(Self {
            legacy,
            cap_supported,
        })
    }
}
#[derive(Clone, Debug)]
pub struct RealizedScene {
    pub mesh: Arc<Mesh>,
    pub normals: Vec<[f32; 3]>,
    pub build_positions_mm: Vec<[f32; 3]>,
    pub triangle_materials: Vec<u32>,
    pub design_edges_mm: Vec<[[f32; 3]; 2]>,
    pub reference_bounds_mm: Bounds,
    pub analysis: Option<SectionAnalysis>,
}
/// Existing STL layer-reveal measurements. Empty/full stages have no cutting
/// plane or cross-section measurements; preserve that distinction and JSON.
#[derive(Clone, Debug)]
pub struct BuildAnalysis {
    pub legacy: Value,
}
impl BuildAnalysis {
    pub fn from_legacy(legacy: Value) -> Result<Self> {
        if ["capped", "closed_input"]
            .iter()
            .any(|key| !legacy[key].is_boolean())
            || legacy["output_triangles"].as_u64().is_none()
            || legacy["visible_layer"]
                .as_i64()
                .is_none_or(|layer| !(-1..50_000).contains(&layer))
            || legacy["z_top_mm"]
                .as_f64()
                .is_none_or(|z| !z.is_finite() || z < 0.)
        {
            return Err("invalid legacy build analysis".into());
        }
        let volume = legacy
            .get("remaining_volume_mm3")
            .ok_or("missing build volume measurement")?;
        if !volume.is_null() && volume.as_f64().is_none_or(|n| !n.is_finite() || n < 0.) {
            return Err("invalid build volume measurement".into());
        }
        if legacy["visible_layer"] == -1
            && (legacy["z_top_mm"] != 0.
                || legacy["output_triangles"] != 0
                || legacy["remaining_volume_mm3"] != 0.
                || legacy["capped"] != false
                || legacy["closed_input"] != false)
        {
            return Err("invalid empty build analysis".into());
        }
        // Intermediate solid stages carry the actual apply_section record;
        // validate its extra fields without adding them to empty/full stages.
        if legacy.get("axis").is_some() {
            SectionAnalysis::from_legacy(legacy.clone(), legacy["closed_input"] == true)?;
        }
        Ok(Self { legacy })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PhysicalMaterial {
    pub name: String,
    pub base_color_linear: [f32; 3],
    pub absorption_color_linear: [f32; 3],
    pub roughness: f32,
    pub metallic: f32,
    pub anisotropy: f32,
    pub coat: f32,
    pub coat_roughness: f32,
    pub transmission: f32,
    pub ior: f32,
    pub absorption_density_per_mm: f32,
    pub scattering_per_mm: f32,
    pub scattering_anisotropy: f32,
}
#[derive(Clone, Debug, Default)]
pub struct MaterialSet {
    pub materials: Vec<PhysicalMaterial>,
    pub triangle_materials: Vec<u32>,
}
#[derive(Clone, Debug)]
pub struct RgbaFrame {
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug)]
pub enum FrameArtifact {
    /// Durable per-frame measurements, emitted before the corresponding pixels.
    /// This is metadata only: it does not complete or count a rendered frame.
    Metadata {
        key: FrameKey,
        section: Option<SectionAnalysis>,
        build: Option<BuildAnalysis>,
    },
    Image {
        key: FrameKey,
        image: RgbaFrame,
    },
    Png {
        key: FrameKey,
        path: PathBuf,
    },
}
pub struct RenderContext<'a> {
    pub cancel: &'a AtomicBool,
    pub limits: &'a RenderLimits,
    pub scratch_dir: &'a Path,
    pub selection: &'a RenderSelection,
    pub identity: &'a BackendIdentity,
    pub pixel_recipe_key: &'a str,
    pub job_id: u64,
    pub revision: u64,
    pub progress: &'a dyn Fn(RenderProgress),
}
#[derive(Clone, Debug)]
pub struct RenderProgress {
    pub key: Option<FrameKey>,
    pub completed_work: u64,
    pub total_work: Option<u64>,
    pub stage: String,
    pub preview: Option<RgbaFrame>,
}
#[derive(Clone, Debug)]
pub struct RenderReport {
    pub frame_count: u32,
    pub warnings: Vec<String>,
    pub identity: BackendIdentity,
    pub quality: BackendQuality,
    pub analyses: BTreeMap<FrameKey, SectionAnalysis>,
    pub build_analyses: BTreeMap<FrameKey, BuildAnalysis>,
    pub phase_timings: BTreeMap<String, Duration>,
}
#[derive(Clone, Debug)]
pub struct RenderLimits {
    pub max_source_vertices: usize,
    pub max_source_triangles: usize,
    pub max_scene_bytes: u64,
    pub max_timeline_bytes: u64,
    pub max_frames: u32,
    pub max_output_side: u32,
    pub host_bytes: u64,
    pub gpu_bytes: u64,
    pub scratch_bytes: u64,
}
impl Default for RenderLimits {
    fn default() -> Self {
        Self {
            max_source_vertices: crate::mesh::MAX_VERTICES,
            max_source_triangles: crate::mesh::MAX_TRIANGLES,
            max_scene_bytes: 1024 * 1024,
            max_timeline_bytes: 8 * 1024 * 1024,
            max_frames: 10_000,
            max_output_side: 8000,
            host_bytes: 2 * 1024 * 1024 * 1024,
            gpu_bytes: 1024 * 1024 * 1024,
            scratch_bytes: 20 * 1024 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum RowOrigin {
    Top,
    Bottom,
}
#[derive(Clone, Copy, Debug)]
pub enum PixelChannels {
    Rgba,
    Bgra,
}
#[derive(Clone, Copy, Debug)]
pub enum PixelAlpha {
    Straight,
    Premultiplied,
}
#[derive(Clone, Copy, Debug)]
pub enum PixelTransfer {
    Linear,
    Srgb,
}
#[derive(Clone, Debug)]
pub struct NativePixels {
    pub width: u32,
    pub height: u32,
    pub row_stride: usize,
    pub bytes: Vec<u8>,
    pub origin: RowOrigin,
    pub channels: PixelChannels,
    pub alpha: PixelAlpha,
    pub transfer: PixelTransfer,
}
/// Construct and use on the owning actor. Native SDK handles have no Send requirement.
pub trait RenderBackend {
    fn id(&self) -> BackendId;
    fn assess(&self, scene: &RenderSceneSnapshot, request: &ExportRequest) -> Vec<FeatureSupport>;
    fn identity(&self) -> BackendIdentity;
    /// Rebind job settings without rebuilding an owner-thread runtime context.
    /// Device and quality changes require preparation again.
    fn configure(&mut self, request: &ExportRequest) -> Result<()> {
        request.selection.validate()?;
        let identity = self.identity();
        if request.selection.backend != self.id()
            || request
                .selection
                .device
                .as_ref()
                .is_some_and(|d| d != &identity.device)
        {
            return Err("request differs from prepared renderer identity".into());
        }
        Ok(())
    }
    fn render_sequence(
        &mut self,
        scene: &RenderSceneSnapshot,
        frames: &FramePlan,
        context: &RenderContext<'_>,
        emit: &mut dyn FnMut(FrameArtifact) -> Result<()>,
    ) -> Result<RenderReport>;
}
