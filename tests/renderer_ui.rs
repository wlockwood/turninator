use turninator::{
    app::{
        renderer_view::{FrameIdentity, InspectionFeatures, LatestFrameQueue, viewport_route},
        renderers::{material_notice, preview_selection, renderer_options},
    },
    export::OutputFormat,
    render::{
        BackendId, RendererPreferences,
        capabilities::{BackendAvailability, BackendStatus},
        selection::{RenderUsage, ViewportRenderer},
    },
};

#[path = "fixtures/renderers/common.rs"]
mod common;

#[test]
fn section_still_assessment_uses_still_capability_and_rejects_unsupported_alpha() {
    use turninator::render::capabilities::{CapabilityDescriptor, FeatureSupport};
    let scene = common::scene(common::cube([0.; 3], 10.));
    let selection = turninator::render::RenderSelection::for_backend(BackendId::Filament, true);
    let mut request = turninator::export::ExportRequest {
        kind: turninator::export::ExportKind::SectionStill,
        selection: selection.clone(),
        section: Some(turninator::export::SectionSpec {
            axis: 2,
            position_mm: 5.,
            keep_positive: true,
            cap: true,
        }),
        ..Default::default()
    };
    let statuses = [BackendStatus {
        backend: BackendId::Filament,
        availability: BackendAvailability::Available,
        capabilities: Some(CapabilityDescriptor {
            usages: vec![RenderUsage::Still],
            alpha: FeatureSupport::Unsupported {
                reason: "fixture alpha unsupported".into(),
            },
            material_volume: FeatureSupport::Supported,
            sections: FeatureSupport::Supported,
            layer_appearance: FeatureSupport::Supported,
            maximum_output_side: 8000,
        }),
    }];
    assert!(
        turninator::app::renderers::assess_selection(&selection, &scene, &request, &statuses)
            .errors
            .is_empty()
    );
    request.transparent = true;
    assert_eq!(
        turninator::app::renderers::assess_selection(&selection, &scene, &request, &statuses)
            .errors,
        ["fixture alpha unsupported"]
    );
}

#[test]
fn picker_keeps_all_engines_and_explains_missing_runtime() {
    let options = renderer_options(
        &[BackendStatus {
            backend: BackendId::Filament,
            availability: BackendAvailability::Unavailable {
                reason: "Missing turninator_filament.dll".into(),
            },
            capabilities: None,
        }],
        RenderUsage::Still,
    );
    assert_eq!(
        options.iter().map(|o| o.label).collect::<Vec<_>>(),
        [
            "Native wgpu",
            "Filament",
            "LuxCore",
            "Blender Eevee",
            "Blender Cycles / OptiX"
        ]
    );
    let filament = &options[1];
    assert_eq!(filament.backend, BackendId::Filament);
    assert_eq!(
        filament.reason.as_deref(),
        Some("Missing turninator_filament.dll")
    );
    assert!(!filament.available);
    assert!(
        !options[2].available,
        "undiscovered SDK must not claim Available"
    );
}

#[test]
fn format_routes_preview_to_independent_saved_quality_and_resin_keeps_intent() {
    let mut prefs = RendererPreferences::default();
    prefs.select_backend(false, BackendId::Filament);
    prefs.select_backend(true, BackendId::LuxCore);
    *prefs.still.quality.samples_mut().unwrap() = 73;
    prefs.select_backend(true, BackendId::BlenderCycles);
    prefs.select_backend(true, BackendId::LuxCore);
    for format in [OutputFormat::Gif, OutputFormat::Mp4] {
        assert_eq!(
            preview_selection(&prefs, format).backend,
            BackendId::Filament
        );
    }
    for format in [OutputFormat::Png, OutputFormat::FourView] {
        assert_eq!(
            preview_selection(&prefs, format).quality.samples(),
            Some(73)
        );
    }
    let before = prefs.clone();
    assert!(
        material_notice("resin", prefs.animation.backend)
            .unwrap()
            .contains("LuxCore")
    );
    assert_eq!(prefs, before);
    assert!(material_notice("resin", BackendId::LuxCore).is_none());
}

fn tag(job: u64, revision: u64, backend: BackendId, generation: u64) -> FrameIdentity {
    FrameIdentity {
        job,
        revision,
        backend,
        generation,
    }
}

#[test]
fn rapid_requests_replace_pending_and_reject_late_decode_after_switch() {
    let mut queue = LatestFrameQueue::default();
    let a = tag(1, 1, BackendId::Filament, 1);
    let b = tag(2, 1, BackendId::Filament, 2);
    let c = tag(3, 2, BackendId::Wgpu, 3);
    queue.submit(a, "A");
    assert_eq!(queue.take_pending(), Some((a, "A")));
    queue.submit(b, "B");
    queue.submit(c, "C");
    assert!(queue.take_pending().is_none(), "only one render may run");
    assert!(!queue.accepts(a));
    queue.finish(a);
    assert_eq!(queue.take_pending(), Some((c, "C")));
    assert!(!queue.accepts(b));
    assert!(queue.accepts(c));
    queue.finish(a);
    assert!(
        queue.take_pending().is_none(),
        "old completion cannot free current slot"
    );
    queue.finish(c);
    assert!(
        queue.accepts(c),
        "matching asynchronous decode remains current"
    );
    queue.invalidate();
    assert!(!queue.accepts(c));
}

