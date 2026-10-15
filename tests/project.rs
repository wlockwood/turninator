use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use turninator::{
    presentation::SceneSettings,
    project::{ProjectFile, SourceStatus, load_project, save_project},
    settings::AppSettings,
    viewport::OrbitCamera,
};

fn scratch() -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "turninator-project-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&p).unwrap();
    p
}
fn project(path: PathBuf, hash: String) -> ProjectFile {
    ProjectFile {
        schema_version: 2,
        source_path: path,
        source_hash: hash,
        scene: SceneSettings::default(),
        camera: OrbitCamera::default(),
        export_size: 960,
        renderers: Default::default(),
        output_options: Default::default(),
    }
}
#[test]
fn project_too_large_to_reload_cannot_replace_an_existing_save() {
    let root = scratch();
    let source = root.join("source.stl");
    fs::write(&source, b"source").unwrap();
    let mut p = project(source, format!("{:x}", Sha256::digest(b"source")));
    p.scene.import.layer_boundaries_mm = (0..=50_000)
        .map(|i| i as f64 * 0.02348849665713213)
        .collect();
    let path = root.join("keep.turninator");
    fs::write(&path, b"sentinel").unwrap();
    assert!(save_project(&path, &p).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"sentinel");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_roundtrip_is_portable_and_reports_source_changes_and_missing_files() {
    let root = scratch();
    let source = root.join("source-é %.stl");
    fs::write(&source, b"source bytes").unwrap();
    let hash = format!("{:x}", Sha256::digest(b"source bytes"));
    let mut p = project(fs::canonicalize(&source).unwrap(), hash);
    p.scene.import.layer_height_mm = 0.28;
    p.scene.import.first_layer_height_mm = 0.30;
    p.scene.display_rotation_degrees = [90., 0., 0.];
    let path = root.join("projects/part.tnproj");
    save_project(&path, &p).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert!(!PathBuf::from(saved["source_path"].as_str().unwrap()).is_absolute());
    let (loaded, status) = load_project(&path).unwrap();
    assert_eq!(status, SourceStatus::Current);
    assert_eq!(loaded.scene.import.layer_height_mm, 0.28);
    assert_eq!(loaded.scene.display_rotation_degrees, [90., 0., 0.]);
    fs::write(&source, b"changed").unwrap();
    assert_eq!(load_project(&path).unwrap().1, SourceStatus::Changed);
    fs::remove_file(&source).unwrap();
    assert_eq!(load_project(&path).unwrap().1, SourceStatus::Missing);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_project_cannot_replace_existing_save_or_source() {
    let root = scratch();
    let source = root.join("source.stl");
    fs::write(&source, b"keep source").unwrap();
    let mut p = project(
        source.clone(),
        format!("{:x}", Sha256::digest(b"keep source")),
    );
    assert!(save_project(&source, &p).is_err());
    assert_eq!(fs::read(&source).unwrap(), b"keep source");
    let path = root.join("part.tnproj");
    save_project(&path, &p).unwrap();
    let before = fs::read(&path).unwrap();
    p.renderers.animation.backend = turninator::render::BackendId::BlenderCycles; // quality remains Eevee: invalid combination
    assert!(save_project(&path, &p).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn settings_keep_machine_tools_and_recent_files_without_duplicates() {
    let root = scratch();
    let path = root.join("settings.json");
    let mut settings = AppSettings::load(&path).unwrap();
    settings.remember(root.join("a.stl"));
    settings.remember(root.join("b.stl"));
    settings.remember(root.join("a.stl"));
    settings.save(&path).unwrap();
    let loaded = AppSettings::load(&path).unwrap();
    assert_eq!(loaded.recent_files.len(), 2);
    assert_eq!(loaded.recent_files[0], root.join("a.stl"));
    assert_eq!(loaded.cache_budget_bytes, 20 * 1024 * 1024 * 1024);
    fs::remove_dir_all(root).unwrap();
}
