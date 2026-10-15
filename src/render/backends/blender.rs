//! Blender's sequence adapter retains one worker request per panel, including
//! the optimized native animation path for a static turntable.
use crate::render::{
    BackendId, BackendQuality, RenderSelection, capabilities::FeatureSupport, contract::*,
    worker::WorkerTransport,
};
use crate::{Result, export::ExportRequest};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    sync::atomic::Ordering,
    time::Duration,
};

pub struct BlenderBackend {
    selection: RenderSelection,
    request: ExportRequest,
    identity: BackendIdentity,
    worker: Box<dyn WorkerTransport>,
}

pub fn validate_quality(selection: &RenderSelection) -> Result<()> {
    selection.validate()?;
    selection.backend.legacy_blender_engine()?;
    if matches!(
        selection.quality,
        BackendQuality::Cycles { denoise: false, .. }
    ) {
        return Err("Blender Cycles denoise=false is not supported by the current worker; saved selection was retained".into());
    }
    Ok(())
}

/// The pre-pipeline actor has no preparation/device binding yet. Reject explicit
/// intent before rendering/cache reuse instead of silently using its default GPU.
pub fn validate_legacy_execution(selection: &RenderSelection) -> Result<()> {
    validate_quality(selection)?;
    if selection.device.is_some() {
        return Err("explicit Blender device requires prepared renderer execution; saved selection was retained".into());
    }
    Ok(())
}

impl BlenderBackend {
    pub fn with_transport(
        selection: RenderSelection,
        request: ExportRequest,
        identity: BackendIdentity,
        worker: Box<dyn WorkerTransport>,
    ) -> Result<Self> {
        validate_quality(&selection)?;
        if identity.backend != selection.backend
            || selection.device.as_deref() != Some(&identity.device)
            || identity.device.is_empty()
            || identity.api.is_empty()
            || identity.driver.is_empty()
            || request.selection.backend != selection.backend
            || request.selection.quality != selection.quality
        {
            return Err("Blender preparation identity/selection mismatch".into());
        }
        Ok(Self {
            selection,
            request,
            identity,
            worker,
        })
    }
}

fn camera_spec(camera: &RenderCamera) -> Result<Value> {
    let delta: [f64; 3] = std::array::from_fn(|i| camera.position_mm[i] - camera.target_mm[i]);
    let distance = delta.iter().map(|n| n * n).sum::<f64>().sqrt();
    let (projection, fov, span) = match camera.projection {
        RenderProjection::Perspective {
            vertical_fov_degrees,
        } => ("perspective", vertical_fov_degrees, 1.),
        RenderProjection::Orthographic { vertical_span_mm } => {
            ("orthographic", 35., vertical_span_mm)
        }
    };
    let value = json!({"target_mm":camera.target_mm,"direction":delta.map(|n|n/distance),
        "up":camera.up,"distance_mm":distance,"projection":projection,
        "vertical_fov_degrees":fov,"ortho_span_mm":span});
    RenderCamera::from_legacy(&value)?;
    Ok(value)
}

