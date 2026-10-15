//! Backend-neutral orchestration, retained recovery and atomic output publication.
use super::{
    ExportKind, ExportLayout, ExportRequest, ExportResult, JobEvent, encode,
    frame_sink::{FrameSink, owned_path},
    identity::{pixel_recipe_key, retained_render_key},
    jobs,
    recipe::{hash, read_json, write_json},
};
use crate::{
    Result,
    render::{
        ToolPaths,
        capabilities::FeatureSupport,
        contract::*,
        requirements::{requirements_for, validate_requirements},
        sequence::{animated, normalized_request, plan_frames},
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn load_scene(request: &ExportRequest) -> Result<RenderSceneSnapshot> {
    let value: Value = serde_json::from_slice(&crate::settings::read_bounded(
        &request.scene_path,
        1024 * 1024,
    )?)?;
    let mesh = crate::mesh::Mesh::read_binary(Path::new(
        value["mesh_path"]
            .as_str()
            .ok_or("scene mesh_path required")?,
    ))?;
    snapshot_scene(Arc::new(mesh), value)
}
fn protect(request: &ExportRequest, scene: &Value) -> Result<()> {
    if !request.job_dir.is_absolute()
        || !request.scene_path.is_absolute()
        || !request.destination.is_absolute()
    {
        return Err("export paths must be absolute".into());
    }
    let root = fs::canonicalize(&request.job_dir)?;
    let mut ancestor = request.destination.as_path();
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        suffix.push(
            ancestor
                .file_name()
                .ok_or("invalid destination path")?
                .to_owned(),
        );
        ancestor = ancestor.parent().ok_or("destination parent missing")?;
    }
    let mut resolved = fs::canonicalize(ancestor)?;
    for component in suffix.iter().rev() {
        resolved.push(component);
    }
    if request
        .destination
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
        || resolved.starts_with(&root)
    {
        return Err("export destination would replace retained job data".into());
    }
    owned_path(&root, &request.scene_path)?;
    let mesh = Path::new(
        scene["mesh_path"]
            .as_str()
            .ok_or("scene mesh_path required")?,
    );
    owned_path(&root, mesh)?;
    if request.destination.exists() {
        let destination = fs::canonicalize(&request.destination)?
            .to_string_lossy()
            .to_lowercase();
        for source in [
            Some(request.scene_path.as_path()),
            Some(mesh),
            scene["source_path"].as_str().map(Path::new),
        ]
        .into_iter()
        .flatten()
        {
            if source.exists()
                && fs::canonicalize(source)?.to_string_lossy().to_lowercase() == destination
            {
                return Err("export destination would replace a source or job input".into());
            }
        }
    }
    Ok(())
}
fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err("cancelled".into())
    } else {
        Ok(())
    }
}

