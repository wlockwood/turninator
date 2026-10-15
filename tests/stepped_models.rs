//! Opt-in real-model regression: TURNINATOR_STL_MODELS points at the supplied
//! STL directory. Sources are read only; generated snapshots go to artifacts.
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::PathBuf, time::Instant};
use turninator::{
    mesh::Mesh,
    presentation::{ImportSettings, load_model},
};

fn check_closed(mesh: &Mesh) {
    let mut edges = BTreeMap::<(u32, u32), (usize, i32)>::new();
    for t in &mesh.triangles {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let value = edges.entry((a.min(b), a.max(b))).or_default();
            value.0 += 1;
            value.1 += if a < b { 1 } else { -1 };
        }
    }
    assert!(
        edges
            .values()
            .all(|&(count, balance)| count == 2 && balance == 0)
    );
}

#[test]
#[ignore = "requires Compost Booster.stl; TURNINATOR_STEPPED_MODEL can override its path"]
fn compost_booster_resolves_subprecision_slice_folds_without_changing_source() {
    let source = std::env::var_os("TURNINATOR_STEPPED_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("D:/Models/Compost Booster.stl"));
    let before = fs::read(&source).unwrap();
    let model = load_model(
        &source,
        &ImportSettings {
            stepped_envelope: true,
            ..ImportSettings::default()
        },
    )
    .unwrap();
    assert_eq!(fs::read(&source).unwrap(), before);
    assert_eq!(model.geometry_kind, "stepped");
    check_closed(&model.mesh);
    turninator::viewport::PickingMesh::new(&model.mesh).unwrap();
    let output =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts/compost-stepped-diagnosis");
    fs::create_dir_all(&output).unwrap();
    model
        .mesh
        .write_binary(&output.join("stepped.bin"))
        .unwrap();
    let decoded = Mesh::read_binary(&output.join("stepped.bin")).unwrap();
    check_closed(&decoded);
    fs::write(
        output.join("acceptance.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "source_sha256":format!("{:x}",Sha256::digest(&before)),
            "vertices":decoded.positions.len(),"triangles":decoded.triangles.len(),
            "warnings":model.warnings,"source_unchanged":true
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "requires Compost Booster.stl; TURNINATOR_STEPPED_MODEL can override its path"]
fn compost_booster_slices_sideways_build_and_preserves_snapshot_integrity() {
    let source = std::env::var_os("TURNINATOR_STEPPED_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("D:/Models/Compost Booster.stl"));
    let before = fs::read(&source).unwrap();
    let options = ImportSettings {
        build_rotation_degrees: [0., 90., 0.],
        stepped_envelope: true,
        ..ImportSettings::default()
    };
    let original =
        turninator::model::stl::load_stl_with_report(&source, options.millimeters_per_unit)
            .unwrap();
    let expected =
        turninator::presentation::make_build_mesh(&original.mesh, options.build_rotation_degrees)
            .unwrap();
    let expected_height = expected.bounds().unwrap().max[2];
    let started = Instant::now();
    let model = load_model(&source, &options).unwrap();
    assert_eq!(fs::read(&source).unwrap(), before);
    assert_eq!(model.geometry_kind, "stepped");
    assert_eq!(model.import_settings.build_rotation_degrees, [0., 90., 0.]);
    let last_layer_height = *model.layer_tops_mm.last().unwrap();
    assert_eq!(last_layer_height, expected_height as f32 as f64);
    check_closed(&model.mesh);
    turninator::viewport::PickingMesh::new(&model.mesh).unwrap();
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts/current-down-proof");
    fs::create_dir_all(&output).unwrap();
    let snapshot = output.join("side-model.bin");
    model.mesh.write_binary(&snapshot).unwrap();
    let decoded = Mesh::read_binary(&snapshot).unwrap();
    check_closed(&decoded);
    turninator::viewport::PickingMesh::new(&decoded).unwrap();
    assert_eq!(decoded.positions, model.mesh.positions);
    assert_eq!(decoded.triangles, model.mesh.triangles);
    assert_eq!(decoded.bounds().unwrap().max[2], last_layer_height);
    fs::write(
        output.join("side-model-acceptance.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "source":source,
            "source_sha256":format!("{:x}",Sha256::digest(&before)),
            "build_rotation_degrees":options.build_rotation_degrees,
            "layer_height_mm":options.layer_height_mm,
            "first_layer_height_mm":options.first_layer_height_mm,
            "expected_height_mm":expected_height,
            "last_layer_height_mm":last_layer_height,
            "snapshot_height_mm":decoded.bounds().unwrap().max[2],
            "layers":model.layer_tops_mm.len(),
            "vertices":decoded.positions.len(),
            "triangles":decoded.triangles.len(),
            "warnings":model.warnings,
            "seconds":started.elapsed().as_secs_f64(),
            "source_unchanged":true,
            "closed_mesh":true,
            "picking_valid":true,
            "snapshot_roundtrip":true
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "requires the supplied STL model directory"]
fn supplied_stls_slice_and_survive_renderer_snapshot_precision() {
    let source = PathBuf::from(
        std::env::var_os("TURNINATOR_STL_MODELS").expect("set TURNINATOR_STL_MODELS"),
    );
    let output =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts/stepped-model-regression");
    fs::create_dir_all(&output).unwrap();
    let options = ImportSettings {
        stepped_envelope: true,
        ..ImportSettings::default()
    };
    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for (i, name) in [
        "Amcrest IP5m Camera Mount.stl",
        "Arctic Nova Pro Cap USB Passthrough.stl",
        "Deck Post Jig.stl",
        "Sam power strip holder.stl",
        "Single Sword Mount.stl",
        "spiral egg - A.stl",
        "Technics_work_support.stl",
        "WandHandHexy.stl",
        "WandRing.stl",
    ]
    .iter()
    .enumerate()
    {
        let path = source.join(name);
        let before = fs::read(&path).unwrap();
        let started = Instant::now();
        let result = load_model(&path, &options).and_then(|model| {
            assert_eq!(model.geometry_kind, "stepped");
            assert!(
                model
                    .mesh
                    .positions
                    .iter()
                    .flatten()
                    .all(|&v| v == v as f32 as f64)
            );
            check_closed(&model.mesh);
            let snapshot = output.join(format!("model-{i}.bin"));
            model.mesh.write_binary(&snapshot)?;
            let decoded = Mesh::read_binary(&snapshot)?;
            check_closed(&decoded);
            Ok(model)
        });
        assert_eq!(before, fs::read(&path).unwrap(), "source changed: {name}");
        let expected_invalid = name == &"Technics_work_support.stl";
        match &result {
            Ok(_) if expected_invalid => failures.push(format!("{name}: invalid source accepted")),
            Err(e) if !expected_invalid || !e.to_string().contains("manifold") => {
                failures.push(format!("{name}: {e}"))
            }
            _ => {}
        }
        let row = serde_json::json!({"name":name,"source_sha256":format!("{:x}",Sha256::digest(&before)),
            "seconds":started.elapsed().as_secs_f64(),"error":result.as_ref().err().map(ToString::to_string),
            "triangles":result.as_ref().ok().map(|m|m.mesh.triangles.len()),
            "warnings":result.as_ref().ok().map(|m|&m.warnings)});
        println!("{row}");
        rows.push(row);
    }
    fs::write(
        output.join("results.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