impl RenderBackend for BlenderBackend {
    fn id(&self) -> BackendId {
        self.selection.backend
    }
    fn assess(&self, scene: &RenderSceneSnapshot, _request: &ExportRequest) -> Vec<FeatureSupport> {
        let resin = scene.appearance.material_preset == "resin";
        vec![if resin && scene.metadata.kind != GeometryKind::Original {
            FeatureSupport::Approximate {reason:"Resin on layered geometry retains internal interfaces; transmission is an optical illustration.".into()}
        } else {
            FeatureSupport::Supported
        }]
    }
    fn identity(&self) -> BackendIdentity {
        self.identity.clone()
    }
    fn render_sequence(
        &mut self,
        scene: &RenderSceneSnapshot,
        frames: &FramePlan,
        context: &RenderContext<'_>,
        emit: &mut dyn FnMut(FrameArtifact) -> Result<()>,
    ) -> Result<RenderReport> {
        if context.cancel.load(Ordering::Acquire) {
            return Err("cancelled".into());
        }
        if context.selection != &self.selection || context.identity != &self.identity {
            return Err("render context differs from immutable prepared Blender selection".into());
        }
        if frames.frames.is_empty()
            || frames.frames.len() > context.limits.max_frames as usize
            || !context.scratch_dir.is_absolute()
            || scene.mesh.positions.len() > context.limits.max_source_vertices
            || scene.mesh.triangles.len() > context.limits.max_source_triangles
        {
            return Err("invalid Blender sequence bounds".into());
        }
        let mut groups: BTreeMap<RenderPanel, Vec<&FrameSpec>> = BTreeMap::new();
        let mut keys = BTreeSet::new();
        let mut scratch_bytes = 0u64;
        for frame in &frames.frames {
            if !keys.insert(frame.key)
                || !(16..=context.limits.max_output_side.min(8000)).contains(&frame.width)
                || !(16..=context.limits.max_output_side.min(8000)).contains(&frame.height)
                || u64::from(frame.width) * u64::from(frame.height) > 64_000_000
                || !frame.angle_degrees.is_finite()
                || frame.angle_degrees.abs() > 1e9
                || !frame.time_seconds.is_finite()
                || frame.time_seconds < 0.
                || frame.visible_layers.is_some_and(|count| count > 50_000)
                || frame
                    .section
                    .as_ref()
                    .is_some_and(|section| section.axis > 2 || !section.position_mm.is_finite())
            {
                return Err("invalid Blender frame specification".into());
            }
            camera_spec(&frame.camera)?;
            let pixels = u64::from(frame.width) * u64::from(frame.height);
            if pixels * 4 > context.limits.host_bytes {
                return Err("Blender frame exceeds host image budget".into());
            }
            scratch_bytes = scratch_bytes
                .checked_add(pixels * 8)
                .ok_or("Blender scratch budget overflow")?;
            groups.entry(frame.key.panel).or_default().push(frame);
        }
        if scratch_bytes > context.limits.scratch_bytes {
            return Err("Blender sequence exceeds scratch budget".into());
        }
        for (repeat, source) in &frames.repeat_of {
            if !keys.contains(repeat)
                || !keys.contains(source)
                || repeat.panel != source.panel
                || source.index >= repeat.index
            {
                return Err(
                    "Blender repeat must refer to an earlier planned frame in the same panel"
                        .into(),
                );
            }
        }
        // Reject invalid later panels before starting any native work.
        for group in groups.values() {
            let first = group[0];
            let section = serde_json::to_value(&first.section)?;
            if group.iter().any(|frame| {
                frame.width != first.width
                    || frame.height != first.height
                    || serde_json::to_value(&frame.section).ok().as_ref() != Some(&section)
            }) || group
                .windows(2)
                .any(|pair| pair[0].key.index >= pair[1].key.index)
            {
                return Err(
                    "Blender panel dimensions/section must be fixed and indices increasing".into(),
                );
            }
            if group
                .iter()
                .any(|frame| frame.visible_layers != first.visible_layers)
                && (group.iter().any(|frame| frame.visible_layers.is_none())
                    || group
                        .windows(2)
                        .any(|pair| pair[0].visible_layers > pair[1].visible_layers))
            {
                return Err("Blender progressive layer counts must be present and monotone".into());
            }
        }
        let mut report = RenderReport {
            frame_count: 0,
            warnings: scene.warnings.clone(),
            identity: self.identity.clone(),
            quality: self.selection.quality.clone(),
            analyses: BTreeMap::new(),
            build_analyses: BTreeMap::new(),
            phase_timings: BTreeMap::new(),
        };
        for (panel, group) in groups {
            let first = group[0];
            let section = serde_json::to_value(&first.section)?;
            let directory = context.scratch_dir.join(format!("blender-{panel:?}"));
            // Only a fresh, owned scratch namespace is written. Never replace source scenes.
            fs::create_dir(&directory)?;
            let scene_path = directory.join("scene.json");
            let mut spec = scene.legacy_scene.clone();
            spec["section"] = section;
            let camera_changed = group.iter().any(|frame| frame.camera != scene.camera);
            let layers_vary = group
                .iter()
                .any(|frame| frame.visible_layers != first.visible_layers);
            let count = group.len();
            let step = if let Some(timing) = self.request.turntable_timing {
                360. / (timing.seconds_per_turn * f64::from(self.request.frame_rate))
            } else {
                360. / count as f64
            };
            let regular_angles = group.iter().enumerate().all(|(i, frame)| {
                let expected = (first.angle_degrees + i as f64 * step).rem_euclid(360.);
                let delta = (frame.angle_degrees.rem_euclid(360.) - expected).abs();
                delta < 1e-8 || (delta - 360.).abs() < 1e-8
            });
            let states_needed = camera_changed || layers_vary || !regular_angles;
            // No-camera legacy scenes must reach Blender without the provisional
            // structural snapshot fit, preserving its historical auto-fit pixels.
            if camera_changed {
                spec["camera"] = camera_spec(&first.camera)?;
            }
            let bytes = serde_json::to_vec(&spec)?;
            if bytes.len() as u64 > context.limits.max_scene_bytes {
                return Err("Blender scene exceeds byte budget".into());
            }
            fs::write(&scene_path, bytes)?;
            let mut request = json!({"request_id":format!("render-{}-{}-{panel:?}",context.job_id,context.revision),
                "scene_revision":context.revision,"kind":"render","scene_path":scene_path,
                "output_dir":directory.join("frames"),"engine":self.id().legacy_blender_engine()?,
                "mode":if count>1 {"turntable"} else {"still"},"size":first.width,
                "width":first.width,"height":first.height,"samples":self.selection.blender_samples()?,
                "frame_count":count,"frame_rate":self.request.frame_rate,"transparent":self.request.transparent,
                "start_angle_degrees":first.angle_degrees,"reuse_verified_frames":false});
            if states_needed {
                let states: Vec<Value> = group.iter().map(|frame| Ok(json!({"camera":if camera_changed {camera_spec(&frame.camera)?}else{Value::Null},
                    "preserve_scene_camera":!camera_changed,
                    "angle_degrees":frame.angle_degrees,"buildup_fraction":null,
                    "visible_layers":if layers_vary {frame.visible_layers} else {None}}))).collect::<Result<_>>()?;
                let bytes = serde_json::to_vec(&states)?;
                if bytes.len() as u64 > context.limits.max_timeline_bytes {
                    return Err("Blender states exceed byte budget".into());
                }
                let path = directory.join("frame-states.json");
                fs::write(&path, bytes)?;
                request["frame_states_path"] = json!(path);
            } else if count > 1 {
                request["seconds_per_turn"] = json!(
                    self.request
                        .turntable_timing
                        .map(|timing| timing.seconds_per_turn)
                );
            }
            if !layers_vary {
                request["build_stage"] = json!(
                    first
                        .visible_layers
                        .map(|visible_layers| crate::export::BuildStage { visible_layers })
                );
            }
            let mut failure = None;
            let mut accepted = BTreeSet::new();
            let completed = self.worker.request(request, context.cancel, &mut |event| {
                if failure.is_some()
                    || context.cancel.load(Ordering::Acquire)
                    || event["kind"] != "frame"
                {
                    return;
                }
                let result = (|| -> Result<()> {
                    let index = event["payload"]["index"]
                        .as_u64()
                        .ok_or("Blender frame index missing")?
                        as usize;
                    let frame = group.get(index).ok_or("Blender frame index outside plan")?;
                    if !accepted.insert(frame.key) {
                        return Err("duplicate Blender frame event".into());
                    }
                    if frames.repeat_of.contains_key(&frame.key) {
                        return Ok(());
                    }
                    let path = PathBuf::from(
                        event["payload"]["path"]
                            .as_str()
                            .ok_or("Blender frame path missing")?,
                    );
                    if !path.starts_with(directory.join("frames"))
                        || path
                            .components()
                            .any(|c| matches!(c, std::path::Component::ParentDir))
                    {
                        return Err("Blender frame path outside owned scratch".into());
                    }
                    emit(FrameArtifact::Png {
                        key: frame.key,
                        path,
                    })?;
                    report.frame_count += 1;
                    (context.progress)(RenderProgress {
                        key: Some(frame.key),
                        completed_work: u64::from(report.frame_count),
                        total_work: Some((frames.frames.len() - frames.repeat_of.len()) as u64),
                        stage: format!("Rendering with {}", self.selection.backend.as_str()),
                        preview: None,
                    });
                    Ok(())
                })();
                if let Err(error) = result {
                    failure = Some(error.to_string());
                    context.cancel.store(true, Ordering::Release);
                }
            });
            if let Some(error) = failure {
                self.worker.cancel();
                return Err(error.into());
            }
            let completed = completed?;
            if context.cancel.load(Ordering::Acquire) {
                self.worker.cancel();
                return Err("cancelled".into());
            }
            if accepted.len() != group.len() {
                return Err("Blender did not produce every planned frame".into());
            }
            let mut payload = completed["payload"].clone();
            if let Some(path) = payload["details_path"].as_str() {
                let path = fs::canonicalize(path)?;
                if !path.starts_with(fs::canonicalize(directory.join("frames"))?) {
                    return Err("Blender completion details outside owned scratch".into());
                }
                let budget = context
                    .limits
                    .max_timeline_bytes
                    .saturating_mul(8)
                    .min(context.limits.host_bytes / 4);
                payload = serde_json::from_slice(&crate::settings::read_bounded(&path, budget)?)?;
            }
            if let Some(warnings) = payload["warnings"].as_array() {
                for warning in warnings {
                    if let Some(warning) = warning.as_str()
                        && !report.warnings.iter().any(|w| w == warning)
                    {
                        report.warnings.push(warning.into());
                    }
                }
            }
            for key in ["prepare_seconds", "total_seconds"] {
                if let Some(seconds) = payload[key].as_f64().filter(|n| n.is_finite() && *n >= 0.) {
                    *report.phase_timings.entry(key.into()).or_default() +=
                        Duration::try_from_secs_f64(seconds)
                            .map_err(|_| "invalid Blender timing")?;
                }
            }
            for field in ["frame_analysis", "frame_build_analysis"] {
                if let Some(analyses) = payload[field].as_array() {
                    if analyses.len() != group.len() {
                        return Err("Blender analysis count differs from panel frame count".into());
                    }
                    for (frame, analysis) in group
                        .iter()
                        .zip(analyses)
                        .filter(|(_, analysis)| !analysis.is_null())
                    {
                        if field == "frame_analysis" {
                            report.analyses.insert(
                                frame.key,
                                SectionAnalysis::from_legacy(
                                    analysis.clone(),
                                    analysis["closed_input"] == true,
                                )?,
                            );
                        } else {
                            report
                                .build_analyses
                                .insert(frame.key, BuildAnalysis::from_legacy(analysis.clone())?);
                        }
                    }
                } else if !payload[field].is_null() {
                    return Err("Blender analysis records must be arrays".into());
                }
            }
        }
        Ok(report)
    }
}
