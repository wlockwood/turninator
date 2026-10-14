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
    assert_eq!(scene["bead_layout"], "independent_stadium_v1");
    assert_eq!(
        std::path::Path::new(scene["source_path"].as_str().unwrap()),
        fs::canonicalize(&source).unwrap()
    );
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
fn explicit_joined_arc_firmware_tools_and_schedule_options_prepare_valid_scene() {
    let root = scratch("expanded-cli");
    let source = root.join("arcs.gcode");
    fs::write(&source,"G90\nM83\nG28\nG0 X0 Y0 Z0.3\n;WIDTH:0.5\nG2 X10 Y0 I5 J0 E1\nT1\nG0 X0 Y0 Z0.5\nG1 X10 E0.4\n").unwrap();
    let output = root.join("job");
    let result = Command::new(env!("CARGO_BIN_EXE_turninator"))
        .arg("prepare")
        .arg(&source)
        .arg("--out")
        .arg(&output)
        .args([
            "--beads",
            "joined",
            "--allow-arcs",
            "true",
            "--firmware",
            "klipper",
            "--allow-tool-changes",
            "true",
            "--chord-error",
            "0.01",
            "--home",
            "0,0,0",
            "--layer-boundaries",
            "0,0.3,0.5",
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
        serde_json::from_slice(&fs::read(output.join("scene.json")).unwrap()).unwrap();
    assert_eq!(scene["bead_layout"], "joined_stadium_v1");
    assert!(scene["bead_side_face_ranges"].as_array().unwrap().len() >= 2);
    assert_eq!(scene["layer_face_ranges"].as_array().unwrap().len(), 2);
    assert_eq!(scene["layer_face_ranges"][0]["z_top_mm"], 0.3);
    assert!(fs::metadata(output.join("scene.json")).unwrap().len() <= 1048576);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_expanded_options_leave_no_output_directory() {
    let root = scratch("invalid-expanded-cli");
    let source = root.join("source.gcode");
    fs::write(&source, "G90\nM83\nG0 Z0.2\nG1 X10 E0.4\n").unwrap();
    for (index, args) in [
        ["--allow-arcs", "yes"],
        ["--beads", "unknown"],
        ["--home", "0,1"],
        ["--layer-boundaries", "0,0.2,0.1"],
        ["--firmware", "unknown"],
        ["--chord-error", "NaN"],
    ]
    .into_iter()
    .enumerate()
    {
        let output = root.join(format!("job-{index}"));
        let result = Command::new(env!("CARGO_BIN_EXE_turninator"))
            .arg("prepare")
            .arg(&source)
            .arg("--out")
            .arg(&output)
            .args(args)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(!output.exists());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn oversized_scene_and_float32_collapse_never_publish_a_job() {
    let root = scratch("cli-budgets");
    let source = root.join("large.gcode");
    let mut text = String::from("G90\nM83\n;WIDTH:0.44\n");
    for layer in 1..=14000 {
        text.push_str(&format!("G0 X0 Z{:.3}\nG1 X1 E0.03\n", layer as f64 * 0.2));
    }
    fs::write(&source, text).unwrap();
    let output = root.join("oversized");
    let result = Command::new(env!("CARGO_BIN_EXE_turninator"))
        .arg("prepare")
        .arg(&source)
        .arg("--out")
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("1 MiB"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!output.exists());
    fs::write(
        &source,
        "G90\nM83\n;WIDTH:0.44\nG0 X1000000 Z0.2\nG1 X1000000.01 E0.0003\n",
    )
    .unwrap();
    let output = root.join("collapsed");
    let result = Command::new(env!("CARGO_BIN_EXE_turninator"))
        .arg("prepare")
        .arg(&source)
        .arg("--out")
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!output.exists());
    assert!(fs::read_dir(&root).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".turninator-prepare-")
    }));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_stepped_stl_uses_shared_pipeline_and_reports_approximation() {
    let root = scratch("stepped-cli");
    let fixture = root.join("fixture");
    assert!(
        Command::new(env!("CARGO_BIN_EXE_turninator"))
            .args(["fixture", "--out"])
            .arg(&fixture)
            .status()
            .unwrap()
            .success()
    );
    let output = root.join("job");
    let result = Command::new(env!("CARGO_BIN_EXE_turninator"))
        .arg("prepare")
        .arg(fixture.join("block.stl"))
        .arg("--out")
        .arg(&output)
        .args(["--surface", "stepped", "--layer-boundaries", "0,2,4,6,8,10"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let scene: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("scene.json")).unwrap()).unwrap();
    assert_eq!(scene["layer_appearance"], false);
    assert!(scene["warnings"].as_array().unwrap().iter().any(|w| {
        w.as_str()
            .unwrap_or("")
            .contains("Experimental mid-layer stepped")
    }));
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
