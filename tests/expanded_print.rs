use std::collections::BTreeMap;
use turninator::print::gcode::{GcodeDialect, ParseOptions, parse_gcode};
use turninator::{
    mesh::Mesh,
    print::{
        beads::{BeadSegment, build_joined_bead_mesh},
        schedule::LayerSchedule,
    },
};

#[test]
fn stepped_dense_source_uses_only_triangles_crossing_each_plane() {
    let mut mesh = Mesh::default();
    let rings = 7000u32;
    for i in 0..=rings {
        let z = 4. * i as f64 / rings as f64;
        mesh.positions
            .extend([[0., 0., z], [2., 0., z], [2., 2., z], [0., 2., z]]);
    }
    mesh.triangles.extend([[0, 2, 1], [0, 3, 2]]);
    for ring in 0..rings {
        for edge in 0..4 {
            let a = ring * 4 + edge;
            let b = ring * 4 + (edge + 1) % 4;
            mesh.triangles.extend([[a, b, b + 4], [a, b + 4, a + 4]]);
        }
    }
    let top = rings * 4;
    mesh.triangles
        .extend([[top, top + 1, top + 2], [top, top + 2, top + 3]]);
    let schedule = LayerSchedule::uniform(0.02, 0.02, 4.).unwrap();
    let output = turninator::print::slice::slice_stepped(&mesh, &schedule.boundaries_mm).unwrap();
    closed_outward(&output);
    assert_eq!(output.bounds().unwrap().max[2], 4.);
}

fn bead(start: [f64; 3], end: [f64; 3]) -> BeadSegment {
    BeadSegment {
        start,
        end,
        width_mm: 0.45,
        height_mm: 0.2,
        volume_mm3: 1.,
        source_line: 1,
        feature: "Outer wall".into(),
        tool: 0,
        layer_index: 0,
        width_source: "fixture".into(),
        height_source: "fixture".into(),
    }
}
fn closed_outward(mesh: &Mesh) {
    let mut edges = BTreeMap::new();
    let mut volume = 0.;
    for t in &mesh.triangles {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let item = edges.entry((a.min(b), a.max(b))).or_insert((0, 0));
            item.0 += 1;
            item.1 += if a < b { 1 } else { -1 };
        }
        let [a, b, c] = t.map(|i| mesh.positions[i as usize]);
        let n = [
            (b[1] - a[1]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[1] - a[1]),
            (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]),
            (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]),
        ];
        assert!(n.iter().any(|v| v.abs() > 0.), "degenerate face");
        volume += (a[0] * (b[1] * c[2] - b[2] * c[1])
            + a[1] * (b[2] * c[0] - b[0] * c[2])
            + a[2] * (b[0] * c[1] - b[1] * c[0]))
            / 6.;
    }
    assert!(
        edges
            .values()
            .all(|&(count, balance)| count == 2 && balance == 0),
        "open or inconsistently oriented edges"
    );
    assert!(volume > 0., "nonpositive signed volume {volume}");
}
#[test]
fn joined_corner_has_one_shared_ring_and_only_terminal_caps() {
    let a = bead([0., 0., 0.2], [10., 0., 0.2]);
    let b = bead(a.end, [10., 10., 0.2]);
    let mesh = build_joined_bead_mesh(&[a, b]).unwrap();
    assert_eq!(mesh.positions.len(), 3 * 32 + 2);
    assert_eq!(mesh.triangles.len(), 2 * 64 + 64);
    closed_outward(&mesh);
    let bounds = mesh.bounds().unwrap();
    assert!(bounds.min[2] >= -1e-12 && bounds.max[2] <= 0.2 + 1e-12);
}

#[test]
fn large_joined_toolpaths_fit_mesh_budgets_without_dropping_segments() {
    use turninator::print::beads::build_joined_bead_mesh_with_report;
    let segments: Vec<_> = (0..65_000)
        .map(|i| bead([i as f64 * 0.1, 0., 0.2], [(i + 1) as f64 * 0.1, 0., 0.2]))
        .collect();
    let joined = build_joined_bead_mesh_with_report(&segments).unwrap();
    assert!(joined.mesh.positions.len() <= turninator::mesh::MAX_VERTICES);
    assert!(joined.mesh.triangles.len() <= turninator::mesh::MAX_TRIANGLES);
    assert_eq!(joined.mesh.bounds().unwrap().max[0], 6500.);
    assert_eq!(joined.smooth_face_ranges.len(), 1);
    assert!(joined.mesh.triangles.len() >= segments.len() * 16);
}

