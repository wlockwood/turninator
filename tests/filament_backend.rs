//! Actual production bridge integration. Run the ignored test from an isolated
//! executable fixture containing a verified renderer package; no fixture DLLs.
#[path = "fixtures/renderers/common.rs"]
mod common;
use common::*;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
use turninator::{
    Result,
    export::{
        ExportKind, ExportLayout, ExportRequest, SectionSpec, pipeline::run_export_with_backend,
    },
    render::{
        ToolPaths, backends::filament::FilamentBackend, contract::*, embedded::EmbeddedConfig,
        selection::*,
    },
};

fn config() -> EmbeddedConfig {
    EmbeddedConfig {
        selection: RenderSelection {
            backend: BackendId::Filament,
            device: None,
            quality: BackendQuality::Filament {
                aa: AaQuality::High,
                shadows: ShadowQuality::High,
            },
        },
        limits: RenderLimits {
            host_bytes: 8 * 1024 * 1024 * 1024,
            gpu_bytes: 10 * 1024 * 1024 * 1024,
            ..Default::default()
        },
        identity: BackendIdentity {
            backend: BackendId::Filament,
            adapter_recipe_version: 1,
            runtime_digest: "untrusted caller placeholder".into(),
            bridge_digest: None,
            shader_digest: None,
            material_digest: None,
            color_digest: "untrusted".into(),
            denoiser_digest: None,
            device: "untrusted".into(),
            api: "untrusted".into(),
            driver: "untrusted".into(),
        },
        cache_directory: None,
    }
}
#[test]
fn discovery_is_lazy_and_wrong_selection_is_rejected_without_sdk() {
    assert!(matches!(
        FilamentBackend::status().availability,
        turninator::render::capabilities::BackendAvailability::NotProbed
    ));
    let mut c = config();
    c.selection = RenderSelection::for_backend(BackendId::Wgpu, false);
    assert!(
        FilamentBackend::new(&c)
            .err()
            .unwrap()
            .to_string()
            .contains("Filament")
    );
}

