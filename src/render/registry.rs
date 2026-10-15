//! Discovery is descriptive and lazy. Runtime work happens only in preparation
//! on the caller's owner actor, before it is allowed to reuse or render pixels.
use super::{
    BackendId, RenderSelection, ToolPaths, WorkerClient,
    backends::blender::{BlenderBackend, validate_quality},
    capabilities::*,
    contract::*,
    requirements::{requirements_for, validate_requirements},
    selection::RenderUsage,
    worker::{WorkerTransport, renderer_script},
};
use crate::{Result, export::ExportRequest};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub struct PreparedBackend {
    pub backend: Box<dyn RenderBackend>,
    pub effective_selection: RenderSelection,
    pub identity: BackendIdentity,
    pub assessment: Vec<FeatureSupport>,
    pub preparation_timings: BTreeMap<String, Duration>,
}

#[derive(Default)]
pub struct BackendRegistry {
    luxcore: RefCell<super::backends::luxcore::LuxCoreRuntimeCache>,
    statuses: RefCell<BTreeMap<BackendId, BackendStatus>>,
}

fn descriptor() -> CapabilityDescriptor {
    CapabilityDescriptor {
        usages: vec![
            RenderUsage::Preview,
            RenderUsage::Animation,
            RenderUsage::Still,
            RenderUsage::Sheet,
        ],
        alpha: FeatureSupport::Supported,
        material_volume: FeatureSupport::Supported,
        sections: FeatureSupport::Supported,
        layer_appearance: FeatureSupport::Supported,
        maximum_output_side: 8000,
    }
}
fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn nonempty<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("Blender probe missing concrete {key}").into())
}

impl BackendRegistry {
    pub fn status(&self, backend: BackendId) -> BackendStatus {
        if matches!(backend, BackendId::Filament | BackendId::LuxCore)
            && !self.statuses.borrow().contains_key(&backend)
        {
            let mut status = match backend {
                BackendId::LuxCore => super::backends::luxcore::LuxCoreBackend::status(),
                _ => super::backends::filament::FilamentBackend::status(),
            };
            if let Err(error) = native_package_files(backend) {
                status.availability = BackendAvailability::Unavailable {
                    reason: error.to_string(),
                };
            }
            return status;
        }
        self.statuses
            .borrow()
            .get(&backend)
            .cloned()
            .unwrap_or_else(|| BackendStatus {
                backend,
                availability: match backend {
                    BackendId::Wgpu | BackendId::BlenderEevee | BackendId::BlenderCycles => {
                        BackendAvailability::NotProbed
                    }
                    _ => BackendAvailability::Unavailable {
                        reason: format!(
                            "{} export renderer is not integrated yet",
                            backend.as_str()
                        ),
                    },
                },
                capabilities: None,
            })
    }

