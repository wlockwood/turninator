#[path = "fixtures/renderers/common.rs"]
mod common;
use common::*;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};
use turninator::{
    Result,
    render::{
        contract::*,
        embedded::{backend::*, *},
        selection::*,
    },
};

#[derive(Default)]
struct Trace {
    uploads: Vec<(u64, f32, [f32; 3])>,
    frames: Vec<(u64, bool)>,
    cancel: u32,
    drift: bool,
    fail_cancel: bool,
}
struct Mock {
    trace: Rc<RefCell<Trace>>,
    pending: bool,
    cancelled: bool,
    dimensions: (u32, u32),
}
impl EmbeddedDriver for Mock {
    fn upload(&mut self, s: &SceneUpload<'_>) -> Result<()> {
        self.trace.borrow_mut().uploads.push((
            s.revision,
            s.positions_mm.first().map_or(0., |p| p[0]),
            s.metadata.materials[0].base_color_linear,
        ));
        Ok(())
    }
    fn begin(&mut self, f: &EmbeddedFrame) -> Result<()> {
        self.trace
            .borrow_mut()
            .frames
            .push((f.geometry_revision, f.transparent));
        self.dimensions = (f.width, f.height);
        self.pending = true;
        self.cancelled = false;
        Ok(())
    }
    fn poll(&mut self) -> Result<EmbeddedPoll> {
        Ok(EmbeddedPoll {
            state: if self.cancelled {
                EmbeddedState::Cancelled
            } else if self.pending {
                EmbeddedState::Ready
            } else {
                EmbeddedState::Idle
            },
            completed_work: 1,
            total_work: Some(1),
        })
    }
    fn read(&mut self, p: &mut [u8]) -> Result<(u32, u32)> {
        self.pending = false;
        p.fill(255);
        Ok(self.dimensions)
    }
    fn cancel(&mut self) -> Result<()> {
        self.trace.borrow_mut().cancel += 1;
        if self.trace.borrow().fail_cancel {
            return Err("injected drain failure".into());
        }
        self.cancelled = true;
        self.pending = false;
        Ok(())
    }
    fn runtime_info(&mut self) -> Result<RuntimeInfo> {
        let sdk = if self.trace.borrow().drift && !self.trace.borrow().frames.is_empty() {
            "changed"
        } else {
            "test"
        };
        Ok(serde_json::from_value(
            serde_json::json!({"schema_version":1,"backend":"filament","sdk_version":sdk,"device_id":"mock","device_name":"mock","api":"mock","driver":"mock"}),
        )?)
    }
}

