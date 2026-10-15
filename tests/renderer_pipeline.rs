use std::{
    collections::BTreeMap,
    fs,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use turninator::{
    Result,
    export::{
        ExportKind, ExportRequest,
        encode::Scratch,
        frame_sink::FrameSink,
        identity::pixel_recipe_key,
        pipeline::{encode_only, run_export_with_backend},
    },
    mesh::Mesh,
    presentation::{SceneSettings, render_scene},
    render::{BackendId, RenderSelection, ToolPaths, contract::*, sequence::plan_frames},
};

fn fixture() -> (Scratch, ExportRequest, RenderSceneSnapshot, BackendIdentity) {
    let scratch = Scratch::new(&std::env::temp_dir()).unwrap();
    let cache = turninator::cache::CacheStore::new(scratch.0.join("cache")).unwrap();
    let job = cache.create_job("pipeline").unwrap();
    let mesh = Arc::new(Mesh {
        positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 1.]],
        triangles: vec![[0, 1, 2]],
    });
    mesh.write_binary(&job.join("mesh.bin")).unwrap();
    let value = render_scene(
        &SceneSettings::default(),
        &job.join("mesh.bin"),
        "original",
        &[],
        None,
    )
    .unwrap();
    fs::write(job.join("scene.json"), serde_json::to_vec(&value).unwrap()).unwrap();
    let scene = snapshot_scene(mesh, value).unwrap();
    let selection = RenderSelection::for_backend(BackendId::Wgpu, true);
    let identity = BackendIdentity {
        backend: BackendId::Wgpu,
        adapter_recipe_version: 1,
        runtime_digest: "test-runtime".into(),
        bridge_digest: None,
        shader_digest: Some("test-shader".into()),
        material_digest: None,
        color_digest: "test-color".into(),
        denoiser_digest: None,
        device: "test-device".into(),
        api: "test-api".into(),
        driver: "test-driver".into(),
    };
    let request = ExportRequest {
        job_dir: job.clone(),
        scene_path: job.join("scene.json"),
        destination: scratch.0.join("out.png"),
        kind: ExportKind::Still,
        selection,
        size: 16,
        ..Default::default()
    };
    (scratch, request, scene, identity)
}
struct Pixels {
    identity: BackendIdentity,
    calls: usize,
    emitted: Vec<FrameKey>,
    bad: bool,
}
impl RenderBackend for Pixels {
    fn id(&self) -> BackendId {
        self.identity.backend
    }
    fn identity(&self) -> BackendIdentity {
        self.identity.clone()
    }
    fn assess(
        &self,
        _: &RenderSceneSnapshot,
        _: &ExportRequest,
    ) -> Vec<turninator::render::capabilities::FeatureSupport> {
        vec![
            turninator::render::capabilities::FeatureSupport::Approximate {
                reason: "Conformance fixture pixels".into(),
            },
        ]
    }
    fn render_sequence(
        &mut self,
        _: &RenderSceneSnapshot,
        plan: &FramePlan,
        ctx: &RenderContext<'_>,
        emit: &mut dyn FnMut(FrameArtifact) -> Result<()>,
    ) -> Result<RenderReport> {
        self.calls += 1;
        for f in plan
            .frames
            .iter()
            .rev()
            .filter(|f| !plan.repeat_of.contains_key(&f.key))
        {
            self.emitted.push(f.key);
            emit(FrameArtifact::Image {
                key: f.key,
                image: RgbaFrame {
                    width: f.width,
                    height: f.height,
                    bytes: vec![
                        255;
                        if self.bad {
                            1
                        } else {
                            (f.width * f.height * 4) as usize
                        }
                    ],
                },
            })?;
        }
        Ok(RenderReport {
            frame_count: (plan.frames.len() - plan.repeat_of.len()) as u32,
            warnings: vec![],
            identity: self.identity.clone(),
            quality: ctx.selection.quality.clone(),
            analyses: BTreeMap::new(),
            build_analyses: BTreeMap::new(),
            phase_timings: BTreeMap::new(),
        })
    }
}
fn tools() -> ToolPaths {
    ToolPaths {
        blender: "missing-blender".into(),
        ffmpeg: "missing-ffmpeg".into(),
        ffprobe: "missing-ffprobe".into(),
    }
}
fn execute(
    request: &ExportRequest,
    scene: &RenderSceneSnapshot,
    backend: &mut dyn RenderBackend,
    cancel: &AtomicBool,
) -> Result<turninator::export::ExportResult> {
    let identity = backend.identity();
    let mut selection = request.selection.clone();
    selection.device = Some(identity.device.clone());
    let key = pixel_recipe_key(
        &scene.legacy_scene,
        &scene.mesh.snapshot_hash()?,
        request,
        &identity,
    )?;
    let limits = RenderLimits::default();
    let ctx = RenderContext {
        cancel,
        limits: &limits,
        scratch_dir: &request.job_dir,
        selection: &selection,
        identity: &identity,
        pixel_recipe_key: &key,
        job_id: 1,
        revision: 2,
        progress: &|_| {},
    };
    run_export_with_backend(backend, &tools(), request, &ctx, &mut |_| {})
}
#[test]
fn png_without_external_tools_and_reuses_verified_pixels() {
    let (_scratch, request, scene, identity) = fixture();
    let mut backend = Pixels {
        identity,
        calls: 0,
        emitted: Vec::new(),
        bad: false,
    };
    let cancel = AtomicBool::new(false);
    execute(&request, &scene, &mut backend, &cancel).unwrap();
    let first = fs::read(&request.destination).unwrap();
    execute(&request, &scene, &mut backend, &cancel).unwrap();
    assert_eq!(backend.calls, 1);
    assert_eq!(first, fs::read(&request.destination).unwrap());
    let manifest = fs::read_dir(&request.job_dir)
        .unwrap()
        .map(|e| e.unwrap().path().join("job.json"))
        .find(|p| p.is_file())
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&fs::read(manifest).unwrap()).unwrap();
    assert_eq!(
        value["recipe"]["assessment"][0]["reason"],
        "Conformance fixture pixels"
    );
}
#[test]
fn failed_frame_keeps_destination_and_retry_state() {
    let (_scratch, request, scene, identity) = fixture();
    fs::write(&request.destination, b"original destination").unwrap();
    let mut backend = Pixels {
        identity,
        calls: 0,
        emitted: Vec::new(),
        bad: true,
    };
    let cancel = AtomicBool::new(false);
    assert!(execute(&request, &scene, &mut backend, &cancel).is_err());
    assert_eq!(
        fs::read(&request.destination).unwrap(),
        b"original destination"
    );
    assert!(
        fs::read_dir(&request.job_dir).unwrap().any(|entry| entry
            .unwrap()
            .path()
            .join("job.json")
            .exists())
    );
    backend.bad = false;
    execute(&request, &scene, &mut backend, &cancel).unwrap();
    assert_eq!(backend.calls, 2);
}
#[test]
fn encode_only_survives_missing_original_renderer() {
    let (_scratch, request, scene, identity) = fixture();
    let mut backend = Pixels {
        identity,
        calls: 0,
        emitted: Vec::new(),
        bad: false,
    };
    let cancel = AtomicBool::new(false);
    execute(&request, &scene, &mut backend, &cancel).unwrap();
    let expected = fs::read(&request.destination).unwrap();
    fs::remove_file(&request.destination).unwrap();
    encode_only(&tools(), &request, &cancel, &mut |_| {})
        .unwrap()
        .unwrap();
    assert_eq!(fs::read(&request.destination).unwrap(), expected);
    assert_eq!(backend.calls, 1);
}
#[test]
fn sink_rejects_duplicate_unknown_missing_outside_and_wrong_alpha() {
    let (_scratch, request, scene, identity) = fixture();
    let plan = plan_frames(&scene, &request).unwrap();
    let cancel = AtomicBool::new(false);
    let limits = RenderLimits::default();
    let ctx = RenderContext {
        cancel: &cancel,
        limits: &limits,
        scratch_dir: &request.job_dir,
        selection: &request.selection,
        identity: &identity,
        pixel_recipe_key: "test",
        job_id: 1,
        revision: 2,
        progress: &|_| {},
    };
    let mut sink = FrameSink::new(&plan, &ctx).unwrap();
    assert!(sink.finish().is_err());
    let key = plan.frames[0].key;
    let image = || RgbaFrame {
        width: 16,
        height: 16,
        bytes: vec![255; 1024],
    };
    assert!(
        sink.accept(FrameArtifact::Image {
            key: FrameKey { index: 99, ..key },
            image: image()
        })
        .is_err()
    );
    assert!(
        sink.accept(FrameArtifact::Image {
            key,
            image: RgbaFrame {
                width: 16,
                height: 16,
                bytes: vec![0; 1024]
            }
        })
        .is_err()
    );
    let outside = request.destination.with_file_name("outside.png");
    image::RgbaImage::from_pixel(16, 16, image::Rgba([255; 4]))
        .save(&outside)
        .unwrap();
    assert!(
        sink.accept(FrameArtifact::Png { key, path: outside })
            .is_err()
    );
    let producer = request.job_dir.join("producer.png");
    image::RgbaImage::from_pixel(16, 16, image::Rgba([255; 4]))
        .save(&producer)
        .unwrap();
    sink.accept(FrameArtifact::Png {
        key,
        path: producer.clone(),
    })
    .unwrap();
    assert!(producer.is_file(), "producer must survive the whole batch");
    assert!(
        sink.accept(FrameArtifact::Image {
            key,
            image: image()
        })
        .is_err()
    );
    sink.finish().unwrap();
    cancel.store(true, Ordering::Release);
    assert!(sink.finish().is_err());
}

