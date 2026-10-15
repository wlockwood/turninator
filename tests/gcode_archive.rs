//! Opt-in compatibility check against a read-only local archive; no printer files are bundled.
use std::path::PathBuf;
use turninator::print::gcode::{ParseOptions, parse_gcode};

#[test]
#[ignore = "requires TURNINATOR_GCODE_ARCHIVE containing real slicer files"]
fn archived_slicer_files_parse() {
    let root = PathBuf::from(std::env::var_os("TURNINATOR_GCODE_ARCHIVE").unwrap());
    let mut failures = Vec::new();
    let mut checked = 0;
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "gcode") {
            continue;
        }
        let before = std::fs::read(&path).unwrap();
        checked += 1;
        let result = parse_gcode(
            std::str::from_utf8(&before).unwrap(),
            &ParseOptions::default(),
        );
        match result {
            Ok(data) => {
                if path.file_name().unwrap() == "HexShape PLA30v.gcode" {
                    let support = data
                        .segments
                        .iter()
                        .find(|s| s.source_line == 1999)
                        .unwrap();
                    assert!((support.height_mm - 0.316).abs() < 0.002);
                    assert!((support.width_mm - 0.35).abs() < 0.025);
                }
                if path.file_name().unwrap() == "fox variable height.gcode" {
                    let adaptive = data
                        .segments
                        .iter()
                        .find(|s| s.source_line == 22121)
                        .unwrap();
                    assert!((adaptive.height_mm - 0.2).abs() < 1e-6);
                }
                println!(
                    "{}: {} segments, {} layers",
                    path.display(),
                    data.segments.len(),
                    data.segments.last().unwrap().layer_index + 1
                );
            }
            Err(error) => failures.push(format!("{}: {error}", path.display())),
        }
        assert_eq!(before, std::fs::read(&path).unwrap());
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(checked > 0, "archive contained no G-code files");
}
