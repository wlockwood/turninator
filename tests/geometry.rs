use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use turninator::{
    mesh::Mesh,
    print::{
        beads::{BeadSegment, build_bead_mesh},
        schedule::LayerSchedule,
    },
};

fn temp() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "turninator-mesh-{}-{}.bin",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn bead() -> BeadSegment {
    BeadSegment {
        start: [0., 0., 0.2],
        end: [10., 0., 0.2],
        width_mm: 0.42,
        height_mm: 0.2,
        volume_mm3: 0.0,
        source_line: 1,
        feature: "wall".into(),
        tool: 0,
        layer_index: 0,
        width_source: "fixture".into(),
        height_source: "fixture".into(),
    }
}

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}

#[test]
fn physical_schedule_and_first_layer() {
    let regular = LayerSchedule::uniform(0.2, 0.2, 20.).unwrap();
    assert_eq!(regular.boundaries_mm.len(), 101);
    near(*regular.boundaries_mm.last().unwrap(), 20.);
    let first = LayerSchedule::uniform(0.2, 0.3, 0.6).unwrap();
    assert_eq!(first.boundaries_mm.len(), 4);
    near(first.boundaries_mm[1], 0.3);
    near(first.boundaries_mm[2], 0.5);
    near(first.boundaries_mm[3], 0.7);
    assert_eq!(
        LayerSchedule::uniform(0.2, 0.2, 0.21)
            .unwrap()
            .boundaries_mm
            .len(),
        3
    );
}

#[test]
fn schedule_rejects_invalid_and_excessive_inputs() {
    for h in [0., -0.1, f64::NAN, f64::INFINITY, 0.019, 2.01] {
        assert!(LayerSchedule::uniform(h, 0.2, 20.).is_err());
        assert!(LayerSchedule::uniform(0.2, h, 20.).is_err());
    }
    for height in [0., -1., f64::NAN, f64::INFINITY, 1e10] {
        assert!(LayerSchedule::uniform(0.2, 0.2, height).is_err());
    }
    assert_eq!(
        LayerSchedule::uniform(0.02, 0.02, 1000.)
            .unwrap()
            .boundaries_mm
            .len(),
        50_001
    );
    assert!(LayerSchedule::uniform(0.02, 0.02, 1000.01).is_err());
}

#[test]
fn stadium_dimensions_volume_and_outward_winding() {
    let mesh = build_bead_mesh(&[bead()]).unwrap();
    let b = mesh.bounds().unwrap();
    for (actual, expected) in b.min.into_iter().zip([0., -0.21, 0.]) {
        near(actual, expected);
    }
    for (actual, expected) in b.max.into_iter().zip([10., 0.21, 0.2]) {
        near(actual, expected);
    }
    let center = [5., 0., 0.1];
    let mut volume = 0.;
    for t in &mesh.triangles {
        let a = mesh.positions[t[0] as usize];
        let b = mesh.positions[t[1] as usize];
        let c = mesh.positions[t[2] as usize];
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        let outward = (0..3)
            .map(|i| n[i] * ((a[i] + b[i] + c[i]) / 3. - center[i]))
            .sum::<f64>();
        assert!(outward > 0., "triangle winding or degenerate face: {t:?}");
        volume += (a[0] * (b[1] * c[2] - b[2] * c[1])
            + a[1] * (b[2] * c[0] - b[0] * c[2])
            + a[2] * (b[0] * c[1] - b[1] * c[0]))
            / 6.;
    }
    let exact = 10. * (0.2 * (0.42 - 0.2) + std::f64::consts::PI * 0.2 * 0.2 / 4.);
    assert!(
        (volume / exact - 1.).abs() < 0.01,
        "faceted area error: {volume} vs {exact}"
    );
}

#[test]
fn bead_corner_bounds_and_invalid_segments() {
    let a = bead();
    let mut b = a.clone();
    b.start = a.end;
    b.end = [10., 10., 0.2];
    let bounds = build_bead_mesh(&[a, b]).unwrap().bounds().unwrap();
    near(bounds.max[0], 10.21);
    near(bounds.max[1], 10.);
    for change in 0..5 {
        let mut s = bead();
        match change {
            0 => s.width_mm = 0.1,
            1 => s.height_mm = 0.,
            2 => s.end = s.start,
            3 => s.end[2] = 0.3,
            _ => s.start[0] = f64::NAN,
        }
        assert!(build_bead_mesh(&[s]).is_err());
    }
    assert!(build_bead_mesh(&[]).is_err());
}

#[test]
fn binary_snapshot_roundtrip_and_validation() {
    let path = temp();
    let mesh = build_bead_mesh(&[bead()]).unwrap();
    mesh.write_binary(&path).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert_eq!(&bytes[..8], b"TNMESH01");
    let copy = Mesh::read_binary(&path).unwrap();
    assert_eq!(copy.triangles, mesh.triangles);
    assert_eq!(copy.positions.len(), mesh.positions.len());
    let mut corrupt = bytes.clone();
    corrupt.push(0);
    fs::write(&path, &corrupt).unwrap();
    assert!(Mesh::read_binary(&path).is_err());
    let mut corrupt = bytes.clone();
    corrupt[20..24].copy_from_slice(&f32::NAN.to_le_bytes());
    fs::write(&path, &corrupt).unwrap();
    assert!(Mesh::read_binary(&path).is_err());
    let mut corrupt = bytes.clone();
    corrupt[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    fs::write(&path, &corrupt).unwrap();
    assert!(Mesh::read_binary(&path).is_err());
    let mut corrupt = bytes;
    let len = corrupt.len();
    corrupt[len - 4..].copy_from_slice(&u32::MAX.to_le_bytes());
    fs::write(&path, &corrupt).unwrap();
    assert!(Mesh::read_binary(&path).is_err());
    fs::remove_file(path).unwrap();
    let mut invalid = mesh;
    invalid.triangles[0][0] = u32::MAX;
    assert!(invalid.bounds().is_err());
    assert!(Mesh::default().bounds().is_err());
}

#[test]
fn circular_profile_closed_edges_and_resource_budget() {
    let mut s = bead();
    s.width_mm = s.height_mm;
    let mesh = build_bead_mesh(&[s.clone()]).unwrap();
    let mut edges = std::collections::BTreeMap::new();
    for t in mesh.triangles {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
        }
    }
    assert!(edges.values().all(|&count| count == 2));
    assert!(
        build_bead_mesh(&vec![s; turninator::mesh::MAX_TRIANGLES / 104 + 1])
            .unwrap_err()
            .to_string()
            .contains("budget")
    );
}

#[test]
fn snapshot_rejects_f32_collapsed_geometry_before_touching_destination() {
    let path = temp();
    fs::write(&path, b"existing result").unwrap();
    let mut mesh = build_bead_mesh(&[bead()]).unwrap();
    for p in &mut mesh.positions {
        p[1] += 10_000_000.;
    }
    assert!(mesh.write_binary(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"existing result");
    fs::remove_file(path).unwrap();
}