#[test]
#[ignore = "requires exclusive GPU grant and verified production package"]
fn production_package_identity_and_distinct_tokens_smoke() -> Result<()> {
    let output = PathBuf::from(std::env::var("FILAMENT_TEST_OUTPUT")?);
    std::fs::create_dir_all(&output)?;
    let mut backend = FilamentBackend::new(&config())?;
    let id = backend.identity();
    let selection = config().selection;
    let limits = config().limits;
    let cancelled = AtomicBool::new(false);
    let fresh = AtomicBool::new(false);
    let progress = |_: RenderProgress| cancelled.store(true, Ordering::Relaxed);
    let context = RenderContext {
        cancel: &cancelled,
        limits: &limits,
        scratch_dir: &output,
        selection: &selection,
        identity: &id,
        pixel_recipe_key: "smoke",
        job_id: 7,
        revision: 1,
        progress: &progress,
    };
    let s = scene(cube([0.; 3], 10.));
    let mut f = frame(&s);
    f.width = 128;
    f.height = 128;
    assert!(
        backend
            .studio()
            .begin_job(&s, &context, true)?
            .render_frame(&f)
            .is_err()
    );
    assert!(cancelled.load(Ordering::Relaxed));
    let resumed = RenderContext {
        cancel: &fresh,
        job_id: 8,
        progress: &|_| {},
        ..context
    };
    let mut job = backend.studio().begin_job(&s, &resumed, true)?;
    let frame = job.render_frame(&f)?;
    assert_eq!(frame.job_id, 8);
    assert!(cancelled.load(Ordering::Relaxed));
    assert!(!fresh.load(Ordering::Relaxed));
    assert_eq!(frame.image.bytes.len(), 128 * 128 * 4);
    println!(
        "distinct-token reuse uploads={} timings={:?}",
        job.upload_count(),
        job.report().phase_timings
    );
    save(&output.join("final-smoke.png"), &frame.image);
    std::fs::write(
        output.join("identity.json"),
        serde_json::to_vec_pretty(&id)?,
    )?;
    println!("final identity {} {} {}", id.device, id.api, id.driver);
    Ok(())
}
fn save(path: &Path, frame: &RgbaFrame) {
    image::save_buffer(
        path,
        &frame.bytes,
        frame.width,
        frame.height,
        image::ColorType::Rgba8,
    )
    .unwrap();
}
// Keep the format/layout/alpha/section/animation matrix explicit at call sites.
#[allow(clippy::too_many_arguments)]
fn execute(
    backend: &mut FilamentBackend,
    output: &Path,
    name: &str,
    kind: ExportKind,
    layout: ExportLayout,
    transparent: bool,
    section: Option<SectionSpec>,
    animation: turninator::export::AnimationMode,
) -> Result<PathBuf> {
    let cache = turninator::cache::CacheStore::new(output.join("cache"))?;
    let job = cache.create_job(name)?;
    let s = scene(cube([0.; 3], 10.));
    s.mesh.write_binary(&job.join("mesh.bin"))?;
    let mut value = s.legacy_scene.clone();
    value["mesh_path"] = job.join("mesh.bin").to_string_lossy().to_string().into();
    value["solid_layer_tops_mm"] = serde_json::json!([5., 10.]);
    std::fs::write(job.join("scene.json"), serde_json::to_vec_pretty(&value)?)?;
    let extension = match kind {
        ExportKind::Mp4 => "mp4",
        ExportKind::Gif => "gif",
        _ => "png",
    };
    let destination = output.join(format!("{name}.{extension}"));
    let request = ExportRequest {
        job_dir: job.clone(),
        scene_path: job.join("scene.json"),
        destination: destination.clone(),
        kind,
        selection: config().selection,
        size: 256,
        frame_count: 3,
        transparent,
        layout,
        section,
        animation,
        ..Default::default()
    };
    let id = backend.identity();
    let mut selection = request.selection.clone();
    selection.device = Some(id.device.clone());
    let cancel = AtomicBool::new(false);
    let limits = config().limits;
    let recipe = turninator::export::identity::pixel_recipe_key(
        &value,
        &s.mesh.snapshot_hash()?,
        &request,
        &id,
    )?;
    let context = RenderContext {
        cancel: &cancel,
        limits: &limits,
        scratch_dir: &job,
        selection: &selection,
        identity: &id,
        pixel_recipe_key: &recipe,
        job_id: 1,
        revision: 1,
        progress: &|_| {},
    };
    let video = matches!(kind, ExportKind::Gif | ExportKind::Mp4);
    let tools = ToolPaths {
        blender: "D:/absent/blender.exe".into(),
        ffmpeg: if video {
            "C:/Program Files/ffmpeg/bin/ffmpeg.exe".into()
        } else {
            "D:/absent/ffmpeg.exe".into()
        },
        ffprobe: if video {
            "C:/Program Files/ffmpeg/bin/ffprobe.exe".into()
        } else {
            "D:/absent/ffprobe.exe".into()
        },
    };
    let start = Instant::now();
    run_export_with_backend(backend, &tools, &request, &context, &mut |_| {})?;
    println!(
        "pipeline {name}: {:.3}s {} bytes",
        start.elapsed().as_secs_f64(),
        std::fs::metadata(&destination)?.len()
    );
    // Retained rerun must validate/reuse existing completed artifacts.
    let before = std::fs::read(&destination)?;
    run_export_with_backend(backend, &tools, &request, &context, &mut |_| {})?;
    assert_eq!(before, std::fs::read(&destination)?);
    Ok(destination)
}

