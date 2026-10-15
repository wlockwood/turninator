use crate::{
    Result,
    mesh::Mesh,
    model::stl::load_stl_with_report,
    print::{
        beads::{BeadSegment, build_bead_mesh, build_joined_bead_mesh_with_report},
        gcode::{GcodeDialect, ParseOptions, parse_gcode},
        schedule::LayerSchedule,
        slice::slice_stepped_with_report,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, sync::Arc};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportSettings {
    pub millimeters_per_unit: f64,
    pub layer_height_mm: f64,
    pub first_layer_height_mm: f64,
    pub filament_diameter_mm: f64,
    pub build_rotation_degrees: [f64; 3],
    pub joined_beads: bool,
    pub firmware: String,
    pub allow_arcs: bool,
    pub allow_tool_changes: bool,
    pub chord_error_mm: f64,
    pub homing_position_mm: Option<[f64; 3]>,
    pub layer_boundaries_mm: Vec<f64>,
    pub stepped_envelope: bool,
}
impl Default for ImportSettings {
    fn default() -> Self {
        Self {
            millimeters_per_unit: 1.,
            layer_height_mm: 0.2,
            first_layer_height_mm: 0.2,
            filament_diameter_mm: 1.75,
            build_rotation_degrees: [0.; 3],
            joined_beads: true,
            firmware: "marlin".into(),
            allow_arcs: false,
            allow_tool_changes: false,
            chord_error_mm: 0.02,
            homing_position_mm: None,
            layer_boundaries_mm: Vec::new(),
            stepped_envelope: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneSettings {
    pub import: ImportSettings,
    pub layer_appearance: bool,
    pub groove_depth_mm: f64,
    pub display_rotation_degrees: [f64; 3],
    pub color: [f32; 3],
    pub roughness: f32,
    pub material_preset: String,
    pub studio_preset: String,
    pub resin_clarity: f32,
    pub resin_cloudiness: f32,
    pub print_settings_edited: bool,
    pub color_mode: String,
    pub tool_colors: Vec<[f32; 3]>,
}
impl Default for SceneSettings {
    fn default() -> Self {
        Self {
            import: ImportSettings::default(),
            layer_appearance: true,
            groove_depth_mm: 0.01,
            display_rotation_degrees: [0.; 3],
            color: [0.035, 0.22, 0.52],
            roughness: 0.42,
            material_preset: "matte_pla".into(),
            studio_preset: "light".into(),
            resin_clarity: 0.85,
            resin_cloudiness: 0.1,
            print_settings_edited: false,
            color_mode: "uniform".into(),
            tool_colors: vec![
                [0.035, 0.22, 0.52],
                [0.85, 0.16, 0.04],
                [0.03, 0.55, 0.19],
                [0.55, 0.10, 0.7],
            ],
        }
    }
}
impl SceneSettings {
    pub fn select_material(&mut self, material: &str) {
        self.material_preset = material.into();
        self.roughness = match material {
            "silk_pla" => 0.24,
            "glossy_plastic" => 0.18,
            "clay" => 0.8,
            "resin" => 0.12,
            _ => 0.55,
        };
        if !self.print_settings_edited {
            self.reset_print_settings();
        }
    }
    pub fn reset_print_settings(&mut self) {
        self.import.layer_boundaries_mm.clear();
        let h = if self.material_preset == "resin" {
            0.05
        } else {
            0.2
        };
        self.import.layer_height_mm = h;
        self.import.first_layer_height_mm = h;
        self.groove_depth_mm = 0.05 * h;
        self.layer_appearance = !matches!(self.material_preset.as_str(), "clay");
        self.print_settings_edited = false;
    }
    pub fn validate(&self) -> Result<()> {
        self.import.validate()?;
        if !["uniform", "tool", "feature"].contains(&self.color_mode.as_str())
            || self.tool_colors.is_empty()
            || self.tool_colors.len() > 16
            || self
                .tool_colors
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err("invalid print color mapping".into());
        }
        if self
            .color
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || [self.roughness, self.resin_clarity, self.resin_cloudiness]
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || self.display_rotation_degrees.iter().any(|v| !v.is_finite())
            || !self.groove_depth_mm.is_finite()
            || self.groove_depth_mm < 0.
            || self.groove_depth_mm
                > 0.25
                    * self
                        .import
                        .layer_height_mm
                        .min(self.import.first_layer_height_mm)
        {
            return Err("appearance settings must be finite and within supported ranges".into());
        }
        if !["matte_pla", "silk_pla", "glossy_plastic", "clay", "resin"]
            .contains(&self.material_preset.as_str())
            || !["light", "dark"].contains(&self.studio_preset.as_str())
        {
            return Err("unsupported material or studio".into());
        }
        Ok(())
    }
}
impl ImportSettings {
    pub fn validate(&self) -> Result<()> {
        if !["marlin", "klipper"].contains(&self.firmware.as_str())
            || !self.chord_error_mm.is_finite()
            || !(0.001..=1.).contains(&self.chord_error_mm)
            || self
                .homing_position_mm
                .is_some_and(|p| p.iter().any(|v| !v.is_finite() || v.abs() > 1e7))
        {
            return Err("invalid firmware, chord error, or homing coordinates".into());
        }
        if !self.layer_boundaries_mm.is_empty() {
            LayerSchedule::explicit(self.layer_boundaries_mm.clone())?;
        }
        LayerSchedule::uniform(
            self.layer_height_mm,
            self.first_layer_height_mm,
            self.first_layer_height_mm,
        )?;
        if !self.millimeters_per_unit.is_finite()
            || self.millimeters_per_unit <= 0.
            || !self.filament_diameter_mm.is_finite()
            || !(0.1..=10.).contains(&self.filament_diameter_mm)
            || self.build_rotation_degrees.iter().any(|v| !v.is_finite())
        {
            return Err("invalid units, filament diameter, or build orientation".into());
        }
        Ok(())
    }
}
#[derive(Default)]
pub struct RevisionGate {
    generation: u64,
}
impl RevisionGate {
    pub fn advance(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }
    pub fn accepts(&self, generation: u64) -> bool {
        generation == self.generation
    }
    pub fn current(&self) -> u64 {
        self.generation
    }
}
pub struct ImportedModel {
    pub path: std::path::PathBuf,
    pub mesh: Arc<Mesh>,
    pub picking: Arc<crate::viewport::PickingMesh>,
    pub geometry_kind: String,
    pub source_hash: String,
    pub warnings: Vec<String>,
    pub segment_count: usize,
    pub bead_layout: Option<String>,
    pub bead_side_face_ranges: Vec<[usize; 2]>,
    pub groups: Vec<GeometryGroup>,
    pub import_settings: ImportSettings,
    pub bounds_mm: crate::mesh::Bounds,
    pub layer_tops_mm: Vec<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GeometryGroup {
    pub start_triangle: usize,
    pub end_triangle: usize,
    pub tool: u32,
    pub feature: String,
    pub layer_index: u32,
    pub z_top_mm: f64,
}
pub fn make_build_mesh(mesh: &Mesh, rotation: [f64; 3]) -> Result<Mesh> {
    mesh.bounds()?;
    if rotation.iter().any(|v| !v.is_finite()) {
        return Err("build orientation must be finite".into());
    }
    let angles = rotation.map(f64::to_radians);
    let mut result = mesh.clone();
    for point in &mut result.positions {
        let [x, y, z] = *point;
        let (s, c) = angles[0].sin_cos();
        let (y, z) = (c * y - s * z, s * y + c * z);
        let (s, c) = angles[1].sin_cos();
        let (x, z) = (c * x + s * z, -s * x + c * z);
        let (s, c) = angles[2].sin_cos();
        *point = [c * x - s * y, s * x + c * y, z];
    }
    let minimum = result.bounds()?.min;
    for point in &mut result.positions {
        for axis in 0..3 {
            point[axis] -= minimum[axis];
        }
    }
    result.bounds()?;
    Ok(result)
}
pub fn load_model(path: &Path, options: &ImportSettings) -> Result<ImportedModel> {
    options.validate()?;
    if fs::metadata(path)?.len() > 128 * 1024 * 1024 {
        return Err("input exceeds 128 MiB budget".into());
    }
    let source = crate::settings::read_bounded(path, 128 * 1024 * 1024)?;
    let source_hash = format!("{:x}", Sha256::digest(&source));
    let extension = path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut bead_layout = None;
    let mut bead_side_face_ranges = Vec::new();
    let mut groups = Vec::new();
    let mut layer_tops_mm = Vec::new();
    let (mesh, geometry_kind, mut warnings, segment_count) = match extension.as_str() {
        "stl" => {
            if !options.stepped_envelope && !options.layer_boundaries_mm.is_empty() {
                return Err("Explicit adaptive boundaries require G-code or experimental stepped STL; Layer Appearance uses first/later uniform heights.".into());
            }
            let imported = load_stl_with_report(path, options.millimeters_per_unit)?;
            let mut warnings = vec![format!(
                "STL units assumed: {} mm per source unit; source Z-up unless build orientation is changed.",
                options.millimeters_per_unit
            )];
            if imported.omitted_degenerate_triangles > 0 {
                warnings.push(format!(
                    "Omitted {} exactly zero-area faces; no tolerance-based repair.",
                    imported.omitted_degenerate_triangles
                ));
            }
            let mut mesh = make_build_mesh(&imported.mesh, options.build_rotation_degrees)?;
            if options.stepped_envelope {
                let schedule = if options.layer_boundaries_mm.is_empty() {
                    LayerSchedule::uniform(
                        options.layer_height_mm,
                        options.first_layer_height_mm,
                        mesh.bounds()?.max[2],
                    )?
                } else {
                    LayerSchedule::explicit(options.layer_boundaries_mm.clone())?
                };
                layer_tops_mm = schedule
                    .clipped(mesh.bounds()?.max[2])?
                    .boundaries_mm
                    .into_iter()
                    .skip(1)
                    .collect();
                let envelope = slice_stepped_with_report(&mesh, &schedule.boundaries_mm)?;
                mesh = envelope.mesh;
                warnings.extend(envelope.warnings);
                warnings.push("Experimental mid-layer stepped solid: this changes the silhouette and does not model nozzle paths, infill or physical print defects.".into());
            }
            (mesh, "original", warnings, 0)
        }
        "gcode" | "gco" => {
            if options.stepped_envelope || options.millimeters_per_unit != 1. {
                return Err("G-code uses its own physical coordinates; STL unit scaling and stepped-envelope mode do not apply".into());
            }
            if options
                .build_rotation_degrees
                .iter()
                .any(|v| v.abs() > 1e-9)
            {
                return Err(
                    "G-code print orientation comes from the file; use presentation rotation"
                        .into(),
                );
            }
            let parsed = parse_gcode(
                std::str::from_utf8(&source)?,
                &ParseOptions {
                    layer_height_mm: options.layer_height_mm,
                    first_layer_height_mm: options.first_layer_height_mm,
                    filament_diameter_mm: options.filament_diameter_mm,
                    dialect: if options.firmware == "klipper" {
                        GcodeDialect::Klipper
                    } else {
                        GcodeDialect::Marlin
                    },
                    allow_arcs: options.allow_arcs,
                    allow_tool_changes: options.allow_tool_changes,
                    chord_error_mm: options.chord_error_mm,
                    homing_position_mm: options.homing_position_mm,
                    layer_schedule: if options.layer_boundaries_mm.is_empty() {
                        None
                    } else {
                        Some(LayerSchedule::explicit(
                            options.layer_boundaries_mm.clone(),
                        )?)
                    },
                },
            )?;
            let count = parsed.segments.len();
            for segment in &parsed.segments {
                if layer_tops_mm.last() != Some(&segment.end[2]) {
                    layer_tops_mm.push(segment.end[2]);
                }
            }
            let mesh = if options.joined_beads {
                let joined = build_joined_bead_mesh_with_report(&parsed.segments)?;
                bead_layout = Some("joined_stadium_v1".into());
                let mut begin = 0;
                for (i, range) in joined.smooth_face_ranges.iter().enumerate() {
                    let segment = &parsed.segments[begin];
                    let mut end = begin + 1;
                    while end < count
                        && compatible_segments(&parsed.segments[end - 1], &parsed.segments[end])
                    {
                        end += 1;
                    }
                    groups.push(group(
                        segment,
                        range[0],
                        joined
                            .smooth_face_ranges
                            .get(i + 1)
                            .map_or(joined.mesh.triangles.len(), |r| r[0]),
                    ));
                    begin = end;
                }
                bead_side_face_ranges = joined.smooth_face_ranges;
                joined.mesh
            } else {
                bead_layout = Some("independent_stadium_v1".into());
                let mut start = 0;
                for segment in &parsed.segments {
                    let end = start
                        + if segment.width_mm == segment.height_mm {
                            96
                        } else {
                            104
                        };
                    groups.push(group(segment, start, end));
                    start = end;
                }
                build_bead_mesh(&parsed.segments)?
            };
            (mesh, "beads", parsed.warnings, count)
        }
        _ => return Err("open an STL or plain G-code file".into()),
    };
    if Sha256::digest(crate::settings::read_bounded(path, 128 * 1024 * 1024)?)
        != Sha256::digest(&source)
    {
        return Err("source changed during import; reload the file".into());
    }
    if geometry_kind == "original" && !options.stepped_envelope {
        warnings.push("Layer Appearance preserves the original silhouette and is not a slicer toolpath prediction.".into());
    }
    let bounds_mm = mesh.bounds()?;
    let mut picking = crate::viewport::PickingMesh::new(&mesh)?;
    if !groups.is_empty() {
        let mut tops = vec![0.; mesh.triangles.len()];
        for group in &groups {
            tops[group.start_triangle..group.end_triangle].fill(group.z_top_mm as f32);
        }
        picking.set_triangle_layer_tops(&tops)?;
    }
    Ok(ImportedModel {
        path: fs::canonicalize(path)?,
        mesh: Arc::new(mesh),
        picking: Arc::new(picking),
        geometry_kind: geometry_kind.into(),
        source_hash,
        warnings,
        segment_count,
        bead_layout,
        bead_side_face_ranges,
        groups,
        import_settings: options.clone(),
        bounds_mm,
        layer_tops_mm,
    })
}
pub fn visible_layer_height(model: &ImportedModel, requested_mm: f64) -> f64 {
    if model.layer_tops_mm.is_empty() {
        requested_mm
    } else {
        let count = visible_layer_count(model, requested_mm);
        count.checked_sub(1).map_or(0., |i| model.layer_tops_mm[i])
    }
}
/// Heights use the same f32 representation as the native build-height slider and GPU.
pub fn visible_layer_count(model: &ImportedModel, requested_mm: f64) -> usize {
    model
        .layer_tops_mm
        .partition_point(|height| *height as f32 <= requested_mm as f32)
}
fn compatible_segments(a: &BeadSegment, b: &BeadSegment) -> bool {
    a.end == b.start && a.tool == b.tool && a.feature == b.feature && a.layer_index == b.layer_index
}
fn group(segment: &BeadSegment, start_triangle: usize, end_triangle: usize) -> GeometryGroup {
    GeometryGroup {
        start_triangle,
        end_triangle,
        tool: segment.tool,
        feature: segment.feature.clone(),
        layer_index: segment.layer_index,
        z_top_mm: segment.end[2],
    }
}
pub fn group_color_index(group: &GeometryGroup, settings: &SceneSettings) -> Option<usize> {
    if settings.color_mode == "uniform" {
        return None;
    }
    let index = if settings.color_mode == "feature" {
        group.feature.bytes().fold(0usize, |sum, b| {
            sum.wrapping_mul(31).wrapping_add(b as usize)
        })
    } else {
        group.tool as usize
    };
    Some(index % settings.tool_colors.len())
}
pub fn group_color(group: &GeometryGroup, settings: &SceneSettings) -> [f32; 3] {
    group_color_index(group, settings).map_or(settings.color, |index| settings.tool_colors[index])
}
pub fn model_scene(
    model: &ImportedModel,
    settings: &SceneSettings,
    path: &Path,
    camera: Option<serde_json::Value>,
) -> Result<serde_json::Value> {
    if settings.import != model.import_settings {
        return Err("Import settings differ from the accepted geometry; finish a successful reimport before exporting.".into());
    }
    let mut scene = render_scene(
        settings,
        path,
        &model.geometry_kind,
        &model.warnings,
        camera,
    )?;
    scene["source_path"] = serde_json::json!(model.path);
    if let Some(layout) = &model.bead_layout {
        scene["bead_layout"] = serde_json::json!(layout);
    }
    if !model.bead_side_face_ranges.is_empty() {
        scene["bead_side_face_ranges"] = serde_json::json!(model.bead_side_face_ranges);
    }
    if settings.import.stepped_envelope {
        scene["layer_appearance"] = serde_json::json!(false);
    }
    if !model.groups.is_empty() {
        if settings.color_mode != "uniform" {
            scene["face_groups"] = serde_json::Value::Array(model.groups.iter().map(|g| serde_json::json!({"start_triangle":g.start_triangle,"end_triangle":g.end_triangle,"tool":g.tool,"label":g.feature,"color":group_color(g,settings)})).collect());
        }
        let mut layers: Vec<serde_json::Value> = Vec::new();
        for group in &model.groups {
            if let Some(last) = layers.last_mut().filter(|last| {
                last["layer_index"] == group.layer_index && last["z_top_mm"] == group.z_top_mm
            }) {
                last["end_triangle"] = serde_json::json!(group.end_triangle);
            } else {
                layers.push(serde_json::json!({"start_triangle":group.start_triangle,"end_triangle":group.end_triangle,"layer_index":group.layer_index,"z_top_mm":group.z_top_mm}));
            }
        }
        scene["layer_face_ranges"] = serde_json::Value::Array(layers);
    }
    if serde_json::to_vec(&scene)?.len() > 1024 * 1024 {
        return Err(
            "Render scene exceeds 1 MiB metadata budget; use uniform colors or a smaller toolpath"
                .into(),
        );
    }
    Ok(scene)
}
pub fn render_scene(
    settings: &SceneSettings,
    path: &Path,
    geometry_kind: &str,
    warnings: &[String],
    camera: Option<serde_json::Value>,
) -> Result<serde_json::Value> {
    settings.validate()?;
    if !path.is_absolute() {
        return Err("render mesh path must be absolute".into());
    }
    if !["original", "beads"].contains(&geometry_kind) {
        return Err("unsupported geometry kind".into());
    }
    let mut result = serde_json::json!({"schema_version":1,"mesh_path":path,"geometry_kind":geometry_kind,
        "layer_height_mm":settings.import.layer_height_mm,"first_layer_height_mm":settings.import.first_layer_height_mm,
        "layer_appearance":geometry_kind=="original" && settings.layer_appearance,"groove_depth_mm":settings.groove_depth_mm,
        "bead_layout":if geometry_kind=="beads" {Some("independent_stadium_v1")} else {None},
        "color":settings.color,"roughness":settings.roughness,"display_rotation_degrees":settings.display_rotation_degrees,
        "material_preset":settings.material_preset,"studio_preset":settings.studio_preset,
        "resin_clarity":settings.resin_clarity,"resin_cloudiness":settings.resin_cloudiness,"warnings":warnings});
    if let Some(camera) = camera {
        result["camera"] = camera;
    }
    Ok(result)
}