#[test]
fn reversing_toolpaths_split_into_closed_paths() {
    use turninator::print::beads::build_joined_bead_mesh_with_report;
    let a = bead([0., 0., 0.2], [10., 0., 0.2]);
    let b = bead(a.end, [0., 0., 0.2]);
    let joined = build_joined_bead_mesh_with_report(&[a, b]).unwrap();
    assert_eq!(joined.smooth_face_ranges.len(), 2);
    closed_outward(&joined.mesh);
}

#[test]
fn generated_beads_omit_only_faces_that_collapse_at_snapshot_precision() {
    use turninator::print::beads::build_joined_bead_mesh_with_report;
    let mut a = bead([104.18244, 112.84288, 5.3], [104.26651, 112.83281, 5.3]);
    a.height_mm = 0.00001;
    a.width_mm = 0.4;
    let joined = build_joined_bead_mesh_with_report(&[a]).unwrap();
    turninator::viewport::PickingMesh::new(&joined.mesh).unwrap();
}
#[test]
fn joined_closed_variable_profile_loop_has_no_terminal_caps() {
    let points = [
        [0., 0., 0.3],
        [10., 0., 0.3],
        [10., 10., 0.3],
        [0., 10., 0.3],
    ];
    let segments: Vec<_> = (0..4)
        .map(|i| {
            let mut s = bead(points[i], points[(i + 1) % 4]);
            s.width_mm = 0.35 + i as f64 * 0.1;
            s.height_mm = 0.1 + i as f64 * 0.05;
            s
        })
        .collect();
    let mesh = build_joined_bead_mesh(&segments).unwrap();
    assert_eq!(mesh.positions.len(), 4 * 32);
    assert_eq!(mesh.triangles.len(), 4 * 64);
    closed_outward(&mesh);
}
#[test]
fn explicit_schedule_is_strict_and_clips_only_the_terminal_interval() {
    let schedule = LayerSchedule::explicit(vec![0., 0.3, 0.42, 0.62]).unwrap();
    assert_eq!(
        schedule.clipped(0.51).unwrap().boundaries_mm,
        vec![0., 0.3, 0.42, 0.51]
    );
    for boundaries in [
        vec![],
        vec![0.],
        vec![0.1, 0.3],
        vec![0., 0.3, 0.2],
        vec![0., f64::NAN],
        vec![0., 0.01],
        vec![0., 3.],
    ] {
        assert!(LayerSchedule::explicit(boundaries).is_err());
    }
    assert!(schedule.clipped(0.7).is_err());
}

