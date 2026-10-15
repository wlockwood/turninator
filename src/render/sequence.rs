//! Resolve current animation semantics once, before any renderer sees a job.
use super::contract::*;
use crate::{
    Result,
    export::{AnimationMode, ExportKind, ExportLayout, ExportRequest, jobs},
};
use std::collections::{BTreeMap, BTreeSet};

pub fn animated(request: &ExportRequest) -> bool {
    matches!(
        request.kind,
        ExportKind::Gif | ExportKind::Mp4 | ExportKind::SectionTurntable
    )
}
pub fn normalized_request(request: &ExportRequest) -> Result<ExportRequest> {
    request.selection.validate()?;
    let mut request = request.clone();
    if !animated(&request) {
        request.frame_count = 1;
        request.frame_rate = crate::animation::DEFAULT_FRAME_RATE;
        request.timeline = None;
        request.turntable_timing = None;
        request.animation = AnimationMode::Turntable;
    } else {
        request.frame_count = request.animation_frame_count()?;
        request.transparent = false;
    }
    crate::animation::validate_frame_rate(request.frame_rate)?;
    if request.kind == ExportKind::Diagram {
        request.transparent = true;
    }
    if request.kind == ExportKind::Diagram || request.timeline.is_some() {
        request.start_angle_degrees = 0.;
    }
    if !(16..=8000).contains(&request.size)
        || !(1..=10000).contains(&request.frame_count)
        || !request.start_angle_degrees.is_finite()
        || request.start_angle_degrees.abs() > 1e9
    {
        return Err("invalid export settings".into());
    }
    if request
        .build_stage
        .is_some_and(|s| s.visible_layers > 50_000)
        || (request.build_stage.is_some()
            && (request.animation == AnimationMode::BuildUp
                || request.timeline.as_ref().is_some_and(|t| t.buildup)))
    {
        return Err("invalid or conflicting fixed/progressive build stage".into());
    }
    if let Some(section) = &request.section {
        if section.axis > 2
            || !section.position_mm.is_finite()
            || request.kind == ExportKind::Diagram
        {
            return Err("invalid section settings".into());
        }
    } else if request.layout == ExportLayout::SideBySide
        || matches!(
            request.kind,
            ExportKind::SectionStill | ExportKind::SectionTurntable
        )
    {
        return Err("section export requires section settings".into());
    }
    if request.layout == ExportLayout::SideBySide && u64::from(request.size).pow(2) * 2 > 64_000_000
    {
        return Err("side-by-side output exceeds 64 million pixels".into());
    }
    Ok(request)
}

pub fn plan_frames(scene: &RenderSceneSnapshot, request: &ExportRequest) -> Result<FramePlan> {
    let request = normalized_request(request)?;
    let mut plan = FramePlan {
        transparent: request.transparent,
        ..Default::default()
    };
    let layers = if scene.metadata.kind == GeometryKind::Original {
        scene.metadata.solid_layer_tops_mm.len()
    } else {
        scene
            .metadata
            .layer_ranges
            .iter()
            .map(|r| r.layer_index)
            .collect::<BTreeSet<_>>()
            .len()
    };
    let building = request.animation == AnimationMode::BuildUp
        || request.timeline.as_ref().is_some_and(|t| t.buildup);
    if (building && layers == 0)
        || request
            .build_stage
            .is_some_and(|s| s.visible_layers as usize > layers)
    {
        return Err("build stage requires available whole layers".into());
    }
    let panels = if request.kind == ExportKind::Diagram {
        let layout = jobs::diagram_layout(request.size)?;
        let b = scene.reference_bounds_mm;
        let cameras = jobs::diagram_cameras(
            std::array::from_fn(|i| b.max[i] - b.min[i]),
            layout.panel_width,
            layout.panel_height,
        );
        [
            RenderPanel::Top,
            RenderPanel::Iso,
            RenderPanel::Front,
            RenderPanel::Right,
        ]
        .into_iter()
        .zip(cameras)
        .map(|(panel, camera)| {
            Ok((
                panel,
                RenderCamera::from_legacy(&camera)?,
                layout.panel_width,
                layout.panel_height,
            ))
        })
        .collect::<Result<Vec<_>>>()?
    } else {
        let paired = request.layout == ExportLayout::SideBySide;
        let height = request.size
            - if paired {
                (request.size / 24).max(11).min(request.size / 4)
            } else {
                0
            };
        let panels = if paired {
            vec![RenderPanel::Full, RenderPanel::Section]
        } else if request.section.is_some() {
            vec![RenderPanel::Section]
        } else {
            vec![RenderPanel::Single]
        };
        panels
            .into_iter()
            .map(|p| (p, scene.camera.clone(), request.size, height))
            .collect()
    };
    let mut timeline = request.timeline.clone();
    if let Some(t) = &mut timeline {
        t.buildup |= building;
    }
    for (panel, camera, width, height) in panels {
        let mut poses = BTreeMap::new();
        for index in 0..request.frame_count {
            let sample = timeline
                .as_ref()
                .map(|t| t.sample_frame_at_rate(index, request.frame_rate))
                .transpose()?;
            let angle = if let Some(sample) = &sample {
                sample.turntable_degrees
            } else if animated(&request) {
                request.start_angle_degrees
                    + f64::from(index)
                        * if let Some(t) = request.turntable_timing {
                            360. / (t.seconds_per_turn * f64::from(request.frame_rate))
                        } else {
                            360. / f64::from(request.frame_count)
                        }
            } else {
                request.start_angle_degrees
            };
            let visible_layers = if building {
                let count = if let Some(fraction) = sample.as_ref().and_then(|s| s.buildup_fraction)
                {
                    (fraction * layers as f64).floor() as u32 + 1
                } else {
                    ((f64::from(index + 1) * layers as f64) / f64::from(request.frame_count)).ceil()
                        as u32
                };
                Some(count.min(layers as u32))
            } else {
                request.build_stage.map(|s| s.visible_layers)
            };
            let frame = FrameSpec {
                key: FrameKey { index, panel },
                camera: sample
                    .as_ref()
                    .map(|s| RenderCamera::from_orbit(&s.camera))
                    .transpose()?
                    .unwrap_or_else(|| camera.clone()),
                angle_degrees: angle.rem_euclid(360.),
                visible_layers,
                section: if panel == RenderPanel::Section {
                    request.section.clone()
                } else {
                    None
                },
                width,
                height,
                time_seconds: f64::from(index) / f64::from(request.frame_rate),
            };
            // Exact pixel-input equality, excluding physical output time and key.
            let pose = serde_json::to_string(&serde_json::json!([
                frame.camera,
                frame.angle_degrees,
                frame.visible_layers,
                frame.section,
                width,
                height
            ]))?;
            if let Some(source) = poses.get(&pose) {
                plan.repeat_of.insert(frame.key, *source);
            } else {
                poses.insert(pose, frame.key);
            }
            plan.frames.push(frame);
        }
    }
    Ok(plan)
}
