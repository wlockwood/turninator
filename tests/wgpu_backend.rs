#[test]
#[ignore = "qualified physical GPU required"]
fn native_studio_can_prepare_without_external_renderer() {
    use turninator::render::backends::wgpu::{WgpuBackend, WgpuConfig};
    assert!(
        WgpuBackend::new(&WgpuConfig::default()).is_ok(),
        "native studio PBR/output is missing"
    );
}

#[path = "fixtures/renderers/common.rs"]
mod common;
use common::*;
use std::{collections::BTreeMap, path::PathBuf, sync::atomic::AtomicBool};
use turninator::{
    export::frame_sink::FrameSink,
    render::{
        BackendId, RenderSelection,
        backends::wgpu::{WgpuBackend, WgpuConfig},
        contract::*,
    },
};

fn evidence() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/wgpu-evidence");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn front(s: &RenderSceneSnapshot) -> FrameSpec {
    let mut f = frame(s);
    f.width = 256;
    f.height = 256;
    f.camera = RenderCamera {
        position_mm: [0., -40., 5.],
        target_mm: [0., 0., 5.],
        up: [0., 0., 1.],
        projection: RenderProjection::Orthographic {
            vertical_span_mm: 20.,
        },
    };
    f
}
fn batch(
    backend: &mut WgpuBackend,
    s: &RenderSceneSnapshot,
    frames: Vec<FrameSpec>,
    transparent: bool,
    label: &str,
) -> (Vec<RgbaFrame>, RenderReport) {
    let identity = backend.identity();
    let selection = RenderSelection {
        backend: BackendId::Wgpu,
        device: Some(identity.device.clone()),
        quality: backend.quality().clone(),
    };
    let dir = evidence().join(format!("{label}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cancel = AtomicBool::new(false);
    let limits = RenderLimits::default();
    let ctx = RenderContext {
        cancel: &cancel,
        limits: &limits,
        scratch_dir: &dir,
        selection: &selection,
        identity: &identity,
        pixel_recipe_key: label,
        job_id: 6,
        revision: 1,
        progress: &|_| {},
    };
    let plan = FramePlan {
        frames,
        transparent,
        repeat_of: BTreeMap::new(),
    };
    let mut sink = FrameSink::new(&plan, &ctx).unwrap();
    let mut images = Vec::new();
    let mut metadata = Vec::new();
    let report = backend
        .render_sequence(s, &plan, &ctx, &mut |a| {
            if let FrameArtifact::Metadata {
                key,
                section,
                build,
            } = a
            {
                metadata.push((key, section, build));
                return Ok(());
            }
            if let FrameArtifact::Image { image, .. } = &a {
                images.push(image.clone());
            }
            sink.accept(a)?;
            Ok(())
        })
        .unwrap();
    sink.finish().unwrap();
    for (key, section, build) in metadata {
        assert_eq!(
            section.as_ref().map(|v| &v.legacy),
            report.analyses.get(&key).map(|v| &v.legacy)
        );
        assert_eq!(
            build.as_ref().map(|v| &v.legacy),
            report.build_analyses.get(&key).map(|v| &v.legacy)
        );
    }
    std::fs::write(
        dir.join("identity.json"),
        serde_json::to_vec_pretty(&identity).unwrap(),
    )
    .unwrap();
    println!(
        "{label}: {} frames {:?}",
        report.frame_count, report.phase_timings
    );
    std::fs::write(dir.join("metrics.json"),serde_json::to_vec_pretty(&serde_json::json!({"frames":report.frame_count,"timings_us":report.phase_timings.iter().map(|(k,v)|(k,v.as_micros())).collect::<BTreeMap<_,_>>(),"uploads_lifetime":backend.upload_count(),"alpha_bounds":images.iter().map(alpha_bounds).collect::<Vec<_>>(),"alpha_partial":images.iter().map(|i|i.bytes.chunks_exact(4).filter(|p|p[3]>0&&p[3]<255).count()).collect::<Vec<_>>()})).unwrap()).unwrap();
    (images, report)
}
fn difference(a: &RgbaFrame, b: &RgbaFrame) -> f64 {
    a.bytes
        .iter()
        .zip(&b.bytes)
        .map(|(a, b)| (*a as f64 - *b as f64).abs())
        .sum::<f64>()
        / a.bytes.len() as f64
}
fn alpha_bounds(a: &RgbaFrame) -> [u32; 4] {
    let mut b = [a.width, a.height, 0, 0];
    for y in 0..a.height {
        for x in 0..a.width {
            if a.bytes[((y * a.width + x) * 4 + 3) as usize] > 127 {
                b[0] = b[0].min(x);
                b[1] = b[1].min(y);
                b[2] = b[2].max(x);
                b[3] = b[3].max(y);
            }
        }
    }
    b
}

// These cases would fail if native pixels bypassed shared framing, used the
// old viewport gamma shader, omitted PBR/environment, retained previous poses,
// dropped metadata, or uploaded unchanged turntable geometry again.
#[test]
#[ignore = "exclusive qualified GPU batch; writes PNG evidence"]
fn gpu_physical_framing_alpha_history_and_upload_reuse() {
    let mut backend = WgpuBackend::new(&WgpuConfig::default()).unwrap();
    let s = scene(cube([0.; 3], 10.));
    let f = front(&s);
    let mut turn = f.clone();
    turn.key.index = 1;
    turn.angle_degrees = 37.;
    let mut again = f.clone();
    again.key.index = 2;
    let start = backend.upload_count();
    let (images, report) = batch(&mut backend, &s, vec![f, turn, again], true, "framing");
    assert_eq!(
        backend.upload_count() - start,
        1,
        "static geometry must upload once"
    );
    assert_eq!(report.frame_count, 3);
    assert_eq!(alpha_bounds(&images[0]), [64, 64, 191, 191]);
    assert_eq!(
        images[0].bytes, images[2].bytes,
        "each pose resets spatial accumulation"
    );
    assert!(difference(&images[0], &images[1]) > 2.);
    assert!(
        images[1]
            .bytes
            .chunks_exact(4)
            .any(|p| p[3] > 0 && p[3] < 255),
        "rotated pose has partially covered edges"
    );
    assert!(
        images[0]
            .bytes
            .chunks_exact(4)
            .filter(|p| p[3] == 0)
            .all(|p| p[..3] == [0, 0, 0])
    );
    assert!(report.identity.shader_digest.is_some());
    let mut shifted = front(&s);
    shifted.camera.position_mm[0] += 1.;
    shifted.camera.target_mm[0] += 1.;
    let (shifted, _) = batch(&mut backend, &s, vec![shifted], true, "physical-shift");
    let b = alpha_bounds(&shifted[0]);
    assert!((b[0] as i32 - 51).abs() <= 1);
    let mut perspective = front(&s);
    perspective.width = 512;
    perspective.camera.projection = RenderProjection::Perspective {
        vertical_fov_degrees: 30.,
    };
    let (perspective, _) = batch(
        &mut backend,
        &s,
        vec![perspective],
        true,
        "perspective-aspect",
    );
    let bounds = alpha_bounds(&perspective[0]);
    for (got, expected) in bounds.into_iter().zip([188, 60, 323, 195]) {
        assert!(
            (got as i32 - expected).abs() <= 1,
            "physical perspective projection {bounds:?}"
        );
    }
}

#[test]
#[ignore = "exclusive qualified GPU batch; writes PNG evidence"]
fn gpu_pbr_layers_studios_mapped_colors_and_metadata() {
    let mut backend = WgpuBackend::new(&WgpuConfig::default()).unwrap();
    let mut s = scene(cube([0.; 3], 10.));
    let mut f = front(&s);
    let (plain, _) = batch(&mut backend, &s, vec![f.clone()], false, "matte-light");
    assert!(plain[0].bytes.chunks_exact(4).all(|p| p[3] == 255));
    s.appearance.studio_preset = "dark".into();
    let (dark, _) = batch(&mut backend, &s, vec![f.clone()], false, "matte-dark");
    assert!(difference(&plain[0], &dark[0]) > 12.);
    s.appearance.studio_preset = "light".into();
    s.appearance.material_preset = "silk_pla".into();
    s.appearance.roughness = 0.12;
    let (silk, _) = batch(&mut backend, &s, vec![f.clone()], false, "silk");
    assert!(difference(&plain[0], &silk[0]) > 0.5);
    s.appearance.roughness = 0.85;
    let (rough, _) = batch(&mut backend, &s, vec![f.clone()], false, "silk-rough");
    assert!(difference(&silk[0], &rough[0]) > 0.5);
    s.appearance.material_preset = "matte_pla".into();
    s.appearance.roughness = 0.45;
    s.appearance.layer_appearance = true;
    s.appearance.groove_depth_mm = 0.04;
    let (far, _) = batch(&mut backend, &s, vec![f.clone()], false, "layers-fade");
    assert!(
        difference(&plain[0], &far[0]) < 0.1,
        "subpixel layers must fade"
    );
    f.camera.projection = RenderProjection::Orthographic {
        vertical_span_mm: 4.,
    };
    let (close, _) = batch(&mut backend, &s, vec![f.clone()], true, "layers-close");
    s.appearance.layer_appearance = false;
    let (close_plain, _) = batch(
        &mut backend,
        &s,
        vec![f.clone()],
        true,
        "layers-close-plain",
    );
    assert!(difference(&close[0], &close_plain[0]) > 1.);
    s.metadata.face_groups = vec![
        FaceGroup {
            start_triangle: 4,
            end_triangle: 5,
            tool: 0,
            label: "red".into(),
            color: [1., 0.01, 0.01],
        },
        FaceGroup {
            start_triangle: 5,
            end_triangle: 6,
            tool: 1,
            label: "green".into(),
            color: [0.01, 1., 0.01],
        },
    ];
    let (mapped, _) = batch(&mut backend, &s, vec![front(&s)], true, "mapped");
    let red = mapped[0]
        .bytes
        .chunks_exact(4)
        .filter(|p| p[0] > p[1].saturating_add(30))
        .count();
    let green = mapped[0]
        .bytes
        .chunks_exact(4)
        .filter(|p| p[1] > p[0].saturating_add(30))
        .count();
    assert!(red > 3000 && green > 3000);
    s.metadata.solid_layer_tops_mm = vec![5., 10.];
    let mut stage = front(&s);
    stage.visible_layers = Some(1);
    stage.section = Some(turninator::export::SectionSpec {
        axis: 0,
        position_mm: 5.,
        keep_positive: false,
        cap: true,
    });
    let (stage_images, r) = batch(&mut backend, &s, vec![stage], true, "stage-section");
    assert_eq!(alpha_bounds(&stage_images[0]), [64, 128, 127, 191]);
    assert_eq!(r.build_analyses.len(), 1);
    assert_eq!(r.analyses.len(), 1);
    let mut empty = front(&s);
    empty.visible_layers = Some(0);
    let (empty, r) = batch(&mut backend, &s, vec![empty], true, "empty");
    assert!(empty[0].bytes.iter().all(|&b| b == 0));
    assert_eq!(r.build_analyses.len(), 1);
    s.appearance.material_preset = "resin".into();
    let (resin, r) = batch(&mut backend, &s, vec![front(&s)], true, "resin");
    assert!(r.warnings.iter().any(|w| w.contains("approximation")));
    assert!(
        resin[0]
            .bytes
            .chunks_exact(4)
            .any(|p| p[3] > 10 && p[3] < 240)
    );
}

#[test]
fn quality_recipes_and_shader_validation_are_explicit() {
    use turninator::render::{
        BackendQuality,
        selection::{AaQuality, ShadowQuality},
        wgpu_studio::*,
    };
    for (aa, shadows, samples, size, ao) in [
        (AaQuality::Low, ShadowQuality::Low, 1, 1024, 8),
        (AaQuality::Medium, ShadowQuality::Medium, 4, 2048, 16),
        (AaQuality::High, ShadowQuality::High, 8, 4096, 32),
    ] {
        let q = StudioQuality::resolve(&BackendQuality::Wgpu { aa, shadows }).unwrap();
        assert_eq!(
            (q.samples, q.shadow_size, q.ao_samples),
            (samples, size, ao)
        );
    }
    assert!(
        StudioQuality::resolve(&BackendQuality::Cycles {
            samples: 8,
            denoise: true
        })
        .is_err()
    );
    for source in [STUDIO_SHADER, OUTPUT_SHADER] {
        let source = format!("{}\n{source}", turninator::render::color::AGX_WGSL);
        let module = eframe::wgpu::naga::front::wgsl::parse_str(&source).unwrap();
        eframe::wgpu::naga::valid::Validator::new(
            eframe::wgpu::naga::valid::ValidationFlags::all(),
            eframe::wgpu::naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
    }
}

#[test]
#[ignore = "exclusive qualified GPU batch"]
fn gpu_frozen_agx_chart_and_straight_alpha_output() {
    let mut backend = WgpuBackend::new(&WgpuConfig::default()).unwrap();
    let chart: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/renderers/color-chart.json")).unwrap();
    let cancel = AtomicBool::new(false);
    let mut worst = 0i32;
    for p in chart["patches"].as_array().unwrap() {
        let rgba = std::array::from_fn(|i| p["scene_rgba"][i].as_f64().unwrap() as f32);
        let recipe = turninator::render::color::AgxRecipe {
            exposure_ev: p["exposure_ev"].as_f64().unwrap() as f32,
        };
        let out = backend
            .studio_mut()
            .encode_hdr(17, 16, &vec![rgba; 17 * 16], recipe, &cancel)
            .unwrap();
        let expected: [u8; 4] = std::array::from_fn(|i| {
            if i < 3 && rgba[3] == 0. {
                0
            } else {
                (p["display_srgb_rgba"][i].as_f64().unwrap() * 255.).round() as u8
            }
        });
        for pixel in out.bytes.chunks_exact(4) {
            for i in 0..4 {
                let error = (pixel[i] as i32 - expected[i] as i32).abs();
                worst = worst.max(error);
                assert!(
                    error <= 1,
                    "{} channel{i} {pixel:?} != {expected:?}",
                    p["name"]
                );
            }
        }
    }
    println!("frozen GPU AgX chart max error = {worst} code values");
    assert!(
        backend
            .studio_mut()
            .check_budget(512, 512, u64::MAX, &RenderLimits::default())
            .is_err()
    );
    let cancelled = AtomicBool::new(true);
    assert!(
        backend
            .studio_mut()
            .encode_hdr(
                1,
                1,
                &[[0.18, 0.18, 0.18, 1.]],
                Default::default(),
                &cancelled
            )
            .is_err()
    );
    assert!(
        backend
            .studio_mut()
            .check_budget(8000, 8000, 0, &RenderLimits::default())
            .is_err()
    );
    assert!(
        backend
            .studio_mut()
            .check_budget(4096, 4096, 0, &RenderLimits::default())
            .is_ok()
    );
    assert!(
        backend
            .studio_mut()
            .check_budget(
                512,
                512,
                0,
                &RenderLimits {
                    gpu_bytes: 1,
                    ..Default::default()
                }
            )
            .is_err()
    );
    let mut constrained = WgpuBackend::new(&WgpuConfig {
        limits: RenderLimits {
            host_bytes: 10_000,
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    assert!(
        constrained
            .studio_mut()
            .encode_hdr(
                17,
                16,
                &vec![[0.18, 0.18, 0.18, 1.]; 17 * 16],
                Default::default(),
                &cancel
            )
            .unwrap_err()
            .to_string()
            .contains("host budget")
    );
}

#[test]
#[ignore = "exclusive qualified GPU batch; common pipeline and 4K PNG evidence"]
fn gpu_common_pipeline_png_sheet_stage_and_4k_without_tools() {
    use turninator::{
        export::{
            BuildStage, ExportKind, ExportRequest, identity::pixel_recipe_key,
            pipeline::run_export_with_backend,
        },
        render::ToolPaths,
    };
    let mut backend = WgpuBackend::new(&WgpuConfig::default()).unwrap();
    let identity = backend.identity();
    let root = evidence().join(format!("pipeline-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let cache = turninator::cache::CacheStore::new(root.join("cache")).unwrap();
    for (name, kind, size, transparent, stage) in [
        ("opaque", ExportKind::Still, 512, false, None),
        ("transparent", ExportKind::Still, 512, true, None),
        ("sheet", ExportKind::Diagram, 1024, true, None),
        (
            "stage",
            ExportKind::Still,
            512,
            true,
            Some(BuildStage { visible_layers: 1 }),
        ),
        ("still4k", ExportKind::Still, 4096, false, None),
    ] {
        let job = cache.create_job(name).unwrap();
        let mut s = scene(cube([0.; 3], 10.));
        s.mesh.write_binary(&job.join("mesh.bin")).unwrap();
        s.legacy_scene["mesh_path"] = serde_json::json!(job.join("mesh.bin"));
        s.legacy_scene["solid_layer_tops_mm"] = serde_json::json!([5., 10.]);
        if kind == ExportKind::Diagram {
            s.legacy_scene["studio_preset"] = serde_json::json!("diagram");
        }
        std::fs::write(
            job.join("scene.json"),
            serde_json::to_vec(&s.legacy_scene).unwrap(),
        )
        .unwrap();
        let selection = RenderSelection {
            backend: BackendId::Wgpu,
            device: Some(identity.device.clone()),
            quality: backend.quality().clone(),
        };
        let request = ExportRequest {
            job_dir: job.clone(),
            scene_path: job.join("scene.json"),
            destination: root.join(format!("{name}.png")),
            kind,
            size,
            transparent,
            selection: selection.clone(),
            build_stage: stage,
            section: stage.map(|_| turninator::export::SectionSpec {
                axis: 0,
                position_mm: 5.,
                keep_positive: false,
                cap: true,
            }),
            ..Default::default()
        };
        let key = pixel_recipe_key(
            &s.legacy_scene,
            &s.mesh.snapshot_hash().unwrap(),
            &request,
            &identity,
        )
        .unwrap();
        let limits = RenderLimits::default();
        let cancel = AtomicBool::new(false);
        let ctx = RenderContext {
            cancel: &cancel,
            limits: &limits,
            scratch_dir: &job,
            selection: &selection,
            identity: &identity,
            pixel_recipe_key: &key,
            job_id: 6,
            revision: 1,
            progress: &|_| {},
        };
        let tools = ToolPaths {
            blender: "missing-blender".into(),
            ffmpeg: "missing-ffmpeg".into(),
            ffprobe: "missing-ffprobe".into(),
        };
        let start = std::time::Instant::now();
        let uploads = backend.upload_count();
        run_export_with_backend(&mut backend, &tools, &request, &ctx, &mut |_| {}).unwrap();
        println!(
            "common pipeline {name} elapsed {:?}, uploads {}",
            start.elapsed(),
            backend.upload_count() - uploads
        );
        let image = image::open(&request.destination).unwrap().to_rgba8();
        assert_eq!((image.width(), image.height()), (size, size));
        assert!(image.pixels().any(|p| p[0] != p[1]));
        if !transparent || kind == ExportKind::Diagram {
            assert!(image.pixels().all(|p| p[3] == 255));
        } else {
            assert!(image.pixels().any(|p| p[3] == 0));
        }
        if kind == ExportKind::Diagram {
            assert_eq!(backend.upload_count() - uploads, 4);
        }
        let uploads = backend.upload_count();
        run_export_with_backend(&mut backend, &tools, &request, &ctx, &mut |_| {}).unwrap();
        assert_eq!(
            uploads,
            backend.upload_count(),
            "verified retained pixels must be reused"
        );
        if stage.is_some() {
            let manifest = std::fs::read_dir(&job)
                .unwrap()
                .map(|e| e.unwrap().path().join("job.json"))
                .find(|p| p.is_file())
                .unwrap();
            let text = std::fs::read_to_string(manifest).unwrap();
            assert!(text.contains("remaining_volume_mm3"));
            assert!(text.contains("z_top_mm"));
        }
    }
    // Animation frames use the same native producer + common frame sink; no
    // external encoder is required merely to generate the 960-square frames.
    let s = scene(cube([0.; 3], 10.));
    let frames = (0..8)
        .map(|i| {
            let mut f = frame(&s);
            f.width = 960;
            f.height = 960;
            f.key.index = i;
            f.angle_degrees = i as f64 * 45.;
            f
        })
        .collect();
    let uploads = backend.upload_count();
    let (images, _) = batch(&mut backend, &s, frames, false, "animation960");
    assert_eq!(images.len(), 8);
    assert_eq!(backend.upload_count() - uploads, 1);
}

#[test]
#[ignore = "exclusive qualified GPU batch; frozen masks and unlit material slot 258"]
fn gpu_frozen_cube_hollow_masks_and_unlit_material_capacity() {
    use turninator::render::{
        BackendQuality,
        color::AgxRecipe,
        geometry::*,
        materials::resolve_materials,
        selection::{AaQuality, ShadowQuality},
    };
    let config = WgpuConfig {
        quality: BackendQuality::Wgpu {
            aa: AaQuality::Low,
            shadows: ShadowQuality::Low,
        },
        ..Default::default()
    };
    let mut backend = WgpuBackend::new(&config).unwrap();
    let limits = RenderLimits::default();
    let cancel = AtomicBool::new(false);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/renderers");
    for name in ["cube", "hollow"] {
        let s = scene(
            turninator::mesh::Mesh::read_binary(&root.join(format!("{name}-10mm.mesh.bin")))
                .unwrap(),
        );
        let mut f = front(&s);
        f.width = 512;
        f.height = 512;
        f.camera.projection = RenderProjection::Orthographic {
            vertical_span_mm: 16.,
        };
        f.section = Some(turninator::export::SectionSpec {
            axis: 1,
            position_mm: 5.,
            keep_positive: true,
            cap: true,
        });
        let mut geometry = realize_frame(&s, &f).unwrap();
        let mut materials = resolve_materials(&s).unwrap();
        let mut unlit = materials.materials[0].clone();
        unlit.unlit = true;
        unlit.base_color_linear = [0.18, 0.07, 0.35];
        materials.materials = vec![unlit.clone(); 259];
        geometry.triangle_materials.fill(258);
        let uploaded = backend
            .studio_mut()
            .upload(&geometry, &materials, &limits)
            .unwrap();
        let (out, _, _) = backend
            .studio_mut()
            .render(&s, &f, &uploaded, true, uploaded.bytes, &limits, &cancel)
            .unwrap();
        let expected = image::open(root.join(format!("{name}-mask-512.png")))
            .unwrap()
            .to_luma8();
        let mut intersection = 0;
        let mut union = 0;
        let mut changed = 0;
        for (p, &want) in out.bytes.chunks_exact(4).zip(expected.as_raw()) {
            let a = p[3] > 0;
            let b = want > 0;
            intersection += usize::from(a && b);
            union += usize::from(a || b);
            changed += usize::from(a != b);
        }
        let iou = intersection as f64 / union as f64;
        println!("{name} mask IoU={iou}, different pixels={changed}");
        assert!(iou >= 0.995);
        assert_eq!(changed, 0);
        assert_eq!(
            out.bytes[(256 * 512 + 256) * 4 + 3],
            if name == "hollow" { 0 } else { 255 }
        );
        let encoded = AgxRecipe::default()
            .map_srgb_rgba([0.18, 0.07, 0.35, 1.])
            .unwrap()
            .map(|v| (v * 255.).round() as u8);
        for p in out.bytes.chunks_exact(4).filter(|p| p[3] == 255) {
            for i in 0..4 {
                assert!(
                    (p[i] as i32 - encoded[i] as i32).abs() <= 1,
                    "unlit must bypass all studio lighting"
                );
            }
        }
        image::save_buffer(
            evidence().join(format!("{name}-native-mask.png")),
            &out.bytes,
            out.width,
            out.height,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
}

#[test]
#[ignore = "exclusive qualified GPU batch; isolated shadow and contact AO probes"]
fn gpu_cast_floor_shadow_and_unshadowed_contact_ao() {
    use turninator::render::{geometry::realize_frame, materials::resolve_materials};
    let mut backend = WgpuBackend::new(&WgpuConfig::default()).unwrap();
    let studio = backend.studio_mut();
    let limits = RenderLimits::default();
    let cancel = AtomicBool::new(false);
    let s = scene(cube([0.; 3], 10.));
    let mut f = frame(&s);
    f.camera = RenderCamera {
        position_mm: [0., -40., 30.],
        target_mm: [0., 0., 5.],
        up: [0., 0., 1.],
        projection: RenderProjection::Orthographic {
            vertical_span_mm: 30.,
        },
    };
    let materials = resolve_materials(&s).unwrap();
    let mut geometry = realize_frame(&s, &f).unwrap();
    let solid = studio.upload(&geometry, &materials, &limits).unwrap();
    let (lit, _, _) = studio
        .render(&s, &f, &solid, false, solid.bytes, &limits, &cancel)
        .unwrap();
    geometry.mesh = std::sync::Arc::new(turninator::mesh::Mesh::default());
    geometry.normals.clear();
    geometry.build_positions_mm.clear();
    geometry.triangle_materials.clear();
    let empty = studio.upload(&geometry, &materials, &limits).unwrap();
    let (floor, _, _) = studio
        .render(
            &s,
            &f,
            &empty,
            false,
            solid.bytes + empty.bytes,
            &limits,
            &cancel,
        )
        .unwrap();
    let probe = |image: &RgbaFrame, x: f64, y: f64| {
        let px = ((x / 30. + 0.5) * 512.) as usize;
        let view_y = (25. * y - 40. * 5.) / 2225f64.sqrt();
        let py = ((0.5 - view_y / 30.) * 512.) as usize;
        let p = &image.bytes[(py * 512 + px) * 4..];
        (p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.
    };
    let shadow = probe(&floor, 6., 7.) - probe(&lit, 6., 7.);
    let contact = probe(&floor, 0., -5.2) - probe(&lit, 0., -5.2);
    let far = probe(&floor, 0., -10.) - probe(&lit, 0., -10.);
    println!(
        "floor shadow delta={shadow}, unshadowed front-contact AO delta={contact}, far-ground delta={far}"
    );
    let deltas = (shadow, contact, far);
    for (name, image) in [("shadow-contact", lit), ("floor-reference", floor)] {
        image::save_buffer(
            evidence().join(format!("{name}.png")),
            &image.bytes,
            image.width,
            image.height,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
    assert!(
        deltas.0 > 10.,
        "caster must darken ground in its light-projected shadow"
    );
    assert!(
        deltas.1 > 2.,
        "AO must darken a contact on the key-facing unshadowed side"
    );
    assert!(deltas.2.abs() < 1., "unoccluded far floor is unchanged");
}

#[test]
#[ignore = "exclusive qualified GPU; constant planar lighting regression"]
fn gpu_planar_face_has_no_false_shadow_bands() {
    let mut backend = WgpuBackend::new(&WgpuConfig::default()).unwrap();
    let s = scene(cube([0.; 3], 10.));
    let (images, _) = batch(&mut backend, &s, vec![front(&s)], true, "planar-bands");
    let image = &images[0];
    let mut low = [255u8; 3];
    let mut high = [0u8; 3];
    for y in 100..156 {
        for x in 100..156 {
            let p = &image.bytes[(y * 256 + x) * 4..];
            for c in 0..3 {
                low[c] = low[c].min(p[c]);
                high[c] = high[c].max(p[c]);
            }
        }
    }
    let ranges: Vec<_> = (0..3).map(|c| high[c] - low[c]).collect();
    println!("unoccluded planar orthographic face RGB ranges={ranges:?}");
    assert!(
        ranges.iter().all(|&r| r <= 1),
        "constant normal/view unoccluded face must not acquire shadow/AO bands"
    );
}

#[test]
#[ignore = "exclusive qualified GPU; actual stadium beads and sparse whole-layer stages"]
fn gpu_actual_mapped_bead_strands_preserve_gap_and_stage() {
    use turninator::print::beads::{BeadSegment, build_bead_mesh};
    let bead = |y, z, tool| BeadSegment {
        start: [0., y, z],
        end: [10., y, z],
        width_mm: 0.42,
        height_mm: 0.2,
        volume_mm3: 0.,
        source_line: 1,
        feature: "wall".into(),
        tool,
        layer_index: tool,
        width_source: "fixture".into(),
        height_source: "fixture".into(),
    };
    let mesh = build_bead_mesh(&[bead(0., 0.2, 0), bead(1., 0.4, 1)]).unwrap();
    let half = mesh.triangles.len() as u32 / 2;
    let mut s = scene(mesh);
    s.metadata.kind = GeometryKind::Beads;
    s.metadata.bead_layout = Some("independent_stadium_v1".into());
    s.metadata.layer_ranges = vec![
        LayerRange {
            start_triangle: 0,
            end_triangle: half,
            layer_index: 2,
            z_top_mm: 0.2,
        },
        LayerRange {
            start_triangle: half,
            end_triangle: half * 2,
            layer_index: 7,
            z_top_mm: 0.4,
        },
    ];
    s.metadata.face_groups = vec![
        FaceGroup {
            start_triangle: 0,
            end_triangle: half,
            tool: 0,
            label: "red strand".into(),
            color: [1., 0.01, 0.01],
        },
        FaceGroup {
            start_triangle: half,
            end_triangle: half * 2,
            tool: 1,
            label: "green strand".into(),
            color: [0.01, 1., 0.01],
        },
    ];
    let mut f = front(&s);
    f.width = 512;
    f.camera = RenderCamera {
        position_mm: [0., 0., 10.],
        target_mm: [0., 0., 0.2],
        up: [0., 1., 0.],
        projection: RenderProjection::Orthographic {
            vertical_span_mm: 6.,
        },
    };
    let mut backend = WgpuBackend::new(&WgpuConfig::default()).unwrap();
    let (images, _) = batch(
        &mut backend,
        &s,
        vec![f.clone()],
        true,
        "mapped-bead-strands",
    );
    let colors = |image: &RgbaFrame| {
        (
            image
                .bytes
                .chunks_exact(4)
                .filter(|p| p[0] > p[1].saturating_add(30))
                .count(),
            image
                .bytes
                .chunks_exact(4)
                .filter(|p| p[1] > p[0].saturating_add(30))
                .count(),
        )
    };
    let (red, green) = colors(&images[0]);
    assert!(
        red > 1000 && green > 1000,
        "both mapped strands visible: {red}/{green}"
    );
    assert_eq!(
        images[0].bytes[(128 * 512 + 256) * 4 + 3],
        0,
        "physical inter-strand gap stays open"
    );
    f.visible_layers = Some(1);
    let (images, report) = batch(&mut backend, &s, vec![f], true, "mapped-bead-stage");
    let (red, green) = colors(&images[0]);
    assert!(red > 1000);
    assert_eq!(green, 0);
    assert_eq!(
        report.build_analyses.values().next().unwrap().legacy["visible_layer"],
        2
    );
}

#[test]
#[ignore = "exclusive qualified GPU; production PBR and build-space layer visual qualification"]
fn gpu_specular_and_posed_layer_showcase() {
    // A closed fine-tessellation sphere fixture exercises changing surface/view
    // directions. It uses the canonical realizer, not a special renderer shader.
    let (rings, columns) = (48u32, 96u32);
    let mut mesh = turninator::mesh::Mesh {
        positions: vec![[0., 0., 10.], [0., 0., 0.]],
        triangles: vec![],
    };
    for i in 1..rings {
        let theta = std::f64::consts::PI * i as f64 / rings as f64;
        for j in 0..columns {
            let phi = std::f64::consts::TAU * j as f64 / columns as f64;
            mesh.positions.push([
                5. * theta.sin() * phi.cos(),
                5. * theta.sin() * phi.sin(),
                5. + 5. * theta.cos(),
            ]);
        }
    }
    for j in 0..columns {
        let next = (j + 1) % columns;
        mesh.triangles.push([0, 2 + j, 2 + next]);
        let last = 2 + (rings - 2) * columns;
        mesh.triangles.push([1, last + next, last + j]);
        for i in 0..rings - 2 {
            let a = 2 + i * columns + j;
            let an = 2 + i * columns + next;
            mesh.triangles
                .extend([[a, a + columns, an], [an, a + columns, an + columns]]);
        }
    }
    let mut s = scene(mesh);
    let mut backend = WgpuBackend::new(&WgpuConfig::default()).unwrap();
    let mut f = front(&s);
    f.width = 512;
    f.height = 512;
    f.camera.projection = RenderProjection::Orthographic {
        vertical_span_mm: 13.,
    };
    let mut images = Vec::new();
    for (preset, rough, label) in [
        ("matte_pla", 0.65, "sphere-matte"),
        ("matte_pla", 0.08, "sphere-smooth"),
        ("glossy_plastic", 0.08, "sphere-coat"),
        ("silk_pla", 0.12, "sphere-silk"),
        ("silk_pla", 0.85, "sphere-silk-rough"),
    ] {
        s.appearance.material_preset = preset.into();
        s.appearance.roughness = rough;
        images.push(
            batch(&mut backend, &s, vec![f.clone()], false, label)
                .0
                .remove(0),
        );
    }
    for pair in images.windows(2) {
        assert!(difference(&pair[0], &pair[1]) > 0.1);
    }
    let mut s = scene(cube([0.; 3], 10.));
    s.appearance.layer_appearance = true;
    s.appearance.groove_depth_mm = 0.04;
    s.appearance.first_layer_height_mm = 0.4;
    let mut f = front(&s);
    f.camera.projection = RenderProjection::Orthographic {
        vertical_span_mm: 4.,
    };
    let (upright, _) = batch(
        &mut backend,
        &s,
        vec![f.clone()],
        true,
        "layers-later-qualified",
    );
    let energy = |image: &RgbaFrame| {
        let mut horizontal = 0u64;
        let mut vertical = 0u64;
        for y in 32..224 {
            for x in 32..224 {
                let at = (y * 256 + x) * 4;
                horizontal +=
                    (image.bytes[at] as i32 - image.bytes[at + 4] as i32).unsigned_abs() as u64;
                vertical += (image.bytes[at] as i32 - image.bytes[at + 256 * 4] as i32)
                    .unsigned_abs() as u64;
            }
        }
        (horizontal, vertical)
    };
    let (h, v) = energy(&upright[0]);
    assert!(v > h * 10 && v > 1000);
    s.appearance.display_rotation_degrees = [0., 90., 0.];
    let (posed, _) = batch(
        &mut backend,
        &s,
        vec![f.clone()],
        true,
        "layers-posed-qualified",
    );
    let (h, v) = energy(&posed[0]);
    assert!(h > v * 10 && h > 1000);
    s.appearance.display_rotation_degrees = [0.; 3];
    f.camera.position_mm[2] = 0.6;
    f.camera.target_mm[2] = 0.6;
    f.camera.projection = RenderProjection::Orthographic {
        vertical_span_mm: 1.2,
    };
    batch(&mut backend, &s, vec![f], true, "layers-first-qualified");
    let f = front(&s);
    let (fade, _) = batch(
        &mut backend,
        &s,
        vec![f.clone()],
        true,
        "layers-faded-qualified",
    );
    s.appearance.layer_appearance = false;
    let (plain, _) = batch(&mut backend, &s, vec![f], true, "layers-faded-reference");
    assert!(difference(&fade[0], &plain[0]) < 0.1);
}
