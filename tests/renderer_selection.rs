use serde_json::{Value, json};
use turninator::export::ExportRequest;

#[test]
fn retained_legacy_request_is_explicitly_upgraded() {
    let value = json!({"job_dir":"job","scene_path":"scene.json","destination":"out.png","kind":"Still",
        "engine":"cycles","samples":71,"size":512,"frame_count":1,"transparent":true});
    let request: ExportRequest = serde_json::from_value(value).unwrap();
    let wire = serde_json::to_value(request).unwrap();
    assert_eq!(wire["schema_version"], 2);
    assert_eq!(wire["selection"]["backend"], "blender_cycles");
    assert_eq!(wire["selection"]["quality"]["samples"], 71);
    assert_eq!(wire["selection"]["quality"]["denoise"], true);
    assert!(wire.get("engine").is_none());
    assert!(wire.get("samples").is_none());
}
#[test]
fn invalid_renderer_quality_is_rejected() {
    let request: Value = serde_json::to_value(ExportRequest::default()).unwrap();
    for mutation in [
        json!({"backend":"unknown","quality":{"kind":"eevee","samples":32}}),
        json!({"backend":"blender_cycles","quality":{"kind":"eevee","samples":32}}),
        json!({"backend":"blender_cycles","quality":{"kind":"cycles","samples":0,"denoise":true}}),
        json!({"backend":"luxcore","quality":{"kind":"luxcore","samples":4097,"denoise":true}}),
    ] {
        let mut wire = request.clone();
        wire["schema_version"] = json!(2);
        wire["selection"] = mutation;
        assert!(serde_json::from_value::<ExportRequest>(wire).is_err());
    }
    let mut future = request;
    future["schema_version"] = json!(99);
    assert!(serde_json::from_value::<ExportRequest>(future).is_err());
}

#[test]
fn stable_renderer_ids_and_quality_profiles_do_not_reinterpret_samples() {
    use turninator::render::{BackendId, BackendQuality, RenderSelection, RendererPreferences};
    for (backend, id) in BackendId::ALL.into_iter().zip([
        "wgpu",
        "filament",
        "luxcore",
        "blender_eevee",
        "blender_cycles",
    ]) {
        assert_eq!(serde_json::to_value(backend).unwrap(), id);
        assert_eq!(BackendId::parse_legacy(id).unwrap(), backend);
    }
    assert!(BackendId::parse_legacy("EEVEE").is_err());
    assert_eq!(
        BackendId::parse_legacy("eevee").unwrap(),
        BackendId::BlenderEevee
    );
    assert_eq!(
        BackendId::parse_legacy("cycles").unwrap(),
        BackendId::BlenderCycles
    );
    for backend in [
        BackendId::LuxCore,
        BackendId::BlenderEevee,
        BackendId::BlenderCycles,
    ] {
        for samples in [0, 4097] {
            let mut selection = RenderSelection::for_backend(backend, false);
            *selection.quality.samples_mut().unwrap() = samples;
            assert!(selection.validate().is_err());
        }
    }
    let mut prefs = RendererPreferences {
        animation: RenderSelection::from_legacy("eevee", 91).unwrap(),
        ..Default::default()
    };
    prefs.select_backend(false, BackendId::Wgpu);
    assert!(matches!(
        prefs.animation.quality,
        BackendQuality::Wgpu { .. }
    ));
    assert_eq!(prefs.animation.quality.samples(), None);
    prefs.select_backend(false, BackendId::BlenderEevee);
    assert_eq!(prefs.animation.quality.samples(), Some(91));
    assert_eq!(prefs.still.quality.samples(), Some(64));
}

