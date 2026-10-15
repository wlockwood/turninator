//! Canonical whole-layer measurements must satisfy every backend's durable pipeline.
#[path = "fixtures/renderers/common.rs"]
mod common;
use common::*;
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};
use turninator::{
    Result,
    render::{contract::*, geometry::realize_frame},
};

fn layered(kind: GeometryKind) -> RenderSceneSnapshot {
    let mut mesh = cube([0.; 3], 1.);
    append(&mut mesh, cube([0., 0., 2.], 1.));
    let mut s = scene(mesh);
    s.legacy_scene["geometry_kind"] = serde_json::to_value(kind).unwrap();
    s.legacy_scene["layer_face_ranges"] = serde_json::json!([
        {"start_triangle":0,"end_triangle":12,"layer_index":2,"z_top_mm":1.},
        {"start_triangle":12,"end_triangle":24,"layer_index":70000,"z_top_mm":3.}]);
    snapshot_scene(s.mesh, s.legacy_scene).unwrap()
}

fn negative_layered(kind: GeometryKind) -> RenderSceneSnapshot {
    let mut s = layered(kind);
    let mut mesh = (*s.mesh).clone();
    for p in &mut mesh.positions {
        p[2] -= 4.;
    }
    s.legacy_scene["layer_face_ranges"][0]["z_top_mm"] = serde_json::json!(-3.);
    s.legacy_scene["layer_face_ranges"][1]["z_top_mm"] = serde_json::json!(-1.);
    snapshot_scene(Arc::new(mesh), s.legacy_scene).unwrap()
}

#[test]
fn deposited_negative_absolute_tops_are_retained_before_section() {
    for kind in [GeometryKind::Beads, GeometryKind::Stepped] {
        let s = negative_layered(kind);
        for count in [0, 1, 2] {
            for section in [false, true] {
                let mut f = frame(&s);
                f.visible_layers = Some(count);
                f.section = section.then_some(turninator::export::SectionSpec {
                    axis: 0,
                    position_mm: 0.5,
                    keep_positive: false,
                    cap: true,
                });
                let r = realize_frame(&s, &f).unwrap();
                let b = r.build_analysis.unwrap().legacy;
                assert_eq!(b["z_top_mm"], [0., -3., -1.][count as usize]);
                near(b["remaining_volume_mm3"].as_f64().unwrap(), count as f64);
            }
        }
    }
}

#[test]
fn selected_zero_face_stepped_layer_has_known_zero_volume_without_open_warning() {
    let mut s = scene(cube([0.; 3], 1.));
    s.legacy_scene["geometry_kind"] = serde_json::json!("stepped");
    s.legacy_scene["layer_face_ranges"] = serde_json::json!([
        {"start_triangle":0,"end_triangle":0,"layer_index":7,"z_top_mm":-0.2},
        {"start_triangle":0,"end_triangle":12,"layer_index":11,"z_top_mm":1.}]);
    let s = snapshot_scene(s.mesh, s.legacy_scene).unwrap();
    let mut f = frame(&s);
    f.visible_layers = Some(1);
    let r = realize_frame(&s, &f).unwrap();
    assert!(r.mesh.triangles.is_empty());
    let b = r.build_analysis.unwrap().legacy;
    assert_eq!(b["visible_layer"], 7);
    assert_eq!(b["z_top_mm"], -0.2);
    assert_eq!(b["output_triangles"], 0);
    assert_eq!(b["closed_input"], false);
    assert_eq!(b["capped"], false);
    assert_eq!(b["remaining_volume_mm3"], 0.);
    assert!(b["warnings"].as_array().unwrap().is_empty());
}

#[test]
fn deposited_empty_partial_full_build_measurements_precede_section_without_geometry_changes() {
    for kind in [GeometryKind::Beads, GeometryKind::Stepped] {
        let s = layered(kind);
        for count in [0, 1, 2] {
            for section in [false, true] {
                let mut f = frame(&s);
                f.visible_layers = Some(count);
                if section {
                    f.section = Some(turninator::export::SectionSpec {
                        axis: 0,
                        position_mm: 0.5,
                        keep_positive: false,
                        cap: true,
                    });
                }
                let r = realize_frame(&s, &f).unwrap();
                let b = &r
                    .build_analysis
                    .as_ref()
                    .expect("whole-layer geometry requires canonical BuildAnalysis")
                    .legacy;
                assert_eq!(b["visible_layer"], [-1, 2, 70000][count as usize]);
                assert_eq!(b["z_top_mm"], [0., 1., 3.][count as usize]);
                assert_eq!(b["output_triangles"], count * 12);
                assert_eq!(b["capped"], false);
                assert_eq!(b["closed_input"], count > 0);
                near(b["remaining_volume_mm3"].as_f64().unwrap(), count as f64);
                assert!(
                    b.get("axis").is_none(),
                    "whole-layer reveal never invents a clipping plane"
                );
                if kind == GeometryKind::Beads && count > 0 {
                    assert!(
                        b["warnings"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|v| v.as_str().unwrap().contains("overlap"))
                    );
                }
                if section {
                    near(
                        r.analysis.as_ref().unwrap().legacy["remaining_volume_mm3"]
                            .as_f64()
                            .unwrap(),
                        count as f64 * 0.5,
                    );
                }
                if count == 2 {
                    let mut full = f.clone();
                    full.visible_layers = None;
                    let all = realize_frame(&s, &full).unwrap();
                    assert_eq!(r.mesh.positions, all.mesh.positions);
                    assert_eq!(r.mesh.triangles, all.mesh.triangles);
                    assert_eq!(r.normals, all.normals);
                    assert_eq!(r.triangle_materials, all.triangle_materials);
                }
            }
        }
    }
}

