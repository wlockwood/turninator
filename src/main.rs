use std::{
    collections::BTreeMap,
    env,
    f64::consts::PI,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};
use turninator::{
    Result,
    model::stl::load_stl_with_report,
    print::{
        beads::build_bead_mesh,
        gcode::{ParseOptions, parse_gcode},
    },
};

const HELP: &str = "Turninator — print geometry foundation\n\n\
Commands:\n\
  turninator fixture --out <new-directory>\n\
  turninator prepare <input.stl|input.gcode> --out <new-job-directory>\n\n\
Prepare options:\n\
  --layer-height <mm>             Default 0.20\n\
  --first-layer-height <mm>       Defaults to layer height\n\
  --filament-diameter <mm>        Default 1.75\n\
  --millimeters-per-unit <scale>  STL only, default 1 (assumed mm)\n\n\
  --surface <clean|layers>        STL appearance, default clean\n\n\
G-code support is a constrained planar, single-tool Marlin subset.\n\
See README.md for unsupported commands and the Blender proof command.";

fn main() {
    if let Err(error) = run() {
        eprintln!("Turninator: {error}");
        std::process::exit(1);
    }
}

fn options(arguments: &[String]) -> Result<BTreeMap<String, String>> {
    if !arguments.len().is_multiple_of(2) {
        return Err("options require --name value pairs".into());
    }
    let mut values = BTreeMap::new();
    for pair in arguments.chunks_exact(2) {
        if !pair[0].starts_with("--") || values.insert(pair[0].clone(), pair[1].clone()).is_some() {
            return Err(format!("invalid or repeated option {}", pair[0]).into());
        }
    }
    Ok(values)
}

fn number(values: &BTreeMap<String, String>, key: &str, default: f64) -> Result<f64> {
    match values.get(key) {
        Some(value) => Ok(value
            .parse::<f64>()
            .map_err(|_| format!("{key} must be numeric"))?),
        None => Ok(default),
    }
}

fn new_directory(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(path)
        .map_err(|error| format!("use a new output directory {}: {error}", path.display()))?;
    Ok(())
}

fn run() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        return turninator::app::run(Default::default());
    }
    if matches!(args[0].as_str(), "--help" | "-h" | "help") {
        println!("{HELP}");
        return Ok(());
    }
    match args[0].as_str() {
        "gui" => {
            let values = options(&args[1..])?;
            if values.keys().any(|key| !["--open", "--data-dir", "--smoke-output"].contains(&key.as_str())) {
                return Err("gui accepts --open, --data-dir, and --smoke-output".into());
            }
            turninator::app::run(turninator::app::LaunchOptions {
                open: values.get("--open").map(PathBuf::from),
                data_directory: values.get("--data-dir").map(PathBuf::from),
                smoke_output: values.get("--smoke-output").map(PathBuf::from),
            })
        }
        "fixture" => {
            let values = options(&args[1..])?;
            if values.keys().any(|key| key != "--out") {
                return Err("fixture accepts only --out".into());
            }
            let output = PathBuf::from(values.get("--out").ok_or("--out is required")?);
            let fixture = fixture_gcode();
            new_directory(&output)?;
            fs::write(output.join("layer-demo.gcode"), fixture)?;
            fs::write(output.join("block.stl"), block_stl())?;
            println!("Generated {}", output.join("layer-demo.gcode").display());
            Ok(())
        }
        "prepare" => {
            let source = PathBuf::from(args.get(1).ok_or("prepare requires an input path")?);
            let values = options(&args[2..])?;
            for key in values.keys() {
                if ![
                    "--out",
                    "--layer-height",
                    "--first-layer-height",
                    "--filament-diameter",
                    "--millimeters-per-unit",
                    "--surface",
                ]
                .contains(&key.as_str())
                {
                    return Err(format!("unsupported option {key}").into());
                }
            }
            let output = PathBuf::from(values.get("--out").ok_or("--out is required")?);
            if output.exists() {
                return Err(format!(
                    "output already exists: {}; use a new directory",
                    output.display()
                )
                .into());
            }
            let height = number(&values, "--layer-height", 0.20)?;
            let first = number(&values, "--first-layer-height", height)?;
            turninator::print::schedule::LayerSchedule::uniform(height, first, height)?;
            let extension = source
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let surface = values
                .get("--surface")
                .map(String::as_str)
                .unwrap_or("clean");
            if !["clean", "layers"].contains(&surface)
                || (extension != "stl" && values.contains_key("--surface"))
            {
                return Err("--surface clean|layers applies only to STL".into());
            }
            let (mesh, geometry_kind, warnings, segment_count) = match extension.as_str() {
                "gcode" => {
                    if values.contains_key("--millimeters-per-unit") {
                        return Err(
                            "G-code already has physical units; --millimeters-per-unit is STL-only"
                                .into(),
                        );
                    }
                    if fs::metadata(&source)?.len() > 128 * 1024 * 1024 {
                        return Err("G-code exceeds the 128 MiB input budget".into());
                    }
                    let parsed = parse_gcode(
                        &fs::read_to_string(&source)?,
                        &ParseOptions {
                            layer_height_mm: height,
                            first_layer_height_mm: first,
                            filament_diameter_mm: number(&values, "--filament-diameter", 1.75)?,
                            ..Default::default()
                        },
                    )?;
                    let count = parsed.segments.len();
                    (
                        build_bead_mesh(&parsed.segments)?,
                        "beads",
                        parsed.warnings,
                        count,
                    )
                }
                "stl" => {
                    let scale = number(&values, "--millimeters-per-unit", 1.0)?;
                    let imported = load_stl_with_report(&source, scale)?;
                    let mut warnings = vec![format!(
                        "STL units interpreted as {scale} millimeters per source unit. {}",
                        if surface == "layers" {
                            "Layer Appearance is a shading approximation; source silhouette is preserved."
                        } else {
                            "Clean source silhouette; printed surface appearance disabled."
                        }
                    )];
                    if imported.omitted_degenerate_triangles > 0 {
                        warnings.push(format!("Omitted {} exactly zero-area triangles; no tolerance-based repair or simplification was applied.", imported.omitted_degenerate_triangles));
                    }
                    (imported.mesh, "original", warnings, 0)
                }
                _ => return Err("supported inputs are .stl and plain .gcode".into()),
            };
            let bounds = mesh.bounds()?;
            new_directory(&output)?;
            let mesh_path = output.join("mesh.bin");
            mesh.write_binary(&mesh_path)?;
            let scene = serde_json::json!({
                "schema_version": 1, "mesh_path": fs::canonicalize(&mesh_path)?,
                "geometry_kind": geometry_kind, "layer_height_mm": height,
                "first_layer_height_mm": first, "layer_appearance": surface == "layers",
                "groove_depth_mm": 0.05 * height.min(first),
                "bead_layout": if geometry_kind == "beads" { Some("independent_stadium_v1") } else { None },
                "color": [0.035, 0.22, 0.52], "roughness": 0.42,
                "display_rotation_degrees": [0.0, 0.0, 0.0], "warnings": warnings,
            });
            let temporary = output.join("scene.partial.json");
            fs::write(&temporary, serde_json::to_vec_pretty(&scene)?)?;
            fs::rename(temporary, output.join("scene.json"))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "scene": fs::canonicalize(output.join("scene.json"))?, "geometry_kind": geometry_kind,
                    "segments": segment_count, "vertices": mesh.positions.len(), "triangles": mesh.triangles.len(),
                    "warnings": scene["warnings"],
                    "bounds_mm": {"min": bounds.min, "max": bounds.max},
                }))?
            );
            Ok(())
        }
        _ => Err(format!("unknown command {}; run --help", args[0]).into()),
    }
}