#[test]
#[ignore = "requires exclusive GPU grant and verified production Filament package beside isolated test executable"]
fn production_pipeline_surface_4096_and_lifecycle() -> Result<()> {
    let output = PathBuf::from(std::env::var("FILAMENT_TEST_OUTPUT")?);
    std::fs::create_dir_all(&output)?;
    let start = Instant::now();
    let mut backend = FilamentBackend::new(&config())?;
    let init = start.elapsed();
    let id = backend.identity();
    assert_ne!(id.device, "untrusted");
    assert!(id.device.starts_with("vulkan:"));
    std::fs::write(
        output.join("identity.json"),
        serde_json::to_vec_pretty(&id)?,
    )?;
    println!(
        "initialization {:.3}s device {} API {} driver {}",
        init.as_secs_f64(),
        id.device,
        id.api,
        id.driver
    );
    let mut selection = config().selection;
    selection.device = Some(id.device.clone());
    let limits = config().limits;
    let cancel = AtomicBool::new(false);
    let context = RenderContext {
        cancel: &cancel,
        limits: &limits,
        scratch_dir: &output,
        selection: &selection,
        identity: &id,
        pixel_recipe_key: "surface",
        job_id: 101,
        revision: 31,
        progress: &|_| {},
    };
    let mut s = scene(cube([-5., -5., 0.], 10.));
    s.appearance.studio_preset = "diagram".into();
    let mut f = frame(&s);
    f.camera = RenderCamera {
        position_mm: [0., -30., 5.],
        target_mm: [0., 0., 5.],
        up: [0., 0., 1.],
        projection: RenderProjection::Orthographic {
            vertical_span_mm: 16.,
        },
    };
    // Qualification can resume at the pipeline boundary without repeating the
    // already captured 4096 handoff or native surface cases.
    if std::env::var_os("FILAMENT_TEST_PIPELINE_ONLY").is_none() {
        let mut job = backend.studio().begin_job(&s, &context, true)?;
        let a = job.render_frame(&f)?;
        save(&output.join("surface-cube.png"), &a.image);
        let foreground = a.image.bytes.chunks_exact(4).filter(|p| p[3] > 127).count();
        let mut intersection = 0;
        let mut union = 0;
        for (i, pixel) in a.image.bytes.chunks_exact(4).enumerate() {
            let (x, y) = (i % 512, i / 512);
            let expected = (96..416).contains(&x) && (96..416).contains(&y);
            let actual = pixel[3] != 0;
            intersection += usize::from(expected && actual);
            union += usize::from(expected || actual);
            if actual {
                assert!((95..417).contains(&x) && (95..417).contains(&y));
            }
        }
        let iou = intersection as f64 / union as f64;
        println!(
            "High analytical mask IoU={iou:.8}, alpha>127 foreground={foreground}, countdelta={}",
            foreground as i64 - 102400
        );
        assert!(iou >= 0.995, "frozen native geometric mask tolerance");
        assert!(
            a.image
                .bytes
                .chunks_exact(4)
                .all(|p| p[3] != 0 || p[..3] == [0, 0, 0])
        );
        for n in 1..=12 {
            f.key.index = n;
            f.angle_degrees = n as f64 * 30.;
            job.render_frame(&f)?;
        }
        f.key.index = 13;
        f.angle_degrees = 0.;
        let again = job.render_frame(&f)?;
        assert_eq!(a.image.bytes, again.image.bytes);
        assert_eq!(job.upload_count(), 1);
        println!(
            "surface mask={} A-B-A exact static uploads={} frames={}",
            foreground,
            job.upload_count(),
            job.report().frame_count
        );
        drop(job);
        f.key.index = 0;
        f.width = 4096;
        f.height = 4096;
        let start = Instant::now();
        let large = backend
            .studio()
            .begin_job(&s, &context, true)?
            .render_frame(&f)?;
        assert_eq!(large.image.bytes.len(), 4096 * 4096 * 4);
        let handoff_seconds = start.elapsed().as_secs_f64();
        save(&output.join("surface-4096.png"), &large.image);
        println!(
            "4096 CPU handoff {:.3}s {} bytes",
            handoff_seconds,
            large.image.bytes.len()
        );
        drop(large);
        s.metadata.solid_layer_tops_mm = vec![5., 10.];
        f.width = 512;
        f.height = 512;
        f.visible_layers = Some(0);
        let empty = backend
            .studio()
            .begin_job(&s, &context, true)?
            .render_frame(&f)?;
        assert!(empty.image.bytes.iter().all(|&p| p == 0));
        assert_eq!(empty.build.unwrap().legacy["visible_layer"], -1);
        save(&output.join("empty-stage.png"), &empty.image);
        f.visible_layers = None;
        // Fresh snapshot, mapping, pose and original-layer shader flow through host.
        s.appearance.layer_appearance = true;
        s.appearance.display_rotation_degrees = [23., 11., 7.];
        s.metadata.face_groups = vec![FaceGroup {
            start_triangle: 0,
            end_triangle: 6,
            tool: 1,
            label: "red".into(),
            color: [0.8, 0.05, 0.02],
        }];
        f.width = 512;
        f.height = 512;
        let layered = backend
            .studio()
            .begin_job(&s, &context, true)?
            .render_frame(&f)?;
        save(&output.join("mapped-layered-posed.png"), &layered.image);
        assert_ne!(layered.image.bytes, a.image.bytes);
        // Cancellation after SDK begin must drain; reuse same owner with a fresh token.
        let interrupted_cancel = AtomicBool::new(false);
        let cancel_progress = |_: RenderProgress| {
            interrupted_cancel.store(true, Ordering::Relaxed);
        };
        let interrupted = RenderContext {
            cancel: &interrupted_cancel,
            progress: &cancel_progress,
            ..context
        };
        assert!(
            backend
                .studio()
                .begin_job(&s, &interrupted, true)?
                .render_frame(&f)
                .is_err()
        );
        backend
            .studio()
            .begin_job(&s, &context, true)?
            .render_frame(&f)?;
        assert!(interrupted_cancel.load(Ordering::Relaxed));
        let mut changed = ExportRequest {
            selection: selection.clone(),
            ..Default::default()
        };
        changed.selection.quality = BackendQuality::Filament {
            aa: AaQuality::Low,
            shadows: ShadowQuality::Low,
        };
        assert!(backend.configure(&changed).is_err());
    }
    use turninator::export::AnimationMode;
    execute(
        &mut backend,
        &output,
        "still",
        ExportKind::Still,
        ExportLayout::Single,
        true,
        None,
        AnimationMode::Turntable,
    )?;
    execute(
        &mut backend,
        &output,
        "preview",
        ExportKind::Still,
        ExportLayout::Single,
        false,
        None,
        AnimationMode::Turntable,
    )?;
    execute(
        &mut backend,
        &output,
        "sheet",
        ExportKind::Diagram,
        ExportLayout::Single,
        false,
        None,
        AnimationMode::Turntable,
    )?;
    execute(
        &mut backend,
        &output,
        "paired-section",
        ExportKind::Still,
        ExportLayout::SideBySide,
        false,
        Some(SectionSpec {
            axis: 0,
            position_mm: 5.,
            keep_positive: false,
            cap: true,
        }),
        AnimationMode::Turntable,
    )?;
    execute(
        &mut backend,
        &output,
        "turntable",
        ExportKind::Gif,
        ExportLayout::Single,
        false,
        None,
        AnimationMode::Turntable,
    )?;
    execute(
        &mut backend,
        &output,
        "build",
        ExportKind::Mp4,
        ExportLayout::Single,
        false,
        None,
        AnimationMode::BuildUp,
    )?;
    // A separate owner context provides paintable CPU RGBA to Task9.
    let mut independent = FilamentBackend::new(&config())?;
    let other = independent
        .studio()
        .begin_job(&s, &context, true)?
        .render_frame(&f)?;
    assert_eq!((other.job_id, other.revision, other.key), (101, 31, f.key));
    save(&output.join("independent-studio.png"), &other.image);
    drop(independent);
    drop(backend);
    let start = Instant::now();
    let rebuilt = FilamentBackend::new(&config())?;
    assert_eq!(rebuilt.identity(), id);
    drop(rebuilt);
    println!("destroy/recreate {:.3}s", start.elapsed().as_secs_f64());
    Ok(())
}
