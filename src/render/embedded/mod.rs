//! Optional in-process renderer boundary. Loading is lazy and never runs tools.
#[cfg(all(windows, target_pointer_width = "64"))]
mod abi;
mod loader;
pub mod backend;
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
    /// Observational worker snapshot, excluded from stable/pixel identity.
    #[serde(default, deserialize_with = "optional_diagnostics")]
    pub diagnostics: Option<RuntimeDiagnostics>,
}
fn optional_diagnostics<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<RuntimeDiagnostics>, D::Error> {
    <RuntimeDiagnostics as serde::Deserialize>::deserialize(deserializer).map(Some)
}
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(try_from = "DiagnosticsDocument")]
pub struct RuntimeDiagnostics {
    pub schema_version: u64,
    pub counters: std::collections::BTreeMap<String, u64>,
    pub timings_ms: std::collections::BTreeMap<String, f64>,
    pub settings: std::collections::BTreeMap<String, String>,
    pub artifacts: std::collections::BTreeMap<String, RuntimeArtifact>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticsDocument {
    schema_version: u64,
    #[serde(deserialize_with = "diagnostics_map")]
    counters: std::collections::BTreeMap<String, u64>,
    #[serde(deserialize_with = "diagnostics_map")]
    timings_ms: std::collections::BTreeMap<String, f64>,
    #[serde(deserialize_with = "diagnostics_map")]
    settings: std::collections::BTreeMap<String, String>,
    #[serde(deserialize_with = "diagnostics_map")]
    artifacts: std::collections::BTreeMap<String, RuntimeArtifact>,
}
fn diagnostics_map<'de, D, T>(
    deserializer: D,
) -> std::result::Result<std::collections::BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    struct Map<T>(std::marker::PhantomData<T>);
    impl<'de, T: serde::Deserialize<'de>> serde::de::Visitor<'de> for Map<T> {
        type Value = std::collections::BTreeMap<String, T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a diagnostics map with at most 64 unique keys")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut access: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut entries = std::collections::BTreeMap::new();
            while let Some((key, value)) = access.next_entry::<String, T>()? {
                if entries.len() == 64 || entries.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom(
                        "duplicate or oversized diagnostics map",
                    ));
                }
            }
            Ok(entries)
        }
    }
    deserializer.deserialize_map(Map(std::marker::PhantomData))
}
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeArtifact {
    pub path: String,
    pub sha256: String,
}
// Lexical Windows drive/UNC/verbatim policy shared with native validation.
// No filesystem reads: a diagnostic is an observation, not loader attestation.
fn diagnostic_path(path: &str) -> bool {
    if path.is_empty() || path.len() > 4096 || path.bytes().any(|b| b < 32) {
        return false;
    }
    let verbatim = path.starts_with(r"\\?\");
    if verbatim && path.contains('/') {
        return false;
    }
    let unc_path;
    let (path, verbatim_drive) = if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        unc_path = format!(r"\\{rest}");
        (unc_path.as_str(), false)
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        (rest, true)
    } else {
        (path, false)
    };
    let bytes = path.as_bytes();
    let sep = |b: u8| b == b'\\' || b == b'/';
    let drive =
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && sep(bytes[2]);
    let unc = bytes.len() >= 5 && sep(bytes[0]) && sep(bytes[1]);
    if !(drive || unc && !verbatim_drive) {
        return false;
    }
    let parts: Vec<_> = path[if drive { 3 } else { 2 }..]
        .split(['\\', '/'])
        .collect();
    let mut count = 0;
    for (index, part) in parts.iter().enumerate() {
        if *part == "."
            || *part == ".."
            || (part.is_empty() && index + 1 != parts.len())
            || part.bytes().any(|b| b"<>\"|?*:".contains(&b))
        {
            return false;
        }
        count += usize::from(!part.is_empty());
    }
    !unc || count >= 2
}
impl TryFrom<DiagnosticsDocument> for RuntimeDiagnostics {
    type Error = &'static str;
    fn try_from(d: DiagnosticsDocument) -> std::result::Result<Self, Self::Error> {
        let valid_key = |key: &String| {
            !key.is_empty()
                && key.len() <= 128
                && key.as_bytes()[0].is_ascii_lowercase()
                && key
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b))
        };
        if d.schema_version != 1
            || [
                d.counters.len(),
                d.timings_ms.len(),
                d.settings.len(),
                d.artifacts.len(),
            ]
            .iter()
            .any(|n| *n > 64)
            || !d
                .counters
                .keys()
                .chain(d.timings_ms.keys())
                .chain(d.settings.keys())
                .chain(d.artifacts.keys())
                .all(valid_key)
            || d.timings_ms.values().any(|v| !v.is_finite() || *v < 0.)
            || d.settings
                .values()
                .any(|v| v.is_empty() || v.len() > 1024 || v.chars().any(char::is_control))
            || d.artifacts.values().any(|v| {
                !diagnostic_path(&v.path)
                    || v.sha256.len() != 64
                    || !v.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            })
        {
            return Err("invalid bounded runtime diagnostics");
        }
        Ok(Self {
            schema_version: d.schema_version,
            counters: d.counters,
            timings_ms: d.timings_ms,
            settings: d.settings,
            artifacts: d.artifacts,
        })
    }
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