fn fixture_gcode() -> String {
    let mut text = String::from(
        "; Turninator deterministic commanded-bead fixture\n; Synthetic paths, not slicer calibration\nG21\nG90\nM83\n;HEIGHT:0.2\n;WIDTH:0.44\n",
    );
    let filament_area = PI * 1.75_f64.powi(2) / 4.0;
    let bead_area = 0.2 * (0.44 - 0.2) + PI * 0.2_f64.powi(2) / 4.0;
    for layer in 0..120 {
        let z = (layer + 1) as f64 * 0.2;
        let phase = layer as f64 / 119.0;
        let radius = 13.0 + 3.0 * (2.0 * PI * phase).cos();
        writeln!(text, ";LAYER_CHANGE\n;Z:{z:.3}\n;TYPE:External perimeter").unwrap();
        for ring in 0..2 {
            let r = radius - ring as f64 * 0.42;
            let point = |i: usize| {
                let angle = 2.0 * PI * i as f64 / 64.0;
                [
                    r * angle.cos() + 1.6 * (2.0 * angle).cos(),
                    0.76 * r * angle.sin(),
                ]
            };
            let mut previous = point(0);
            writeln!(text, "G0 X{:.5} Y{:.5} Z{z:.3}", previous[0], previous[1]).unwrap();
            for index in 1..=64 {
                let next = point(index);
                let length = (next[0] - previous[0]).hypot(next[1] - previous[1]);
                let extrusion = length * bead_area / filament_area;
                writeln!(
                    text,
                    "G1 X{:.5} Y{:.5} E{extrusion:.7} F1800",
                    next[0], next[1]
                )
                .unwrap();
                previous = next;
            }
        }
    }
    text.push_str("G0 Z30\nM104 S0\nM140 S0\n");
    text
}

fn block_stl() -> String {
    let points = [
        [0, 0, 0],
        [20, 0, 0],
        [20, 12, 0],
        [0, 12, 0],
        [0, 0, 10],
        [20, 0, 10],
        [20, 12, 10],
        [0, 12, 10],
    ];
    let faces = [
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
    ];
    let mut text = String::from("solid block\n");
    for face in faces {
        text.push_str("facet normal 0 0 0\nouter loop\n");
        for index in face {
            let p = points[index];
            writeln!(text, "vertex {} {} {}", p[0], p[1], p[2]).unwrap();
        }
        text.push_str("endloop\nendfacet\n");
    }
    text.push_str("endsolid block\n");
    text
}
