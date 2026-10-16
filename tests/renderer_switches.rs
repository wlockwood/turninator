//! Opt-in cross-engine lifetime/resource qualification, not a quality benchmark.
#[path = "fixtures/renderers/common.rs"]
mod common;
use common::*;
use serde_json::json;
use std::{path::PathBuf, sync::atomic::AtomicBool};
use turninator::{
    Result,
    export::{ExportKind, ExportRequest, pipeline::run_export_with_backend},
    render::{ToolPaths, contract::*, registry::BackendRegistry, selection::*},
};

#[cfg(windows)]
fn resources() -> (usize, u32) {
    use windows_sys::Win32::System::{
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
        Threading::{GetCurrentProcess, GetProcessHandleCount},
    };
    let mut memory: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
    memory.cb = std::mem::size_of_val(&memory) as u32;
    let mut handles = 0;
    // SAFETY: current-process pseudo handle and correctly sized output buffers.
    unsafe {
        assert_ne!(
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
                memory.cb
            ),
            0
        );
        assert_ne!(GetProcessHandleCount(GetCurrentProcess(), &mut handles), 0);
    }
    (memory.PrivateUsage, handles)
}

#[test]
#[cfg(windows)]
#[ignore = "exclusive GPU; full qualified runtime and Blender required; twenty complete engine cycles"]
fn twenty_cross_engine_cycles_release_job_contexts() -> Result<()> {
    let output = PathBuf::from(std::env::var("TN_SWITCH_OUTPUT")?);
    std::fs::create_dir_all(&output)?;
    let cache = turninator::cache::CacheStore::new(output.join("jobs"))?;
    let registry = BackendRegistry::default();
    let mesh = cube([-5., -5., 0.], 10.);
    let limits = RenderLimits {
        host_bytes: 8 << 30,
        gpu_bytes: 6 << 30,
        ..Default::default()
    };
    let tools = ToolPaths::default();
    let cancel = AtomicBool::new(false);
    let mut measurements = Vec::new();
    let mut warm = None;
    for cycle in 0..20 {
        for engine in [
            BackendId::Wgpu,
            BackendId::Filament,
            BackendId::LuxCore,
            BackendId::BlenderEevee,
            BackendId::BlenderCycles,
            BackendId::Wgpu,
        ] {
            let step = measurements.len();
            let job = cache.create_job(&format!("cycle-{cycle}-{step}"))?;
            mesh.write_binary(&job.join("mesh.bin"))?;
            let mut snapshot = scene(mesh.clone());
            snapshot.legacy_scene["mesh_path"] = json!(job.join("mesh.bin"));
            std::fs::write(
                job.join("scene.json"),
                serde_json::to_vec(&snapshot.legacy_scene)?,
            )?;
            let quality = match engine {
                BackendId::Wgpu => BackendQuality::Wgpu {
                    aa: AaQuality::Low,
                    shadows: ShadowQuality::Low,
                },
                BackendId::Filament => BackendQuality::Filament {
                    aa: AaQuality::Low,
                    shadows: ShadowQuality::Low,
                },
                BackendId::LuxCore => BackendQuality::LuxCore {
                    samples: 4,
                    denoise: false,
                },
                BackendId::BlenderEevee => BackendQuality::Eevee { samples: 4 },
                BackendId::BlenderCycles => BackendQuality::Cycles {
                    samples: 4,
                    denoise: false,
                },
            };
            let request = ExportRequest {
                job_dir: job.clone(),
                scene_path: job.join("scene.json"),
                destination: output.join(format!("{cycle}-{step}.png")),
                kind: ExportKind::Still,
                size: 64,
                selection: RenderSelection {
                    backend: engine,
                    device: None,
                    quality,
                },
                ..Default::default()
            };
            let mut prepared = registry.prepare(
                &request.selection,
                &snapshot,
                &request,
                &tools,
                &limits,
                &cancel,
            )?;
            let key = turninator::export::identity::pixel_recipe_key(
                &snapshot.legacy_scene,
                &mesh.snapshot_hash()?,
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
                job_id: step as u64,
                revision: 1,
                progress: &|_| {},
            };
            run_export_with_backend(
                prepared.backend.as_mut(),
                &tools,
                &request,
                &context,
                &mut |_| {},
            )?;
            let identity = prepared.identity.clone();
            drop(prepared);
            let (private, handles) = resources();
            measurements.push(json!({"cycle":cycle,"step":step,"engine":engine,"private_bytes_after_context_drop":private,"handles_after_context_drop":handles,"identity":identity}));
            std::fs::write(
                output.join("measurements.json"),
                serde_json::to_vec_pretty(&measurements)?,
            )?;
        }
        let sample = resources();
        if cycle == 2 {
            warm = Some(sample);
        }
        if cycle > 2 {
            let (private, handles) = warm.unwrap();
            assert!(
                sample.0 <= private + (128 << 20),
                "private memory exceeded warmed retained-cache baseline: {sample:?} vs {private}/{handles}"
            );
            assert!(
                sample.1 <= handles + 32,
                "handle growth after full cycle: {sample:?} vs {private}/{handles}"
            );
        }
    }
    drop(registry);
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(
            &json!({"cycles":20,"contexts_rendered_and_dropped":120,"warm_baseline":warm,"after_registry_drop":resources(),"note":"64px/Low/4 samples, denoise off; lifetime probe only. Native per-backend WDDM tests separately establish released GPU residency."}),
        )?,
    )?;
    Ok(())
}