#[test]
fn legacy_recipe_recovery_keeps_original_provenance_and_rejects_other_pixels() {
    use serde_json::json;
    use sha2::{Digest, Sha256};
    let (_scratch, mut request, scene, _) = fixture();
    request.selection = RenderSelection::from_legacy("eevee", 8).unwrap();
    let digest = |path: &std::path::Path| format!("{:x}", Sha256::digest(fs::read(path).unwrap()));
    let recipe = json!({"schema_version":2,"scene_hash":digest(&request.scene_path),"mesh_hash":scene.mesh.snapshot_hash().unwrap(),"engine":"eevee","size":16,"samples":8,"frame_count":1,"frame_rate":null,"transparent":false,"section":null,"layout":"Single","animation":"Turntable","timeline":null,"turntable_timing":null,"build_stage":null,"start_angle_degrees":0.,"kind":"still","worker_hash":"a".repeat(64),"scene_script_hash":"b".repeat(64),"frame_pipeline_hash":"c".repeat(64),"blender_hash":"d".repeat(64)});
    let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&recipe).unwrap()));
    let directory = request.job_dir.join(&fingerprint[..20]);
    let frames = directory.join("frames");
    fs::create_dir_all(&frames).unwrap();
    let job = json!({"recipe":recipe,"stage":"complete"});
    let job_bytes = serde_json::to_vec(&job).unwrap();
    fs::write(directory.join("job.json"), &job_bytes).unwrap();
    let manifest = json!({"schema_version":1,"scene":scene.legacy_scene,"width":16,"height":16,"frame_rate":20,"angles":[0.],"engine":"eevee","size":16,"samples":8,"mode":"still","transparent":false,"geometry_hash":recipe["mesh_hash"],"worker_hash":recipe["worker_hash"],"scene_script_hash":recipe["scene_script_hash"],"frame_pipeline_hash":recipe["frame_pipeline_hash"]});
    fs::write(
        frames.join("render-manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(
        frames.join("render-result.json"),
        serde_json::to_vec(
            &json!({"frame_count":1,"frame_rate":20,"engine":"eevee","size":16,"samples":8}),
        )
        .unwrap(),
    )
    .unwrap();
    image::RgbaImage::from_pixel(16, 16, image::Rgba([17, 38, 92, 255]))
        .save(frames.join("still.png"))
        .unwrap();
    let pixels = fs::read(frames.join("still.png")).unwrap();
    let cancel = AtomicBool::new(false);
    assert!(
        encode_only(&tools(), &request, &cancel, &mut |_| {})
            .unwrap()
            .is_some()
    );
    assert_eq!(fs::read(&request.destination).unwrap(), pixels);
    request.selection = RenderSelection::for_backend(BackendId::Wgpu, true);
    assert!(
        encode_only(&tools(), &request, &cancel, &mut |_| {})
            .unwrap()
            .is_none()
    );
    request.selection = RenderSelection::from_legacy("eevee", 16).unwrap();
    assert!(
        encode_only(&tools(), &request, &cancel, &mut |_| {})
            .unwrap()
            .is_none()
    );
    assert_eq!(fs::read(directory.join("job.json")).unwrap(), job_bytes);
    assert_eq!(fs::read(frames.join("still.png")).unwrap(), pixels);
}