pub fn run_export_with_backend(
    backend: &mut dyn RenderBackend,
    tools: &ToolPaths,
    request: &ExportRequest,
    context: &RenderContext<'_>,
    callback: &mut dyn FnMut(JobEvent),
) -> Result<ExportResult> {
    let cache = crate::cache::CacheStore::new(
        request.job_dir.parent().ok_or("job parent missing")?.into(),
    )?;
    let _lock = cache.lock_job(&request.job_dir)?;
    run_export_with_backend_budget(backend, tools, request, context, None, callback)
}
pub(crate) fn run_export_with_backend_budget(
    backend: &mut dyn RenderBackend,
    tools: &ToolPaths,
    request: &ExportRequest,
    context: &RenderContext<'_>,
    budget: Option<u64>,
    callback: &mut dyn FnMut(JobEvent),
) -> Result<ExportResult> {
    let request = normalized_request(request)?;
    check(context.cancel)?;
    // Renderer availability was established by preparation. Only output tools are needed here.
    validate_requirements(
        tools,
        &requirements_for(&request.selection, request.kind, true),
    )?;
    let scene = load_scene(&request)?;
    protect(&request, &scene.legacy_scene)?;
    if scene.mesh.positions.len() > context.limits.max_source_vertices
        || scene.mesh.triangles.len() > context.limits.max_source_triangles
    {
        return Err("source snapshot exceeds render limits".into());
    }
    if context.selection.backend != request.selection.backend
        || context.selection.quality != request.selection.quality
        || context.selection.device.as_deref() != Some(&context.identity.device)
        || backend.identity() != *context.identity
        || backend.id() != context.identity.backend
    {
        return Err("prepared backend identity/quality differs".into());
    }
    backend.configure(&request)?;
    let assessment = backend.assess(&scene, &request);
    for assessment in assessment.iter().cloned() {
        match assessment {
            FeatureSupport::Unsupported { reason } => return Err(reason.into()),
            FeatureSupport::Approximate { reason } => callback(JobEvent::Log(reason)),
            FeatureSupport::Supported => {}
        }
    }
    let mesh_path = Path::new(
        scene.legacy_scene["mesh_path"]
            .as_str()
            .ok_or("mesh path required")?,
    );
    let mesh_hash = hash(mesh_path)?;
    let scene_hash = hash(&request.scene_path)?;
    let key = pixel_recipe_key(&scene.legacy_scene, &mesh_hash, &request, context.identity)?;
    if key != context.pixel_recipe_key {
        return Err("context pixel recipe differs from immutable job".into());
    }
    let plan = plan_frames(&scene, &request)?;
    let directory = request.job_dir.join(&key[..20]);
    fs::create_dir_all(&directory)?;
    owned_path(&request.job_dir, &directory)?;
    let manifest = directory.join("job.json");
    let recipe = json!({"schema_version":3,"intent_key":retained_render_key(&scene.legacy_scene,&mesh_hash,&request)?,"pixel_recipe_key":key,"identity":context.identity,"effective_selection":context.selection,"scene_hash":scene_hash,"mesh_hash":mesh_hash,"frame_plan":plan.frames,"repeats":plan.repeat_of.iter().collect::<Vec<_>>(),"transparent":plan.transparent,"assessment":assessment});
    if manifest.exists() && read_json(&manifest)?["recipe"] != recipe {
        return Err("existing job recipe differs".into());
    }
    let prior = read_json(&manifest).ok();
    let mut state = json!({"recipe":recipe,"stage":"rendering","destination":request.destination,"encoding":{"kind":request.kind,"codec":request.video_codec,"frame_rate":request.frame_rate}});
    if let Some(prior) = prior {
        state["report"] = prior["report"].clone();
    }
    write_json(&manifest, &state)?;
    let host_context = RenderContext {
        scratch_dir: &directory,
        ..*context
    };
    let operation = (|| -> Result<ExportResult> {
        let mut sink = FrameSink::new(&plan, &host_context)?;
        if !sink.complete() || state["report"].is_null() {
            let work_plan = if state["report"].is_null() {
                plan.clone()
            } else {
                FramePlan {
                    frames: plan
                        .frames
                        .iter()
                        .filter(|f| !sink.contains(f.key) && !plan.repeat_of.contains_key(&f.key))
                        .cloned()
                        .collect(),
                    repeat_of: Default::default(),
                    transparent: plan.transparent,
                }
            };
            if !work_plan.frames.is_empty() {
                let reserve = work_plan.frames.iter().try_fold(0u64, |n, f| {
                    n.checked_add(u64::from(f.width) * u64::from(f.height) * 12)
                        .ok_or("render reservation overflow")
                })?;
                jobs::preflight_storage(&request.job_dir, reserve)?;
                if let Some(budget) = budget {
                    let cache = crate::cache::CacheStore::new(
                        request.job_dir.parent().ok_or("job parent missing")?.into(),
                    )?;
                    cache.owned_job(&request.job_dir)?;
                    cache.check_budget(budget, reserve)?;
                }
                let producer = encode::Scratch::new(&directory)?;
                let producer_context = RenderContext {
                    scratch_dir: &producer.0,
                    ..*context
                };
                let styled;
                let render_scene = if request.kind == ExportKind::Diagram {
                    styled = snapshot_scene(
                        scene.mesh.clone(),
                        jobs::diagram_panel_scene(
                            &scene.legacy_scene,
                            &scene.legacy_scene["camera"],
                        ),
                    )?;
                    &styled
                } else {
                    &scene
                };
                let mut emitted = std::collections::BTreeSet::new();
                let report = backend.render_sequence(
                    render_scene,
                    &work_plan,
                    &producer_context,
                    &mut |artifact| {
                        let key = match &artifact {
                            FrameArtifact::Image { key, .. } | FrameArtifact::Png { key, .. } => {
                                *key
                            }
                        };
                        if !work_plan.frames.iter().any(|f| f.key == key)
                            || work_plan.repeat_of.contains_key(&key)
                            || !emitted.insert(key)
                        {
                            return Err("unknown, repeated or duplicate renderer frame key".into());
                        }
                        if sink.contains(key) {
                            check(context.cancel)?;
                            return Ok(());
                        }
                        let path = sink.accept(artifact)?;
                        callback(JobEvent::Frame(path));
                        Ok(())
                    },
                )?;
                if emitted.len() != work_plan.frames.len() - work_plan.repeat_of.len() {
                    return Err("renderer omitted unique frames".into());
                }
                if report.identity != *context.identity
                    || report.quality != context.selection.quality
                    || report.frame_count as usize
                        != work_plan.frames.len() - work_plan.repeat_of.len()
                {
                    return Err("renderer report identity/quality/count differs".into());
                }
                let old_report = state["report"].clone();
                state["report"] = json!({"rendered_frame_count":report.frame_count,"warnings":report.warnings,"analyses":report.analyses.iter().map(|(key,a)|json!({"key":key,"legacy":a.legacy,"cap_supported":a.cap_supported})).collect::<Vec<_>>(),"build_analyses":report.build_analyses.iter().map(|(key,a)|json!({"key":key,"legacy":a.legacy})).collect::<Vec<_>>(),"phase_seconds":report.phase_timings.iter().map(|(k,v)|(k.clone(),v.as_secs_f64())).collect::<std::collections::BTreeMap<_,_>>()});
                for field in ["analyses", "build_analyses"] {
                    for record in old_report[field].as_array().into_iter().flatten() {
                        let key: FrameKey = serde_json::from_value(record["key"].clone())?;
                        if !work_plan.frames.iter().any(|f| f.key == key) {
                            state["report"][field]
                                .as_array_mut()
                                .ok_or("report records missing")?
                                .push(record.clone());
                        }
                    }
                }
            }
            // Producer paths are still present until the entire render_sequence returns.
        } else {
            callback(JobEvent::Progress {
                completed: plan.frames.len() as u32,
                total: plan.frames.len() as u32,
                stage: "Reusing retained frames".into(),
            });
        }
        validate_report(&state["report"], &plan)?;
        sink.finish()?;
        if hash(mesh_path)? != mesh_hash || hash(&request.scene_path)? != scene_hash {
            return Err("job source changed during render".into());
        }
        write_json(&directory.join("analysis.json"), &state["report"])?;
        for record in state["report"]["analyses"].as_array().into_iter().flatten() {
            callback(JobEvent::Analysis(record["legacy"].clone()));
        }
        state["stage"] = json!("encoding");
        write_json(&manifest, &state)?;
        let frames = compose(&request, &scene, &plan, &sink, &directory, context.cancel)?;
        publish(&request, tools, &frames, context.cancel, callback)
    })();
    match &operation {
        Ok(result) => {
            state["stage"] = json!("complete");
            state["result"] = json!(result);
        }
        Err(error) => {
            state["stage"] = json!(if context.cancel.load(Ordering::Acquire) {
                "cancelled"
            } else {
                "failed"
            });
            state["error"] = json!(error.to_string());
        }
    }
    write_json(&manifest, &state)?;
    operation
}

