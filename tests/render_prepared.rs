#[path = "fixtures/renderers/common.rs"]
mod common;
use common::*;
use turninator::render::{
    contract::RenderLimits, geometry::realize_frame, prepared::write_geometry,
};

#[test]
fn prepared_wire_roundtrip_has_exact_counts_and_preserves_source() {
    let s = scene(cube([0.; 3], 10.));
    let r = realize_frame(&s, &frame(&s)).unwrap();
    let dir = turninator::export::encode::Scratch::new(&std::env::temp_dir()).unwrap();
    let path = dir.0.join("prepared.bin");
    let bytes = write_geometry(&path, &r, 2, &RenderLimits::default()).unwrap();
    let data = std::fs::read(&path).unwrap();
    assert_eq!(&data[..8], b"TNPRP001");
    assert_eq!(bytes as usize, data.len());
    assert_eq!(
        data.len(),
        24 + r.mesh.positions.len() * 36 + r.mesh.triangles.len() * 16
    );
    assert!(write_geometry(&path, &r, 2, &RenderLimits::default()).is_err());
}

#[test]
fn prepared_rejects_invalid_attributes_and_budget_before_creating_file() {
    let s = scene(cube([0.; 3], 10.));
    let mut r = realize_frame(&s, &frame(&s)).unwrap();
    let dir = turninator::export::encode::Scratch::new(&std::env::temp_dir()).unwrap();
    let path = dir.0.join("prepared.bin");
    let limits = RenderLimits {
        host_bytes: 1,
        ..Default::default()
    };
    assert!(write_geometry(&path, &r, 2, &limits).is_err());
    assert!(!path.exists());
    r.normals.pop();
    assert!(write_geometry(&path, &r, 2, &RenderLimits::default()).is_err());
    assert!(!path.exists());
}

#[test]
fn canonical_no_camera_fit_matches_legacy_70mm_sensor_and_preserves_explicit_camera() {
    let s = scene(cube([0.; 3], 10.));
    for (width, height) in [(512, 512), (800, 400), (400, 800)] {
        let camera = turninator::render::prepared::fit_legacy_camera(&s, width, height).unwrap();
        let sphere = (75f64).sqrt();
        let half = (36. / 140. * (height as f64 / width as f64).min(1.)).atan();
        let distance = sphere / half.sin() * 1.1;
        let actual = (0..3)
            .map(|i| (camera.position_mm[i] - camera.target_mm[i]).powi(2))
            .sum::<f64>()
            .sqrt();
        near(actual, distance);
        assert_eq!(camera.target_mm, [0., 0., 5.]);
    }
}

