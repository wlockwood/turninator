use turninator::print::gcode::{ParseOptions, parse_gcode};

fn parse(text: &str) -> turninator::Result<turninator::print::gcode::ToolpathData> {
    parse_gcode(text, &ParseOptions::default())
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-8, "{a} != {b}");
}

#[test]
fn compact_words_absolute_relative_and_marlin_mode_reset() {
    let p=parse("G21\nG90\nM82\nG1Z.2\n;WIDTH:0.42\nG1X10E.4\nG91\nG1X10E.4\nM82\nG1X10E1.2\nG90\nG1X40E1.6").unwrap();
    assert_eq!(p.segments.len(), 4);
    for (i, s) in p.segments.iter().enumerate() {
        near(s.start[0], i as f64 * 10.);
        near(s.end[0], (i + 1) as f64 * 10.);
    }
    assert!(!p.warnings.is_empty());
}

#[test]
fn g92_resets_coordinates_without_moving_or_clearing_debt() {
    let p = parse("G90\nM83\nG1Z.2\n;WIDTH:0.42\nG1X10E.4\nG1E-1\nG92X0E0\nG1X10E1.4").unwrap();
    assert_eq!(p.segments.len(), 2);
    near(p.segments[1].start[0], 10. + 10. / 1.4);
    near(p.segments[1].end[0], 20.);
    near(
        p.segments[1].volume_mm3,
        0.4 * std::f64::consts::PI * 1.75f64.powi(2) / 4.,
    );
}

#[test]
fn unretraction_prime_and_travel_hop_do_not_create_layers() {
    let p = parse(
        "G90\nM83\nG1Z.2\n;WIDTH:0.42\nG1E3\nG1X10E.4\nG1E-1\nG1Z.8\nG1X20\nG1Z.2\nG1E1\nG1X30E.4",
    )
    .unwrap();
    assert_eq!(p.segments.len(), 2);
    assert_eq!(p.segments[0].layer_index, p.segments[1].layer_index);
    near(p.segments[1].start[0], 20.);
}

#[test]
fn first_layer_metadata_features_and_inferred_width() {
    let o = ParseOptions {
        first_layer_height_mm: 0.3,
        ..Default::default()
    };
    let p=parse_gcode("G90\nM83\nG1Z.3\n;TYPE:Outer wall\nG1X10E.5\nG1Z.42\n;HEIGHT:0.12\n;WIDTH:0.45\n;TYPE:Unknown future feature\nG1X20E.3",&o).unwrap();
    assert_eq!(p.segments.len(), 2);
    near(p.segments[0].height_mm, 0.3);
    near(p.segments[1].height_mm, 0.12);
    near(p.segments[1].width_mm, 0.45);
    assert_eq!(p.segments[1].feature, "Unknown future feature");
    assert_eq!(p.segments[1].layer_index, 1);
    let area = 0.5 * std::f64::consts::PI * 1.75f64.powi(2) / 4. / 10.;
    near(
        p.segments[0].width_mm,
        area / 0.3 + 0.3 * (1. - std::f64::consts::PI / 4.),
    );
}

#[test]
fn unsupported_or_malformed_geometry_fails_instead_of_guessing() {
    let good = "G90\nM83\nG1Z.2\n;WIDTH:0.42\nG1X10E.4\n";
    for bad in [
        "G2X20Y0I5E1",
        "M200D1.75",
        "T1",
        "G10",
        "G28",
        "SET_GCODE_OFFSET Z=1",
        "G1XNaNE.4",
        "G1X10X20E1",
        "G1X",
        "G1X20Y0Z.4E1",
        "G1X20E.4Q5",
        "G1 X2e3 E1",
        "G90.1",
        "M221S90",
    ] {
        assert!(parse(&format!("{good}{bad}")).is_err(), "accepted {bad}");
    }
    assert!(parse("G1Z.2\nM83\nG1X10E.001").is_err());
    assert!(parse("G1Z.2\nG1X10").is_err());
}

#[test]
fn benign_commands_units_and_invalid_options() {
    let p=parse("M104S200\nM140S60\nM106S128\nG20\nG90\nM83\nG1Z0.0078740157480315\n;WIDTH:0.42\nG1X1E.04\nM400").unwrap();
    near(p.segments[0].end[0], 25.4);
    near(p.segments[0].end[2], 0.2);
    for value in [0., -1., f64::NAN, f64::INFINITY] {
        assert!(
            parse_gcode(
                "G1X1E1",
                &ParseOptions {
                    filament_diameter_mm: value,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
}

#[test]
fn g92_xyz_offsets_and_absolute_e_debt_survive_mode_changes() {
    let p = parse(
        "G90\nM82\nG1Z.2\n;WIDTH:0.42\nG1X10Y5E1\nG92X0Y0Z0E0\nG1E-1\nG1X10Y5Z0E-.5\nG1X20Y5Z0E.5",
    )
    .unwrap();
    assert_eq!(p.segments.len(), 2);
    near(p.segments[1].start[0], 25.);
    near(p.segments[1].end[0], 30.);
    near(p.segments[1].end[1], 10.);
    near(p.segments[1].end[2], 0.2);
    assert_eq!(p.segments[1].source_line, 9);
}

#[test]
fn parser_rejects_bad_metadata_backwards_layers_and_segment_budget() {
    let good = "G90\nM83\nG1Z.4\n;WIDTH:0.42\nG1X10E.4\n";
    for bad in [
        ";HEIGHT:NaN",
        ";HEIGHT:0",
        ";WIDTH:NaN",
        ";WIDTH:0.1\nG1X20E.4",
        "G1Z.2\nG1X20E.4",
        "G1E1e999",
    ] {
        assert!(parse(&format!("{good}{bad}")).is_err(), "accepted {bad}");
    }
    let mut text = String::from("G91\nM83\nG1Z.2\n;WIDTH:0.42\n");
    text.push_str(&"G1X1E.04\n".repeat(30_001));
    assert!(parse(&text).unwrap_err().to_string().contains("budget"));
}