#[test]
fn inspection_override_resumes_preference_without_touching_output_profiles() {
    let prefs = RendererPreferences {
        viewport: ViewportRenderer::Filament,
        ..Default::default()
    };
    let clean = viewport_route(prefs.viewport, InspectionFeatures::default());
    assert_eq!(clean.backend, BackendId::Filament);
    assert!(clean.badge.is_none());
    for features in [
        InspectionFeatures {
            edges: true,
            ..Default::default()
        },
        InspectionFeatures {
            ghost_or_xray: true,
            ..Default::default()
        },
        InspectionFeatures {
            face_selection: true,
            ..Default::default()
        },
        InspectionFeatures {
            cut: true,
            ..Default::default()
        },
    ] {
        let route = viewport_route(prefs.viewport, features);
        assert_eq!(route.backend, BackendId::Wgpu);
        assert_eq!(route.badge, Some("Inspection uses Native wgpu"));
    }
    assert_eq!(
        viewport_route(prefs.viewport, InspectionFeatures::default()).backend,
        BackendId::Filament
    );
    assert_eq!(prefs.animation.backend, BackendId::BlenderEevee);
}

#[test]
fn application_memory_caps_roundtrip_without_changing_renderer_quality() {
    let mut settings = turninator::settings::AppSettings::default();
    let renderers = settings.renderers.clone();
    assert_eq!(settings.render_memory.host_gib, 8);
    assert_eq!(settings.render_memory.gpu_gib, 6);
    settings.render_memory.host_gib = 12;
    settings.render_memory.gpu_gib = 4;
    let saved = serde_json::to_vec(&settings).unwrap();
    let reopened: turninator::settings::AppSettings = serde_json::from_slice(&saved).unwrap();
    assert_eq!(
        reopened.render_memory.limits().unwrap().gpu_bytes,
        4 * 1024 * 1024 * 1024
    );
    assert_eq!(reopened.renderers, renderers);
    settings.render_memory.gpu_gib = 0;
    assert!(settings.render_memory.limits().is_err());
}

#[test]
fn paused_interactive_queue_keeps_last_image_but_cancels_pending_work() {
    let mut queue = LatestFrameQueue::default();
    let first = tag(1, 1, BackendId::Filament, 1);
    queue.submit(first, "frame");
    assert_eq!(queue.take_pending(), Some((first, "frame")));
    queue.set_paused(true);
    queue.submit(tag(2, 2, BackendId::Filament, 2), "newest");
    queue.finish(first);
    assert!(queue.take_pending().is_none());
    queue.set_paused(false);
    assert_eq!(queue.take_pending().unwrap().1, "newest");
}

#[test]
fn interactive_owner_drops_context_on_switch_and_delivers_only_latest_request() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };
    use std::time::Duration;
    use turninator::app::renderer_view::LatestFrameWorker;
    let dropped = Arc::new(AtomicUsize::new(0));
    let drop_count = dropped.clone();
    let (started, receive) = mpsc::channel();
    struct Owned(std::thread::ThreadId, Arc<AtomicUsize>, std::rc::Rc<()>);
    impl Drop for Owned {
        fn drop(&mut self) {
            assert_eq!(self.0, std::thread::current().id());
            self.1.fetch_add(1, Ordering::SeqCst);
        }
    }
    let worker = LatestFrameWorker::new(move || {
        let owned = Owned(
            std::thread::current().id(),
            drop_count.clone(),
            std::rc::Rc::new(()),
        );
        let started = started.clone();
        Box::new(move |value: u32, cancel: &std::sync::atomic::AtomicBool| {
            let _keep_thread_bound = &owned.2;
            started.send(value).unwrap();
            if value == 1 {
                while !cancel.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            Ok(value * 10)
        })
    });
    worker.submit(tag(1, 1, BackendId::Filament, 1), 1);
    assert_eq!(receive.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
    worker.submit(tag(2, 2, BackendId::Filament, 2), 2);
    assert_eq!(receive.recv_timeout(Duration::from_secs(2)).unwrap(), 2);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let result = loop {
        if let Some(result) = worker.take_completed() {
            break result;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(result.0.job, 2);
    assert_eq!(result.1.unwrap(), 20);
    worker.release();
    while dropped.load(Ordering::SeqCst) == 0 {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        worker.is_released(),
        "export can start only after owner teardown"
    );
}