fn publish(
    request: &ExportRequest,
    tools: &ToolPaths,
    frames: &Path,
    cancel: &AtomicBool,
    callback: &mut dyn FnMut(JobEvent),
) -> Result<ExportResult> {
    check(cancel)?;
    callback(JobEvent::Progress {
        completed: request.frame_count,
        total: request.frame_count,
        stage: "Encoding and validating".into(),
    });
    let width = request.size
        * if request.layout == ExportLayout::SideBySide {
            2
        } else {
            1
        };
    match request.kind {
        ExportKind::Gif | ExportKind::SectionTurntable => encode::encode_gif_dimensions_at_rate(
            frames,
            &request.destination,
            tools,
            request.frame_count,
            width,
            request.size,
            request.frame_rate,
            cancel,
        ),
        ExportKind::Mp4 => encode::encode_mp4_with_codec_at_rate(
            frames,
            &request.destination,
            tools,
            request.frame_count,
            width,
            request.size,
            request.video_codec,
            request.frame_rate,
            cancel,
        ),
        _ => encode::publish_png(
            &frames.join("still.png"),
            &request.destination,
            width,
            request.size,
            cancel,
        ),
    }
}

fn validate_report(report: &Value, plan: &FramePlan) -> Result<()> {
    let mut sections = std::collections::BTreeSet::new();
    let mut builds = std::collections::BTreeSet::new();
    for (field, keys) in [("analyses", &mut sections), ("build_analyses", &mut builds)] {
        for record in report[field].as_array().ok_or("analysis records missing")? {
            let key: FrameKey = serde_json::from_value(record["key"].clone())?;
            if !plan.frames.iter().any(|f| f.key == key) || !keys.insert(key) {
                return Err("unknown or duplicate analysis frame key".into());
            }
            if field == "analyses" {
                SectionAnalysis::from_legacy(
                    record["legacy"].clone(),
                    record["cap_supported"]
                        .as_bool()
                        .ok_or("cap support missing")?,
                )?;
            } else {
                BuildAnalysis::from_legacy(record["legacy"].clone())?;
            }
        }
    }
    for frame in &plan.frames {
        if frame.section.is_some() && !sections.contains(&frame.key) {
            return Err("renderer omitted section analysis".into());
        }
        if frame.visible_layers.is_some() && !builds.contains(&frame.key) {
            return Err("renderer omitted build analysis".into());
        }
    }
    Ok(())
}

