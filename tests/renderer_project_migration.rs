use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use turninator::{
    project::{load_project, save_project},
    settings::AppSettings,
    viewport::OrbitCamera,
};

fn scratch() -> PathBuf {
    static ID: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "turninator-renderer-migration-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
fn legacy() -> Value {
    let camera = OrbitCamera {
        yaw_degrees: 137.,
        pitch_degrees: 22.,
        distance_mm: 123.,
        target_mm: [12., 23., 34.],
        orthographic: true,
        ortho_span_mm: 71.,
        ..Default::default()
    };
    json!({"schema_version":1,"source_path":"missing.stl","source_hash":"a".repeat(64),
        "scene":{"material_preset":"resin","resin_clarity":0.72},"camera":camera,
        "export_size":960,"engine":"eevee","output_options":{"still_engine":"cycles","samples":23,"still_samples":137,
        "transparent":true,"frame_rate":30,"timeline":turninator::animation::Timeline::default(),"turntable_timing":{"seconds_per_turn":4.,"duration_seconds":8.}}})
}
#[test]
fn legacy_project_preserves_render_intent() {
    let root = scratch();
    let path = root.join("legacy.tnproj");
    let value = legacy();
    let bytes = serde_json::to_vec(&value).unwrap();
    fs::write(&path, &bytes).unwrap();
    let (project, _) = load_project(&path).unwrap();
    assert_eq!(
        fs::read(&path).unwrap(),
        bytes,
        "loading must never rewrite old files"
    );
    let saved = root.join("new.tnproj");
    save_project(&saved, &project).unwrap();
    let wire: Value = serde_json::from_slice(&fs::read(saved).unwrap()).unwrap();
    assert_eq!(wire["schema_version"], 2);
    assert_eq!(wire["renderers"]["animation"]["backend"], "blender_eevee");
    assert_eq!(wire["renderers"]["animation"]["quality"]["samples"], 23);
    assert_eq!(wire["renderers"]["still"]["backend"], "blender_cycles");
    assert_eq!(wire["renderers"]["still"]["quality"]["samples"], 137);
    assert_eq!(wire["renderers"]["still"]["quality"]["denoise"], true);
    assert!(wire.get("engine").is_none());
    for key in ["samples", "still_samples", "still_engine"] {
        assert!(wire["output_options"].get(key).is_none());
    }
    assert_eq!(wire["camera"], value["camera"]);
    assert_eq!(
        wire["output_options"]["timeline"],
        value["output_options"]["timeline"]
    );
    assert_eq!(wire["scene"]["material_preset"], "resin");
    assert_eq!(wire["scene"]["resin_clarity"], json!(0.72));
    assert_eq!(wire["output_options"]["transparent"], true);
    assert_eq!(wire["output_options"]["frame_rate"], 30);
    assert_eq!(
        wire["output_options"]["turntable_timing"],
        value["output_options"]["turntable_timing"]
    );
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn missing_app_renderer_settings_use_defaults_without_rewrite() {
    let root = scratch();
    let path = root.join("settings.json");
    let bytes = b"{\"schema_version\":1}";
    fs::write(&path, bytes).unwrap();
    let settings = AppSettings::load(&path).unwrap();
    let wire = serde_json::to_value(settings).unwrap();
    assert_eq!(wire["renderers"]["animation"]["backend"], "blender_eevee");
    assert_eq!(wire["renderers"]["still"]["backend"], "blender_cycles");
    assert_eq!(fs::read(path).unwrap(), bytes);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn schema_two_roundtrips_every_backend_and_preserves_unavailable_devices() {
    use turninator::render::{BackendId, RenderSelection};
    let root = scratch();
    let input = root.join("legacy.tnproj");
    fs::write(&input, serde_json::to_vec(&legacy()).unwrap()).unwrap();
    let (mut project, _) = load_project(&input).unwrap();
    for backend in BackendId::ALL {
        project.renderers.animation = RenderSelection::for_backend(backend, false);
        project.renderers.still = RenderSelection::for_backend(backend, true);
        project.renderers.animation.device = Some("unavailable-adapter/vendor:deadbeef".into());
        project.renderers.still.device = Some("unavailable-optix/device:123".into());
        let path = root.join(format!("{}.tnproj", backend.as_str()));
        save_project(&path, &project).unwrap();
        let before = fs::read(&path).unwrap();
        let (loaded, _) = load_project(&path).unwrap();
        assert_eq!(loaded.renderers, project.renderers);
        assert_eq!(fs::read(path).unwrap(), before);
        let request = turninator::export::ExportRequest {
            selection: project.renderers.still.clone(),
            ..Default::default()
        };
        let restored: turninator::export::ExportRequest =
            serde_json::from_slice(&serde_json::to_vec(&request).unwrap()).unwrap();
        assert_eq!(restored.selection, request.selection);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_project_renderers_and_future_schemas_are_rejected_without_rewrite() {
    let root = scratch();
    let path = root.join("invalid.tnproj");
    let mut inputs = Vec::new();
    for samples in [0, 4097] {
        let mut v = legacy();
        v["output_options"]["samples"] = json!(samples);
        inputs.push(v);
    }
    let mut unknown = legacy();
    unknown["engine"] = json!("unknown");
    inputs.push(unknown);
    let mut future = legacy();
    future["schema_version"] = json!(3);
    inputs.push(future);
    fs::write(&path, serde_json::to_vec(&legacy()).unwrap()).unwrap();
    let (valid, _) = load_project(&path).unwrap();
    let canonical = serde_json::to_value(valid).unwrap();
    let mut mismatch = canonical.clone();
    mismatch["renderers"]["animation"]["backend"] = json!("blender_cycles");
    inputs.push(mismatch);
    let mut unknown = canonical.clone();
    unknown["renderers"]["still"]["backend"] = json!("missing_engine");
    inputs.push(unknown);
    let mut missing_selection = canonical.clone();
    missing_selection["renderers"]
        .as_object_mut()
        .unwrap()
        .remove("still");
    inputs.push(missing_selection);
    let mut duplicate = canonical.clone();
    duplicate["engine"] = json!("cycles");
    inputs.push(duplicate);
    let mut duplicate = canonical.clone();
    duplicate["output_options"]["samples"] = json!(16);
    inputs.push(duplicate);
    let mut bad_profile = canonical;
    bad_profile["renderers"]["profiles"]["wgpu"]["animation"] =
        json!({"kind":"eevee","samples":32});
    inputs.push(bad_profile);
    for input in inputs {
        let bytes = serde_json::to_vec(&input).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(load_project(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    fs::remove_dir_all(root).unwrap();
}
