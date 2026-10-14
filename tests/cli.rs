use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn scratch(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "turninator-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn fixture_prepares_a_physical_bead_job_and_keeps_source_read_only() {
    let root = scratch("cli % unicode-é");
    let fixture = root.join("fixture");
    let result = Command::new(env!("CARGO_BIN_EXE_turninator"))
        .args(["fixture", "--out"])
        .arg(&fixture)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let source = fixture.join("layer-demo.gcode");
    let before = fs::read(&source).unwrap();
    let job = root.join("job [test]");
    let result = Command::new(env!("CARGO_BIN_EXE_turninator"))
        .arg("prepare")
        .arg(&source)
        .arg("--out")
        .arg(&job)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(before, fs::read(&source).unwrap());
    let scene: serde_json::Value =
        serde_json::from_slice(&fs::read(job.join("scene.json")).unwrap()).unwrap();
    assert_eq!(scene["schema_version"], 1);
    assert_eq!(scene["geometry_kind"], "beads");
    let mesh_path = std::path::Path::new(scene["mesh_path"].as_str().unwrap());
    assert!(mesh_path.is_absolute());
    let mesh = turninator::mesh::Mesh::read_binary(mesh_path).unwrap();
    let bounds = mesh.bounds().unwrap();
    assert!(mesh.triangles.len() > 1000);
    assert!(bounds.min[2].abs() < 1e-5, "{:?}", bounds.min);
    assert!((bounds.max[2] - 24.0).abs() < 1e-4, "{:?}", bounds.max);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unsupported_input_does_not_publish_a_partial_job() {
    let root = scratch("bad-cli");
    let source = root.join("bad.gcode");
    fs::write(&source, "G90\nM83\nG1 Z0.2\nG2 X10 Y0 I5 E1\n").unwrap();
    let job = root.join("job");
    let result = Command::new(env!("CARGO_BIN_EXE_turninator"))
        .arg("prepare")
        .arg(&source)
        .arg("--out")
        .arg(&job)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!job.join("scene.json").exists());
    assert!(!job.join("mesh.bin").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn preparation_preserves_an_existing_job() {
    let root = scratch("existing-cli");
    let source = root.join("source.gcode");
    fs::write(&source, "G90\nM83\nG1 Z0.2\nG1 X10 E0.4\n").unwrap();
    let job = root.join("job");
    fs::create_dir_all(&job).unwrap();
    fs::write(job.join("scene.json"), "keep this").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_turninator"))
        .arg("prepare")
        .arg(&source)
        .arg("--out")
        .arg(&job)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(
        fs::read_to_string(job.join("scene.json")).unwrap(),
        "keep this"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stl_layer_appearance_records_pitch_without_changing_the_source_silhouette() {
    let root = scratch("appearance");
    let fixture = root.join("fixture");
    let result = Command::new(env!("CARGO_BIN_EXE_turninator"))
        .args(["fixture", "--out"])
        .arg(&fixture)
        .output()
        .unwrap();
    assert!(result.status.success());
    let job = root.join("job");
    let result = Command::new(env!("CARGO_BIN_EXE_turninator"))
        .arg("prepare")
        .arg(fixture.join("block.stl"))
        .arg("--out")
        .arg(&job)
        .args([
            "--surface",
            "layers",
            "--layer-height",
            "0.28",
            "--first-layer-height",
            "0.3",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let scene: serde_json::Value =
        serde_json::from_slice(&fs::read(job.join("scene.json")).unwrap()).unwrap();
    assert_eq!(scene["layer_appearance"], true);
    assert_eq!(scene["layer_height_mm"], 0.28);
    assert_eq!(scene["first_layer_height_mm"], 0.3);
    let mesh = turninator::mesh::Mesh::read_binary(std::path::Path::new(
        scene["mesh_path"].as_str().unwrap(),
    ))
    .unwrap();
    assert_eq!(mesh.bounds().unwrap().max, [20.0, 12.0, 10.0]);
    fs::remove_dir_all(root).unwrap();
}