pub(crate) fn validate_recovery_recipe(job: &Path, key: Option<&str>) -> Result<()> {
    let mut found = false;
    for entry in fs::read_dir(job)? {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }
        owned_path(job, &path)?;
        let Ok(state) = read_json(&path.join("job.json")) else {
            continue;
        };
        found = true;
        if state["recipe"]["schema_version"] == 2 {
            return Err("Legacy retained frames are incomplete. Start a new export to create fresh pixels with the current renderer recipe; original provenance was retained.".into());
        }
        if let Some(key) = key
            && state["recipe"]["pixel_recipe_key"] == key
        {
            return Ok(());
        }
    }
    if key.is_some() && found {
        return Err("Prepared renderer differs from retained pixel provenance. Start a new export to render a fresh artifact set.".into());
    }
    Ok(())
}
fn frame_name(request: &ExportRequest, index: u32) -> String {
    if animated(request) {
        format!("frame_{index:04}.png")
    } else {
        "still.png".into()
    }
}

fn compose(
    request: &ExportRequest,
    scene: &RenderSceneSnapshot,
    plan: &FramePlan,
    sink: &FrameSink<'_>,
    directory: &Path,
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    let frames = directory.join("output-frames");
    fs::create_dir_all(&frames)?;
    owned_path(directory, &frames)?;
    if request.kind == ExportKind::Diagram {
        let layout = jobs::diagram_layout(request.size)?;
        let b = scene.reference_bounds_mm;
        let cameras = jobs::diagram_cameras(
            std::array::from_fn(|i| b.max[i] - b.min[i]),
            layout.panel_width,
            layout.panel_height,
        );
        let measurements = if scene.metadata.kind == GeometryKind::Original
            && let Some(stage) = request.build_stage
        {
            let count = stage.visible_layers as usize;
            let height = if count == 0 {
                0.
            } else {
                *scene
                    .metadata
                    .solid_layer_tops_mm
                    .get(count - 1)
                    .ok_or("solid build stage exceeds layers")?
            };
            jobs::diagram_solid_measurements(&scene.mesh, &cameras, height)?
        } else {
            let visibility = jobs::diagram_stage_visibility(
                &scene.mesh,
                &scene.legacy_scene,
                request.build_stage,
            )?;
            jobs::diagram_measurements(&scene.mesh, &cameras, visibility.as_deref())?
        };
        let labels = ["TOP", "ISO", "FRONT", "RIGHT"];
        write_json(
            &directory.join("four-view-dimensions.json"),
            &json!({"units":"mm","views":labels,"measurements":measurements}),
        )?;
        let mut sheet =
            image::RgbaImage::from_pixel(request.size, request.size, image::Rgba([255; 4]));
        for (index, frame) in plan.frames.iter().enumerate() {
            check(cancel)?;
            let mut panel = image::open(sink.path(frame.key))?.to_rgba8();
            let alpha = panel.clone();
            for y in 1..panel.height() - 1 {
                for x in 1..panel.width() - 1 {
                    if alpha[(x, y)][3] > 128
                        && [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]
                            .iter()
                            .any(|&(a, b)| alpha[(a, b)][3] < 64)
                    {
                        panel[(x, y)] = image::Rgba([40, 45, 52, 255]);
                    }
                }
            }
            if let Some(dimension) = measurements[index] {
                jobs::draw_dimension(
                    &mut panel,
                    dimension,
                    cameras[index]["ortho_span_mm"]
                        .as_f64()
                        .ok_or("diagram span missing")?,
                );
            }
            let x = layout.margin + (index as u32 % 2) * (layout.panel_width + layout.margin);
            let y = layout.margin
                + (index as u32 / 2) * (layout.panel_height + layout.label_height + layout.margin);
            image::imageops::overlay(&mut sheet, &panel, i64::from(x), i64::from(y));
            jobs::draw_label(
                &mut sheet,
                labels[index],
                x,
                y + layout.panel_height,
                layout.panel_width,
                layout.label_height,
            );
        }
        let partial = frames.join("still.partial.png");
        super::frame_sink::writable_path(directory, &partial)?;
        sheet.save(&partial)?;
        check(cancel)?;
        encode::publish(&partial, &frames.join("still.png"))?;
    } else {
        for index in 0..request.frame_count {
            check(cancel)?;
            let name = frame_name(request, index);
            let partial = frames.join(format!("{name}.partial.png"));
            super::frame_sink::writable_path(directory, &partial)?;
            if request.layout == ExportLayout::SideBySide {
                let bar = (request.size / 24).max(11).min(request.size / 4);
                let mut sheet = image::RgbaImage::from_pixel(
                    request.size * 2,
                    request.size,
                    image::Rgba([255; 4]),
                );
                for (column, panel) in [RenderPanel::Full, RenderPanel::Section]
                    .into_iter()
                    .enumerate()
                {
                    let pixels = image::open(sink.path(FrameKey { index, panel }))?.to_rgba8();
                    image::imageops::replace(
                        &mut sheet,
                        &pixels,
                        i64::from(column as u32 * request.size),
                        0,
                    );
                    jobs::draw_label(
                        &mut sheet,
                        if column == 0 { "FULL" } else { "SECTION" },
                        column as u32 * request.size,
                        request.size - bar,
                        request.size,
                        bar,
                    );
                }
                super::frame_sink::writable_path(directory, &partial)?;
                sheet.save(&partial)?;
            } else {
                let panel = if request.section.is_some() {
                    RenderPanel::Section
                } else {
                    RenderPanel::Single
                };
                fs::copy(sink.path(FrameKey { index, panel }), &partial)?;
            }
            check(cancel)?;
            encode::publish(&partial, &frames.join(name))?;
        }
    }
    Ok(frames)
}