#[test]
fn structural_scene_conversion_preserves_build_coordinates_and_camera() {
    use std::{path::Path, sync::Arc};
    use turninator::{
        mesh::Mesh,
        presentation::{SceneSettings, render_scene},
        render::contract::snapshot_scene,
        viewport::OrbitCamera,
    };
    let mesh = Arc::new(Mesh {
        positions: vec![[100., 200., 300.], [110., 200., 300.], [100., 210., 300.]],
        triangles: vec![[0, 1, 2]],
    });
    let camera = OrbitCamera {
        target_mm: [105., 205., 300.],
        yaw_degrees: 0.,
        pitch_degrees: 0.,
        ..Default::default()
    };
    let scene = render_scene(
        &SceneSettings::default(),
        Path::new("D:/job/mesh.bin"),
        "original",
        &["source warning".into()],
        Some(camera.camera_spec()),
    )
    .unwrap();
    let snapshot = snapshot_scene(mesh.clone(), scene.clone()).unwrap();
    assert_eq!(snapshot.mesh.positions, mesh.positions);
    assert_eq!(snapshot.camera.position_mm, [105., 105., 300.]);
    assert_eq!(snapshot.reference_bounds_mm.min, [100., 200., 300.]);
    assert_eq!(snapshot.legacy_scene, scene);
    assert_eq!(snapshot.warnings, vec!["source warning"]);
    let mut bad = scene.clone();
    bad["layer_face_ranges"] =
        json!([{"start_triangle":0,"end_triangle":2,"layer_index":0,"z_top_mm":0.2}]);
    assert!(snapshot_scene(mesh.clone(), bad).is_err());
    let mut bad = scene;
    bad["camera"]["direction"] = json!([0., 0., 0.]);
    assert!(snapshot_scene(mesh, bad).is_err());
}

#[test]
fn camera_converter_reorthogonalizes_axes_and_preserves_both_projections() {
    use turninator::render::contract::{RenderCamera, RenderProjection};
    let mut value = json!({"target_mm":[10.,20.,30.],"direction":[0.,-2.,0.],"up":[0.,1.,1.],"distance_mm":40.,"projection":"perspective","vertical_fov_degrees":42.,"ortho_span_mm":65.});
    let camera = RenderCamera::from_legacy(&value).unwrap();
    assert_eq!(camera.position_mm, [10., -20., 30.]);
    assert_eq!(camera.up, [0., 0., 1.]);
    assert_eq!(
        camera.projection,
        RenderProjection::Perspective {
            vertical_fov_degrees: 42.
        }
    );
    value["projection"] = json!("orthographic");
    let camera = RenderCamera::from_legacy(&value).unwrap();
    assert_eq!(
        camera.projection,
        RenderProjection::Orthographic {
            vertical_span_mm: 65.
        }
    );
    assert_eq!(camera.clip_planes_mm(), (0.004, 4100.));
}

#[test]
fn structural_metadata_validates_complete_layers_and_bounded_materials() {
    use std::{path::Path, sync::Arc};
    use turninator::{
        mesh::Mesh,
        presentation::{SceneSettings, render_scene},
        render::contract::snapshot_scene,
    };
    let mesh = Arc::new(Mesh {
        positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 1.]],
        triangles: vec![[0, 1, 2], [2, 1, 0]],
    });
    let original = render_scene(
        &SceneSettings::default(),
        Path::new("D:/job/mesh.bin"),
        "original",
        &[],
        None,
    )
    .unwrap();
    let mut gap = original.clone();
    gap["layer_face_ranges"] =
        json!([{"start_triangle":1,"end_triangle":2,"layer_index":0,"z_top_mm":1.}]);
    assert!(snapshot_scene(mesh.clone(), gap).is_err());
    for (key, value) in [
        ("tool", json!(256)),
        ("label", json!("x".repeat(257))),
        ("color", json!([1.1, 0., 0.])),
    ] {
        let mut scene = original.clone();
        scene["face_groups"] = json!([{"start_triangle":0,"end_triangle":2,"tool":0,"label":"Perimeter","color":[0.1,0.2,0.3]}]);
        scene["face_groups"][0][key] = value;
        assert!(
            snapshot_scene(mesh.clone(), scene).is_err(),
            "invalid {key}"
        );
    }
    let mut stepped = original;
    stepped["geometry_kind"] = json!("stepped");
    stepped["layer_appearance"] = json!(false);
    stepped["layer_face_ranges"] = json!([{"start_triangle":0,"end_triangle":0,"layer_index":0,"z_top_mm":0.2},{"start_triangle":0,"end_triangle":2,"layer_index":1,"z_top_mm":1.}]);
    let snapshot = snapshot_scene(mesh, stepped).unwrap();
    assert_eq!(snapshot.metadata.layer_ranges[0].end_triangle, 0);
}