    pub fn prepare(
        &self,
        selection: &RenderSelection,
        scene: &RenderSceneSnapshot,
        request: &ExportRequest,
        tools: &ToolPaths,
        limits: &RenderLimits,
        cancel: &AtomicBool,
    ) -> Result<PreparedBackend> {
        if cancel.load(Ordering::Acquire) {
            return Err("cancelled".into());
        }
        if selection.backend == BackendId::Wgpu {
            selection.validate()?;
            validate_requirements(tools, &requirements_for(selection, request.kind, false))?;
            if request.selection != *selection {
                return Err("request selection differs from renderer preparation".into());
            }
            let started = Instant::now();
            let mut backend =
                super::backends::wgpu::WgpuBackend::new(&super::backends::wgpu::WgpuConfig {
                    device: selection.device.clone(),
                    quality: selection.quality.clone(),
                    limits: limits.clone(),
                })?;
            backend.configure(request)?;
            let identity = backend.identity();
            let assessment = backend.assess(scene, request);
            let mut effective_selection = selection.clone();
            effective_selection.device = Some(identity.device.clone());
            self.statuses.borrow_mut().insert(
                selection.backend,
                BackendStatus {
                    backend: selection.backend,
                    availability: BackendAvailability::Available,
                    capabilities: Some(descriptor()),
                },
            );
            return Ok(PreparedBackend {
                backend: Box::new(backend),
                effective_selection,
                identity,
                assessment,
                preparation_timings: BTreeMap::from([("total".into(), started.elapsed())]),
            });
        }
        if matches!(selection.backend, BackendId::Filament | BackendId::LuxCore) {
            selection.validate()?;
            validate_requirements(tools, &requirements_for(selection, request.kind, false))?;
            if request.selection != *selection {
                return Err("request selection differs from renderer preparation".into());
            }
            let started = Instant::now();
            let cache_directory = if selection.backend == BackendId::LuxCore {
                let parent = request
                    .job_dir
                    .parent()
                    .ok_or("job cache parent missing")?
                    .canonicalize()?;
                let directory = parent.join(".luxcore-kernels");
                match std::fs::create_dir(&directory) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error.into()),
                }
                if !directory.is_dir() || directory.canonicalize()? != directory {
                    return Err(
                        "LuxCore compiler cache must be an owned directory without redirection"
                            .into(),
                    );
                }
                Some(directory)
            } else {
                None
            };
            let config = super::embedded::EmbeddedConfig {
                selection: selection.clone(),
                limits: limits.clone(),
                identity: bootstrap_identity(selection.backend),
                cache_directory,
            };
            let mut backend: Box<dyn RenderBackend> = match selection.backend {
                BackendId::LuxCore => Box::new(self.luxcore.borrow_mut().prepare(&config)?),
                _ => Box::new(super::backends::filament::FilamentBackend::new(&config)?),
            };
            backend.configure(request)?;
            let identity = backend.identity();
            let assessment = backend.assess(scene, request);
            let mut effective_selection = selection.clone();
            effective_selection.device = Some(identity.device.clone());
            let mut status = match selection.backend {
                BackendId::LuxCore => super::backends::luxcore::LuxCoreBackend::status(),
                _ => super::backends::filament::FilamentBackend::status(),
            };
            status.availability = BackendAvailability::Available;
            self.statuses.borrow_mut().insert(selection.backend, status);
            return Ok(PreparedBackend {
                backend,
                effective_selection,
                identity,
                assessment,
                preparation_timings: BTreeMap::from([("total".into(), started.elapsed())]),
            });
        }
        self.validate(selection, scene, request, tools, limits)?;
        let started = Instant::now();
        let worker = Box::new(WorkerClient::spawn(tools)?);
        let launch = started.elapsed();
        let mut prepared =
            self.prepare_with_transport(selection, scene, request, tools, limits, cancel, worker)?;
        prepared
            .preparation_timings
            .insert("worker_launch".into(), launch);
        *prepared
            .preparation_timings
            .entry("total".into())
            .or_default() += launch;
        Ok(prepared)
    }

    /// Also permits protocol qualification without installing or launching Blender.
    /// Production callers use prepare(), retaining WorkerClient's Job Object ownership.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_with_transport(
        &self,
        selection: &RenderSelection,
        scene: &RenderSceneSnapshot,
        request: &ExportRequest,
        tools: &ToolPaths,
        limits: &RenderLimits,
        cancel: &AtomicBool,
        mut worker: Box<dyn WorkerTransport>,
    ) -> Result<PreparedBackend> {
        if cancel.load(Ordering::Acquire) {
            worker.cancel();
            return Err("cancelled".into());
        }
        self.validate(selection, scene, request, tools, limits)?;
        let started = Instant::now();
        let prepared: Result<PreparedBackend> = (|| {
            let probe=worker.request(json!({"request_id":"prepare-blender","scene_revision":0,"kind":"probe","initialize_gpu":true}),
                cancel,&mut |_| {});
            let probe = match probe {
                Ok(probe) => probe,
                Err(error) => {
                    worker.cancel();
                    return Err(error);
                }
            };
            if cancel.load(Ordering::Acquire) {
                worker.cancel();
                return Err("cancelled".into());
            }
            let probe = &probe["payload"];
            let engine = selection.backend.legacy_blender_engine()?;
            if !probe["engines"]
                .as_array()
                .is_some_and(|engines| engines.iter().any(|value| value == engine))
            {
                return Err(format!(
                    "Blender {engine} is unavailable; no renderer or CPU fallback"
                )
                .into());
            }
            let gpu = &probe["gpu"];
            let api = nonempty(gpu, "backend")?.to_owned();
            let driver = nonempty(gpu, "version")?.to_owned();
            let device = if selection.backend == BackendId::BlenderCycles {
                if probe["cycles_backend"] != "OPTIX" {
                    return Err("Blender Cycles requires OPTIX; no CPU fallback".into());
                }
                let mut devices = probe["devices"]
                    .as_array()
                    .ok_or("Blender probe missing devices")?
                    .iter()
                    .filter(|d| d["type"] == "OPTIX" && d["enabled"] == true)
                    .map(|d| nonempty(d, "id").map(str::to_owned))
                    .collect::<Result<Vec<_>>>()?;
                devices.sort();
                devices.dedup();
                if devices.is_empty() {
                    return Err("Blender probe has no enabled OPTIX device".into());
                }
                format!("OPTIX:{}", devices.join("|"))
            } else {
                format!(
                    "{}:{}:{}",
                    api,
                    nonempty(gpu, "vendor")?,
                    nonempty(gpu, "renderer")?
                )
            };
            if selection
                .device
                .as_ref()
                .is_some_and(|saved| saved != &device)
            {
                return Err(format!("saved Blender device is unavailable: {}; current configuration is {device}; saved selection was retained",selection.device.as_deref().unwrap_or("")).into());
            }
            let version = nonempty(probe, "blender_version")?;
            let build = nonempty(probe, "blender_build_hash")?;
            let mut effective_selection = selection.clone();
            effective_selection.device = Some(device.clone());
            let runtime = json!({"executable":hash_file(&tools.blender)?,"version":version,"build":build,
                "worker":hash_file(&renderer_script("worker.py"))?,"frame_pipeline":hash_file(&renderer_script("frame_pipeline.py"))?});
            let scene_digest = hash_file(&renderer_script("scene.py"))?;
            let runtime_digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&runtime)?));
            let identity = BackendIdentity {
                backend: selection.backend,
                adapter_recipe_version: crate::render::prepared::BLENDER_ADAPTER_VERSION,
                runtime_digest: runtime_digest.clone(),
                bridge_digest: None,
                shader_digest: None,
                material_digest: Some(scene_digest.clone()),
                color_digest: scene_digest,
                denoiser_digest: if selection.backend == BackendId::BlenderCycles {
                    Some(runtime_digest)
                } else {
                    None
                },
                device,
                api: if selection.backend == BackendId::BlenderCycles {
                    "OPTIX".into()
                } else {
                    api
                },
                driver,
            };
            let backend = BlenderBackend::with_transport(
                effective_selection.clone(),
                request.clone(),
                identity.clone(),
                worker,
            )?;
            let assessment = backend.assess(scene, request);
            if let Some(FeatureSupport::Unsupported { reason }) = assessment
                .iter()
                .find(|s| matches!(s, FeatureSupport::Unsupported { .. }))
            {
                return Err(reason.clone().into());
            }
            let mut preparation_timings = BTreeMap::from([("total".into(), started.elapsed())]);
            if let Some(seconds) = probe["initialization_seconds"]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.)
            {
                preparation_timings.insert(
                    "runtime_initialization".into(),
                    Duration::from_secs_f64(seconds),
                );
            }
            Ok(PreparedBackend {
                backend: Box::new(backend),
                effective_selection,
                identity,
                assessment,
                preparation_timings,
            })
        })();
        if prepared.is_ok() {
            self.statuses.borrow_mut().insert(
                selection.backend,
                BackendStatus {
                    backend: selection.backend,
                    availability: match &prepared {
                        Ok(_) => BackendAvailability::Available,
                        Err(error) => BackendAvailability::Unavailable {
                            reason: error.to_string(),
                        },
                    },
                    capabilities: if prepared.is_ok() {
                        Some(descriptor())
                    } else {
                        None
                    },
                },
            );
        }
        prepared
    }
    fn validate(
        &self,
        selection: &RenderSelection,
        scene: &RenderSceneSnapshot,
        request: &ExportRequest,
        tools: &ToolPaths,
        limits: &RenderLimits,
    ) -> Result<()> {
        selection.validate()?;
        validate_requirements(tools, &requirements_for(selection, request.kind, false))?;
        validate_quality(selection)?;
        if request.selection != *selection {
            return Err("request selection differs from renderer preparation".into());
        }
        if !(16..=limits.max_output_side.min(8000)).contains(&request.size)
            || request.frame_count == 0
            || request.frame_count > limits.max_frames.min(10000)
            || scene.mesh.positions.len() > limits.max_source_vertices
            || scene.mesh.triangles.len() > limits.max_source_triangles
            || serde_json::to_vec(&scene.legacy_scene)?.len() as u64 > limits.max_scene_bytes
        {
            return Err("Blender preparation exceeds scene/output limits".into());
        }
        crate::animation::validate_frame_rate(request.frame_rate)?;
        Ok(())
    }
}

