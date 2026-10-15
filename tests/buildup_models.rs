//! Opt-in checks of real STL build-up caps after native f32 source conversion.
use std::{fs, path::PathBuf};
use turninator::{
    mesh::Mesh,
    presentation::{ImportSettings, load_model},
    print::slice::BuildUpSolid,
};

#[test]
#[ignore = "set TURNINATOR_BUILDUP_MODEL to an original STL; writes cap diagnostics"]
fn real_model_caps_survive_native_source_precision() {
    let path = PathBuf::from(std::env::var_os("TURNINATOR_BUILDUP_MODEL").expect("set model path"));
    let before = fs::read(&path).unwrap();
    let model = load_model(&path, &ImportSettings::default()).unwrap();
    let mesh = Mesh {
        positions: model
            .mesh
            .positions
            .iter()
            .map(|p| p.map(|v| v as f32 as f64))
            .collect(),
        triangles: model.mesh.triangles.clone(),
    };
    let bounds = mesh.bounds().unwrap();
    let solid = BuildUpSolid::new(&mesh).unwrap();
    let height = bounds.max[2] - bounds.min[2];
    let mut failures = Vec::new();
    let mut samples = vec![28.4, height * 0.25, height * 0.5, height * 0.75];
    if std::env::var_os("TURNINATOR_BUILDUP_ALL_LAYERS").is_some() {
        samples.extend((1..(height / 0.2).ceil() as usize).map(|i| i as f64 * 0.2));
    }
    for sample in samples {
        let z = (bounds.min[2] + sample) as f32 as f64;
        if z <= bounds.min[2] || z >= bounds.max[2] {
            continue;
        }
        match solid.cap(z - bounds.min[2]) {
            Ok(cap) => {
                if !cap.triangles.is_empty() {
                    turninator::viewport::PickingMesh::new(&cap).unwrap();
                }
                for triangle in &cap.triangles {
                    let [a, b, c] = triangle.map(|i| cap.positions[i as usize]);
                    assert_eq!(a[2], z);
                    assert!((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]) > 0.);
                }
                println!(
                    "cap {:?} at {z}: {} triangles",
                    path.file_name().unwrap(),
                    cap.triangles.len()
                );
            }
            Err(error) => failures.push(format!("height {z}: {error}")),
        }
    }
    assert_eq!(before, fs::read(&path).unwrap());
    assert!(
        failures.is_empty(),
        "{}: {}",
        path.display(),
        failures.join("\n")
    );
}