#[test]
#[ignore = "requires immutable pre-Task4b actual Blender v3 artifact; CPU encode-only, never loads renderer"]
fn old_v3_no_camera_encode_only_preserves_pixels_and_provenance() {
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use std::{fs, path::Path, sync::atomic::AtomicBool};
    use turninator::{
        export::{ExportRequest, pipeline::encode_only},
        render::ToolPaths,
    };
    let original = Path::new(
        "D:/code/turninator/.worktrees/selectable-renderers/artifacts/renderers/task-3/1792081112/jobs/job2",
    );
    let scratch = turninator::export::encode::Scratch::new(&std::env::temp_dir()).unwrap();
    let cache = turninator::cache::CacheStore::new(scratch.0.join("cache")).unwrap();
    let job = cache.create_job("old-v3").unwrap();
    fn copy_dir(source: &Path, target: &Path) {
        fs::create_dir_all(target).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let dest = target.join(entry.file_name());
            if entry.path().is_dir() {
                copy_dir(&entry.path(), &dest)
            } else {
                fs::copy(entry.path(), dest).unwrap();
            }
        }
    }
    copy_dir(original, &job);
    let original_request = fs::read(original.join("request.json")).unwrap();
    let mut request: ExportRequest = serde_json::from_slice(&original_request).unwrap();
    request.job_dir = job.clone();
    request.scene_path = job.join("scene.json");
    request.destination = scratch.0.join("recovered.png");
    let mut scene: Value =
        serde_json::from_slice(&fs::read(job.join("scene.json")).unwrap()).unwrap();
    assert!(scene.get("camera").is_none());
    scene["mesh_path"] = json!(job.join("mesh.bin"));
    let scene_bytes = serde_json::to_vec(&scene).unwrap();
    fs::write(&request.scene_path, &scene_bytes).unwrap();
    let manifest = job.join("f35ebd4ffc981b53028d/job.json");
    let mut state: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    let identity = state["recipe"]["identity"].clone();
    let plan = state["recipe"]["frame_plan"].clone();
    let key = state["recipe"]["pixel_recipe_key"].clone();
    // Relocation changes only the absolute source-path bytes and their file hash.
    // The pixel recipe deliberately excludes locations; retain its original key,
    // actual renderer identity, old camera plan, report and PNG bytes verbatim.
    state["recipe"]["scene_hash"] = json!(format!("{:x}", Sha256::digest(&scene_bytes)));
    fs::write(&manifest, serde_json::to_vec(&state).unwrap()).unwrap();
    let tools = ToolPaths {
        blender: "missing".into(),
        ffmpeg: "missing".into(),
        ffprobe: "missing".into(),
    };
    let result = encode_only(&tools, &request, &AtomicBool::new(false), &mut |_| {})
        .unwrap()
        .unwrap();
    assert_eq!(
        fs::read(&request.destination).unwrap(),
        fs::read(
            original
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("output-2.png")
        )
        .unwrap()
    );
    let after: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    assert_eq!(after["recipe"]["identity"], identity);
    assert_eq!(after["recipe"]["frame_plan"], plan);
    assert_eq!(after["recipe"]["pixel_recipe_key"], key);
    assert_eq!(
        fs::read(job.join("request.json")).unwrap(),
        original_request
    );
    println!(
        "Actual historical v3 encode-only recovered {:?}; unchanged identity/plan/key/request bytes",
        result
    );
}