#[test]
fn minimal_legacy_scene_uses_existing_defaults_without_changing_wire_identity() {
    use std::sync::Arc;
    use turninator::{mesh::Mesh, render::contract::snapshot_scene};
    let mesh = Arc::new(Mesh {
        positions: vec![[10., 20., 30.], [20., 20., 30.], [10., 30., 40.]],
        triangles: vec![[0, 1, 2]],
    });
    let value = json!({"schema_version":1,"mesh_path":"D:/job/mesh.bin","geometry_kind":"original","layer_height_mm":0.28,
        "color":[0.1,0.2,0.3],"roughness":0.4,"display_rotation_degrees":[10.,20.,30.],"warnings":[]});
    let snapshot = snapshot_scene(mesh, value.clone()).unwrap();
    assert_eq!(snapshot.legacy_scene, value);
    assert!(!snapshot.appearance.layer_appearance);
    assert_eq!(snapshot.appearance.first_layer_height_mm, 0.28);
    assert!((snapshot.appearance.groove_depth_mm - 0.014).abs() < 1e-12);
    assert_eq!(snapshot.appearance.material_preset, "matte_pla");
    assert_eq!(snapshot.appearance.studio_preset, "light");
    assert_eq!(snapshot.appearance.resin_clarity, 0.8);
    assert_eq!(snapshot.appearance.resin_cloudiness, 0.1);
    assert_eq!(snapshot.camera.target_mm, [15., 25., 35.]);
    assert!(snapshot.legacy_scene.get("camera").is_none());
}

#[test]
fn historical_empty_stage_section_analysis_preserves_producer_payload() {
    use turninator::render::contract::SectionAnalysis;
    // Exact shape emitted by renderer/scene.py prepare_buildup_section's empty stage.
    let empty = json!({"axis":0,"position_mm":0.75,"keep_positive":true,
        "capped":false,"closed_input":false,"cross_section_area_mm2":0.0,
        "original_volume_mm3":0.0,"remaining_volume_mm3":0.0,"visible_layer":-1,
        "visible_source_triangles":0,"output_triangles":0,"warnings":[],
        "meaning":"Empty build stage: no deposited geometry is visible."});
    let analysis = SectionAnalysis::from_legacy(empty.clone(), true).unwrap();
    assert_eq!(analysis.legacy, empty);
    assert!(analysis.cap_supported);
    assert!(analysis.legacy.get("volume_basis").is_none());

    // Missing volume_basis must remain invalid for nonempty or malformed analyses.
    for (key, value) in [
        ("output_triangles", json!(1)),
        ("visible_source_triangles", json!(1)),
        ("remaining_volume_mm3", json!(1.0)),
        ("cross_section_area_mm2", Value::Null),
        ("closed_input", json!(true)),
        ("capped", json!(true)),
        ("visible_layer", json!("invalid")),
        ("warnings", json!(["unexpected"])),
        (
            "meaning",
            json!("Geometric intersection of the modeled mesh."),
        ),
        ("volume_basis", Value::Null),
    ] {
        let mut malformed = empty.clone();
        malformed[key] = value;
        assert!(
            SectionAnalysis::from_legacy(malformed, true).is_err(),
            "invalid {key}"
        );
    }
    let mut malformed = empty.clone();
    malformed
        .as_object_mut()
        .unwrap()
        .remove("visible_source_triangles");
    assert!(SectionAnalysis::from_legacy(malformed, true).is_err());
}

#[test]
fn ordinary_and_stepped_section_analysis_require_and_preserve_volume_basis() {
    use turninator::render::contract::SectionAnalysis;
    for basis in [
        "Original: signed closed-mesh volume magnitude, additive for separate shells. Remaining: closed cut-mesh volume magnitude.",
        "Additive volumes of independently closed, nonoverlapping stepped layers.",
    ] {
        let payload = json!({"axis":1,"position_mm":5.0,"keep_positive":false,"capped":true,
            "closed_input":true,"cross_section_area_mm2":20.0,"original_volume_mm3":100.0,
            "remaining_volume_mm3":50.0,"output_triangles":12,"warnings":[],
            "volume_basis":basis,"meaning":"Geometric intersection of the modeled mesh; not strength, mass, or measured print behavior."});
        assert_eq!(
            SectionAnalysis::from_legacy(payload.clone(), true)
                .unwrap()
                .legacy,
            payload
        );
        let mut missing_basis = payload;
        missing_basis
            .as_object_mut()
            .unwrap()
            .remove("volume_basis");
        assert!(SectionAnalysis::from_legacy(missing_basis, true).is_err());
    }
}
