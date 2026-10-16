//! Owner-thread native backend; no renderer subprocess, SDK bridge, or UI dependency.
use crate::{
    Result,
    export::ExportRequest,
    render::{
        BackendId, BackendQuality,
        capabilities::FeatureSupport,
        color::AgxRecipe,
        contract::*,
        geometry::{expand_design_edges, realize_frame},
        materials::resolve_materials,
        prepared::frame_geometry_key,
        selection::{AaQuality, ShadowQuality},
        wgpu_studio::{self, OUTPUT_SHADER, STUDIO_RECIPE, STUDIO_SHADER, WgpuStudio},
    },
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct WgpuConfig {
    pub device: Option<String>,
    pub quality: BackendQuality,
    pub limits: RenderLimits,
}
impl Default for WgpuConfig {
    fn default() -> Self {
        Self {
            device: None,
            quality: BackendQuality::Wgpu {
                aa: AaQuality::Medium,
                shadows: ShadowQuality::Medium,
            },
            limits: RenderLimits::default(),
        }
    }
}
pub struct WgpuBackend {
    studio: WgpuStudio,
    quality: BackendQuality,
    identity: BackendIdentity,
}
fn digest(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        let p = part.replace("\r\n", "\n");
        hash.update((p.len() as u64).to_le_bytes());
        hash.update(p);
    }
    format!("{:x}", hash.finalize())
}
impl WgpuBackend {
    pub fn new(config: &WgpuConfig) -> Result<Self> {
        let studio = WgpuStudio::new(config.device.as_deref(), &config.quality, &config.limits)?;
        let info = studio.adapter_info();
        let settings = serde_json::to_string(&studio.quality())?;
        let identity = BackendIdentity {
            backend: BackendId::Wgpu,
            adapter_recipe_version: 1,
            runtime_digest: digest(&[
                "wgpu-30.0.1",
                include_str!("../../../Cargo.lock"),
                include_str!("wgpu.rs"),
                include_str!("../wgpu_studio.rs"),
            ]),
            bridge_digest: None,
            shader_digest: Some(digest(&[
                STUDIO_RECIPE,
                STUDIO_SHADER,
                OUTPUT_SHADER,
                &settings,
            ])),
            material_digest: Some(digest(&[include_str!("../materials.rs")])),
            color_digest: AgxRecipe::default().digest(),
            denoiser_digest: None,
            device: wgpu_studio::adapter_name(info),
            api: format!("wgpu 30.0.1 / {:?}", info.backend),
            driver: format!("{} / {}", info.driver, info.driver_info),
        };
        Ok(Self {
            studio,
            quality: config.quality.clone(),
            identity,
        })
    }
    pub fn quality(&self) -> &BackendQuality {
        &self.quality
    }
    pub fn studio_mut(&mut self) -> &mut WgpuStudio {
        &mut self.studio
    }
    pub fn upload_count(&self) -> u64 {
        self.studio.upload_count()
    }
}
impl RenderBackend for WgpuBackend {
    fn id(&self) -> BackendId {
        BackendId::Wgpu
    }
    fn identity(&self) -> BackendIdentity {
        self.identity.clone()
    }
    fn assess(&self, scene: &RenderSceneSnapshot, _: &ExportRequest) -> Vec<FeatureSupport> {
        if scene.appearance.material_preset == "resin" {
            vec![FeatureSupport::Approximate{reason:"Native raster resin approximation: nearest visible surface and representative thickness; no refractive rays, multiple interfaces, or volume path tracing.".into()}]
        } else {
            vec![FeatureSupport::Supported]
        }
    }
    fn configure(&mut self, request: &ExportRequest) -> Result<()> {
        request.selection.validate()?;
        if request.selection.backend != BackendId::Wgpu
            || request.selection.quality != self.quality
            || request
                .selection
                .device
                .as_ref()
                .is_some_and(|d| d != &self.identity.device)
        {
            return Err("request differs from prepared wgpu device/quality".into());
        }
        Ok(())
    }
    fn render_sequence(
        &mut self,
        scene: &RenderSceneSnapshot,
        frames: &FramePlan,
        context: &RenderContext<'_>,
        emit: &mut dyn FnMut(FrameArtifact) -> Result<()>,
    ) -> Result<RenderReport> {
        if context.identity != &self.identity
            || context.selection.backend != BackendId::Wgpu
            || context.selection.quality != self.quality
            || context.selection.device.as_deref() != Some(&self.identity.device)
        {
            return Err("wgpu execution identity/quality mismatch".into());
        }
        if frames.frames.len() > context.limits.max_frames as usize * 4 {
            return Err("wgpu frame count exceeds limits".into());
        }
        let mut cache = BTreeMap::new();
        let mut resident = 0u64;
        let mut analyses = BTreeMap::new();
        let mut build_analyses = BTreeMap::new();
        let mut count = 0;
        let mut preparation = Duration::ZERO;
        let mut render = Duration::ZERO;
        let mut readback = Duration::ZERO;
        for f in frames
            .frames
            .iter()
            .filter(|f| !frames.repeat_of.contains_key(&f.key))
        {
            if context.cancel.load(Ordering::Acquire) {
                return Err("cancelled".into());
            }
            let start = Instant::now();
            let key = frame_geometry_key(f, true)?;
            if !cache.contains_key(&key) {
                // One immutable realization at a time bounds arbitrarily long
                // build sequences. Transform-only frames retain this upload.
                cache.clear();
                resident = 0;
                let mut geometry = realize_frame(scene, f)?;
                let mut materials = resolve_materials(scene)?;
                expand_design_edges(scene, f, &mut geometry, &mut materials, context.limits)?;
                let bytes = geometry.mesh.triangles.len() as u64 * 120
                    + materials.materials.len() as u64 * 80;
                let new_resident = resident
                    .checked_add(bytes)
                    .ok_or("wgpu geometry cache overflow")?;
                self.studio
                    .prepare_frame(f.width, f.height, new_resident, context.limits)?;
                let upload = self.studio.upload(&geometry, &materials, context.limits)?;
                resident = new_resident;
                cache.insert(
                    key.clone(),
                    (upload, geometry.analysis, geometry.build_analysis),
                );
            }
            preparation += start.elapsed();
            let (geometry, section, build) = &cache[&key];
            if f.section.is_some() || f.visible_layers.is_some() {
                emit(FrameArtifact::Metadata {
                    key: f.key,
                    section: section.clone(),
                    build: build.clone(),
                })?;
            }
            if let Some(a) = section {
                analyses.insert(f.key, a.clone());
            }
            if let Some(a) = build {
                build_analyses.insert(f.key, a.clone());
            }
            let (image, rt, bt) = self.studio.render(
                scene,
                f,
                geometry,
                frames.transparent,
                resident,
                context.limits,
                context.cancel,
            )?;
            render += rt;
            readback += bt;
            if context.cancel.load(Ordering::Acquire) {
                return Err("cancelled".into());
            }
            emit(FrameArtifact::Image { key: f.key, image })?;
            count += 1;
            (context.progress)(RenderProgress {
                key: Some(f.key),
                completed_work: u64::from(count),
                total_work: Some((frames.frames.len() - frames.repeat_of.len()) as u64),
                stage: "Native studio frame complete".into(),
                preview: None,
            });
        }
        let mut warnings = scene.warnings.clone();
        if scene.appearance.material_preset == "resin" {
            warnings.push("Native raster resin approximation: nearest surface with representative thickness; refraction, multiple interfaces and volume scattering are not path traced.".into());
        }
        Ok(RenderReport {
            frame_count: count,
            warnings,
            identity: self.identity.clone(),
            quality: self.quality.clone(),
            analyses,
            build_analyses,
            phase_timings: BTreeMap::from([
                ("prepare_upload".into(), preparation),
                ("render".into(), render),
                ("readback".into(), readback),
            ]),
        })
    }
}