#[test]
fn open_or_inconsistently_wound_deposited_geometry_does_not_claim_volume() {
    for kind in [GeometryKind::Beads, GeometryKind::Stepped] {
        let mut s = layered(kind);
        let mut mesh = (*s.mesh).clone();
        mesh.triangles[0].swap(0, 1);
        s.mesh = Arc::new(mesh);
        let mut f = frame(&s);
        f.visible_layers = Some(1);
        let r = realize_frame(&s, &f).unwrap();
        let b = r.build_analysis.expect("canonical build metadata").legacy;
        assert_eq!(b["closed_input"], false);
        assert!(b["remaining_volume_mm3"].is_null());
        assert!(
            b["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v.as_str().unwrap().contains("volume unavailable"))
        );
        let complete = realize_frame(&s, &frame(&s)).unwrap();
        assert_eq!(&r.mesh.triangles[..], &complete.mesh.triangles[..12]);
        assert_eq!(
            r.mesh.positions,
            &complete.mesh.positions[..r.mesh.positions.len()]
        );
    }
}

#[test]
fn build_metadata_layer_id_bound_matches_deposited_schedule_ids() {
    let record = serde_json::json!({"visible_layer":1_000_000,"z_top_mm":1.,"output_triangles":12,"capped":false,"closed_input":true,"remaining_volume_mm3":1.});
    assert!(BuildAnalysis::from_legacy(record.clone()).is_ok());
    for kind in [GeometryKind::Beads, GeometryKind::Stepped] {
        let mut s = layered(kind);
        s.legacy_scene["layer_face_ranges"][1]["layer_index"] = serde_json::json!(1_000_000);
        let s = snapshot_scene(s.mesh, s.legacy_scene).unwrap();
        let mut f = frame(&s);
        f.visible_layers = Some(2);
        assert_eq!(
            realize_frame(&s, &f)
                .unwrap()
                .build_analysis
                .unwrap()
                .legacy["visible_layer"],
            1_000_000
        );
    }
    for id in [
        serde_json::json!(-2),
        serde_json::json!(1_000_001),
        serde_json::json!(0.5),
        serde_json::json!(true),
    ] {
        let mut bad = record.clone();
        bad["visible_layer"] = serde_json::json!(id);
        assert!(BuildAnalysis::from_legacy(bad).is_err());
    }
}

#[test]
fn measuring_mixed_global_component_winding_preserves_faces_normals_and_additive_volume() {
    for kind in [GeometryKind::Beads, GeometryKind::Stepped] {
        let mut s = layered(kind);
        let mut mesh = (*s.mesh).clone();
        for t in &mut mesh.triangles[12..] {
            t.swap(0, 1);
        }
        s.mesh = Arc::new(mesh);
        let baseline = realize_frame(&s, &frame(&s)).unwrap();
        let mut f = frame(&s);
        f.visible_layers = Some(2);
        let measured = realize_frame(&s, &f).unwrap();
        assert_eq!(measured.mesh.positions, baseline.mesh.positions);
        assert_eq!(measured.mesh.triangles, baseline.mesh.triangles);
        assert_eq!(measured.normals, baseline.normals);
        let b = measured.build_analysis.unwrap().legacy;
        assert_eq!(b["closed_input"], true);
        near(b["remaining_volume_mm3"].as_f64().unwrap(), 2.);
    }
}

struct CanonicalBackend {
    identity: BackendIdentity,
}
impl RenderBackend for CanonicalBackend {
    fn id(&self) -> turninator::render::BackendId {
        self.identity.backend
    }
    fn identity(&self) -> BackendIdentity {
        self.identity.clone()
    }
    fn assess(
        &self,
        _: &RenderSceneSnapshot,
        _: &turninator::export::ExportRequest,
    ) -> Vec<turninator::render::capabilities::FeatureSupport> {
        vec![]
    }
    fn render_sequence(
        &mut self,
        s: &RenderSceneSnapshot,
        p: &FramePlan,
        c: &RenderContext<'_>,
        emit: &mut dyn FnMut(FrameArtifact) -> Result<()>,
    ) -> Result<RenderReport> {
        let mut sections = BTreeMap::new();
        let mut builds = BTreeMap::new();
        for f in &p.frames {
            let r = realize_frame(s, f)?;
            if let Some(b) = &r.build_analysis {
                builds.insert(f.key, b.clone());
            }
            if let Some(a) = &r.analysis {
                sections.insert(f.key, a.clone());
            }
            emit(FrameArtifact::Metadata {
                key: f.key,
                section: r.analysis,
                build: r.build_analysis,
            })?;
            emit(FrameArtifact::Image {
                key: f.key,
                image: RgbaFrame {
                    width: f.width,
                    height: f.height,
                    bytes: vec![255; (f.width * f.height * 4) as usize],
                },
            })?;
        }
        Ok(RenderReport {
            frame_count: p.frames.len() as u32,
            warnings: vec![],
            identity: self.identity(),
            quality: c.selection.quality.clone(),
            analyses: sections,
            build_analyses: builds,
            phase_timings: BTreeMap::new(),
        })
    }
}

#[test]
fn layered_measurements_survive_prepared_wire_and_durable_pipeline_journal() {
    use turninator::{
        export::{
            BuildStage, ExportKind, ExportRequest, identity::pixel_recipe_key,
            pipeline::run_export_with_backend,
        },
        render::{BackendId, RenderSelection, ToolPaths, prepared::write_geometry},
    };
    let scratch = turninator::export::encode::Scratch::new(&std::env::temp_dir()).unwrap();
    let cache = turninator::cache::CacheStore::new(scratch.0.join("cache")).unwrap();
    let identity = BackendIdentity {
        backend: BackendId::Wgpu,
        adapter_recipe_version: 1,
        runtime_digest: "fixture".into(),
        bridge_digest: None,
        shader_digest: None,
        material_digest: None,
        color_digest: "fixture".into(),
        denoiser_digest: None,
        device: "fixture".into(),
        api: "fixture".into(),
        driver: "fixture".into(),
    };
    let mut backend = CanonicalBackend {
        identity: identity.clone(),
    };
    let tools = ToolPaths {
        blender: "missing".into(),
        ffmpeg: "missing".into(),
        ffprobe: "missing".into(),
    };
    let limits = RenderLimits::default();
    let cancel = AtomicBool::new(false);
    for kind in [GeometryKind::Beads, GeometryKind::Stepped] {
        for count in [0, 1, 2] {
            for section in [false, true] {
                let job = cache
                    .create_job(&format!("layered-{kind:?}-{count}-{section}"))
                    .unwrap();
                // Both signs are accepted absolute BUILD-Z; the independent
                // geometry cases above also cover all positive-top stages.
                let mut s = negative_layered(kind);
                s.mesh.write_binary(&job.join("mesh.bin")).unwrap();
                s.legacy_scene["mesh_path"] = serde_json::json!(job.join("mesh.bin"));
                std::fs::write(
                    job.join("scene.json"),
                    serde_json::to_vec(&s.legacy_scene).unwrap(),
                )
                .unwrap();
                let mut selection = RenderSelection::for_backend(BackendId::Wgpu, true);
                selection.device = Some(identity.device.clone());
                let request = ExportRequest {
                    kind: ExportKind::Still,
                    size: 16,
                    job_dir: job.clone(),
                    scene_path: job.join("scene.json"),
                    destination: scratch.0.join("out.png"),
                    selection: selection.clone(),
                    build_stage: Some(BuildStage {
                        visible_layers: count,
                    }),
                    section: section.then_some(turninator::export::SectionSpec {
                        axis: 0,
                        position_mm: 0.5,
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
                let ctx = RenderContext {
                    cancel: &cancel,
                    limits: &limits,
                    scratch_dir: &job,
                    selection: &selection,
                    identity: &identity,
                    pixel_recipe_key: &key,
                    job_id: 4,
                    revision: 1,
                    progress: &|_| {},
                };
                run_export_with_backend(&mut backend, &tools, &request, &ctx, &mut |_| {}).unwrap();
                let manifest = std::fs::read_dir(&job)
                    .unwrap()
                    .map(|e| e.unwrap().path().join("job.json"))
                    .find(|p| p.is_file())
                    .unwrap();
                let saved: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(manifest).unwrap()).unwrap();
                assert_eq!(
                    saved["frame_metadata"]["build_analyses"],
                    saved["report"]["build_analyses"]
                );
                assert_eq!(
                    saved["frame_metadata"]["analyses"],
                    saved["report"]["analyses"]
                );
                let mut f = frame(&s);
                f.visible_layers = Some(count);
                f.section = request.section.clone();
                let r = realize_frame(&s, &f).unwrap();
                assert_eq!(
                    saved["report"]["build_analyses"][0]["legacy"],
                    r.build_analysis.as_ref().unwrap().legacy
                );
                write_geometry(&job.join("prepared.bin"), &r, 2, &limits).unwrap();
                assert_eq!(
                    &std::fs::read(job.join("prepared.bin")).unwrap()[..8],
                    b"TNPRP001"
                );
            }
        }
    }
}