#[test]
fn arcs_keep_true_length_volume_and_chord_error() {
    let options = ParseOptions {
        allow_arcs: true,
        chord_error_mm: 0.01,
        ..Default::default()
    };
    let data = parse_gcode("G90\nM83\nG1X10Z.2\nG3X0Y10I-10J0E1", &options).unwrap();
    assert!(data.segments.len() > 8);
    let volume: f64 = data.segments.iter().map(|s| s.volume_mm3).sum();
    let expected = std::f64::consts::PI * 1.75f64.powi(2) / 4.;
    assert!((volume - expected).abs() < 1e-10);
    let width =
        expected / (5. * std::f64::consts::PI) / 0.2 + 0.2 * (1. - std::f64::consts::PI / 4.);
    for s in &data.segments {
        assert!((s.width_mm - width).abs() < 1e-10);
        let m = [(s.start[0] + s.end[0]) * 0.5, (s.start[1] + s.end[1]) * 0.5];
        assert!(10. - m[0].hypot(m[1]) <= options.chord_error_mm);
    }
    assert_eq!(data.segments.last().unwrap().end, [0., 10., 0.2]);
    let circle = parse_gcode("G90\nM83\nG1X10Z.2\n;WIDTH:0.45\nG2I-10J0E3", &options).unwrap();
    assert_eq!(
        circle.segments.first().unwrap().start,
        circle.segments.last().unwrap().end
    );
    closed_outward(&build_joined_bead_mesh(&circle.segments).unwrap());
    for arc in ["G2X0Y10R10E1", "G2X0Y10R-10E1", "G3X0Y10R10E1"] {
        assert!(parse_gcode(&format!("G90\nM83\nG1X10Z.2\n;WIDTH:0.45\n{arc}"), &options).is_ok());
    }
}
#[test]
fn dialect_homing_tools_and_explicit_adaptive_schedule_are_visible_contracts() {
    let options = ParseOptions {
        dialect: GcodeDialect::Klipper,
        allow_tool_changes: true,
        homing_position_mm: Some([0.; 3]),
        layer_schedule: Some(LayerSchedule::explicit(vec![0., 0.3, 0.42]).unwrap()),
        ..Default::default()
    };
    let data=parse_gcode("G28\nG90\nM83\nG1Z.3\n;WIDTH:0.45\nG1X10E.4\nG1E-1\nT1\nG90\nG1X20E.4\nT0\nG1X30E1.4\nG1Z.42\nG1X40E.4",&options).unwrap();
    assert_eq!(data.segments.len(), 4);
    assert_eq!(data.segments[1].tool, 1);
    assert!((data.segments[2].start[0] - (20. + 10. / 1.4)).abs() < 1e-8);
    assert!((data.segments[3].height_mm - 0.12).abs() < 1e-10);
    assert!(data.warnings.iter().any(|s| s.contains("homing")));
    assert!(parse_gcode("G20\nG1X10E1", &options).is_err());
    assert!(
        parse_gcode(
            "G28\nG1Z.2\nM83\n;WIDTH:.45\nG1X10E1",
            &ParseOptions::default()
        )
        .is_err()
    );
}

fn box_mesh(top: [f64; 4]) -> Mesh {
    Mesh {
        positions: vec![
            [0., 0., 0.],
            [2., 0., 0.],
            [2., 2., 0.],
            [0., 2., 0.],
            [0., 0., top[0]],
            [2., 0., top[1]],
            [2., 2., top[2]],
            [0., 2., top[3]],
        ],
        triangles: vec![
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 0, 4],
            [3, 4, 7],
        ],
    }
}
fn square_tube(inner: f64) -> Mesh {
    let mut m = Mesh::default();
    for z in [0., 1.] {
        for radius in [1., inner] {
            m.positions.extend([
                [-radius, -radius, z],
                [radius, -radius, z],
                [radius, radius, z],
                [-radius, radius, z],
            ]);
        }
    }
    for i in 0..4u32 {
        let j = (i + 1) % 4;
        m.triangles.extend([
            [i, j, j + 8],
            [i, j + 8, i + 8],
            [i + 4, j + 12, j + 4],
            [i + 4, i + 12, j + 12],
            [i + 8, j + 8, j + 12],
            [i + 8, j + 12, i + 12],
            [i, j + 4, j],
            [i, i + 4, j + 4],
        ]);
    }
    m
}
#[test]
fn stepped_cube_wedge_holes_and_thin_walls_are_closed_without_repairs() {
    use turninator::print::slice::{slice_stepped, slice_stepped_with_report};
    for mesh in [
        box_mesh([1.; 4]),
        box_mesh([1., 2., 2., 1.]),
        square_tube(0.5),
        square_tube(0.999),
    ] {
        closed_outward(&mesh);
        let source = mesh.positions.clone();
        let top = mesh.bounds().unwrap().max[2];
        let schedule = LayerSchedule::uniform(0.3, 0.3, top).unwrap();
        let report = slice_stepped_with_report(&mesh, &schedule.boundaries_mm).unwrap();
        let tolerance = report.coordinate_tolerance_mm;
        let stepped = report.mesh;
        closed_outward(&stepped);
        let b = stepped.bounds().unwrap();
        assert_eq!(b.min[2], 0.);
        assert_eq!(b.max[2], top);
        assert_eq!(mesh.positions, source);
        if mesh.positions.len() == 16 {
            assert!(
                stepped
                    .positions
                    .iter()
                    .any(|p| (p[0].abs() - mesh.positions[4][0].abs()).abs() <= tolerance)
            );
        }
        if mesh.positions.len() == 16 {
            let inner = mesh.positions[4][0].abs();
            for t in &stepped.triangles {
                let p = t.map(|i| stepped.positions[i as usize]);
                if p[0][2] == p[1][2] && p[1][2] == p[2][2] {
                    let x = (p[0][0] + p[1][0] + p[2][0]) / 3.;
                    let y = (p[0][1] + p[1][1] + p[2][1]) / 3.;
                    assert!(
                        x.abs() >= inner - 1e-12 || y.abs() >= inner - 1e-12,
                        "cap filled a real hole"
                    );
                }
            }
        }
    }
    let mut open = box_mesh([1.; 4]);
    open.triangles.pop();
    assert!(slice_stepped(&open, &[0., 0.5, 1.]).is_err());
}

