use crate::{
    Result,
    mesh::Mesh,
    model::stl::load_stl_with_report,
    print::{
        beads::build_bead_mesh,
        gcode::{ParseOptions, parse_gcode},
        schedule::LayerSchedule,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, sync::Arc};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportSettings {
    pub millimeters_per_unit: f64,
    pub layer_height_mm: f64,
    pub first_layer_height_mm: f64,
    pub filament_diameter_mm: f64,
    pub build_rotation_degrees: [f64; 3],
}
impl Default for ImportSettings {
    fn default() -> Self {
        Self {
            millimeters_per_unit: 1.,
            layer_height_mm: 0.2,
            first_layer_height_mm: 0.2,
            filament_diameter_mm: 1.75,
            build_rotation_degrees: [0.; 3],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    pub geometry_kind: String,
    pub source_hash: String,
    pub warnings: Vec<String>,
    pub segment_count: usize,
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
    let source = fs::read(path)?;
    let source_hash = format!("{:x}", Sha256::digest(&source));
    let extension = path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let (mesh, geometry_kind, mut warnings, segment_count) = match extension.as_str() {
        "stl" => {
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
            (
                make_build_mesh(&imported.mesh, options.build_rotation_degrees)?,
                "original",
                warnings,
                0,
            )
        }
        "gcode" => {
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
                    ..Default::default()
                },
            )?;
            let count = parsed.segments.len();
            (
                build_bead_mesh(&parsed.segments)?,
                "beads",
                parsed.warnings,
                count,
            )
        }
        _ => return Err("open an STL or plain G-code file".into()),
    };
    if Sha256::digest(fs::read(path)?) != Sha256::digest(&source) {
        return Err("source changed during import; reload the file".into());
    }
    if geometry_kind == "original" {
        warnings.push("Layer Appearance preserves the original silhouette and is not a slicer toolpath prediction.".into());
    }
    Ok(ImportedModel {
        path: fs::canonicalize(path)?,
        mesh: Arc::new(mesh),
        geometry_kind: geometry_kind.into(),
        source_hash,
        warnings,
        segment_count,
    })
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