/// Recover only fully verified original pixels. No registry, GPU context or renderer tools.
/// None means there is no complete compatible artifact set; callers may prepare a fresh renderer.
pub fn encode_only(
    tools: &ToolPaths,
    request: &ExportRequest,
    cancel: &AtomicBool,
    callback: &mut dyn FnMut(JobEvent),
) -> Result<Option<ExportResult>> {
    let cache = crate::cache::CacheStore::new(
        request.job_dir.parent().ok_or("job parent missing")?.into(),
    )?;
    let _lock = cache.lock_job(&request.job_dir)?;
    encode_only_locked(tools, request, cancel, callback)
}
pub(crate) fn encode_only_locked(
    tools: &ToolPaths,
    request: &ExportRequest,
    cancel: &AtomicBool,
    callback: &mut dyn FnMut(JobEvent),
) -> Result<Option<ExportResult>> {
    let request = normalized_request(request)?;
    check(cancel)?;
    let scene = load_scene(&request)?;
    protect(&request, &scene.legacy_scene)?;
    let mesh_hash = scene.mesh.snapshot_hash()?;
    let intent = retained_render_key(&scene.legacy_scene, &mesh_hash, &request)?;
    for entry in fs::read_dir(&request.job_dir)? {
        check(cancel)?;
        let directory = entry?.path();
        if !directory.is_dir() {
            continue;
        }
        owned_path(&request.job_dir, &directory)?;
        let Ok(state) = read_json(&directory.join("job.json")) else {
            continue;
        };
        let recipe = &state["recipe"];
        if recipe["schema_version"] != 3 {
            continue;
        }
        if recipe["intent_key"] != intent
            || recipe["mesh_hash"] != mesh_hash
            || recipe["scene_hash"] != hash(&request.scene_path)?
            || state["report"].is_null()
        {
            continue;
        }
        let identity: BackendIdentity = serde_json::from_value(recipe["identity"].clone())?;
        let key = pixel_recipe_key(&scene.legacy_scene, &mesh_hash, &request, &identity)?;
        if recipe["pixel_recipe_key"] != key
            || !matches!(directory.file_name().and_then(|n| n.to_str()),Some(name) if name==key || name==&key[..20])
        {
            return Err("retained pixel identity differs".into());
        }
        let selection: crate::render::RenderSelection =
            serde_json::from_value(recipe["effective_selection"].clone())?;
        if selection.backend != identity.backend
            || selection.quality != request.selection.quality
            || selection.device.as_deref() != Some(&identity.device)
        {
            return Err("retained effective selection differs".into());
        }
        let limits = RenderLimits::default();
        let context = RenderContext {
            cancel,
            limits: &limits,
            scratch_dir: &directory,
            selection: &selection,
            identity: &identity,
            pixel_recipe_key: &key,
            job_id: 0,
            revision: 0,
            progress: &|_| {},
        };
        let plan = plan_frames(&scene, &request)?;
        if recipe["frame_plan"] != serde_json::to_value(&plan.frames)?
            || recipe["repeats"] != serde_json::to_value(plan.repeat_of.iter().collect::<Vec<_>>())?
            || recipe["transparent"] != plan.transparent
        {
            return Err("retained frame plan differs".into());
        }
        validate_report(&state["report"], &plan)?;
        let mut sink = FrameSink::new(&plan, &context)?;
        if !sink.complete() {
            continue;
        }
        sink.finish()?;
        validate_requirements(
            tools,
            &requirements_for(&request.selection, request.kind, true),
        )?;
        callback(JobEvent::Progress {
            completed: plan.frames.len() as u32,
            total: plan.frames.len() as u32,
            stage: "Reusing retained frames".into(),
        });
        let frames = compose(&request, &scene, &plan, &sink, &directory, cancel)?;
        return Ok(Some(publish(&request, tools, &frames, cancel, callback)?));
    }
    encode_legacy(tools, &request, &scene, cancel, callback)
}