#[test]
#[ignore = "writes bounded Blender qualification input fixtures; TN_PREPARED_QUALIFICATION_DIR must be a new absolute directory"]
fn generate_blender_prepared_qualification() {
    use serde_json::json;
    use turninator::{
        export::SectionSpec,
        mesh::Mesh,
        render::{contract::*, materials::resolve_materials},
    };
    let root = std::path::PathBuf::from(std::env::var("TN_PREPARED_QUALIFICATION_DIR").unwrap());
    assert!(root.is_absolute());
    std::fs::create_dir_all(root.parent().unwrap()).unwrap();
    std::fs::create_dir(&root).unwrap();
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/renderers");
    let hollow = Mesh::read_binary(&fixtures.join("hollow-10mm.mesh.bin")).unwrap();
    let mut open = cube([0.; 3], 10.);
    open.triangles.pop();
    let mut stepped = cube([0.; 3], 5.);
    append(&mut stepped, cube([0., 0., 5.], 5.));
    use turninator::print::beads::{BeadSegment, build_bead_mesh};
    let bead = |y| BeadSegment {
        start: [0., y, 0.1],
        end: [10., y, 0.1],
        width_mm: 0.4,
        height_mm: 0.2,
        volume_mm3: 0.7,
        source_line: 1,
        feature: "wall".into(),
        tool: 0,
        layer_index: 0,
        width_source: "fixture".into(),
        height_source: "fixture".into(),
    };
    let beads = build_bead_mesh(&[bead(0.), bead(1.)]).unwrap();
    let mut cases = Vec::new();
    for (name, mesh) in [
        ("solid", cube([0.; 3], 10.)),
        ("hollow", hollow),
        ("open", open),
        ("bead", beads),
        ("stepped", stepped),
    ] {
        for variant in ["full", "section"] {
            let directory = root.join(format!("{name}-{variant}"));
            std::fs::create_dir(&directory).unwrap();
            mesh.write_binary(&directory.join("source.bin")).unwrap();
            let mut value = scene(mesh.clone()).legacy_scene;
            value["mesh_path"] = json!(directory.join("source.bin"));
            if name == "bead" {
                value["geometry_kind"] = json!("beads");
                value["bead_layout"] = json!("independent_stadium_v1");
            }
            if name == "stepped" {
                value["geometry_kind"] = json!("stepped");
                value["layer_face_ranges"] = json!([
                {"start_triangle":0,"end_triangle":12,"layer_index":0,"z_top_mm":5.},
                {"start_triangle":12,"end_triangle":24,"layer_index":1,"z_top_mm":10.}]);
            }
            if variant == "section" {
                value["section"] = json!({"axis":0,"position_mm":if name=="stepped"{2.5}else{5.},"keep_positive":false,"cap":true});
            }
            let s = snapshot_scene(std::sync::Arc::new(mesh.clone()), value.clone()).unwrap();
            let mut f = frame(&s);
            f.section = value
                .get("section")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .unwrap();
            write_case(&directory, &s, &f, &mut cases);
        }
    }
    for name in [
        "mapped",
        "build-empty",
        "build-partial",
        "build-full",
        "build-section",
        "sheet",
        "layer-rotated",
        "cube-mask",
        "hollow-mask",
        "camera-wide",
        "camera-tall",
        "seam-closeup",
    ] {
        let directory = root.join(name);
        std::fs::create_dir(&directory).unwrap();
        let mut mesh = if name == "hollow-mask" {
            Mesh::read_binary(&fixtures.join("hollow-10mm.mesh.bin")).unwrap()
        } else {
            cube([0.; 3], 10.)
        };
        if name == "seam-closeup" {
            for p in &mut mesh.positions {
                p[0] += 0.1237 * p[2];
                p[1] += 0.4561 * p[2];
            }
        }
        mesh.write_binary(&directory.join("source.bin")).unwrap();
        let mesh = Mesh::read_binary(&directory.join("source.bin")).unwrap();
        let mut value = scene(mesh.clone()).legacy_scene;
        value["mesh_path"] = json!(directory.join("source.bin"));
        value["solid_layer_tops_mm"] = json!([5., 10.]);
        if name == "mapped" {
            value["face_groups"] = json!([{"start_triangle":0,"end_triangle":6,"tool":1,"label":"red","color":[0.8,0.03,0.03]}]);
        }
        if name == "sheet" {
            value["studio_preset"] = json!("diagram");
            value["camera"] = json!({"target_mm":[0.,0.,5.],"direction":[1.25,-1.7,1.],"up":[0.,0.,1.],"distance_mm":100.,"projection":"orthographic","vertical_fov_degrees":35.,"ortho_span_mm":16.});
        }
        if name == "layer-rotated" {
            value["layer_appearance"] = json!(true);
            value["display_rotation_degrees"] = json!([53., 19., 37.]);
        }
        if name.ends_with("mask") {
            value["camera"] = json!({"target_mm":[0.,0.,5.],"direction":[0.,-1.,0.],"up":[0.,0.,1.],"distance_mm":100.,"projection":"orthographic","vertical_fov_degrees":35.,"ortho_span_mm":16.});
            value["section"] = json!({"axis":1,"position_mm":5.,"keep_positive":true,"cap":true});
        }
        if name == "seam-closeup" {
            value["solid_layer_tops_mm"] = json!([3.7, 10.]);
            value["camera"] = json!({"target_mm":[5.2-11.237/2.,5.-14.561/2.,3.7],"direction":[1.,-1.,1.],"up":[0.,0.,1.],"distance_mm":100.,"projection":"orthographic","vertical_fov_degrees":35.,"ortho_span_mm":0.02});
        }
        let s = snapshot_scene(std::sync::Arc::new(mesh), value.clone()).unwrap();
        let mut f = frame(&s);
        f.visible_layers = match name {
            "build-empty" => Some(0),
            "build-partial" | "build-section" => Some(1),
            "build-full" => Some(2),
            _ => None,
        };
        if name == "seam-closeup" {
            f.visible_layers = Some(1);
            f.section = Some(SectionSpec {
                axis: 0,
                position_mm: 5.2,
                keep_positive: false,
                cap: true,
            });
            f.width = 1024;
            f.height = 1024;
        }
        if name == "build-section" {
            f.section = Some(SectionSpec {
                axis: 0,
                position_mm: 5.,
                keep_positive: false,
                cap: true,
            });
        }
        if name.ends_with("mask") {
            f.section = serde_json::from_value(value["section"].clone()).unwrap();
        }
        if name == "sheet" {
            f.key.panel = RenderPanel::Iso;
        }
        if name == "camera-wide" {
            f.width = 768;
            f.height = 384;
            f.camera =
                turninator::render::prepared::fit_legacy_camera(&s, f.width, f.height).unwrap();
        }
        if name == "camera-tall" {
            f.width = 384;
            f.height = 768;
            f.camera =
                turninator::render::prepared::fit_legacy_camera(&s, f.width, f.height).unwrap();
        }
        write_case(&directory, &s, &f, &mut cases);
    }
    // Review regressions use valid absolute BUILD-Z wire schedules, including
    // a source crossing Z=0 while all scheduled planes remain positive.
    for (sign, minimum) in [("positive", 10.), ("negative", -2.)] {
        for count in [0, 1, 2] {
            for section in [false, true] {
                let directory = root.join(format!("review-build-{sign}-{count}-section-{section}"));
                std::fs::create_dir(&directory).unwrap();
                let mesh = cube([0., 0., minimum], 10.);
                mesh.write_binary(&directory.join("source.bin")).unwrap();
                let mut value = scene(mesh.clone()).legacy_scene;
                value["mesh_path"] = json!(directory.join("source.bin"));
                value["solid_layer_tops_mm"] = json!([minimum + 5., minimum + 10.]);
                let s = snapshot_scene(std::sync::Arc::new(mesh), value).unwrap();
                let mut f = frame(&s);
                f.visible_layers = Some(count);
                f.section = section.then_some(SectionSpec {
                    axis: 0,
                    position_mm: 5.,
                    keep_positive: false,
                    cap: true,
                });
                write_case(&directory, &s, &f, &mut cases);
            }
        }
    }
    {
        let directory = root.join("review-rounded-materials");
        std::fs::create_dir(&directory).unwrap();
        let mesh = cube([0.; 3], 10.);
        mesh.write_binary(&directory.join("source.bin")).unwrap();
        let mut value = scene(mesh.clone()).legacy_scene;
        value["mesh_path"] = json!(directory.join("source.bin"));
        value["face_groups"] = json!([
            {"start_triangle":0,"end_triangle":2,"tool":1,"label":"first","color":[0.1,0.,0.]},
            {"start_triangle":2,"end_triangle":4,"tool":1,"label":"duplicate","color":[0.100000001,-0.,0.]},
            {"start_triangle":4,"end_triangle":8,"tool":1,"label":"later","color":[0.,0.7,0.]}
        ]);
        let s = snapshot_scene(std::sync::Arc::new(mesh), value).unwrap();
        let materials = resolve_materials(&s).unwrap();
        assert_eq!(materials.materials.len(), 4);
        assert_eq!(
            &materials.triangle_materials[..8],
            &[1, 1, 1, 1, 2, 2, 2, 2]
        );
        let mut f = frame(&s);
        f.section = Some(SectionSpec {
            axis: 0,
            position_mm: 5.,
            keep_positive: false,
            cap: true,
        });
        write_case(&directory, &s, &f, &mut cases);
    }
    fn write_case(
        directory: &std::path::Path,
        s: &RenderSceneSnapshot,
        f: &FrameSpec,
        cases: &mut Vec<serde_json::Value>,
    ) {
        let realized = realize_frame(s, f).unwrap();
        let materials = resolve_materials(s).unwrap();
        write_geometry(
            &directory.join("prepared.bin"),
            &realized,
            materials.materials.len(),
            &RenderLimits::default(),
        )
        .unwrap();
        let mut spec = s.legacy_scene.clone();
        spec["section"] = serde_json::to_value(&f.section).unwrap();
        std::fs::write(
            directory.join("scene.json"),
            serde_json::to_vec_pretty(&spec).unwrap(),
        )
        .unwrap();
        cases.push(json!({"name":directory.file_name().unwrap().to_string_lossy(),"directory":directory,"material_count":materials.materials.len(),"canonical_materials":materials.materials,"frame":f,
            "section":realized.analysis.map(|s|s.legacy),"build":realized.build_analysis.map(|s|s.legacy)}));
    }
    if let Ok(prefix) = std::env::var("TN_PREPARED_QUALIFICATION_PREFIX") {
        cases.retain(|case| case["name"].as_str().unwrap().starts_with(&prefix));
        assert!(
            !cases.is_empty(),
            "qualification prefix selected no fixtures"
        );
    }
    std::fs::write(
        root.join("cases.json"),
        serde_json::to_vec_pretty(&cases).unwrap(),
    )
    .unwrap();
}