fn dome_mesh(stagger: bool) -> Mesh {
    let count = 16;
    let rings = 5;
    let mut mesh = Mesh::default();
    for ring in 0..rings {
        let angle = ring as f64 / rings as f64 * std::f64::consts::FRAC_PI_2;
        for i in 0..count {
            let theta = 2.
                * std::f64::consts::PI
                * (i as f64 + if stagger { 0.5 * (ring % 2) as f64 } else { 0. })
                / count as f64;
            mesh.positions.push([
                angle.cos() * theta.cos(),
                angle.cos() * theta.sin(),
                angle.sin(),
            ]);
        }
    }
    for ring in 0..rings - 1 {
        for i in 0..count {
            let a = (ring * count + i) as u32;
            let b = (ring * count + (i + 1) % count) as u32;
            mesh.triangles.extend([
                [a, b, b + count as u32],
                [a, b + count as u32, a + count as u32],
            ]);
        }
    }
    let apex = mesh.positions.len() as u32;
    mesh.positions.push([0., 0., 1.]);
    let center = mesh.positions.len() as u32;
    mesh.positions.push([0., 0., 0.]);
    for i in 0..count {
        let j = (i + 1) % count;
        let a = ((rings - 1) * count + i) as u32;
        let b = ((rings - 1) * count + j) as u32;
        mesh.triangles
            .extend([[a, b, apex], [center, j as u32, i as u32]]);
    }
    mesh
}
#[test]
fn stepped_dome_is_closed_and_sampling_stays_inside_each_interval() {
    use turninator::print::slice::slice_stepped_with_report;
    let mesh = dome_mesh(true);
    closed_outward(&mesh);
    let schedule = LayerSchedule::uniform(0.3, 0.3, 1.).unwrap();
    let result = slice_stepped_with_report(&mesh, &schedule.boundaries_mm).unwrap();
    closed_outward(&result.mesh);
    for (i, z) in result.sample_heights_mm.iter().enumerate() {
        assert!(*z > schedule.boundaries_mm[i] && *z < schedule.boundaries_mm[i + 1].min(1.));
    }
    assert_eq!(result.mesh.bounds().unwrap().max[2], 1.);
    assert!(!result.warnings.is_empty());
}

#[test]
fn stepped_vertex_plane_contacts_do_not_create_artificial_micro_edges() {
    use turninator::print::slice::slice_stepped_with_report;
    let mesh = box_mesh([1.5, 2., 2., 1.5]);
    let schedule = LayerSchedule::uniform(0.2, 0.2, 2.).unwrap();
    let result = slice_stepped_with_report(&mesh, &schedule.boundaries_mm).unwrap();
    assert!(result.sample_heights_mm.contains(&1.5));
    closed_outward(&result.mesh);
    assert_eq!(result.mesh.bounds().unwrap().max[2], 2.);
}

