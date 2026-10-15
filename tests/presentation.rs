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
fn palette_control_indices_match_colors_used_by_geometry() {
    use turninator::presentation::{GeometryGroup, group_color, group_color_index};
    let mut scene = SceneSettings::default();
    let group = GeometryGroup {
        start_triangle: 0,
        end_triangle: 1,
        tool: 5,
        feature: "Outer wall".into(),
        layer_index: 0,
        z_top_mm: 0.2,
    };
    assert_eq!(group_color_index(&group, &scene), None);
    assert_eq!(group_color(&group, &scene), scene.color);
    for mode in ["tool", "feature"] {
        scene.color_mode = mode.into();
        let index = group_color_index(&group, &scene).unwrap();
        assert!(index < scene.tool_colors.len());
        assert_eq!(group_color(&group, &scene), scene.tool_colors[index]);
        let unused = (index + 1) % scene.tool_colors.len();
        scene.tool_colors[unused] = [1., 1., 1.];
        assert_eq!(group_color(&group, &scene), scene.tool_colors[index]);
        if mode == "tool" {
            assert_eq!(index, 1);
        }
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

#[test]
fn adaptive_build_slider_roundtrip_retains_the_selected_complete_layer() {
    let directory = std::env::temp_dir().join(format!(
        "turninator-stage-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("layers.gcode");
    let mut source = "G90\nM83\n;WIDTH:0.4\n;HEIGHT:0.2\n".to_owned();
    for height in [0.3, 0.7, 1.1, 1.3] {
        source.push_str(&format!("G1X0Y0Z{height}\nG1X8E1\n"));
    }
    fs::write(&path, &source).unwrap();
    let model = load_model(&path, &ImportSettings::default()).unwrap();
    assert_eq!(turninator::presentation::visible_layer_count(&model, 0.), 0);
    for (i, height) in model.layer_tops_mm.iter().enumerate() {
        let slider_height = f64::from(*height as f32);
        assert_eq!(
            turninator::presentation::visible_layer_count(&model, slider_height),
            i + 1
        );
        assert_eq!(
            turninator::presentation::visible_layer_height(&model, slider_height),
            *height
        );
    }
    assert_eq!(fs::read_to_string(path).unwrap(), source);
    fs::remove_dir_all(directory).unwrap();
}