#[test]
fn runtime_change_is_typed_rejection_even_when_drain_fails_and_context_is_poisoned() {
    let trace = Rc::new(RefCell::new(Trace {
        drift: true,
        fail_cancel: true,
        ..Default::default()
    }));
    let mut studio = EmbeddedStudio::from_driver(
        Mock {
            trace,
            pending: false,
            cancelled: false,
            dimensions: (0, 0),
        },
        selection(),
        identity(),
        RenderLimits::default(),
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    let limits = RenderLimits::default();
    let id = identity();
    let sel = selection();
    let context = RenderContext {
        cancel: &cancel,
        limits: &limits,
        scratch_dir: std::path::Path::new("D:/unused"),
        selection: &sel,
        identity: &id,
        pixel_recipe_key: "test",
        job_id: 9,
        revision: 42,
        progress: &|_| {},
    };
    let s = scene(cube([0.; 3], 10.));
    let error = studio
        .begin_job(&s, &context, false)
        .unwrap()
        .render_frame(&frame(&s))
        .err()
        .unwrap();
    assert!(error.source().unwrap().is::<RenderReportRejected>());
    assert!(studio.begin_job(&s, &context, false).is_err());
}
fn identity() -> BackendIdentity {
    BackendIdentity {
        backend: BackendId::Filament,
        adapter_recipe_version: 1,
        runtime_digest: "mock".into(),
        bridge_digest: None,
        shader_digest: None,
        material_digest: None,
        color_digest: "mock".into(),
        denoiser_digest: None,
        device: "mock".into(),
        api: "mock".into(),
        driver: "mock".into(),
    }
}
fn selection() -> RenderSelection {
    RenderSelection {
        backend: BackendId::Filament,
        device: None,
        quality: BackendQuality::Filament {
            aa: AaQuality::Low,
            shadows: ShadowQuality::Low,
        },
    }
}
#[test]
fn static_reuses_one_upload_but_new_job_invalidates_model_material_and_alpha() {
    let trace = Rc::new(RefCell::new(Trace::default()));
    let mut studio = EmbeddedStudio::from_driver(
        Mock {
            trace: trace.clone(),
            pending: false,
            cancelled: false,
            dimensions: (0, 0),
        },
        selection(),
        identity(),
        RenderLimits::default(),
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    let limits = RenderLimits::default();
    let id = identity();
    let sel = selection();
    let context = RenderContext {
        cancel: &cancel,
        limits: &limits,
        scratch_dir: std::path::Path::new("D:/unused"),
        selection: &sel,
        identity: &id,
        pixel_recipe_key: "test",
        job_id: 9,
        revision: 42,
        progress: &|_| {},
    };
    let s = scene(cube([0.; 3], 10.));
    let f = frame(&s);
    {
        let mut job = studio.begin_job(&s, &context, false).unwrap();
        let first = job.render_frame(&f).unwrap();
        assert_eq!(
            (first.job_id, first.revision, first.backend),
            (9, 42, BackendId::Filament)
        );
        let mut orbit = f.clone();
        orbit.key.index = 1;
        orbit.angle_degrees = 90.;
        assert_eq!(
            job.render_frame(&orbit).unwrap().upload_generation,
            first.upload_generation
        );
    }
    let mut changed = scene(cube([20., 0., 0.], 8.));
    changed.appearance.color = [0.8, 0.1, 0.2];
    {
        let mut job = studio.begin_job(&changed, &context, true).unwrap();
        job.render_frame(&frame(&changed)).unwrap();
    }
    let t = trace.borrow();
    assert_eq!(t.uploads.len(), 2);
    assert_ne!(t.uploads[0].0, t.uploads[1].0);
    assert_ne!(t.uploads[0].1, t.uploads[1].1);
    assert_ne!(t.uploads[0].2, t.uploads[1].2);
    assert_eq!(
        t.frames.iter().map(|f| f.1).collect::<Vec<_>>(),
        vec![false, false, true]
    );
}
#[test]
fn cancellation_after_begin_drains_and_new_job_can_render() {
    let trace = Rc::new(RefCell::new(Trace::default()));
    let mut studio = EmbeddedStudio::from_driver(
        Mock {
            trace: trace.clone(),
            pending: false,
            cancelled: false,
            dimensions: (0, 0),
        },
        selection(),
        identity(),
        RenderLimits::default(),
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    let limits = RenderLimits::default();
    let id = identity();
    let sel = selection();
    let progress = |_: RenderProgress| {
        cancel.store(true, Ordering::Relaxed);
    };
    let context = RenderContext {
        cancel: &cancel,
        limits: &limits,
        scratch_dir: std::path::Path::new("D:/unused"),
        selection: &sel,
        identity: &id,
        pixel_recipe_key: "test",
        job_id: 9,
        revision: 42,
        progress: &progress,
    };
    let s = scene(cube([0.; 3], 10.));
    assert!(
        studio
            .begin_job(&s, &context, false)
            .unwrap()
            .render_frame(&frame(&s))
            .is_err()
    );
    assert_eq!(trace.borrow().cancel, 1);
    let fresh_cancel = AtomicBool::new(false);
    let resumed = RenderContext {
        cancel: &fresh_cancel,
        progress: &|_| {},
        job_id: 10,
        ..context
    };
    studio
        .begin_job(&s, &resumed, false)
        .unwrap()
        .render_frame(&frame(&s))
        .unwrap();
    assert!(cancel.load(Ordering::Relaxed));
}

#[test]
fn sequence_keeps_distinct_build_section_metadata_and_skips_repeats() {
    let trace = Rc::new(RefCell::new(Trace::default()));
    let mut studio = EmbeddedStudio::from_driver(
        Mock {
            trace: trace.clone(),
            pending: false,
            cancelled: false,
            dimensions: (0, 0),
        },
        selection(),
        identity(),
        RenderLimits::default(),
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    let limits = RenderLimits::default();
    let id = identity();
    let sel = selection();
    let context = RenderContext {
        cancel: &cancel,
        limits: &limits,
        scratch_dir: std::path::Path::new("D:/unused"),
        selection: &sel,
        identity: &id,
        pixel_recipe_key: "test",
        job_id: 9,
        revision: 42,
        progress: &|_| {},
    };
    let mut s = scene(cube([0.; 3], 10.));
    s.metadata.solid_layer_tops_mm = vec![5., 10.];
    let mut f = frame(&s);
    f.visible_layers = Some(1);
    f.section = Some(turninator::export::SectionSpec {
        axis: 0,
        position_mm: 5.,
        keep_positive: false,
        cap: true,
    });
    let mut repeat = f.clone();
    repeat.key.index = 1;
    let plan = FramePlan {
        frames: vec![f.clone(), repeat.clone()],
        repeat_of: [(repeat.key, f.key)].into(),
        transparent: false,
    };
    let mut artifacts = Vec::new();
    let report = render_sequence(&mut studio, &s, &plan, &context, &mut |a| {
        artifacts.push(a);
        Ok(())
    })
    .unwrap();
    assert_eq!(report.frame_count, 1);
    assert_eq!(artifacts.len(), 2);
    match &artifacts[0] {
        FrameArtifact::Metadata {
            key,
            section: Some(section),
            build: Some(build),
        } => {
            assert_eq!(*key, f.key);
            near(build.legacy["remaining_volume_mm3"].as_f64().unwrap(), 500.);
            near(
                section.legacy["remaining_volume_mm3"].as_f64().unwrap(),
                250.,
            );
            assert_eq!(report.analyses[key].legacy, section.legacy);
            assert_eq!(report.build_analyses[key].legacy, build.legacy);
        }
        _ => panic!("metadata must precede image"),
    }
    assert!(matches!(artifacts[1], FrameArtifact::Image { .. }));
    assert_eq!(trace.borrow().uploads.len(), 1);
}

#[test]
fn sheet_camera_changes_upload_and_limits_reject_before_driver_work() {
    let trace = Rc::new(RefCell::new(Trace::default()));
    let mut studio = EmbeddedStudio::from_driver(
        Mock {
            trace: trace.clone(),
            pending: false,
            cancelled: false,
            dimensions: (0, 0),
        },
        selection(),
        identity(),
        RenderLimits::default(),
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    let limits = RenderLimits::default();
    let id = identity();
    let sel = selection();
    let context = RenderContext {
        cancel: &cancel,
        limits: &limits,
        scratch_dir: std::path::Path::new("D:/unused"),
        selection: &sel,
        identity: &id,
        pixel_recipe_key: "test",
        job_id: 9,
        revision: 42,
        progress: &|_| {},
    };
    let s = scene(cube([0.; 3], 10.));
    let mut f = frame(&s);
    f.key.panel = RenderPanel::Top;
    let mut job = studio.begin_job(&s, &context, false).unwrap();
    let first = job.render_frame(&f).unwrap();
    f.key.index = 1;
    f.width += 1;
    assert_ne!(
        first.upload_generation,
        job.render_frame(&f).unwrap().upload_generation
    );
    assert_eq!(job.upload_count(), 2);
    f.key.index = 2;
    f.width = 8001;
    assert!(job.render_frame(&f).is_err());
    assert_eq!(trace.borrow().frames.len(), 2);
}
