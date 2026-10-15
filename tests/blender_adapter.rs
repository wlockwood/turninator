use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use turninator::{
    Result,
    export::{ExportKind, ExportRequest},
    mesh::Mesh,
    presentation::{SceneSettings, render_scene},
    render::{
        BackendId, BackendQuality, RenderSelection, backends::blender::BlenderBackend, contract::*,
        worker::WorkerTransport,
    },
};

struct RecordingWorker {
    requests: Rc<RefCell<Vec<Value>>>,
    cancel_during_request: bool,
}
impl WorkerTransport for RecordingWorker {
    fn request(
        &mut self,
        request: Value,
        cancel: &AtomicBool,
        event: &mut dyn FnMut(Value),
    ) -> Result<Value> {
        if cancel.load(Ordering::Acquire) {
            return Err("cancelled".into());
        }
        self.requests.borrow_mut().push(request.clone());
        if self.cancel_during_request {
            cancel.store(true, Ordering::Release);
            return Err("cancelled".into());
        }
        let animated = request["mode"] != "still";
        for index in 0..request["frame_count"].as_u64().unwrap() {
            let path = Path::new(request["output_dir"].as_str().unwrap()).join(if animated {
                format!("frame_{index:04}.png")
            } else {
                "still.png".into()
            });
            event(json!({"kind":"frame","payload":{"index":index,"path":path}}));
        }
        Ok(
            json!({"kind":"complete","payload":{"frame_count":request["frame_count"],"warnings":["worker warning"],"total_seconds":0.5}}),
        )
    }
    fn cancel(&mut self) {}
}
fn fixture() -> RenderSceneSnapshot {
    let mesh = Arc::new(Mesh {
        positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 1.]],
        triangles: vec![[0, 1, 2]],
    });
    let scene = render_scene(
        &SceneSettings::default(),
        Path::new("D:/owned/mesh.bin"),
        "original",
        &[],
        None,
    )
    .unwrap();
    snapshot_scene(mesh, scene).unwrap()
}
fn identity(selection: &RenderSelection) -> BackendIdentity {
    BackendIdentity {
        backend: selection.backend,
        adapter_recipe_version: 1,
        runtime_digest: "test-runtime".into(),
        bridge_digest: None,
        shader_digest: None,
        material_digest: None,
        color_digest: "test-color".into(),
        denoiser_digest: None,
        device: selection.device.clone().unwrap(),
        api: "test-api".into(),
        driver: "test-driver".into(),
    }
}
fn plan(scene: &RenderSceneSnapshot) -> FramePlan {
    FramePlan {
        frames: (0..8)
            .map(|index| FrameSpec {
                key: FrameKey {
                    index,
                    panel: RenderPanel::Single,
                },
                camera: scene.camera.clone(),
                angle_degrees: f64::from(index) * 45.,
                visible_layers: None,
                section: None,
                width: 256,
                height: 256,
                time_seconds: f64::from(index) / 20.,
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn prepared_blender_rebinds_job_rate_timing_and_alpha_without_new_transport() {
    let scene = fixture();
    let mut selection = RenderSelection::for_backend(BackendId::BlenderEevee, false);
    selection.device = Some("test".into());
    let identity = identity(&selection);
    let requests = Rc::new(RefCell::new(Vec::new()));
    let initial = ExportRequest {
        selection: selection.clone(),
        frame_rate: 20,
        transparent: false,
        ..Default::default()
    };
    let mut backend = BlenderBackend::with_transport(
        selection.clone(),
        initial.clone(),
        identity.clone(),
        Box::new(RecordingWorker {
            requests: requests.clone(),
            cancel_during_request: false,
        }),
    )
    .unwrap();
    let mut rebound = initial;
    rebound.kind = ExportKind::Still;
    rebound.transparent = true;
    rebound.frame_rate = 30;
    rebound.turntable_timing = Some(turninator::animation::TurntableTiming {
        seconds_per_turn: 2.,
        duration_seconds: 0.3,
    });
    backend.configure(&rebound).unwrap();
    let scratch = turninator::export::encode::Scratch::new(&std::env::temp_dir()).unwrap();
    let cancel = AtomicBool::new(false);
    let limits = RenderLimits::default();
    let context = RenderContext {
        cancel: &cancel,
        limits: &limits,
        scratch_dir: &scratch.0,
        selection: &selection,
        identity: &identity,
        pixel_recipe_key: "test",
        job_id: 9,
        revision: 4,
        progress: &|_| {},
    };
    let mut frames = plan(&scene);
    frames.transparent = true;
    for (index, frame) in frames.frames.iter_mut().enumerate() {
        frame.angle_degrees = index as f64 * 6.;
    }
    backend
        .render_sequence(&scene, &frames, &context, &mut |_| Ok(()))
        .unwrap();
    assert_eq!(requests.borrow().len(), 1);
    assert_eq!(requests.borrow()[0]["frame_rate"], 30);
    assert_eq!(requests.borrow()[0]["transparent"], true);
    assert_eq!(requests.borrow()[0]["seconds_per_turn"], 2.);
    rebound.selection.device = Some("other".into());
    assert!(backend.configure(&rebound).is_err());
}

#[test]
fn blender_adapter_keeps_batch_request_and_owned_cancel() {
    for backend in [BackendId::BlenderEevee, BackendId::BlenderCycles] {
        let scene = fixture();
        let frames = plan(&scene);
        let mut selection = RenderSelection::for_backend(backend, false);
        *selection.quality.samples_mut().unwrap() = 8;
        selection.device = Some("concrete-test-gpu".into());
        let identity = identity(&selection);
        let request = ExportRequest {
            kind: ExportKind::Gif,
            selection: selection.clone(),
            size: 256,
            frame_count: 8,
            ..Default::default()
        };
        let requests = Rc::new(RefCell::new(Vec::new()));
        let mut adapter: Box<dyn RenderBackend> = Box::new(
            BlenderBackend::with_transport(
                selection.clone(),
                request.clone(),
                identity.clone(),
                Box::new(RecordingWorker {
                    requests: requests.clone(),
                    cancel_during_request: false,
                }),
            )
            .unwrap(),
        );
        let scratch = std::env::temp_dir().join(format!(
            "turninator-adapter-{}-{}",
            std::process::id(),
            backend.as_str()
        ));
        std::fs::create_dir_all(&scratch).unwrap();
        let cancel = AtomicBool::new(false);
        let limits = RenderLimits::default();
        let progress = RefCell::new(Vec::new());
        let on_progress = |p| progress.borrow_mut().push(p);
        let context = RenderContext {
            cancel: &cancel,
            limits: &limits,
            scratch_dir: &scratch,
            selection: &selection,
            identity: &identity,
            pixel_recipe_key: "test",
            job_id: 7,
            revision: 11,
            progress: &on_progress,
        };
        let mut artifacts = Vec::new();
        let report = adapter
            .render_sequence(&scene, &frames, &context, &mut |artifact| {
                artifacts.push(artifact);
                Ok(())
            })
            .unwrap();
        assert_eq!(report.frame_count, 8);
        assert_eq!(report.warnings, vec!["worker warning"]);
        assert_eq!(artifacts.len(), 8);
        assert_eq!(progress.borrow().len(), 8);
        let requests = requests.borrow();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]["engine"],
            backend.legacy_blender_engine().unwrap()
        );
        assert_eq!(requests[0]["samples"], 8);
        assert_eq!(requests[0]["mode"], "turntable");
        assert!(requests[0].get("frame_states").is_none());
        assert!(requests[0].get("frame_states_path").is_none());
        let saved_scene: Value = serde_json::from_slice(
            &std::fs::read(requests[0]["scene_path"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        assert!(
            saved_scene.get("camera").is_none() || saved_scene["camera"].is_null(),
            "preserve legacy Blender auto-fit"
        );
        drop(requests);
        let other_cancel = AtomicBool::new(false);
        let mut cancelled = BlenderBackend::with_transport(
            selection.clone(),
            request,
            identity.clone(),
            Box::new(RecordingWorker {
                requests: Rc::new(RefCell::new(Vec::new())),
                cancel_during_request: true,
            }),
        )
        .unwrap();
        let cancel_scratch = scratch.join("cancel");
        std::fs::create_dir_all(&cancel_scratch).unwrap();
        let cancelled_context = RenderContext {
            scratch_dir: &cancel_scratch,
            ..context
        };
        assert!(
            cancelled
                .render_sequence(&scene, &frames, &cancelled_context, &mut |_| Ok(()))
                .is_err()
        );
        assert!(cancel.load(Ordering::Acquire));
        assert!(!other_cancel.load(Ordering::Acquire));
        std::fs::remove_dir_all(scratch).unwrap();
    }
}

#[test]
fn blender_rejects_unhonored_denoise_before_any_request() {
    let selection = RenderSelection {
        backend: BackendId::BlenderCycles,
        device: Some("gpu".into()),
        quality: BackendQuality::Cycles {
            samples: 8,
            denoise: false,
        },
    };
    let requests = Rc::new(RefCell::new(Vec::new()));
    assert!(
        BlenderBackend::with_transport(
            selection.clone(),
            ExportRequest {
                selection: selection.clone(),
                ..Default::default()
            },
            identity(&selection),
            Box::new(RecordingWorker {
                requests: requests.clone(),
                cancel_during_request: false
            })
        )
        .is_err()
    );
    assert!(requests.borrow().is_empty());
}

struct ProbeWorker {
    probe: Value,
    requests: Rc<RefCell<Vec<Value>>>,
}

struct CancellingProbe {
    stopped: Rc<Cell<bool>>,
    dropped: Rc<Cell<bool>>,
}
impl WorkerTransport for CancellingProbe {
    fn request(
        &mut self,
        _: Value,
        cancel: &AtomicBool,
        _: &mut dyn FnMut(Value),
    ) -> Result<Value> {
        cancel.store(true, Ordering::Release);
        Err("cancelled during probe".into())
    }
    fn cancel(&mut self) {
        self.stopped.set(true);
    }
}
impl Drop for CancellingProbe {
    fn drop(&mut self) {
        self.dropped.set(true);
    }
}
#[test]
fn preparation_uses_the_job_cancel_token_and_releases_its_transport() {
    use turninator::render::{ToolPaths, registry::BackendRegistry};
    let scene = fixture();
    let selection = RenderSelection::default();
    let request = ExportRequest {
        kind: ExportKind::Still,
        selection: selection.clone(),
        ..Default::default()
    };
    let tools = ToolPaths {
        blender: std::env::current_exe().unwrap(),
        ffmpeg: "missing".into(),
        ffprobe: "missing".into(),
    };
    let stopped = Rc::new(Cell::new(false));
    let dropped = Rc::new(Cell::new(false));
    let cancel = AtomicBool::new(false);
    let other = AtomicBool::new(false);
    let result = BackendRegistry::default().prepare_with_transport(
        &selection,
        &scene,
        &request,
        &tools,
        &RenderLimits::default(),
        &cancel,
        Box::new(CancellingProbe {
            stopped: stopped.clone(),
            dropped: dropped.clone(),
        }),
    );
    assert!(result.is_err());
    assert!(cancel.load(Ordering::Acquire));
    assert!(!other.load(Ordering::Acquire));
    assert!(stopped.get());
    assert!(dropped.get());
}

#[test]
#[ignore = "requires installed Blender; cancels preparation during its explicit runtime probe"]
fn real_preparation_cancellation_terminates_owned_worker() {
    use std::time::{Duration, Instant};
    use turninator::render::{ToolPaths, registry::BackendRegistry};
    let scene = fixture();
    let selection = RenderSelection::default();
    let request = ExportRequest {
        kind: ExportKind::Still,
        selection: selection.clone(),
        ..Default::default()
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let trigger = cancel.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(700));
        trigger.store(true, Ordering::Release);
    });
    let started = Instant::now();
    let result = BackendRegistry::default().prepare(
        &selection,
        &scene,
        &request,
        &ToolPaths::default(),
        &RenderLimits::default(),
        &cancel,
    );
    thread.join().unwrap();
    assert!(result.is_err());
    assert!(result.err().unwrap().to_string().contains("cancelled"));
    assert!(started.elapsed() < Duration::from_secs(10));
    eprintln!(
        "owned preparation cancellation returned in {:?}",
        started.elapsed()
    );
}
impl WorkerTransport for ProbeWorker {
    fn request(
        &mut self,
        request: Value,
        _: &AtomicBool,
        _: &mut dyn FnMut(Value),
    ) -> Result<Value> {
        self.requests.borrow_mut().push(request);
        Ok(json!({"kind":"probe","payload":self.probe}))
    }
    fn cancel(&mut self) {}
}
#[test]
fn preparation_binds_only_the_probed_configuration_and_rejects_missing_identity() {
    use turninator::render::{
        ToolPaths, capabilities::BackendAvailability, registry::BackendRegistry,
    };
    let scene = fixture();
    let limits = RenderLimits::default();
    let tools = ToolPaths {
        blender: std::env::current_exe().unwrap(),
        ffmpeg: "missing-ffmpeg".into(),
        ffprobe: "missing-ffprobe".into(),
    };
    for backend in [BackendId::BlenderEevee, BackendId::BlenderCycles] {
        let selection = RenderSelection::for_backend(backend, false);
        let request = ExportRequest {
            kind: ExportKind::Still,
            selection: selection.clone(),
            ..Default::default()
        };
        let probe = json!({"blender_version":"test-version","blender_build_hash":"test-build","engines":["eevee","cycles"],"cycles_backend":"OPTIX",
            "devices":[{"type":"OPTIX","id":"gpu-1","enabled":true}],
            "gpu":{"backend":"OPENGL","vendor":"vendor","renderer":"actual-gpu","version":"actual-driver"}});
        let requests = Rc::new(RefCell::new(Vec::new()));
        let registry = BackendRegistry::default();
        let prepared = registry
            .prepare_with_transport(
                &selection,
                &scene,
                &request,
                &tools,
                &limits,
                &AtomicBool::new(false),
                Box::new(ProbeWorker {
                    probe: probe.clone(),
                    requests: requests.clone(),
                }),
            )
            .unwrap();
        assert_eq!(requests.borrow().len(), 1);
        assert_eq!(requests.borrow()[0]["kind"], "probe");
        assert_eq!(requests.borrow()[0]["initialize_gpu"], true);
        assert_eq!(
            prepared.identity.device,
            if backend == BackendId::BlenderCycles {
                "OPTIX:gpu-1"
            } else {
                "OPENGL:vendor:actual-gpu"
            }
        );
        assert_eq!(prepared.identity.driver, "actual-driver");
        assert_eq!(
            prepared.effective_selection.device.as_deref(),
            Some(prepared.identity.device.as_str())
        );
        assert_eq!(selection.device, None);
        assert_eq!(
            registry.status(backend).availability,
            BackendAvailability::Available
        );
        let saved = prepared.effective_selection.clone();
        let saved_request = ExportRequest {
            selection: saved.clone(),
            ..request.clone()
        };
        registry
            .prepare_with_transport(
                &saved,
                &scene,
                &saved_request,
                &tools,
                &limits,
                &AtomicBool::new(false),
                Box::new(ProbeWorker {
                    probe: probe.clone(),
                    requests: requests.clone(),
                }),
            )
            .unwrap();
        let mut absent = saved;
        absent.device = Some("unavailable-saved-device".into());
        let absent_request = ExportRequest {
            selection: absent.clone(),
            ..request.clone()
        };
        let error = registry
            .prepare_with_transport(
                &absent,
                &scene,
                &absent_request,
                &tools,
                &limits,
                &AtomicBool::new(false),
                Box::new(ProbeWorker {
                    probe: probe.clone(),
                    requests: requests.clone(),
                }),
            )
            .err()
            .unwrap();
        assert!(
            error
                .to_string()
                .contains("saved Blender device is unavailable")
        );
        assert_eq!(absent.device.as_deref(), Some("unavailable-saved-device"));
        let mut missing = probe;
        missing["gpu"]["version"] = json!("");
        assert!(
            registry
                .prepare_with_transport(
                    &selection,
                    &scene,
                    &request,
                    &tools,
                    &limits,
                    &AtomicBool::new(false),
                    Box::new(ProbeWorker {
                        probe: missing,
                        requests
                    })
                )
                .is_err()
        );
    }
}

#[test]
fn legacy_execution_rejects_explicit_device_and_embedded_backends() {
    use turninator::render::backends::blender::validate_legacy_execution;
    for backend in BackendId::ALL {
        let mut selection = RenderSelection::for_backend(backend, false);
        assert_eq!(
            validate_legacy_execution(&selection).is_ok(),
            matches!(backend, BackendId::BlenderEevee | BackendId::BlenderCycles)
        );
        selection.device = Some("saved-device".into());
        assert!(validate_legacy_execution(&selection).is_err());
    }
}

#[test]
fn malformed_repeats_fail_before_worker_execution() {
    let scene = fixture();
    let mut frames = plan(&scene);
    frames.repeat_of.insert(
        FrameKey {
            index: 99,
            panel: RenderPanel::Single,
        },
        frames.frames[0].key,
    );
    let mut selection = RenderSelection::from_legacy("eevee", 8).unwrap();
    selection.device = Some("test-gpu".into());
    let identity = identity(&selection);
    let request = ExportRequest {
        selection: selection.clone(),
        ..Default::default()
    };
    let requests = Rc::new(RefCell::new(Vec::new()));
    let mut adapter = BlenderBackend::with_transport(
        selection.clone(),
        request,
        identity.clone(),
        Box::new(RecordingWorker {
            requests: requests.clone(),
            cancel_during_request: false,
        }),
    )
    .unwrap();
    let scratch =
        std::env::temp_dir().join(format!("turninator-invalid-repeat-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let cancel = AtomicBool::new(false);
    let limits = RenderLimits::default();
    let context = RenderContext {
        cancel: &cancel,
        limits: &limits,
        scratch_dir: &scratch,
        selection: &selection,
        identity: &identity,
        pixel_recipe_key: "test",
        job_id: 1,
        revision: 1,
        progress: &|_| {},
    };
    assert!(
        adapter
            .render_sequence(&scene, &frames, &context, &mut |_| Ok(()))
            .is_err()
    );
    assert!(requests.borrow().is_empty());
    std::fs::remove_dir_all(scratch).unwrap();
}

struct AnalysisWorker {
    requests: Rc<RefCell<Vec<Value>>>,
}
impl WorkerTransport for AnalysisWorker {
    fn request(
        &mut self,
        request: Value,
        cancel: &AtomicBool,
        event: &mut dyn FnMut(Value),
    ) -> Result<Value> {
        let section = request["scene_path"].as_str().unwrap().contains("Section");
        let mut delegate = RecordingWorker {
            requests: self.requests.clone(),
            cancel_during_request: false,
        };
        let build0 = json!({"closed_input":false,"capped":false,"remaining_volume_mm3":0.,"output_triangles":0,"visible_layer":-1,"z_top_mm":0.});
        let build1 = json!({"closed_input":true,"capped":false,"remaining_volume_mm3":1.,"output_triangles":12,"visible_layer":0,"z_top_mm":1.});
        let empty = json!({"axis":2,"position_mm":0.5,"keep_positive":false,"capped":false,"closed_input":false,
            "cross_section_area_mm2":0.,"original_volume_mm3":0.,"remaining_volume_mm3":0.,"visible_layer":-1,
            "visible_source_triangles":0,"output_triangles":0,"warnings":[],"meaning":"Empty build stage: no deposited geometry is visible."});
        let cut = json!({"axis":2,"position_mm":0.5,"keep_positive":false,"capped":true,"closed_input":true,
            "cross_section_area_mm2":1.,"original_volume_mm3":1.,"remaining_volume_mm3":0.5,"output_triangles":12,
            "warnings":[],"volume_basis":"Closed source mesh.","meaning":"Geometric intersection."});
        let analyses = if section {
            json!([empty, cut])
        } else {
            json!([null, null])
        };
        let builds = json!([build0, build1]);
        let mut completed = delegate.request(request, cancel, &mut |mut frame| {
            let index = frame["payload"]["index"].as_u64().unwrap() as usize;
            frame["payload"]["section_analysis"] = analyses[index].clone();
            frame["payload"]["build_analysis"] = builds[index].clone();
            event(frame);
        })?;
        completed["payload"]["frame_analysis"] = analyses;
        completed["payload"]["frame_build_analysis"] = builds;
        Ok(completed)
    }
    fn cancel(&mut self) {}
}
#[test]
fn paired_sections_and_build_stages_keep_frame_keys_and_empty_stage_analysis() {
    use turninator::export::SectionSpec;
    let mut scene = fixture();
    scene.legacy_scene["solid_layer_tops_mm"] = json!([1.]);
    scene.metadata.solid_layer_tops_mm = vec![1.];
    let mut frames = FramePlan::default();
    for panel in [RenderPanel::Full, RenderPanel::Section] {
        for (index, count) in [(5, 0), (9, 1)] {
            frames.frames.push(FrameSpec {
                key: FrameKey { index, panel },
                camera: scene.camera.clone(),
                angle_degrees: 0.,
                visible_layers: Some(count),
                section: if panel == RenderPanel::Section {
                    Some(SectionSpec {
                        axis: 2,
                        position_mm: 0.5,
                        keep_positive: false,
                        cap: true,
                    })
                } else {
                    None
                },
                width: 256,
                height: 256,
                time_seconds: f64::from(index) / 20.,
            });
        }
    }
    let mut selection = RenderSelection::from_legacy("eevee", 8).unwrap();
    selection.device = Some("test-gpu".into());
    let identity = identity(&selection);
    let request = ExportRequest {
        selection: selection.clone(),
        ..Default::default()
    };
    let requests = Rc::new(RefCell::new(Vec::new()));
    let mut adapter = BlenderBackend::with_transport(
        selection.clone(),
        request,
        identity.clone(),
        Box::new(AnalysisWorker {
            requests: requests.clone(),
        }),
    )
    .unwrap();
    let scratch =
        std::env::temp_dir().join(format!("turninator-paired-analysis-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let cancel = AtomicBool::new(false);
    let limits = RenderLimits::default();
    let context = RenderContext {
        cancel: &cancel,
        limits: &limits,
        scratch_dir: &scratch,
        selection: &selection,
        identity: &identity,
        pixel_recipe_key: "test",
        job_id: 1,
        revision: 1,
        progress: &|_| {},
    };
    let mut metadata = std::collections::BTreeMap::new();
    let mut pixels = Vec::new();
    let report = adapter
        .render_sequence(&scene, &frames, &context, &mut |artifact| {
            match artifact {
                FrameArtifact::Metadata {
                    key,
                    section,
                    build,
                } => {
                    assert!(!pixels.contains(&key));
                    assert!(metadata.insert(key, (section, build)).is_none());
                }
                FrameArtifact::Png { key, .. } => {
                    assert!(metadata.contains_key(&key), "analysis must precede PNG");
                    pixels.push(key);
                }
                FrameArtifact::Image { .. } => panic!("expected Blender PNG"),
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(pixels.len(), 4);
    for (key, (section, build)) in metadata {
        assert_eq!(
            section.as_ref().map(|a| &a.legacy),
            report.analyses.get(&key).map(|a| &a.legacy)
        );
        assert_eq!(
            build.as_ref().map(|a| &a.legacy),
            report.build_analyses.get(&key).map(|a| &a.legacy)
        );
    }
    assert_eq!(report.analyses.len(), 2);
    assert!(report.analyses.contains_key(&FrameKey {
        index: 5,
        panel: RenderPanel::Section
    }));
    assert!(report.analyses.contains_key(&FrameKey {
        index: 9,
        panel: RenderPanel::Section
    }));
    assert!(!report.analyses.contains_key(&FrameKey {
        index: 5,
        panel: RenderPanel::Full
    }));
    assert_eq!(report.build_analyses.len(), 4);
    for panel in [RenderPanel::Full, RenderPanel::Section] {
        let empty = &report.build_analyses[&FrameKey { index: 5, panel }].legacy;
        assert_eq!(empty["visible_layer"], -1);
        assert_eq!(empty["output_triangles"], 0);
        assert!(empty.get("axis").is_none());
        assert!(empty.get("volume_basis").is_none());
        assert_eq!(
            report.build_analyses[&FrameKey { index: 9, panel }].legacy["remaining_volume_mm3"],
            1.
        );
    }
    for request in requests.borrow().iter() {
        let states: Value = serde_json::from_slice(
            &std::fs::read(request["frame_states_path"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(states[0]["visible_layers"], 0);
        assert_eq!(states[1]["visible_layers"], 1);
        assert_eq!(states[0]["preserve_scene_camera"], true);
        assert!(states[0]["camera"].is_null());
    }
    std::fs::remove_dir_all(scratch).unwrap();
}

#[test]
#[ignore = "requires installed Blender/GPU and retained pre-adapter baseline; writes artifacts/renderers/blender-adapter"]
fn real_adapter_matches_retained_still_and_animation_pixels() {
    use sha2::{Digest, Sha256};
    use turninator::render::{ToolPaths, registry::BackendRegistry};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = Path::new("D:/code/turninator/artifacts/render-performance/axolotl.scene.json");
    let legacy: Value = serde_json::from_slice(&std::fs::read(source).unwrap()).unwrap();
    let mesh =
        Arc::new(Mesh::read_binary(Path::new(legacy["mesh_path"].as_str().unwrap())).unwrap());
    let geometry_hash = format!(
        "{:x}",
        Sha256::digest(std::fs::read(legacy["mesh_path"].as_str().unwrap()).unwrap())
    );
    let baseline = root.join("artifacts/renderers/blender-pre-adapter");
    let out = std::env::var_os("TURNINATOR_ADAPTER_EVIDENCE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("artifacts/renderers/blender-adapter"));
    std::fs::create_dir(&out).unwrap();
    let mut evidence = Vec::new();
    for animation in [false, true] {
        let mut legacy = legacy.clone();
        legacy["fit_turntable"] = json!(true);
        let scene = snapshot_scene(mesh.clone(), legacy).unwrap();
        let mut frames = plan(&scene);
        if !animation {
            frames.frames.truncate(1);
        }
        let selection = RenderSelection::from_legacy("eevee", 8).unwrap();
        let request = ExportRequest {
            kind: if animation {
                ExportKind::Gif
            } else {
                ExportKind::Still
            },
            selection: selection.clone(),
            size: 256,
            frame_count: frames.frames.len() as u32,
            ..Default::default()
        };
        let tools = ToolPaths::default();
        let limits = RenderLimits::default();
        let mut prepared = BackendRegistry::default()
            .prepare(
                &selection,
                &scene,
                &request,
                &tools,
                &limits,
                &AtomicBool::new(false),
            )
            .unwrap();
        let scratch = out.join(if animation { "animation" } else { "still" });
        std::fs::create_dir(&scratch).unwrap();
        let cancel = AtomicBool::new(false);
        let context = RenderContext {
            cancel: &cancel,
            limits: &limits,
            scratch_dir: &scratch,
            selection: &prepared.effective_selection,
            identity: &prepared.identity,
            pixel_recipe_key: "qualified-baseline",
            job_id: 1,
            revision: 1,
            progress: &|_| {},
        };
        let mut pixels = Vec::new();
        let report=prepared.backend.render_sequence(&scene,&frames,&context,&mut |artifact|{
            let FrameArtifact::Png{key,path}=artifact else {panic!("expected PNG")};
            let expected=baseline.join(if animation{"1-worker-animation"}else{"0-still-loop"}).join(format!("frame_{:04}.png",key.index));
            let actual=image::open(&path)?.into_rgba8();let previous=image::open(&expected)?.into_rgba8();
            assert_eq!(actual,previous,"decoded pixel mismatch at {key:?}");
            pixels.push(json!({"index":key.index,"png":path,"rgba_sha256":format!("{:x}",Sha256::digest(actual.as_raw()))}));Ok(())
        }).unwrap();
        let result: Value = serde_json::from_slice(
            &std::fs::read(scratch.join("blender-Single/frames/render-result.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(result["geometry_hash"], geometry_hash);
        assert_eq!(result["triangles"], 58308);
        assert_eq!(
            result["render_strategy"],
            if animation {
                "animation"
            } else {
                "individual_frames"
            }
        );
        evidence.push(json!({"animation":animation,"identity":report.identity,"preparation_timings":prepared.preparation_timings,
            "frame_count":report.frame_count,"phase_timings":report.phase_timings,"worker_result":result,"pixels":pixels}));
    }
    std::fs::write(
        out.join("evidence.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
}