#[test]
fn stepped_decimal_schedule_roundoff_does_not_split_a_vertex_contact() {
    use turninator::print::slice::slice_stepped_with_report;
    let mesh = box_mesh([42.5, 43., 43., 42.5]);
    let schedule = LayerSchedule::uniform(0.2, 0.2, 43.).unwrap();
    let result = slice_stepped_with_report(&mesh, &schedule.boundaries_mm).unwrap();
    closed_outward(&result.mesh);
    assert_eq!(result.mesh.bounds().unwrap().max[2], 43.);
}
#[test]
fn aligned_dome_layers_do_not_manufacture_boolean_slivers() {
    let mesh = dome_mesh(false);
    let before = mesh.positions.clone();
    let schedule = LayerSchedule::uniform(0.3, 0.3, 1.).unwrap();
    let output = turninator::print::slice::slice_stepped(&mesh, &schedule.boundaries_mm).unwrap();
    closed_outward(&output);
    assert_eq!(mesh.positions, before);
}

#[test]
fn stepped_rejects_touching_contours_and_gaps_lost_at_renderer_precision() {
    for shift in [2., 2. + 1e-8] {
        let mut mesh = box_mesh([1.; 4]);
        let other = box_mesh([1.; 4]);
        let offset = mesh.positions.len() as u32;
        mesh.positions
            .extend(other.positions.iter().map(|p| [p[0] + shift, p[1], p[2]]));
        mesh.triangles
            .extend(other.triangles.iter().map(|t| t.map(|i| i + offset)));
        let error = turninator::print::slice::slice_stepped(&mesh, &[0., 0.5, 1.])
            .unwrap_err()
            .to_string();
        assert!(error.contains("intersect, touch"), "{error}");
    }
}

#[test]
fn stepped_layer_ranges_preserve_empty_space_and_closed_print_stages() {
    let mut mesh = box_mesh([0.5; 4]);
    let other = box_mesh([0.5; 4]);
    let offset = mesh.positions.len() as u32;
    mesh.positions
        .extend(other.positions.iter().map(|p| [p[0], p[1], p[2] + 1.5]));
    mesh.triangles
        .extend(other.triangles.iter().map(|t| t.map(|i| i + offset)));
    let proof = turninator::print::slice::slice_stepped_with_report(&mesh, &[0., 0.5, 1., 1.5, 2.])
        .unwrap();
    assert_eq!(proof.layer_face_ranges.len(), 4);
    assert_eq!(proof.layer_face_ranges[1][0], proof.layer_face_ranges[1][1]);
    assert_eq!(proof.layer_face_ranges[2][0], proof.layer_face_ranges[2][1]);
    for range in proof.layer_face_ranges {
        if range[0] == range[1] {
            continue;
        }
        let slab = Mesh {
            positions: proof.mesh.positions.clone(),
            triangles: proof.mesh.triangles[range[0]..range[1]].to_vec(),
        };
        closed_outward(&slab);
    }
    assert!(
        proof
            .mesh
            .positions
            .iter()
            .flatten()
            .all(|&v| v == v as f32 as f64)
    );
}
#[test]
fn expanded_parser_rejects_ambiguous_arcs_and_preserves_repayment_along_curve() {
    let options = ParseOptions {
        allow_arcs: true,
        ..Default::default()
    };
    let start = "G90\nM83\nG1X10Z.2\n;WIDTH:.45\n";
    for invalid in [
        "G2X0Y10I-10R10E1",
        "G3X0Y10R2E1",
        "G2R10E1",
        "G3X0Y10I-1E1",
        "G3X0Y10I-10Z.4E1",
        "G18",
        "G90.1",
        "M200D1.75",
    ] {
        assert!(
            parse_gcode(&format!("{start}{invalid}"), &options).is_err(),
            "accepted {invalid}"
        );
    }
    let result = parse_gcode(&format!("{start}G1E-1\nG3X0Y10I-10E2"), &options).unwrap();
    let first = result.segments.first().unwrap().start;
    let theta = first[1].atan2(first[0]);
    assert!((theta - std::f64::consts::FRAC_PI_4).abs() < 0.01);
    let volume: f64 = result.segments.iter().map(|s| s.volume_mm3).sum();
    assert!((volume - std::f64::consts::PI * 1.75f64.powi(2) / 4.).abs() < 1e-10);
}