#[test]
fn duplicate_panel_plan_and_repeat_inputs_are_rejected() {
    let (_scratch, request, scene, identity) = fixture();
    let mut plan = plan_frames(&scene, &request).unwrap();
    let first = plan.frames[0].clone();
    plan.frames.push(first.clone());
    let cancel = AtomicBool::new(false);
    let limits = RenderLimits::default();
    let ctx = RenderContext {
        cancel: &cancel,
        limits: &limits,
        scratch_dir: &request.job_dir,
        selection: &request.selection,
        identity: &identity,
        pixel_recipe_key: "test",
        job_id: 1,
        revision: 1,
        progress: &|_| {},
    };
    assert!(FrameSink::new(&plan, &ctx).is_err());
    plan.frames[1].key.index = 1;
    plan.frames[1].angle_degrees = 90.;
    plan.repeat_of.insert(plan.frames[1].key, first.key);
    assert!(FrameSink::new(&plan, &ctx).is_err());
}

#[test]
fn planner_preserves_historical_whole_layer_schedules() {
    let (_scratch, mut request, mut scene, _) = fixture();
    scene.metadata.solid_layer_tops_mm = vec![0.25, 0.5, 0.75, 1.];
    request.kind = ExportKind::Gif;
    request.frame_count = 4;
    request.animation = turninator::export::AnimationMode::BuildUp;
    let plan = plan_frames(&scene, &request).unwrap();
    assert_eq!(
        plan.frames
            .iter()
            .map(|f| f.visible_layers.unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    request.timeline = Some(turninator::animation::Timeline {
        duration_seconds: 0.2,
        buildup: true,
        ..Default::default()
    });
    let plan = plan_frames(&scene, &request).unwrap();
    assert_eq!(
        plan.frames
            .iter()
            .map(|f| f.visible_layers.unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    request.kind = ExportKind::Still;
    request.build_stage = Some(turninator::export::BuildStage { visible_layers: 0 });
    assert_eq!(
        plan_frames(&scene, &request).unwrap().frames[0].visible_layers,
        Some(0)
    );
}

#[test]
fn nonexistent_destination_inside_retained_job_is_rejected_before_render() {
    let (_scratch, mut request, scene, identity) = fixture();
    request.destination = request.job_dir.join("new.png");
    let mut backend = Pixels {
        identity,
        calls: 0,
        emitted: Vec::new(),
        bad: false,
    };
    let cancel = AtomicBool::new(false);
    assert!(execute(&request, &scene, &mut backend, &cancel).is_err());
    assert_eq!(backend.calls, 0);
    assert!(!request.destination.exists());
}

#[test]
fn corrupt_panel_retry_renders_only_missing_pixels_and_keeps_good_files() {
    let (_scratch, mut request, scene, identity) = fixture();
    request.kind = ExportKind::Diagram;
    request.size = 256;
    let mut backend = Pixels {
        identity,
        calls: 0,
        bad: false,
        emitted: Vec::new(),
    };
    let cancel = AtomicBool::new(false);
    execute(&request, &scene, &mut backend, &cancel).unwrap();
    assert_eq!(backend.emitted.len(), 4);
    backend.emitted.clear();
    let directory = fs::read_dir(&request.job_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.join("host-frames").exists())
        .unwrap();
    let kept = directory.join("host-frames/Top-0000.png");
    let before = fs::read(&kept).unwrap();
    let modified = fs::metadata(&kept).unwrap().modified().unwrap();
    fs::write(directory.join("host-frames/Right-0000.png"), b"corrupt").unwrap();
    execute(&request, &scene, &mut backend, &cancel).unwrap();
    assert_eq!(
        backend.emitted,
        vec![FrameKey {
            index: 0,
            panel: RenderPanel::Right
        }]
    );
    assert_eq!(fs::read(&kept).unwrap(), before);
    assert_eq!(fs::metadata(&kept).unwrap().modified().unwrap(), modified);
}

struct InterruptedPixels {
    pixels: Pixels,
    fail_after_first: bool,
}
impl RenderBackend for InterruptedPixels {
    fn id(&self) -> BackendId {
        self.pixels.id()
    }
    fn identity(&self) -> BackendIdentity {
        self.pixels.identity()
    }
    fn assess(
        &self,
        _: &RenderSceneSnapshot,
        _: &ExportRequest,
    ) -> Vec<turninator::render::capabilities::FeatureSupport> {
        vec![]
    }
    fn render_sequence(
        &mut self,
        scene: &RenderSceneSnapshot,
        plan: &FramePlan,
        ctx: &RenderContext<'_>,
        emit: &mut dyn FnMut(FrameArtifact) -> Result<()>,
    ) -> Result<RenderReport> {
        let fail = self.fail_after_first;
        self.pixels
            .render_sequence(scene, plan, ctx, &mut |artifact| {
                emit(artifact)?;
                if fail {
                    ctx.cancel.store(true, Ordering::Release);
                    Err("interrupted after durable first frame".into())
                } else {
                    Ok(())
                }
            })
    }
}

#[test]
fn interrupted_first_attempt_retries_only_missing_pixels() {
    let (_scratch, mut request, scene, identity) = fixture();
    request.kind = ExportKind::Diagram;
    request.size = 256;
    fs::write(&request.destination, b"original destination").unwrap();
    let mut backend = InterruptedPixels {
        pixels: Pixels {
            identity,
            calls: 0,
            bad: false,
            emitted: vec![],
        },
        fail_after_first: true,
    };
    assert!(
        execute(&request, &scene, &mut backend, &AtomicBool::new(false))
            .unwrap_err()
            .to_string()
            .contains("interrupted")
    );
    assert_eq!(
        fs::read(&request.destination).unwrap(),
        b"original destination"
    );
    let directory = fs::read_dir(&request.job_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.join("host-frames").exists())
        .unwrap();
    let kept = directory.join("host-frames/Right-0000.png");
    let bytes = fs::read(&kept).unwrap();
    let mtime = fs::metadata(&kept).unwrap().modified().unwrap();
    backend.fail_after_first = false;
    backend.pixels.emitted.clear();
    execute(&request, &scene, &mut backend, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        backend
            .pixels
            .emitted
            .iter()
            .map(|k| k.panel)
            .collect::<Vec<_>>(),
        vec![RenderPanel::Front, RenderPanel::Iso, RenderPanel::Top]
    );
    assert_eq!(fs::read(&kept).unwrap(), bytes);
    assert_eq!(fs::metadata(&kept).unwrap().modified().unwrap(), mtime);
}

struct AnalyzedPixels {
    identity: BackendIdentity,
    fail_after: Option<usize>,
    emitted: Vec<FrameKey>,
    generation: u32,
}
impl RenderBackend for AnalyzedPixels {
    fn id(&self) -> BackendId {
        self.identity.backend
    }
    fn identity(&self) -> BackendIdentity {
        self.identity.clone()
    }
    fn assess(
        &self,
        _: &RenderSceneSnapshot,
        _: &ExportRequest,
    ) -> Vec<turninator::render::capabilities::FeatureSupport> {
        vec![]
    }
    fn render_sequence(
        &mut self,
        _: &RenderSceneSnapshot,
        plan: &FramePlan,
        ctx: &RenderContext<'_>,
        emit: &mut dyn FnMut(FrameArtifact) -> Result<()>,
    ) -> Result<RenderReport> {
        let mut report = RenderReport {
            frame_count: plan.frames.len() as u32,
            warnings: vec![],
            identity: self.identity(),
            quality: ctx.selection.quality.clone(),
            analyses: BTreeMap::new(),
            build_analyses: BTreeMap::new(),
            phase_timings: BTreeMap::new(),
        };
        for (index, f) in plan.frames.iter().rev().enumerate() {
            if f.section.is_some() {
                let legacy = serde_json::json!({"axis":0,"position_mm":0.5,"keep_positive":true,"capped":true,"closed_input":true,
                    "cross_section_area_mm2":1.,"original_volume_mm3":1.,"remaining_volume_mm3":0.5,"output_triangles":12,
                    "warnings":[],"volume_basis":"Closed source mesh.","meaning":"Geometric intersection.","generation":self.generation});
                report
                    .analyses
                    .insert(f.key, SectionAnalysis::from_legacy(legacy, true)?);
            }
            if f.visible_layers.is_some() {
                let legacy = serde_json::json!({"closed_input":true,"capped":false,"remaining_volume_mm3":1.,"output_triangles":12,"visible_layer":0,"z_top_mm":1.,"generation":self.generation});
                report
                    .build_analyses
                    .insert(f.key, BuildAnalysis::from_legacy(legacy)?);
            }
            self.emitted.push(f.key);
            emit(FrameArtifact::Metadata {
                key: f.key,
                section: report.analyses.get(&f.key).cloned(),
                build: report.build_analyses.get(&f.key).cloned(),
            })?;
            emit(FrameArtifact::Image {
                key: f.key,
                image: RgbaFrame {
                    width: f.width,
                    height: f.height,
                    bytes: vec![255; (f.width * f.height * 4) as usize],
                },
            })?;
            if self.fail_after == Some(index + 1) {
                return Err("interrupted after analyzed pixels".into());
            }
        }
        Ok(report)
    }
}

#[test]
fn interrupted_paired_analysis_survives_retry_and_completed_frame_repair() {
    for fail_after in [1, 2] {
        let (_scratch, mut request, mut scene, identity) = fixture();
        request.layout = turninator::export::ExportLayout::SideBySide;
        request.size = 32;
        request.section = Some(turninator::export::SectionSpec {
            axis: 0,
            position_mm: 0.5,
            keep_positive: true,
            cap: true,
        });
        request.build_stage = Some(turninator::export::BuildStage { visible_layers: 1 });
        scene.legacy_scene["solid_layer_tops_mm"] = serde_json::json!([1.]);
        scene.metadata.solid_layer_tops_mm = vec![1.];
        fs::write(
            &request.scene_path,
            serde_json::to_vec(&scene.legacy_scene).unwrap(),
        )
        .unwrap();
        fs::write(&request.destination, b"original destination").unwrap();
        let mut backend = AnalyzedPixels {
            identity,
            fail_after: Some(fail_after),
            emitted: vec![],
            generation: 1,
        };
        assert!(
            execute(&request, &scene, &mut backend, &AtomicBool::new(false))
                .unwrap_err()
                .to_string()
                .contains("interrupted")
        );
        assert_eq!(
            fs::read(&request.destination).unwrap(),
            b"original destination"
        );
        let directory = fs::read_dir(&request.job_dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.join("host-frames").exists())
            .unwrap();
        let kept = directory.join("host-frames/Section-0000.png");
        let bytes = fs::read(&kept).unwrap();
        let mtime = fs::metadata(&kept).unwrap().modified().unwrap();
        let read_report = || -> serde_json::Value {
            serde_json::from_slice::<serde_json::Value>(
                &fs::read(directory.join("job.json")).unwrap(),
            )
            .unwrap()["report"]
                .clone()
        };
        assert!(read_report().is_null());
        let partial: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.join("job.json")).unwrap()).unwrap();
        let retained_section = partial["frame_metadata"]["analyses"].clone();
        let retained_build = partial["frame_metadata"]["build_analyses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["key"]["panel"] == "Section")
            .unwrap()
            .clone();
        backend.fail_after = None;
        backend.generation = 2;
        backend.emitted.clear();
        execute(&request, &scene, &mut backend, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            backend.emitted,
            if fail_after == 1 {
                vec![FrameKey {
                    index: 0,
                    panel: RenderPanel::Full,
                }]
            } else {
                vec![]
            }
        );
        let first = read_report();
        assert_eq!(first["analyses"], retained_section);
        assert!(
            first["build_analyses"]
                .as_array()
                .unwrap()
                .contains(&retained_build)
        );
        assert_eq!(first["analyses"].as_array().unwrap().len(), 1);
        assert_eq!(
            first["analyses"][0]["key"],
            serde_json::json!({"index":0,"panel":"Section"})
        );
        assert_eq!(first["analyses"][0]["legacy"]["generation"], 1);
        assert_eq!(first["build_analyses"].as_array().unwrap().len(), 2);
        let full = first["build_analyses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["key"]["panel"] == "Full")
            .unwrap()
            .clone();
        assert_eq!(
            full["legacy"]["generation"],
            if fail_after == 1 { 2 } else { 1 }
        );
        let section = first["build_analyses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["key"]["panel"] == "Section")
            .unwrap();
        assert_eq!(section["legacy"]["generation"], 1);
        assert_eq!(fs::read(&kept).unwrap(), bytes);
        assert_eq!(fs::metadata(&kept).unwrap().modified().unwrap(), mtime);
        fs::write(&kept, b"corrupt").unwrap();
        backend.generation = 3;
        backend.emitted.clear();
        execute(&request, &scene, &mut backend, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            backend.emitted,
            vec![FrameKey {
                index: 0,
                panel: RenderPanel::Section
            }]
        );
        let repaired = read_report();
        assert_eq!(repaired["analyses"][0]["key"], first["analyses"][0]["key"]);
        assert_eq!(repaired["analyses"][0]["legacy"]["generation"], 3);
        assert!(
            repaired["build_analyses"]
                .as_array()
                .unwrap()
                .contains(&full)
        );
        let section = repaired["build_analyses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["key"]["panel"] == "Section")
            .unwrap();
        assert_eq!(section["legacy"]["generation"], 3);
    }
}

struct RejectedReportPixels {
    pixels: AnalyzedPixels,
    rejection: &'static str,
}
impl RenderBackend for RejectedReportPixels {
    fn id(&self) -> BackendId {
        self.pixels.id()
    }
    fn identity(&self) -> BackendIdentity {
        self.pixels.identity()
    }
    fn assess(
        &self,
        _: &RenderSceneSnapshot,
        _: &ExportRequest,
    ) -> Vec<turninator::render::capabilities::FeatureSupport> {
        vec![]
    }
    fn render_sequence(
        &mut self,
        scene: &RenderSceneSnapshot,
        plan: &FramePlan,
        ctx: &RenderContext<'_>,
        emit: &mut dyn FnMut(FrameArtifact) -> Result<()>,
    ) -> Result<RenderReport> {
        let mut report = self.pixels.render_sequence(scene, plan, ctx, emit)?;
        match self.rejection {
            "identity" => report.identity.runtime_digest.push_str("-unexpected"),
            "quality" => report.quality = turninator::render::BackendQuality::Eevee { samples: 1 },
            "count" => report.frame_count += 1,
            "missing" => report.analyses.clear(),
            "malformed" => {
                report.build_analyses.values_mut().next().unwrap().legacy["visible_layer"] =
                    serde_json::json!(-8)
            }
            "disagreement" => {
                report.analyses.values_mut().next().unwrap().legacy["remaining_volume_mm3"] =
                    serde_json::json!(0.25)
            }
            "valid" => {}
            _ => unreachable!(),
        }
        Ok(report)
    }
}

fn rejected_report_cannot_recover(rejection: &'static str) {
    for repair in [false, true] {
        let (_scratch, mut request, mut scene, identity) = fixture();
        request.layout = turninator::export::ExportLayout::SideBySide;
        request.size = 32;
        request.section = Some(turninator::export::SectionSpec {
            axis: 0,
            position_mm: 0.5,
            keep_positive: true,
            cap: true,
        });
        request.build_stage = Some(turninator::export::BuildStage { visible_layers: 1 });
        scene.legacy_scene["solid_layer_tops_mm"] = serde_json::json!([1.]);
        scene.metadata.solid_layer_tops_mm = vec![1.];
        fs::write(
            &request.scene_path,
            serde_json::to_vec(&scene.legacy_scene).unwrap(),
        )
        .unwrap();
        fs::write(&request.destination, b"original destination").unwrap();
        let mut backend = RejectedReportPixels {
            pixels: AnalyzedPixels {
                identity,
                fail_after: None,
                emitted: vec![],
                generation: 1,
            },
            rejection: "valid",
        };
        let directory = request.job_dir.join(
            &pixel_recipe_key(
                &scene.legacy_scene,
                &scene.mesh.snapshot_hash().unwrap(),
                &request,
                &backend.identity(),
            )
            .unwrap()[..20],
        );
        let full = directory.join("host-frames/Full-0000.png");
        let section = directory.join("host-frames/Section-0000.png");
        if repair {
            execute(&request, &scene, &mut backend, &AtomicBool::new(false)).unwrap();
            fs::write(&section, b"corrupt").unwrap();
        }
        let good_before = repair.then(|| {
            (
                fs::read(&full).unwrap(),
                fs::metadata(&full).unwrap().modified().unwrap(),
            )
        });
        let destination = fs::read(&request.destination).unwrap();
        backend.rejection = rejection;
        assert!(execute(&request, &scene, &mut backend, &AtomicBool::new(false)).is_err());
        let retained: Vec<_> = [&full, &section]
            .into_iter()
            .map(|p| {
                (
                    p,
                    fs::read(p).unwrap(),
                    fs::metadata(p).unwrap().modified().unwrap(),
                )
            })
            .collect();
        if let Some((bytes, mtime)) = good_before {
            assert_eq!(fs::read(&full).unwrap(), bytes);
            assert_eq!(fs::metadata(&full).unwrap().modified().unwrap(), mtime);
        }
        assert_eq!(fs::read(&request.destination).unwrap(), destination);
        backend.rejection = "valid";
        backend.pixels.emitted.clear();
        let recovery = encode_only(&tools(), &request, &AtomicBool::new(false), &mut |_| {});
        let retry = execute(&request, &scene, &mut backend, &AtomicBool::new(false));
        assert!(
            retry.is_err(),
            "{rejection}, repair={repair}: retry adopted rejected artifacts"
        );
        assert!(
            recovery.is_err(),
            "{rejection}, repair={repair}: recovery did not reject known invalid artifacts"
        );
        assert!(backend.pixels.emitted.is_empty());
        assert_eq!(fs::read(&request.destination).unwrap(), destination);
        if rejection == "identity" {
            // A new frozen runtime identity owns a distinct artifact directory;
            // the rejected original remains intact and cannot shadow good output.
            backend
                .pixels
                .identity
                .runtime_digest
                .push_str("-new-runtime");
            execute(&request, &scene, &mut backend, &AtomicBool::new(false)).unwrap();
            assert_eq!(backend.pixels.emitted.len(), 2);
            assert!(
                encode_only(&tools(), &request, &AtomicBool::new(false), &mut |_| {})
                    .unwrap()
                    .is_some()
            );
        }
        for (path, bytes, mtime) in retained {
            assert_eq!(fs::read(path).unwrap(), bytes);
            assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), mtime);
        }
    }
}
#[test]
fn rejected_identity_remains_rejected_on_retry_and_recovery() {
    rejected_report_cannot_recover("identity");
}
#[test]
fn rejected_quality_remains_rejected_on_retry_and_recovery() {
    rejected_report_cannot_recover("quality");
}
#[test]
fn rejected_count_remains_rejected_on_retry_and_recovery() {
    rejected_report_cannot_recover("count");
}
#[test]
fn rejected_missing_analysis_remains_rejected_on_retry_and_recovery() {
    rejected_report_cannot_recover("missing");
}
#[test]
fn rejected_malformed_analysis_remains_rejected_on_retry_and_recovery() {
    rejected_report_cannot_recover("malformed");
}
#[test]
fn rejected_analysis_disagreement_remains_rejected_on_retry_and_recovery() {
    rejected_report_cannot_recover("disagreement");
}

struct InvalidCompletionWorker(serde_json::Value);
impl turninator::render::worker::WorkerTransport for InvalidCompletionWorker {
    fn request(
        &mut self,
        request: serde_json::Value,
        _: &AtomicBool,
        emit: &mut dyn FnMut(serde_json::Value),
    ) -> Result<serde_json::Value> {
        let directory = std::path::Path::new(request["output_dir"].as_str().unwrap());
        fs::create_dir_all(directory)?;
        let path = directory.join("still.png");
        image::RgbaImage::from_pixel(
            request["width"].as_u64().unwrap() as u32,
            request["height"].as_u64().unwrap() as u32,
            image::Rgba([255; 4]),
        )
        .save(&path)?;
        emit(serde_json::json!({"kind":"frame","payload":{"index":0,"path":path}}));
        Ok(serde_json::json!({"kind":"complete","payload":{"frame_analysis":self.0}}))
    }
    fn cancel(&mut self) {}
}
#[test]
fn rejected_blender_completion_remains_rejected_on_retry_and_recovery() {
    for analysis in [
        serde_json::json!("malformed final report"),
        serde_json::json!([]),
        serde_json::json!([{}]),
    ] {
        let (_scratch, mut request, scene, mut identity) = fixture();
        request.selection = RenderSelection::for_backend(BackendId::BlenderEevee, true);
        identity.backend = BackendId::BlenderEevee;
        let mut selection = request.selection.clone();
        selection.device = Some(identity.device.clone());
        let mut backend = turninator::render::backends::blender::BlenderBackend::with_transport(
            selection,
            request.clone(),
            identity,
            Box::new(InvalidCompletionWorker(analysis)),
        )
        .unwrap();
        fs::write(&request.destination, b"original destination").unwrap();
        assert!(execute(&request, &scene, &mut backend, &AtomicBool::new(false)).is_err());
        let recovery = encode_only(&tools(), &request, &AtomicBool::new(false), &mut |_| {});
        let retry = execute(&request, &scene, &mut backend, &AtomicBool::new(false));
        assert!(retry.is_err(), "retry adopted malformed Blender completion");
        assert!(
            recovery.is_err(),
            "recovery did not reject malformed Blender completion"
        );
        assert_eq!(
            fs::read(&request.destination).unwrap(),
            b"original destination"
        );
    }
}

#[test]
#[ignore = "requires installed FFmpeg/ffprobe; fake renderer exercises real GIF/MP4 encoding without Blender"]
fn gif_and_mp4_share_verified_pixels_without_rerendering() {
    let (_scratch, mut request, scene, identity) = fixture();
    request.kind = ExportKind::Gif;
    request.size = 32;
    request.frame_count = 2;
    request.destination.set_extension("gif");
    let mut backend = Pixels {
        identity: identity.clone(),
        calls: 0,
        bad: false,
        emitted: Vec::new(),
    };
    let cancel = AtomicBool::new(false);
    let limits = RenderLimits::default();
    let mut selection = request.selection.clone();
    selection.device = Some(identity.device.clone());
    let tools = ToolPaths {
        blender: "missing".into(),
        ..Default::default()
    };
    let mut run = |request: &ExportRequest| {
        let key = pixel_recipe_key(
            &scene.legacy_scene,
            &scene.mesh.snapshot_hash().unwrap(),
            request,
            &identity,
        )
        .unwrap();
        let context = RenderContext {
            cancel: &cancel,
            limits: &limits,
            scratch_dir: &request.job_dir,
            selection: &selection,
            identity: &identity,
            pixel_recipe_key: &key,
            job_id: 1,
            revision: 1,
            progress: &|_| {},
        };
        run_export_with_backend(&mut backend, &tools, request, &context, &mut |_| {}).unwrap()
    };
    let gif = run(&request);
    assert_eq!(gif.frame_count, 2);
    request.kind = ExportKind::Mp4;
    request.destination.set_extension("mp4");
    let video = run(&request);
    assert_eq!(video.frame_count, 2);
    assert_eq!(backend.calls, 1);
    assert_eq!(backend.emitted.len(), 2);
}

#[test]
fn actor_fresh_prepares_changed_runtime_but_recovery_needs_no_renderer() {
    use std::{sync::atomic::AtomicUsize, time::Duration};
    use turninator::{
        render::registry::PreparedBackend,
        session::{RenderEvent, RenderService, RenderTask},
    };
    let (_scratch, request, scene, identity) = fixture();
    for (id, runtime, recovery) in [
        (1, "runtime-one", false),
        (2, "runtime-two", false),
        (3, "absent-runtime", true),
    ] {
        let preparations = Arc::new(AtomicUsize::new(0));
        let counted = preparations.clone();
        let mut identity = identity.clone();
        identity.runtime_digest = runtime.into();
        let service = RenderService::with_backend_factory(move |selection, _, _, _, _, _| {
            counted.fetch_add(1, Ordering::SeqCst);
            if recovery {
                return Err("renderer is deliberately absent".into());
            }
            let mut selection = selection.clone();
            selection.device = Some(identity.device.clone());
            Ok(PreparedBackend {
                backend: Box::new(Pixels {
                    identity: identity.clone(),
                    calls: 0,
                    bad: false,
                    emitted: Vec::new(),
                }),
                identity: identity.clone(),
                effective_selection: selection,
                assessment: vec![],
                preparation_timings: BTreeMap::new(),
            })
        });
        let task = RenderTask {
            id,
            revision: id,
            mesh: scene.mesh.clone(),
            scene: scene.legacy_scene.clone(),
            request: request.clone(),
            preview: false,
            tools: tools(),
            cache_budget_bytes: 100_000_000,
        };
        let original_request = if recovery {
            let mut bytes = fs::read(request.job_dir.join("request.json")).unwrap();
            bytes.extend_from_slice(b"\n  ");
            fs::write(request.job_dir.join("request.json"), &bytes).unwrap();
            Some(bytes)
        } else {
            None
        };
        if recovery {
            service.submit_recovery(task).unwrap();
        } else {
            service.submit(task).unwrap();
        }
        loop {
            match service.events.recv_timeout(Duration::from_secs(3)).unwrap() {
                RenderEvent::Completed {
                    id: done,
                    revision,
                    engine,
                    ..
                } => {
                    assert_eq!((done, revision, engine), (id, id, BackendId::Wgpu));
                    break;
                }
                RenderEvent::Failed { message, .. } => panic!("actor{id}: {message}"),
                _ => {}
            }
        }
        drop(service);
        if let Some(original) = original_request {
            assert_eq!(
                fs::read(request.job_dir.join("request.json")).unwrap(),
                original,
                "recovery must preserve the stored original request bytes"
            );
        }
        assert_eq!(preparations.load(Ordering::SeqCst), usize::from(!recovery));
    }
    assert_eq!(
        fs::read_dir(&request.job_dir)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().path().join("host-frames").is_dir())
            .count(),
        2,
        "fresh runtime creates a separate artifact set; recovery preserves both originals"
    );
}

#[test]
#[ignore = "requires installed Blender GPU and FFmpeg; records real common-pipeline batching and keyed timeline/build evidence"]
fn real_blender_24_frame_batch_rebind_and_paired_timeline_build() {
    use serde_json::{Value, json};
    use std::{
        path::PathBuf,
        sync::atomic::AtomicUsize,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    use turninator::{
        animation::{CameraKeyframe, Timeline, TurntableTiming},
        export::{ExportLayout, SectionSpec},
        render::registry::BackendRegistry,
        session::{RenderEvent, RenderService, RenderTask},
    };
    struct Capture {
        inner: Box<dyn RenderBackend>,
        root: PathBuf,
    }
    impl RenderBackend for Capture {
        fn id(&self) -> BackendId {
            self.inner.id()
        }
        fn identity(&self) -> BackendIdentity {
            self.inner.identity()
        }
        fn configure(&mut self, r: &ExportRequest) -> Result<()> {
            self.inner.configure(r)
        }
        fn assess(
            &self,
            s: &RenderSceneSnapshot,
            r: &ExportRequest,
        ) -> Vec<turninator::render::capabilities::FeatureSupport> {
            self.inner.assess(s, r)
        }
        fn render_sequence(
            &mut self,
            s: &RenderSceneSnapshot,
            p: &FramePlan,
            c: &RenderContext<'_>,
            e: &mut dyn FnMut(FrameArtifact) -> Result<()>,
        ) -> Result<RenderReport> {
            let report = self.inner.render_sequence(s, p, c, e)?;
            fs::write(
                self.root.join(format!("plan-{}.json", c.job_id)),
                serde_json::to_vec_pretty(
                    &json!({"frames":p.frames,"transparent":p.transparent,"repeats":p.repeat_of.iter().collect::<Vec<_>>(),"identity":report.identity,"analyses":report.analyses.iter().map(|(k,a)|json!({"key":k,"legacy":a.legacy})).collect::<Vec<_>>(),"build_analyses":report.build_analyses.iter().map(|(k,a)|json!({"key":k,"legacy":a.legacy})).collect::<Vec<_>>()}),
                )?,
            )?;
            for entry in fs::read_dir(c.scratch_dir)? {
                let directory = entry?.path();
                let result = directory.join("frames/render-result.json");
                if result.is_file() {
                    let label = directory.file_name().unwrap().to_string_lossy();
                    fs::copy(
                        result,
                        self.root
                            .join(format!("producer-{}-{label}.json", c.job_id)),
                    )?;
                }
            }
            Ok(report)
        }
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts/renderers/task-3")
        .join(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                .to_string(),
        );
    fs::create_dir_all(&root).unwrap();
    let cache = turninator::cache::CacheStore::new(root.join("jobs")).unwrap();
    let mesh = Arc::new(Mesh {
        positions: vec![
            [-1., -1., 0.],
            [1., -1., 0.],
            [1., 1., 0.],
            [-1., 1., 0.],
            [-1., -1., 4.],
            [1., -1., 4.],
            [1., 1., 4.],
            [-1., 1., 4.],
        ],
        triangles: vec![
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
        ],
    });
    let prepared = Arc::new(AtomicUsize::new(0));
    let count = prepared.clone();
    let capture_root = root.clone();
    let service = RenderService::with_backend_factory(
        move |selection, scene, request, tools, limits, cancel| {
            let mut prepared = BackendRegistry::default()
                .prepare(selection, scene, request, tools, limits, cancel)?;
            count.fetch_add(1, Ordering::SeqCst);
            fs::write(
                capture_root.join("preparation.json"),
                serde_json::to_vec_pretty(
                    &json!({"identity":prepared.identity,"phase_seconds":prepared.preparation_timings.iter().map(|(k,v)|(k.clone(),v.as_secs_f64())).collect::<BTreeMap<_,_>>()}),
                )?,
            )?;
            prepared.backend = Box::new(Capture {
                inner: prepared.backend,
                root: capture_root.clone(),
            });
            Ok(prepared)
        },
    );
    let selection = RenderSelection::from_legacy("eevee", 8).unwrap();
    let mut evidence = Vec::new();
    for id in 1..=3 {
        let job = cache.create_job(&format!("job{id}")).unwrap();
        let mut value = render_scene(
            &SceneSettings::default(),
            &job.join("mesh.bin"),
            "original",
            &[],
            None,
        )
        .unwrap();
        value["solid_layer_tops_mm"] = json!([1., 2., 3., 4.]);
        let mut request = ExportRequest {
            job_dir: job.clone(),
            scene_path: job.join("scene.json"),
            destination: root.join(format!(
                "output-{id}.{}",
                if id == 2 { "png" } else { "gif" }
            )),
            kind: if id == 2 {
                ExportKind::Still
            } else {
                ExportKind::Gif
            },
            selection: selection.clone(),
            size: 128,
            frame_count: 24,
            frame_rate: 24,
            turntable_timing: Some(TurntableTiming {
                seconds_per_turn: 1.,
                duration_seconds: 1.,
            }),
            ..Default::default()
        };
        if id == 2 {
            request.transparent = true;
            request.frame_rate = 30;
            request.turntable_timing = None;
            request.start_angle_degrees = 35.;
        }
        if id == 3 {
            request.frame_rate = 20;
            request.turntable_timing = None;
            request.timeline = Some(Timeline {
                duration_seconds: 0.3,
                buildup: true,
                turntable: true,
                keyframes: vec![CameraKeyframe {
                    camera: turninator::viewport::OrbitCamera {
                        distance_mm: 16.,
                        target_mm: [0., 0., 2.],
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            });
            request.layout = ExportLayout::SideBySide;
            request.section = Some(SectionSpec {
                axis: 0,
                position_mm: 0.,
                keep_positive: true,
                cap: true,
            });
        }
        let started = Instant::now();
        service
            .submit(RenderTask {
                id,
                revision: id,
                mesh: mesh.clone(),
                scene: value,
                request,
                preview: false,
                tools: ToolPaths::default(),
                cache_budget_bytes: 1_000_000_000,
            })
            .unwrap();
        loop {
            match service
                .events
                .recv_timeout(Duration::from_secs(120))
                .unwrap()
            {
                RenderEvent::Completed {
                    id: done,
                    revision,
                    engine,
                    result,
                    ..
                } => {
                    assert_eq!((done, revision, engine), (id, id, BackendId::BlenderEevee));
                    evidence.push(
                        json!({"id":id,"seconds":started.elapsed().as_secs_f64(),"output":result}),
                    );
                    break;
                }
                RenderEvent::Failed { message, .. } => panic!("real export{id}: {message}"),
                _ => {}
            }
        }
    }
    assert_eq!(
        prepared.load(Ordering::SeqCst),
        1,
        "job settings must rebind without repeated runtime startup"
    );
    drop(service);
    let batch: Value =
        serde_json::from_slice(&fs::read(root.join("producer-1-blender-Single.json")).unwrap())
            .unwrap();
    assert_eq!(batch["render_strategy"], "animation");
    assert_eq!(batch["frame_count"], 24);
    let plan: Value = serde_json::from_slice(&fs::read(root.join("plan-3.json")).unwrap()).unwrap();
    assert_eq!(plan["frames"].as_array().unwrap().len(), 12);
    assert_eq!(plan["analyses"].as_array().unwrap().len(), 6);
    assert_eq!(plan["build_analyses"].as_array().unwrap().len(), 12);
    let still = image::open(root.join("output-2.png")).unwrap().to_rgba8();
    assert!(still.pixels().any(|p| p[3] < 255));
    fs::write(
        root.join("evidence.json"),
        serde_json::to_vec_pretty(
            &json!({"exports":evidence,"preparations":1,"batch_strategy":batch["render_strategy"]}),
        )
        .unwrap(),
    )
    .unwrap();
    eprintln!("Task3 real evidence: {}", root.display());
}
