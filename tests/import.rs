use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use turninator::model::stl::{load_stl, load_stl_with_report};

fn temp() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "turninator-stl-{}-{}.stl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}
const ASCII: &str = "solid example\nfacet normal 0 0 1\nouter loop\nvertex 1000000 0 0\nvertex 1000001 0 0\nvertex 1000000 1 0\nendloop\nendfacet\nendsolid example\n";
fn binary() -> Vec<u8> {
    let mut v = vec![0; 80];
    v[..5].copy_from_slice(b"solid");
    v.extend(1u32.to_le_bytes());
    for f in [
        0f32, 0., 1., 1000000., 0., 0., 1000001., 0., 0., 1000000., 1., 0.,
    ] {
        v.extend(f.to_le_bytes());
    }
    v.extend(0u16.to_le_bytes());
    v
}

#[test]
fn ascii_binary_solid_header_units_rebasing_and_source_preservation() {
    let path = temp();
    fs::write(&path, ASCII).unwrap();
    let a = load_stl(&path, 25.4).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), ASCII);
    let data = binary();
    fs::write(&path, &data).unwrap();
    let b = load_stl(&path, 25.4).unwrap();
    assert_eq!(fs::read(&path).unwrap(), data);
    assert_eq!(a.positions, b.positions);
    assert_eq!(a.triangles, b.triangles);
    let bounds = a.bounds().unwrap();
    assert!((bounds.max[0] - bounds.min[0] - 25.4).abs() < 1e-7);
    assert!(bounds.max[0].abs() < 100.);
    fs::remove_file(path).unwrap();
}

#[test]
fn stl_rejects_corruption_empty_nonfinite_and_bad_units() {
    let path = temp();
    for bytes in [
        vec![],
        b"solid empty\nendsolid empty".to_vec(),
        binary()[..100].to_vec(),
        ASCII.replace("1000001", "NaN").into_bytes(),
        b"solid a\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendsolid a".to_vec(),
    ] {
        fs::write(&path, bytes).unwrap();
        assert!(load_stl(&path, 1.).is_err());
    }
    fs::write(&path, ASCII).unwrap();
    for unit in [0., -1., f64::NAN, f64::INFINITY] {
        assert!(load_stl(&path, unit).is_err());
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn ascii_whitespace_and_degenerate_triangle_are_handled_explicitly() {
    let path = temp();
    fs::write(
        &path,
        ASCII
            .replace("outer loop", "outer\t  loop")
            .replace("solid example", "solid\texample"),
    )
    .unwrap();
    assert!(load_stl(&path, 1.).is_ok());
    fs::write(&path, ASCII.replace("1000001 0 0", "1000000 0 0")).unwrap();
    assert!(
        load_stl(&path, 1.)
            .unwrap_err()
            .to_string()
            .contains("degenerate")
    );
    fs::remove_file(path).unwrap();
}

fn facet(points: [[f64; 3]; 3]) -> String {
    format!(
        "facet normal 0 0 1\nouter loop\nvertex {} {} {}\nvertex {} {} {}\nvertex {} {} {}\nendloop\nendfacet\n",
        points[0][0],
        points[0][1],
        points[0][2],
        points[1][0],
        points[1][1],
        points[1][2],
        points[2][0],
        points[2][1],
        points[2][2]
    )
}

#[test]
fn report_omits_only_exact_zero_area_and_preserves_tiny_faces_and_source() {
    let path = temp();
    let data = format!(
        "solid test\n{}{}endsolid test\n",
        facet([[0., 0., 0.], [1., 0., 0.], [2., 0., 0.]]),
        facet([[0., 0., 0.], [1e-6, 0., 0.], [0., 1e-6, 0.]])
    );
    fs::write(&path, &data).unwrap();
    let imported = load_stl_with_report(&path, 1.).unwrap();
    assert_eq!(imported.omitted_degenerate_triangles, 1);
    assert_eq!(imported.mesh.triangles.len(), 1);
    assert_eq!(
        imported.mesh.positions.len(),
        3,
        "omitted faces must not leave unused vertices"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), data);
    assert_eq!(load_stl(&path, 1.).unwrap().triangles.len(), 1);
    fs::remove_file(path).unwrap();
}

#[test]
fn technics_real_model_reports_six_duplicate_vertex_faces() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-models/Technics_work_support.stl");
    let before = fs::read(&path).unwrap();
    let imported = load_stl_with_report(&path, 1.).unwrap();
    assert_eq!(imported.omitted_degenerate_triangles, 6);
    assert_eq!(imported.mesh.triangles.len(), 3438);
    let snapshot = temp();
    imported.mesh.write_binary(&snapshot).unwrap();
    fs::remove_file(snapshot).unwrap();
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn nonzero_faces_are_not_omitted_when_naive_area_underflows_or_rounds_to_zero() {
    let path = temp();
    let epsilon = f64::EPSILON;
    for points in [
        [[0., 0., 0.], [1e-200, 0., 0.], [0., 1e-200, 0.]],
        [[0., 0., 0.], [1., 1. + epsilon, 0.], [1. - epsilon, 1., 0.]],
    ] {
        fs::write(
            &path,
            format!("solid tiny\n{}endsolid tiny\n", facet(points)),
        )
        .unwrap();
        let imported = load_stl_with_report(&path, 1.).unwrap();
        assert_eq!(imported.omitted_degenerate_triangles, 0);
        assert_eq!(imported.mesh.triangles.len(), 1);
        let output = temp();
        assert!(
            imported.mesh.write_binary(&output).is_err(),
            "unrepresentable f32 geometry must remain an explicit snapshot error"
        );
        assert!(!output.exists());
    }
    fs::remove_file(path).unwrap();
}
