//! Thin Filament SDK adapter. All native rendering policy lives in the pinned bridge.
use crate::{
    Result,
    export::ExportRequest,
    render::{
        capabilities::*,
        contract::*,
        embedded::{
            backend::{self, EmbeddedStudio},
            *,
        },
        selection::*,
    },
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

pub const SDK_VERSION: &str = "1.78.0+c1dc4e444784850d6dc891a1fb38747c30b85463";
const SDK_ARCHIVE: &str = "38643cfeec54ce58c4d5d57f94d935a247ad9c1f76968e24d5367d22b396f40f";
const NATIVE_RECIPE: &str = "turninator-filament-native-v1";
pub const RESIN_APPROXIMATION: &str = "Filament raster resin uses screen-space refraction and approximate thickness; offscreen refraction, overlapping interfaces and directional/multiple volume scattering are unavailable.";

/// Task10 writes this receipt alongside the bridge after verifying its build inputs.
/// Embedded shader/material/environment bytes are bound by the whole bridge digest.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageReceipt {
    schema_version: u32,
    sdk_version: String,
    sdk_archive_sha256: String,
    native_recipe: String,
    bridge_sha256: String,
}
impl PackageReceipt {
    fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 16384 {
            return Err("Filament package receipt exceeds 16 KiB".into());
        }
        let receipt: Self = serde_json::from_slice(bytes)?;
        if receipt.schema_version != 1
            || receipt.sdk_version != SDK_VERSION
            || receipt.sdk_archive_sha256 != SDK_ARCHIVE
            || receipt.native_recipe != NATIVE_RECIPE
            || receipt.bridge_sha256.len() != 64
            || !receipt
                .bridge_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(
                "Filament package uses an invalid hash, unqualified SDK or native recipe".into(),
            );
        }
        Ok(receipt)
    }
    fn verify_bridge(&self, actual: &str) -> Result<()> {
        if actual != self.bridge_sha256 {
            return Err("Filament bridge hash differs from its package receipt".into());
        }
        Ok(())
    }
}
fn digest(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}
fn file_digest(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 65536];
    loop {
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        hasher.update(&chunk[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
fn package() -> Result<(PathBuf, String)> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let root = exe
        .parent()
        .ok_or("missing executable directory")?
        .join("renderers/filament");
    let canonical = root.canonicalize().map_err(|e| {
        format!(
            "Filament is unavailable: install the pinned renderer package in {}: {e}",
            root.display()
        )
    })?;
    if canonical != root {
        return Err("Filament package directory must not be redirected".into());
    }
    let receipt_path = root.join("filament-package.json");
    if receipt_path.canonicalize()? != receipt_path {
        return Err("invalid Filament package receipt path".into());
    }
    let mut bytes = Vec::new();
    File::open(receipt_path)?
        .take(16385)
        .read_to_end(&mut bytes)?;
    let receipt = PackageReceipt::parse(&bytes)?;
    let dll = root.join("tn_renderer_filament.dll");
    if dll.canonicalize()? != dll {
        return Err("Filament bridge must not be redirected".into());
    }
    let actual = file_digest(&dll)?;
    receipt.verify_bridge(&actual)?;
    Ok((dll, actual))
}

pub struct FilamentBackend {
    studio: EmbeddedStudio,
    configured_transparent: Option<bool>,
}
impl FilamentBackend {
    /// Call on an export/interactive owner thread, never the UI thread.
    pub fn new(config: &EmbeddedConfig) -> Result<Self> {
        config.selection.validate()?;
        if config.selection.backend != BackendId::Filament {
            return Err("Filament requires a Filament selection".into());
        }
        let (dll, bridge_digest) = package().map_err(|e|format!("Filament package unavailable: {e}. Reinstall the verified renderers/filament package beside the application executable."))?;
        let library = EmbeddedLibrary::load(&dll, BackendId::Filament)?;
        let mut driver = library.create(config)?;
        let runtime = driver.runtime_info()?;
        if runtime.backend != "filament"
            || runtime.sdk_version != SDK_VERSION
            || !runtime.api.starts_with("Vulkan ")
            || !runtime.device_id.starts_with("vulkan:")
            || runtime.compiler.is_some()
        {
            return Err("Filament runtime does not match pinned Vulkan SDK package".into());
        }
        // Recheck after load/create: identity comes from the actual package bytes,
        // never the caller's provisional config.identity or mutable diagnostics.
        if file_digest(&dll)? != bridge_digest {
            return Err("Filament package changed during initialization".into());
        }
        let recipe = serde_json::to_vec(
            &serde_json::json!({"native_recipe":NATIVE_RECIPE,"bridge":bridge_digest,"quality":config.selection.quality}),
        )?;
        let identity = BackendIdentity {
            backend: BackendId::Filament,
            adapter_recipe_version: 1,
            runtime_digest: digest(format!("{}:{bridge_digest}", runtime.sdk_version)),
            bridge_digest: Some(bridge_digest.clone()),
            shader_digest: Some(digest(
                [b"filament-shaders-v1:".as_slice(), &recipe].concat(),
            )),
            material_digest: Some(digest(format!(
                "filament-embedded-materials-environment-v1:{bridge_digest}"
            ))),
            color_digest: digest(format!("filament-agx-v1:{bridge_digest}")),
            denoiser_digest: None,
            device: runtime.device_id,
            api: runtime.api,
            driver: runtime.driver,
        };
        Ok(Self {
            studio: EmbeddedStudio::from_driver(
                driver,
                config.selection.clone(),
                identity,
                config.limits.clone(),
            )?,
            configured_transparent: None,
        })
    }
    /// Independently construct a second backend for interactive ownership, then
    /// borrow this surface to render a resolved frame without export side effects.
    pub fn studio(&mut self) -> &mut EmbeddedStudio {
        &mut self.studio
    }
    pub fn diagnostics(&self) -> Option<&RuntimeDiagnostics> {
        self.studio.diagnostics()
    }
    /// Discovery is descriptive only; constructing this never initializes a GPU.
    pub fn status() -> BackendStatus {
        BackendStatus {
            backend: BackendId::Filament,
            availability: BackendAvailability::NotProbed,
            capabilities: Some(CapabilityDescriptor {
                usages: vec![
                    RenderUsage::Interactive,
                    RenderUsage::Preview,
                    RenderUsage::Animation,
                    RenderUsage::Still,
                    RenderUsage::Sheet,
                ],
                alpha: FeatureSupport::Supported,
                material_volume: FeatureSupport::Approximate {
                    reason: RESIN_APPROXIMATION.into(),
                },
                sections: FeatureSupport::Supported,
                layer_appearance: FeatureSupport::Supported,
                maximum_output_side: 8000,
            }),
        }
    }
}

impl RenderBackend for FilamentBackend {
    fn id(&self) -> BackendId {
        BackendId::Filament
    }
    fn assess(&self, scene: &RenderSceneSnapshot, _request: &ExportRequest) -> Vec<FeatureSupport> {
        if scene.appearance.material_preset == "resin" {
            vec![FeatureSupport::Approximate {
                reason: RESIN_APPROXIMATION.into(),
            }]
        } else {
            vec![FeatureSupport::Supported]
        }
    }
    fn identity(&self) -> BackendIdentity {
        self.studio.identity()
    }
    fn configure(&mut self, request: &ExportRequest) -> Result<()> {
        self.studio.validate_selection(&request.selection)?;
        self.configured_transparent = Some(request.transparent);
        Ok(())
    }
    fn render_sequence(
        &mut self,
        scene: &RenderSceneSnapshot,
        frames: &FramePlan,
        context: &RenderContext<'_>,
        emit: &mut dyn FnMut(FrameArtifact) -> Result<()>,
    ) -> Result<RenderReport> {
        // The resolved plan owns panel alpha (sheet panels can be transparent
        // even when the composed final document is opaque).
        if self.configured_transparent.is_none() {
            return Err("configure Filament before rendering an export sequence".into());
        }
        let mut report = backend::render_sequence(&mut self.studio, scene, frames, context, emit)?;
        if scene.appearance.material_preset == "resin" {
            report.warnings.push(RESIN_APPROXIMATION.into());
        }
        Ok(report)
    }
}

#[cfg(test)]
mod package_tests {
    use super::*;
    fn receipt() -> serde_json::Value {
        serde_json::json!({"schema_version":1,"sdk_version":SDK_VERSION,"sdk_archive_sha256":SDK_ARCHIVE,"native_recipe":NATIVE_RECIPE,"bridge_sha256":"a".repeat(64)})
    }
    #[test]
    fn package_receipt_requires_pinned_inputs_and_actual_bridge_bytes() {
        let valid = PackageReceipt::parse(&serde_json::to_vec(&receipt()).unwrap()).unwrap();
        assert!(valid.verify_bridge(&"a".repeat(64)).is_ok());
        assert!(valid.verify_bridge(&"b".repeat(64)).is_err());
        for key in [
            "sdk_version",
            "sdk_archive_sha256",
            "native_recipe",
            "bridge_sha256",
        ] {
            let mut changed = receipt();
            changed[key] = "invalid".into();
            assert!(
                PackageReceipt::parse(&serde_json::to_vec(&changed).unwrap()).is_err(),
                "{key}"
            );
        }
        let mut rebuilt = receipt();
        rebuilt["bridge_sha256"] = "b".repeat(64).into();
        assert!(
            PackageReceipt::parse(&serde_json::to_vec(&rebuilt).unwrap())
                .unwrap()
                .verify_bridge(&"b".repeat(64))
                .is_ok()
        );
    }
    #[test]
    fn package_receipt_rejects_missing_unknown_and_oversized_documents() {
        assert!(PackageReceipt::parse(b"{}").is_err());
        let mut unknown = receipt();
        unknown["fallback"] = true.into();
        assert!(PackageReceipt::parse(&serde_json::to_vec(&unknown).unwrap()).is_err());
        let mut large = serde_json::to_vec(&receipt()).unwrap();
        large.resize(16385, b' ');
        assert!(PackageReceipt::parse(&large).is_err());
    }
}