fn encode_legacy(
    tools: &ToolPaths,
    request: &ExportRequest,
    scene: &RenderSceneSnapshot,
    cancel: &AtomicBool,
    callback: &mut dyn FnMut(JobEvent),
) -> Result<Option<ExportResult>> {
    // Legacy recipes bind script/runtime hashes already recorded at render time.
    // They are never compared to a newly installed runtime or filled with new pixels.
    let Ok(engine) = request.selection.backend.legacy_blender_engine() else {
        return Ok(None);
    };
    if request.selection.device.is_some()
        || matches!(
            request.selection.quality,
            crate::render::BackendQuality::Cycles { denoise: false, .. }
        )
    {
        return Ok(None);
    }
    let count = if request.kind == ExportKind::Diagram {
        4
    } else {
        request.frame_count
    };
    let expected = json!({"scene_hash":hash(&request.scene_path)?,"mesh_hash":scene.mesh.snapshot_hash()?,"engine":engine,"size":request.size,"samples":request.selection.blender_samples()?,"frame_count":count,"frame_rate":if animated(request){Some(request.frame_rate)}else{None},"transparent":request.transparent,"section":request.section,"layout":request.layout,"animation":request.animation,"timeline":request.timeline,"turntable_timing":request.turntable_timing,"build_stage":request.build_stage,"start_angle_degrees":request.start_angle_degrees,"kind":if animated(request){"animation"}else if request.kind==ExportKind::Diagram{"diagram"}else{"still"}});
    for entry in fs::read_dir(&request.job_dir)? {
        check(cancel)?;
        let directory = entry?.path();
        if !directory.is_dir() {
            continue;
        }
        owned_path(&request.job_dir, &directory)?;
        let Ok(state) = read_json(&directory.join("job.json")) else {
            continue;
        };
        let recipe = &state["recipe"];
        if recipe["schema_version"] != 2
            || expected
                .as_object()
                .unwrap()
                .iter()
                .any(|(k, v)| &recipe[k] != v)
        {
            continue;
        }
        if [
            "worker_hash",
            "scene_script_hash",
            "frame_pipeline_hash",
            "blender_hash",
        ]
        .iter()
        .any(|k| recipe[k].as_str().is_none_or(|s| s.len() != 64))
        {
            continue;
        }
        let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(recipe)?));
        if directory.file_name().and_then(|n| n.to_str()) != Some(&fingerprint[..20]) {
            return Err("legacy recipe fingerprint differs".into());
        }
        let mut producers = Vec::new();
        if request.kind == ExportKind::Diagram {
            let layout = jobs::diagram_layout(request.size)?;
            for i in 0..4 {
                producers.push((
                    directory.join(format!("panel-{i}")),
                    directory.join(format!("diagram-{i}.json")),
                    layout.panel_width,
                    layout.panel_height,
                    1,
                ));
            }
        } else if request.section.is_some() {
            let paired = request.layout == ExportLayout::SideBySide;
            let height = request.size
                - if paired {
                    (request.size / 24).max(11).min(request.size / 4)
                } else {
                    0
                };
            for p in if paired {
                vec!["full", "section"]
            } else {
                vec!["section"]
            } {
                producers.push((
                    directory.join(p),
                    directory.join(format!("{p}-scene.json")),
                    request.size,
                    height,
                    request.frame_count,
                ));
            }
        } else {
            producers.push((
                directory.join("frames"),
                request.scene_path.clone(),
                request.size,
                request.size,
                request.frame_count,
            ));
        }
        let mut valid = true;
        for (output, scene_path, width, height, count) in producers {
            if !output.is_dir() || owned_path(&request.job_dir, &output).is_err() {
                valid = false;
                break;
            }
            owned_path(&request.job_dir, &scene_path)?;
            for index in 0..count {
                let file = output.join(frame_name(request, index));
                if !file.is_file() {
                    valid = false;
                    break;
                }
                owned_path(&request.job_dir, &file)?;
            }
            if !valid {
                break;
            }
            let query = json!({"output_dir":output,"scene_path":scene_path,"engine":engine,"size":request.size,"samples":request.selection.blender_samples()?,"mode":if animated(request){if request.animation==super::AnimationMode::BuildUp{"buildup"}else{"turntable"}}else{"still"},"width":width,"height":height,"frame_count":count,"frame_rate":request.frame_rate,"seconds_per_turn":request.turntable_timing.map(|t|t.seconds_per_turn),"build_stage":request.build_stage,"transparent":request.transparent});
            if jobs::inspect_retained_frames(&query, cancel)?.0.is_none() {
                valid = false;
                break;
            }
        }
        if !valid {
            continue;
        }
        validate_requirements(
            tools,
            &requirements_for(&request.selection, request.kind, true),
        )?;
        callback(JobEvent::Log(
            "Reusing original legacy frames and provenance".into(),
        ));
        let result = if request.kind == ExportKind::Diagram {
            encode::publish_png(
                &directory.join("four-view.png"),
                &request.destination,
                request.size,
                request.size,
                cancel,
            )?
        } else {
            let frames = if request.section.is_some() && request.layout == ExportLayout::Single {
                directory.join("section")
            } else {
                directory.join("frames")
            };
            publish(request, tools, &frames, cancel, callback)?
        };
        return Ok(Some(result));
    }
    Ok(None)
}
