use std::fs;
use turninator::presentation::{ImportSettings, SceneSettings, load_model, model_scene};
#[test]
fn joined_import_preserves_tool_feature_and_layer_ranges_for_rendering() {
    let directory =
        std::env::temp_dir().join(format!("turninator-studio-print-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("tools.gcode");
    fs::write(&source, "G90\nM83\n;TYPE:Outer wall\nG1 Z0.2\nG1 X10 E0.33\nG1 Y10 E0.33\nT1\n;TYPE:Sparse infill\nG1 X0 E0.33\nG1 Z0.4\nG1 Y0 E0.33\n").unwrap();
    assert!(load_model(&source, &ImportSettings::default()).is_err());
    let settings = ImportSettings {
        allow_tool_changes: true,
        joined_beads: true,
        ..Default::default()
    };
    let model = load_model(&source, &settings).unwrap();
    assert_eq!(model.import_settings, settings);
    assert_eq!(model.layer_tops_mm, vec![0.2, 0.4]);
    assert_eq!(
        turninator::presentation::visible_layer_height(&model, 0.3),
        0.2
    );
    assert_eq!(model.bead_layout.as_deref(), Some("joined_stadium_v1"));
    assert_eq!(model.groups.len(), 3);
    assert_eq!(model.groups[1].tool, 1);
    assert_eq!(model.groups[2].layer_index, 1);
    assert_eq!(model.groups[2].end_triangle, model.mesh.triangles.len());
    let scene = SceneSettings {
        import: settings,
        color_mode: "tool".into(),
        ..Default::default()
    };
    let rendered = model_scene(&model, &scene, &directory.join("mesh.bin"), None).unwrap();
    assert_eq!(rendered["bead_layout"], "joined_stadium_v1");
    assert_eq!(rendered["face_groups"].as_array().unwrap().len(), 3);
    assert_ne!(
        rendered["face_groups"][0]["color"],
        rendered["face_groups"][1]["color"]
    );
    assert_eq!(rendered["layer_face_ranges"].as_array().unwrap().len(), 2);
    assert_eq!(rendered["layer_face_ranges"][1]["z_top_mm"], 0.4);
    assert_eq!(
        rendered["source_path"],
        source.canonicalize().unwrap().to_str().unwrap()
    );
    let mut changed = scene.clone();
    changed.import.layer_height_mm = 0.3;
    assert!(model_scene(&model, &changed, &directory.join("mesh.bin"), None).is_err());
}
