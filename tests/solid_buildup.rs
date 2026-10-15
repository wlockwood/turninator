use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use turninator::presentation::{ImportSettings, SceneSettings, load_model, model_scene};

fn tetra_stl(height: f64) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "turninator-solid-build-{}-{}.stl",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let points = [[0., 0., 0.], [10., 0., 0.], [0., 10., 0.], [0., 0., height]];
    let mut text = String::from("solid tetra\n");
    for tri in [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]] {
        text.push_str("facet normal 0 0 0\nouter loop\n");
        for i in tri {
            let [x, y, z] = points[i];
            text.push_str(&format!("vertex {x} {y} {z}\n"));
        }
        text.push_str("endloop\nendfacet\n");
    }
    text.push_str("endsolid tetra\n");
    fs::write(&path, text).unwrap();
    path
}

#[test]
fn original_stl_carries_print_stages_into_render_snapshot() {
    let path = tetra_stl(1.01);
    let settings = ImportSettings {
        first_layer_height_mm: 0.2,
        layer_height_mm: 0.3,
        ..Default::default()
    };
    let model = load_model(&path, &settings).unwrap();
    assert_eq!(model.layer_tops_mm.len(), 4);
    for (&actual, expected) in model.layer_tops_mm.iter().zip([0.2, 0.5, 0.8, 1.01]) {
        assert!((actual - expected).abs() < 1e-10);
    }
    assert_eq!(model.geometry_kind, "original");
    assert!(model.groups.is_empty());
    assert_eq!(
        turninator::presentation::visible_layer_count(&model, 0.51),
        2
    );
    let scene = model_scene(
        &model,
        &SceneSettings {
            import: settings,
            ..Default::default()
        },
        &path.with_extension("mesh.bin"),
        None,
    )
    .unwrap();
    assert_eq!(
        scene["solid_layer_tops_mm"],
        serde_json::json!(model.layer_tops_mm)
    );
    assert!(scene.get("layer_face_ranges").is_none());
    fs::remove_file(path).unwrap();
}

#[test]
fn a_thin_final_solid_layer_is_retained_without_changing_the_stl() {
    let path = tetra_stl(0.21);
    let before = fs::read(&path).unwrap();
    let model = load_model(&path, &ImportSettings::default()).unwrap();
    assert_eq!(model.layer_tops_mm, vec![0.2, 0.21]);
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::remove_file(path).unwrap();
}