#[test]
#[ignore = "writes proof snapshots; run explicitly for Blender geometry verification"]
fn write_expanded_geometry_proofs() {
    use turninator::print::{
        beads::build_joined_bead_mesh_with_report, slice::slice_stepped_with_report,
    };
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/expanded-print");
    std::fs::create_dir_all(&directory).unwrap();
    let write = |name: &str,
                 mesh: &Mesh,
                 layout: Option<Vec<[usize; 2]>>,
                 warnings: Vec<String>| {
        closed_outward(mesh);
        let path = directory.join(format!("{name}.tnmesh"));
        mesh.write_binary(&path).unwrap();
        let joined = layout.is_some();
        let scene = serde_json::json!({"schema_version":1,"mesh_path":path,"geometry_kind":if joined {"beads"} else {"original"},"bead_layout":if joined {Some("joined_stadium_v1")} else {None},"bead_side_face_ranges":layout,"layer_height_mm":0.2,"first_layer_height_mm":0.2,"layer_appearance":false,"color":[0.035,0.22,0.52],"roughness":0.42,"display_rotation_degrees":[0.,0.,0.],"warnings":warnings});
        std::fs::write(
            directory.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&scene).unwrap(),
        )
        .unwrap();
        eprintln!(
            "{name}: {} vertices, {} triangles",
            mesh.positions.len(),
            mesh.triangles.len()
        );
    };
    let mut gcode = String::from("G90\nM83\n;WIDTH:0.45\n;TYPE:Outer wall\n");
    for layer in 1..=16 {
        gcode.push_str(&format!("G1X4Y0Z{:.6}\nG3I-4J0E1\n", layer as f64 * 0.2));
    }
    std::fs::write(directory.join("joined-arcs.gcode"), &gcode).unwrap();
    let parsed = parse_gcode(
        &gcode,
        &ParseOptions {
            allow_arcs: true,
            ..Default::default()
        },
    )
    .unwrap();
    let joined = build_joined_bead_mesh_with_report(&parsed.segments).unwrap();
    write(
        "joined-arcs",
        &joined.mesh,
        Some(joined.smooth_face_ranges),
        parsed.warnings,
    );
    for (name, mut source) in [
        ("stepped-cube", box_mesh([1.; 4])),
        ("stepped-wedge", box_mesh([1., 2., 2., 1.])),
        ("stepped-dome", dome_mesh(true)),
        ("stepped-hole", square_tube(0.5)),
        ("stepped-thinwall", square_tube(0.99)),
    ] {
        for p in &mut source.positions {
            for v in p {
                *v *= 4.;
            }
        }
        let top = source.bounds().unwrap().max[2];
        let schedule = LayerSchedule::uniform(0.2, 0.2, top).unwrap();
        let proof = slice_stepped_with_report(&source, &schedule.boundaries_mm).unwrap();
        write(
            &name.replace("stepped-", "original-"),
            &source,
            None,
            vec![
                "Original closed fixture; paired reference for experimental stepped geometry."
                    .into(),
            ],
        );
        let measure = |mesh: &Mesh| {
            let volume: f64 = mesh
                .triangles
                .iter()
                .map(|t| {
                    let a = mesh.positions[t[0] as usize];
                    let b = mesh.positions[t[1] as usize];
                    let c = mesh.positions[t[2] as usize];
                    (a[0] * (b[1] * c[2] - b[2] * c[1])
                        + a[1] * (b[2] * c[0] - b[0] * c[2])
                        + a[2] * (b[0] * c[1] - b[1] * c[0]))
                        / 6.
                })
                .sum();
            let bounds = mesh.bounds().unwrap();
            serde_json::json!({"vertices":mesh.positions.len(),"triangles":mesh.triangles.len(),"signed_volume_mm3":volume,"bounds_mm":{"min":bounds.min,"max":bounds.max}})
        };
        std::fs::write(
            directory.join(format!("{name}-comparison.json")),
            serde_json::to_vec_pretty(&serde_json::json!({"pitch_mm":0.2,"original":measure(&source),"stepped":measure(&proof.mesh),"sample_heights_mm":proof.sample_heights_mm,"warnings":proof.warnings})).unwrap(),
        ).unwrap();
        write(name, &proof.mesh, None, proof.warnings);
    }
}