fn bootstrap_identity(backend: BackendId) -> BackendIdentity {
    // Required native configuration fields are provisional only. The adapter
    // replaces every value with attested package/runtime data before returning.
    BackendIdentity {
        backend,
        adapter_recipe_version: 1,
        runtime_digest: "pending-runtime-attestation".into(),
        bridge_digest: None,
        shader_digest: None,
        material_digest: None,
        color_digest: "pending-runtime-attestation".into(),
        denoiser_digest: None,
        device: "pending-runtime-attestation".into(),
        api: "pending-runtime-attestation".into(),
        driver: "pending-runtime-attestation".into(),
    }
}

/// Package changes invalidate retained contexts. This describes files only; GPU
/// initialization and attestation remain inside the backend's owning actor.
pub(crate) fn native_runtime_fingerprint(backend: BackendId) -> Result<String> {
    let mut digest = Sha256::new();
    for file in native_package_files(backend)? {
        digest.update(file.to_string_lossy().as_bytes());
        digest.update(hash_file(&file)?.as_bytes());
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn native_package_files(backend: BackendId) -> Result<Vec<std::path::PathBuf>> {
    let name = match backend {
        BackendId::Filament => "filament",
        BackendId::LuxCore => "luxcore",
        _ => return Ok(Vec::new()),
    };
    let exe = std::env::current_exe()?.canonicalize()?;
    let root = exe
        .parent()
        .ok_or("missing executable directory")?
        .join("renderers")
        .join(name);
    let mut files = vec![
        root.join(format!("{name}-package.json")),
        root.join(format!("tn_renderer_{name}.dll")),
    ];
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("dll"))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files.dedup();
    for file in &files {
        if !file.is_file() {
            return Err(format!("{} package is missing {}. Install the verified renderer package beside the application, then retry.", backend.as_str(), file.display()).into());
        }
    }
    Ok(files)
}
