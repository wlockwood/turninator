//! Opt-in production benchmark, invoked serially by tools/benchmark-renderers.ps1.
#[path = "fixtures/renderers/common.rs"]
mod common;
use common::*;
use serde_json::json;
use std::{path::PathBuf, sync::atomic::AtomicBool, time::Instant};
use turninator::{
    Result,
    export::{ExportKind, ExportRequest, SectionSpec, pipeline::run_export_with_backend},
    presentation::{SceneSettings, render_scene},
    render::{ToolPaths, contract::*, registry::BackendRegistry, selection::*},
};

struct Measured<'a> {
    inner: &'a mut dyn RenderBackend,
    producer_seconds: f64,
    sink_seconds: f64,
    frames: Vec<serde_json::Value>,
}
impl RenderBackend for Measured<'_> {
    fn id(&self) -> BackendId {
        self.inner.id()
    }
    fn identity(&self) -> BackendIdentity {
        self.inner.identity()
    }
    fn assess(
        &self,
        scene: &RenderSceneSnapshot,
        request: &ExportRequest,
    ) -> Vec<turninator::render::capabilities::FeatureSupport> {
        self.inner.assess(scene, request)
    }
    fn render_sequence(
        &mut self,
        scene: &RenderSceneSnapshot,
        plan: &FramePlan,
        context: &RenderContext<'_>,
        emit: &mut dyn FnMut(FrameArtifact) -> Result<()>,
    ) -> Result<RenderReport> {
        let start = Instant::now();
        let mut last = Instant::now();
        let report = self
            .inner
            .render_sequence(scene, plan, context, &mut |frame| {
                let produced = last.elapsed().as_secs_f64();
                let accepting = Instant::now();
                let result = emit(frame);
                let accepted = accepting.elapsed().as_secs_f64();
                self.sink_seconds += accepted;
                self.frames
                    .push(json!({"producer_interval_seconds":produced,"sink_seconds":accepted}));
                last = Instant::now();
                result
            })?;
        self.producer_seconds += start.elapsed().as_secs_f64() - self.sink_seconds;
        Ok(report)
    }
}