#[cfg(test)]
mod diagnostics_tests {
    use super::*;
    fn empty() -> serde_json::Value {
        serde_json::json!({"schema_version":1,"counters":{},"timings_ms":{},"settings":{},"artifacts":{}})
    }
    fn accepts(value: serde_json::Value) {
        serde_json::from_value::<RuntimeDiagnostics>(value).unwrap();
    }
    fn rejects(value: serde_json::Value) {
        assert!(
            serde_json::from_value::<RuntimeDiagnostics>(value.clone()).is_err(),
            "{value}"
        );
    }
    #[test]
    fn runtime_accepts_old_and_optional_diagnostics() {
        let mut info = serde_json::json!({"schema_version":1,"backend":"fixture","sdk_version":"test","device_id":"cpu","device_name":"CPU","api":"fixture","driver":"none"});
        assert!(
            serde_json::from_value::<RuntimeInfo>(info.clone())
                .unwrap()
                .diagnostics
                .is_none()
        );
        info["diagnostics"] = serde_json::json!({"schema_version":1,"counters":{},"timings_ms":{},"settings":{},"artifacts":{}});
        serde_json::from_value::<RuntimeInfo>(info.clone()).unwrap();
        info["diagnostics"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<RuntimeInfo>(info).is_err());
    }
    #[test]
    fn diagnostics_rejects_duplicate_map_keys() {
        for (map, value) in [
            ("counters", serde_json::json!(1)),
            ("timings_ms", serde_json::json!(0.5)),
            ("settings", serde_json::json!("value")),
            (
                "artifacts",
                serde_json::json!({"path":r"C:\module.dll","sha256":"a".repeat(64)}),
            ),
        ] {
            let text = empty().to_string().replacen(
                &format!("\"{map}\":{{}}"),
                &format!("\"{map}\":{{\"a\":{value},\"a\":{value}}}"),
                1,
            );
            assert!(
                serde_json::from_str::<RuntimeDiagnostics>(&text).is_err(),
                "duplicate {map} key accepted"
            );
        }
    }
    #[test]
    fn diagnostics_accepts_maximum_maps_and_legal_windows_paths() {
        let mut d = empty();
        for i in 0..64 {
            let key = format!("a{i:0127}");
            d["counters"][&key] = u64::MAX.into();
            d["timings_ms"][&key] = f64::MAX.into();
            d["settings"][&key] = "é".repeat(512).into();
            d["artifacts"][&key] =
                serde_json::json!({"path":r"C:\module.dll","sha256":"A".repeat(64)});
        }
        accepts(d);
        for path in [
            r"C:/module.dll".to_owned(),
            r"\\server\share\module.dll".into(),
            r"\\?\C:\module.dll".into(),
            r"\\?\UNC\server\share\module.dll".into(),
            format!("C:\\{}", "x".repeat(4093)),
            "C:\\legal-\u{7f}\u{85}.dll".into(),
        ] {
            let mut d = empty();
            d["artifacts"]["module"] = serde_json::json!({"path":path,"sha256":"a".repeat(64)});
            accepts(d);
        }
    }
    #[test]
    fn diagnostics_rejects_malformed_bounds_types_and_unknown_fields() {
        use serde_json::{Value, json};
        for map in ["counters", "timings_ms", "settings", "artifacts"] {
            let sample = match map {
                "counters" => json!(1),
                "timings_ms" => json!(0.5),
                "settings" => json!("value"),
                _ => json!({"path":r"C:\module.dll","sha256":"a".repeat(64)}),
            };
            let mut d = empty();
            for i in 0..65 {
                d[map][format!("a{i}")] = sample.clone();
            }
            rejects(d);
            for key in [
                "".to_owned(),
                "A".into(),
                "1a".into(),
                "a b".into(),
                "a/".into(),
                "a\n".into(),
                "é".into(),
                "a".repeat(129),
            ] {
                let mut d = empty();
                d[map][key] = sample.clone();
                rejects(d);
            }
            let mut d = empty();
            d.as_object_mut().unwrap().remove(map);
            rejects(d);
            for value in [Value::Null, json!([]), json!(1)] {
                let mut d = empty();
                d[map] = value;
                rejects(d);
            }
        }
        for value in [
            json!(-1),
            json!(1.0),
            json!(true),
            json!("1"),
            Value::Null,
            json!(18446744073709551616.0),
        ] {
            let mut d = empty();
            d["counters"]["a"] = value;
            rejects(d);
        }
        for value in [json!(-1), json!(true), json!("1"), Value::Null] {
            let mut d = empty();
            d["timings_ms"]["a"] = value;
            rejects(d);
        }
        for value in [
            json!(""),
            json!("é".repeat(513)),
            json!("bad\ntext"),
            json!("bad\u{85}"),
            json!(true),
            json!(1),
            Value::Null,
        ] {
            let mut d = empty();
            d["settings"]["a"] = value;
            rejects(d);
        }
        for value in [json!(0), json!(2), json!(1.0), json!(true), Value::Null] {
            let mut d = empty();
            d["schema_version"] = value;
            rejects(d);
        }
        let mut d = empty();
        d["unknown"] = json!(1);
        rejects(d);
        rejects(Value::Null);
        for path in [
            "".to_owned(),
            "module.dll".into(),
            r"C:module.dll".into(),
            r"\module.dll".into(),
            r"\\?\C:/module.dll".into(),
            r"\\.\pipe\x".into(),
            r"C:\..\x".into(),
            r"C:\bad|x".into(),
            "C:\\bad\0x".into(),
            format!("C:\\{}", "x".repeat(4094)),
        ] {
            let mut d = empty();
            d["artifacts"]["module"] = json!({"path":path,"sha256":"a".repeat(64)});
            rejects(d);
        }
        for value in [
            json!({"path":r"C:\module.dll"}),
            json!({"path":r"C:\module.dll","sha256":"a".repeat(64),"unknown":1}),
            json!({"path":r"C:\module.dll","sha256":"g".repeat(64)}),
            json!({"path":r"C:\module.dll","sha256":"a".repeat(63)}),
            json!({"path":r"C:\module.dll","sha256":"a".repeat(65)}),
        ] {
            let mut d = empty();
            d["artifacts"]["module"] = value;
            rejects(d);
        }
    }
}
