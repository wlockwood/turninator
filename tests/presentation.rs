use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use turninator::{
    mesh::Mesh,
    presentation::{
        ImportSettings, RevisionGate, SceneSettings, load_model, make_build_mesh, render_scene,
    },
};

fn triangle() -> Mesh {
    Mesh {
        positions: vec![[0., 0., 0.], [20., 0., 0.], [0., 10., 5.]],
        triangles: vec![[0, 1, 2]],
    }
}

#[test]
fn build_pose_reorients_print_coordinates_and_places_the_bottom_on_the_bed() {
    let original = triangle();
    let built = make_build_mesh(&original, [90., 0., 0.]).unwrap();
    assert_eq!(original.positions[2], [0., 10., 5.]);
    let b = built.bounds().unwrap();
    assert!((b.max[2] - 10.).abs() < 1e-10);
    assert_eq!(b.min[2], 0.);
    assert!((b.max[1] - b.min[1] - 5.).abs() < 1e-10);
}

#[test]
fn explicit_print_settings_survive_material_changes() {
    let mut scene = SceneSettings::default();
    scene.import.layer_height_mm = 0.28;
    scene.import.first_layer_height_mm = 0.30;
    scene.print_settings_edited = true;
    scene.select_material("resin");
    assert_eq!(scene.import.layer_height_mm, 0.28);
    assert_eq!(scene.import.first_layer_height_mm, 0.30);
    scene.reset_print_settings();
    assert_eq!(scene.import.layer_height_mm, 0.05);
}

#[test]
fn late_async_completions_cannot_replace_the_selected_model() {
    let mut gate = RevisionGate::default();
    let old = gate.advance();
    let selected = gate.advance();
    assert!(!gate.accepts(old));
    assert!(gate.accepts(selected));
}

#[test]
fn scene_snapshot_keeps_independent_presentation_and_first_layer() {
    let mut scene = SceneSettings {
        display_rotation_degrees: [90., 0., 0.],
        ..Default::default()
    };
    scene.import.first_layer_height_mm = 0.3;
    let result = render_scene(
        &scene,
        &PathBuf::from("D:/job/mesh.bin"),
        "original",
        &[],
        None,
    )
    .unwrap();
    assert_eq!(
        result["display_rotation_degrees"],
        serde_json::json!([90., 0., 0.])
    );
    assert_eq!(result["first_layer_height_mm"], 0.3);
    assert!(result["layer_appearance"].as_bool().unwrap());
    assert_eq!(result["bead_layout"], serde_json::Value::Null);
}

#[test]
fn model_loading_reports_assumed_units_and_preserves_source_hash() {
    let directory = std::env::temp_dir().join(format!(
        "turninator-present-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("part.stl");
    let data = "solid part\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid part";
    fs::write(&path, data).unwrap();
    let options = ImportSettings {
        millimeters_per_unit: 25.4,
        ..ImportSettings::default()
    };
    let model = load_model(&path, &options).unwrap();
    assert_eq!(model.mesh.bounds().unwrap().max, [25.4, 25.4, 0.]);
    assert!(model.warnings.iter().any(|s| s.contains("25.4")));
    assert_eq!(fs::read_to_string(&path).unwrap(), data);
    assert_eq!(model.source_hash.len(), 64);
    fs::remove_dir_all(directory).unwrap();
}