#[test]
#[ignore = "exclusive GPU; requires qualified production package and installed Blender/encoders"]
fn matched_production_case() -> Result<()> {
    let output = PathBuf::from(std::env::var("TN_BENCH_OUTPUT")?);
    std::fs::create_dir_all(&output)?;
    let backend = BackendId::parse_legacy(&std::env::var("TN_BENCH_BACKEND")?)?;
    let fixture = std::env::var("TN_BENCH_SCENE")?;
    let still = std::env::var("TN_BENCH_OUTPUT_KIND")? == "still";
    let start = Instant::now();
    let cache = turninator::cache::CacheStore::new(output.join("jobs"))?;
    let job = cache.create_job("matched")?;
    let mut settings = SceneSettings::default();
    let mut mesh = cube([-5., -5., 0.], 10.);
    let mut kind = "original";
    let mut ranges = vec![];
    let mut groups = vec![];
    match fixture.as_str() {
        "opaque" => {}
        "resin" => settings.select_material("resin"),
        "stepped-cutaway" => {
            append(&mut mesh, cube([-4., -4., 10.], 8.));
            kind = "stepped";
            ranges = vec![
                json!({"start_triangle":0,"end_triangle":12,"layer_index":0,"z_top_mm":10.}),
                json!({"start_triangle":12,"end_triangle":24,"layer_index":1,"z_top_mm":18.}),
            ];
        }
        "beads" => {
            use turninator::print::beads::{BeadSegment, build_bead_mesh};
            let segments: Vec<_> = (0..8)
                .map(|i| BeadSegment {
                    start: [-5., i as f64 * 0.8 - 2.8, 0.2],
                    end: [5., i as f64 * 0.8 - 2.8, 0.2],
                    width_mm: 0.42,
                    height_mm: 0.2,
                    volume_mm3: 0.,
                    source_line: i + 1,
                    feature: "wall".into(),
                    tool: (i % 2) as u32,
                    layer_index: 0,
                    width_source: "benchmark-v1".into(),
                    height_source: "benchmark-v1".into(),
                })
                .collect();
            mesh = build_bead_mesh(&segments)?;
            kind = "beads";
            let count = mesh.triangles.len() as u32 / 8;
            ranges.push(
                json!({"start_triangle":0,"end_triangle":count*8,"layer_index":0,"z_top_mm":0.2}),
            );
            groups = (0..8).map(|i| json!({"start_triangle":i*count,"end_triangle":(i+1)*count,"tool":i%2,"label":format!("strand{i}"),"color":if i%2==0 {[0.8,0.03,0.02]} else {[0.02,0.6,0.05]}})).collect();
        }
        _ => return Err("unknown matched scene".into()),
    }
    mesh.write_binary(&job.join("mesh.bin"))?;
    let mut value = render_scene(&settings, &job.join("mesh.bin"), kind, &[], None)?;
    if !ranges.is_empty() {
        value["layer_face_ranges"] = json!(ranges);
    }
    if !groups.is_empty() {
        value["face_groups"] = json!(groups);
    }
    std::fs::write(job.join("scene.json"), serde_json::to_vec_pretty(&value)?)?;
    let snapshot = snapshot_scene(std::sync::Arc::new(mesh), value.clone())?;
    let mut selection = RenderSelection::for_backend(backend, still);
    selection.quality = match backend {
        BackendId::Wgpu => BackendQuality::Wgpu {
            aa: if still {
                AaQuality::High
            } else {
                AaQuality::Medium
            },
            shadows: if still {
                ShadowQuality::High
            } else {
                ShadowQuality::Medium
            },
        },
        BackendId::Filament => BackendQuality::Filament {
            aa: if still {
                AaQuality::High
            } else {
                AaQuality::Medium
            },
            shadows: if still {
                ShadowQuality::High
            } else {
                ShadowQuality::Medium
            },
        },
        BackendId::LuxCore => BackendQuality::LuxCore {
            samples: if still { 64 } else { 16 },
            denoise: true,
        },
        BackendId::BlenderCycles => BackendQuality::Cycles {
            samples: if still { 64 } else { 16 },
            denoise: true,
        },
        BackendId::BlenderEevee => BackendQuality::Eevee {
            samples: if still { 64 } else { 16 },
        },
    };
    let request = ExportRequest {
        scene_path: job.join("scene.json"),
        job_dir: job.clone(),
        destination: output.join(if still { "result.png" } else { "result.mp4" }),
        kind: if still {
            ExportKind::Still
        } else {
            ExportKind::Mp4
        },
        size: if still { 4096 } else { 960 },
        frame_count: 80,
        frame_rate: 20,
        selection,
        transparent: false,
        section: (fixture == "stepped-cutaway").then_some(SectionSpec {
            axis: 0,
            position_mm: 0.,
            keep_positive: false,
            cap: true,
        }),
        ..Default::default()
    };
    std::fs::write(
        output.join("request.json"),
        serde_json::to_vec_pretty(&request)?,
    )?;
    let tools = ToolPaths::default();
    let limits = RenderLimits {
        host_bytes: 16 << 30,
        gpu_bytes: 10 << 30,
        ..Default::default()
    };
    let cancel = AtomicBool::new(false);
    let registry = BackendRegistry::default();
    let preparing = Instant::now();
    let mut prepared = registry.prepare(
        &request.selection,
        &snapshot,
        &request,
        &tools,
        &limits,
        &cancel,
    )?;
    let prepare_seconds = preparing.elapsed().as_secs_f64();
    let key = turninator::export::identity::pixel_recipe_key(
        &value,
        &snapshot.mesh.snapshot_hash()?,
        &request,
        &prepared.identity,
    )?;
    let context = RenderContext {
        cancel: &cancel,
        limits: &limits,
        scratch_dir: &job,
        selection: &prepared.effective_selection,
        identity: &prepared.identity,
        pixel_recipe_key: &key,
        job_id: 1,
        revision: 1,
        progress: &|_| {},
    };
    let rendering = Instant::now();
    let mut measured = Measured {
        inner: prepared.backend.as_mut(),
        producer_seconds: 0.,
        sink_seconds: 0.,
        frames: vec![],
    };
    let encoding_started = std::cell::Cell::new(None);
    let result =
        run_export_with_backend(&mut measured, &tools, &request, &context, &mut |event| {
            if let turninator::export::JobEvent::Progress { stage, .. } = event
                && stage == "Encoding and validating"
            {
                encoding_started.set(Some(Instant::now()));
            }
        })?;
    let pipeline_seconds = rendering.elapsed().as_secs_f64();
    let encoding_and_validation_seconds = encoding_started.get().map(|t| t.elapsed().as_secs_f64());
    let producer_seconds = measured.producer_seconds;
    let sink_seconds = measured.sink_seconds;
    let frame_intervals = std::mem::take(&mut measured.frames);
    drop(measured);
    let report = json!({"schema_version":1,"recipe":"matched-renderers-v1","backend":backend,"scene":fixture,"still":still,
        "size":request.size,"frames":if still {1} else {80},"fps":20,"quality":request.selection.quality,
        "identity":prepared.identity,"mesh_sha256":snapshot.mesh.snapshot_hash()?,"pixel_recipe_key":key,
        "prepare_seconds":prepare_seconds,"preparation_phases":prepared.preparation_timings.iter().map(|(k,v)|(k,v.as_secs_f64())).collect::<std::collections::BTreeMap<_,_>>(),
        "producer_seconds":producer_seconds,"sink_seconds":sink_seconds,"encoding_and_validation_seconds":encoding_and_validation_seconds,"frame_intervals":frame_intervals,
        "pipeline_seconds":pipeline_seconds,"end_to_end_seconds":start.elapsed().as_secs_f64(),"output":result.path,"output_bytes":std::fs::metadata(&result.path)?.len(),
        "memory_allowance":{"host_bytes":limits.host_bytes,"gpu_bytes":limits.gpu_bytes},"driver_timing_scope":"host elapsed; not isolated GPU kernel time"});
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "{} {} still={} pipeline={pipeline_seconds:.3}s",
        backend.as_str(),
        fixture,
        still
    );
    // Explicitly release contexts before the registry's idle SDK lease.
    drop(prepared);
    drop(registry);
    Ok(())
}
