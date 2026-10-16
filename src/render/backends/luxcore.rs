//! Qualified in-process LuxCore adapter; native policy is bound by its package.
use crate::{
    Result,
    export::{ExportKind, ExportRequest},
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
    cell::RefCell,
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    rc::Rc,
    sync::OnceLock,
};

pub const RESIN_ALPHA_APPROXIMATION: &str = "LuxCore resin in transparent images or four-view sheets uses approximate RGBA compositing. Use an opaque studio still for consistent refraction.";
pub const LAYER_DENOISE_APPROXIMATION: &str = "LuxCore denoising can vary fine layer contrast between animation frames. Disable denoising for stable fine-layer detail.";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Contract {
    schema_version: u32,
    sdk_version: String,
    sdk_source_commit: String,
    sdk_source_archive_sha256: String,
    compiler_pair_sha256: String,
    native_recipe: String,
    runtime_files: BTreeMap<String, String>,
    runtime_artifacts: BTreeMap<String, String>,
    policies: BTreeMap<String, String>,
    driver_artifacts: BTreeMap<String, String>,
}
fn contract() -> &'static Contract {
    static VALUE: OnceLock<Contract> = OnceLock::new();
    VALUE.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../native/luxcore/package-contract.json"
        ))
        .expect("compiled LuxCore contract")
    })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageReceipt {
    schema_version: u32,
    sdk_version: String,
    sdk_source_commit: String,
    sdk_source_archive_sha256: String,
    compiler_pair_sha256: String,
    native_recipe: String,
    bridge_sha256: String,
    #[serde(deserialize_with = "unique_files")]
    runtime_files: BTreeMap<String, String>,
}
fn unique_files<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<BTreeMap<String, String>, D::Error> {
    struct Map;
    impl<'de> serde::de::Visitor<'de> for Map {
        type Value = BTreeMap<String, String>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a bounded map of unique runtime filenames")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut access: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut values = BTreeMap::new();
            while let Some((key, value)) = access.next_entry::<String, String>()? {
                if values.len() == 64 || values.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom(
                        "duplicate or oversized runtime map",
                    ));
                }
            }
            Ok(values)
        }
    }
    deserializer.deserialize_map(Map)
}
impl PackageReceipt {
    fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 16384 {
            return Err("LuxCore package receipt exceeds 16 KiB".into());
        }
        let receipt: Self = serde_json::from_slice(bytes)?;
        let c = contract();
        if receipt.schema_version != c.schema_version
            || receipt.sdk_version != c.sdk_version
            || receipt.sdk_source_commit != c.sdk_source_commit
            || receipt.sdk_source_archive_sha256 != c.sdk_source_archive_sha256
            || receipt.compiler_pair_sha256 != c.compiler_pair_sha256
            || receipt.native_recipe != c.native_recipe
            || !valid_hash(&receipt.bridge_sha256)
            || receipt.runtime_files != c.runtime_files
        {
            return Err(
                "LuxCore package has unqualified SDK, compiler, runtime files or native recipe"
                    .into(),
            );
        }
        Ok(receipt)
    }
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn digest(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}
fn file_digest(path: &Path) -> Result<String> {
    let mut input = File::open(path)?;
    let mut hash = Sha256::new();
    let mut block = [0u8; 65536];
    loop {
        let n = input.read(&mut block)?;
        if n == 0 {
            break;
        }
        hash.update(&block[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn owned_file(root: &Path, name: &str) -> Result<PathBuf> {
    let path = root.join(name);
    if path.canonicalize()? != path {
        return Err("LuxCore package files must not be redirected".into());
    }
    Ok(path)
}
struct Package {
    root: PathBuf,
    dll: PathBuf,
    receipt: PackageReceipt,
    fingerprint: String,
}
fn package() -> Result<Package> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let root = exe
        .parent()
        .ok_or("missing executable directory")?
        .join("renderers/luxcore");
    if root.canonicalize()? != root {
        return Err("LuxCore package directory must not be redirected".into());
    }
    let receipt_path = owned_file(&root, "luxcore-package.json")?;
    let mut bytes = Vec::new();
    File::open(receipt_path)?
        .take(16385)
        .read_to_end(&mut bytes)?;
    let receipt = PackageReceipt::parse(&bytes)?;
    let dll = owned_file(&root, "tn_renderer_luxcore.dll")?;
    if file_digest(&dll)? != receipt.bridge_sha256 {
        return Err("LuxCore bridge differs from its package receipt".into());
    }
    for (name, expected) in &receipt.runtime_files {
        if file_digest(&owned_file(&root, name)?)? != *expected {
            return Err(format!("LuxCore runtime hash mismatch: {name}").into());
        }
    }
    Ok(Package {
        root,
        dll,
        receipt,
        fingerprint: digest(bytes),
    })
}
fn verify_runtime(runtime: &RuntimeInfo, package: &Package) -> Result<BTreeMap<String, String>> {
    let c = contract();
    if runtime.backend != "luxcore"
        || runtime.sdk_version != c.sdk_version
        || runtime.api != "CUDA/OptiX"
        || !runtime.device_id.starts_with("cuda:")
        || !runtime.driver.starts_with("NVIDIA ")
    {
        return Err("LuxCore actual CUDA/OptiX runtime differs from qualified package".into());
    }
    let diagnostics = runtime
        .diagnostics
        .as_ref()
        .ok_or("LuxCore runtime attestation missing")?;
    for (key, expected) in &c.policies {
        if diagnostics.settings.get(key) != Some(expected) {
            return Err(format!("LuxCore native policy differs: {key}").into());
        }
    }
    let mut hashes = BTreeMap::new();
    for (file, key) in &c.runtime_artifacts {
        let artifact = diagnostics
            .artifacts
            .get(key)
            .ok_or_else(|| format!("LuxCore runtime artifact missing: {key}"))?;
        if artifact.sha256 != c.runtime_files[file]
            || Path::new(&artifact.path).canonicalize()? != owned_file(&package.root, file)?
        {
            return Err(
                format!("LuxCore loaded runtime differs from owned package: {file}").into(),
            );
        }
        hashes.insert(key.clone(), artifact.sha256.clone());
    }
    for (key, name) in &c.driver_artifacts {
        let artifact = diagnostics
            .artifacts
            .get(key)
            .ok_or("actual CUDA/OptiX driver artifact missing")?;
        let path = Path::new(&artifact.path);
        if !path.is_absolute()
            || !path
                .file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
            || !valid_hash(&artifact.sha256)
            || file_digest(path)? != artifact.sha256
        {
            return Err("LuxCore actual driver file identity differs".into());
        }
        hashes.insert(key.clone(), artifact.sha256.clone());
    }
    let compiler = runtime
        .compiler
        .as_ref()
        .ok_or("LuxCore compiler attestation missing")?;
    if compiler.name != "NVRTC"
        || compiler.version != "12.8"
        || compiler.sha256 != c.runtime_files["nvrtc64_120_0.dll"]
        || Path::new(&compiler.path).canonicalize()?
            != owned_file(&package.root, "nvrtc64_120_0.dll")?
    {
        return Err("LuxCore compiler differs from pinned NVRTC 12.8.93 package".into());
    }
    Ok(hashes)
}
// Explicit actor-owned state: never join SDK workers from a Windows TLS
// destructor, which may run while the loader lock is held.
struct Lease {
    standby: RefCell<Option<EmbeddedContext>>,
    library: EmbeddedLibrary,
    fingerprint: String,
}
/// Construct on the owning actor. Dropping it before thread exit releases the
/// idle SDK owner once all returned backends have also been dropped.
#[derive(Default)]
pub struct LuxCoreRuntimeCache {
    lease: Option<Rc<Lease>>,
}
impl LuxCoreRuntimeCache {
    pub fn prepare(&mut self, config: &EmbeddedConfig) -> Result<LuxCoreBackend> {
        LuxCoreBackend::prepare(config, self)
    }
}
fn create_context(
    package: &Package,
    config: &EmbeddedConfig,
    cache: &mut LuxCoreRuntimeCache,
) -> Result<(EmbeddedContext, Rc<Lease>)> {
    if cache
        .lease
        .as_ref()
        .is_some_and(|lease| lease.fingerprint != package.fingerprint)
    {
        return Err("LuxCore package changed while loaded. Restart the application before using the new package.".into());
    }
    if cache.lease.is_none() {
        cache.lease = Some(Rc::new(Lease {
            standby: RefCell::new(None),
            library: EmbeddedLibrary::load(&package.dll, BackendId::LuxCore)?,
            fingerprint: package.fingerprint.clone(),
        }));
    }
    let lease = cache
        .lease
        .as_ref()
        .ok_or("LuxCore runtime lease missing")?
        .clone();
    {
        let mut idle = lease.standby.borrow_mut();
        if idle.is_none() {
            let mut selection = config.selection.clone();
            selection.device = None;
            let standby_config = EmbeddedConfig {
                selection,
                limits: config.limits.clone(),
                identity: config.identity.clone(),
                cache_directory: config.cache_directory.clone(),
            };
            let mut standby = lease.library.create(&standby_config)?;
            verify_runtime(&standby.runtime_info()?, package)?;
            *idle = Some(standby);
        }
    }
    Ok((lease.library.create(config)?, lease))
}
pub(crate) fn warnings(
    scene: &RenderSceneSnapshot,
    kind: ExportKind,
    transparent: bool,
    quality: &BackendQuality,
) -> Vec<String> {
    let mut result = Vec::new();
    let animated = matches!(
        kind,
        ExportKind::Gif | ExportKind::Mp4 | ExportKind::SectionTurntable
    );
    let alpha = kind == ExportKind::Diagram || (transparent && !animated);
    if alpha && scene.appearance.material_preset == "resin" {
        result.push(RESIN_ALPHA_APPROXIMATION.into());
    }
    if scene.appearance.layer_appearance
        && matches!(quality, BackendQuality::LuxCore { denoise: true, .. })
        && animated
    {
        result.push(LAYER_DENOISE_APPROXIMATION.into());
    }
    result
}
pub struct LuxCoreBackend {
    studio: EmbeddedStudio,
    configured: Option<(ExportKind, bool)>,
    _runtime: Rc<Lease>,
}
impl LuxCoreBackend {
    pub fn new(config: &EmbeddedConfig) -> Result<Self> {
        LuxCoreRuntimeCache::default().prepare(config)
    }
    fn prepare(config: &EmbeddedConfig, cache: &mut LuxCoreRuntimeCache) -> Result<Self> {
        config.selection.validate()?;
        if config.selection.backend != BackendId::LuxCore {
            return Err("LuxCore requires a LuxCore selection".into());
        }
        if !cfg!(all(windows, target_arch = "x86_64")) {
            return Err("The optional LuxCore SDK is qualified only for Windows x64".into());
        }
        #[cfg(target_arch = "x86_64")]
        if !std::is_x86_feature_detected!("avx") {
            return Err("The qualified LuxCore SDK requires an AVX-capable x64 CPU".into());
        }
        if config.limits.host_bytes < (2u64 << 30) || config.limits.gpu_bytes < (1u64 << 30) {
            return Err("LuxCore requires at least 2 GiB host and 1 GiB GPU allowance. Increase the render memory allowance.".into());
        }
        if config.cache_directory.is_none() {
            return Err(
                "LuxCore requires an existing application-owned compiler cache directory".into(),
            );
        }
        let package=package().map_err(|e|format!("LuxCore package unavailable: {e}. Install the verified renderers/luxcore package beside the application executable."))?;
        let (mut driver, runtime_lease) = create_context(&package, config, cache)?;
        let runtime = driver.runtime_info()?;
        let hashes = verify_runtime(&runtime, &package)?;
        if file_digest(&package.dll)? != package.receipt.bridge_sha256 {
            return Err(
                "LuxCore package changed during initialization; restart the application".into(),
            );
        }
        let bridge = package.receipt.bridge_sha256;
        let identity = BackendIdentity {
            backend: BackendId::LuxCore,
            adapter_recipe_version: 1,
            runtime_digest: digest(serde_json::to_vec(
                &serde_json::json!({"sdk":runtime.sdk_version,"source":contract().sdk_source_commit,"artifacts":hashes,"device":runtime.device_id,"name":runtime.device_name,"policies":contract().policies}),
            )?),
            bridge_digest: Some(bridge.clone()),
            shader_digest: Some(digest(format!("luxcore-shaders-v1:{bridge}"))),
            material_digest: Some(digest(format!("luxcore-physical-materials-v1:{bridge}"))),
            color_digest: digest(format!("luxcore-agx-v1:{bridge}")),
            denoiser_digest: matches!(
                config.selection.quality,
                BackendQuality::LuxCore { denoise: true, .. }
            )
            .then(|| digest(format!("oidn-2.5-cpu-high-guide-prefilter-v1:{bridge}"))),
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
            configured: None,
            _runtime: runtime_lease,
        })
    }
    pub fn studio(&mut self) -> &mut EmbeddedStudio {
        &mut self.studio
    }
    pub fn diagnostics(&self) -> Option<&RuntimeDiagnostics> {
        self.studio.diagnostics()
    }
    pub fn status() -> BackendStatus {
        BackendStatus {
            backend: BackendId::LuxCore,
            availability: BackendAvailability::NotProbed,
            capabilities: Some(CapabilityDescriptor {
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
            }),
        }
    }
}
impl RenderBackend for LuxCoreBackend {
    fn id(&self) -> BackendId {
        BackendId::LuxCore
    }
    fn identity(&self) -> BackendIdentity {
        self.studio.identity()
    }
    fn assess(&self, scene: &RenderSceneSnapshot, request: &ExportRequest) -> Vec<FeatureSupport> {
        let warnings = warnings(
            scene,
            request.kind,
            request.transparent,
            &request.selection.quality,
        );
        if warnings.is_empty() {
            vec![FeatureSupport::Supported]
        } else {
            warnings
                .into_iter()
                .map(|reason| FeatureSupport::Approximate { reason })
                .collect()
        }
    }
    fn configure(&mut self, request: &ExportRequest) -> Result<()> {
        self.studio.validate_selection(&request.selection)?;
        self.configured = Some((request.kind, request.transparent));
        Ok(())
    }
    fn render_sequence(
        &mut self,
        scene: &RenderSceneSnapshot,
        frames: &FramePlan,
        context: &RenderContext<'_>,
        emit: &mut dyn FnMut(FrameArtifact) -> Result<()>,
    ) -> Result<RenderReport> {
        let (kind, _transparent) = self
            .configured
            .ok_or("configure LuxCore before rendering a sequence")?;
        let mut report = backend::render_sequence(&mut self.studio, scene, frames, context, emit)?;
        report.warnings.extend(warnings(
            scene,
            kind,
            frames.transparent,
            &context.selection.quality,
        ));
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn receipt() -> serde_json::Value {
        let mut value: serde_json::Value = serde_json::from_str(include_str!(
            "../../../native/luxcore/package-contract.json"
        ))
        .unwrap();
        let map = value.as_object_mut().unwrap();
        for key in ["policies", "driver_artifacts", "runtime_artifacts"] {
            map.remove(key);
        }
        value["bridge_sha256"] = "a".repeat(64).into();
        value
    }
    #[test]
    fn receipt_accepts_qualified_inputs_and_rebuilt_bridge() {
        assert!(PackageReceipt::parse(&serde_json::to_vec(&receipt()).unwrap()).is_ok());
        let mut rebuilt = receipt();
        rebuilt["bridge_sha256"] = "b".repeat(64).into();
        assert!(PackageReceipt::parse(&serde_json::to_vec(&rebuilt).unwrap()).is_ok());
    }
    #[test]
    fn receipt_rejects_unknown_sdk_dependencies_and_ambiguous_documents() {
        for key in [
            "sdk_version",
            "sdk_source_commit",
            "sdk_source_archive_sha256",
            "compiler_pair_sha256",
            "native_recipe",
            "bridge_sha256",
        ] {
            let mut value = receipt();
            value[key] = "wrong".into();
            assert!(
                PackageReceipt::parse(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{key}"
            );
        }
        let mut changed = receipt();
        changed["runtime_files"]["nvrtc64_120_0.dll"] = "b".repeat(64).into();
        assert!(PackageReceipt::parse(&serde_json::to_vec(&changed).unwrap()).is_err());
        let mut changed = receipt();
        changed["runtime_files"]["../nvrtc64_120_0.dll"] = "a".repeat(64).into();
        assert!(PackageReceipt::parse(&serde_json::to_vec(&changed).unwrap()).is_err());
        let mut unknown = receipt();
        unknown["download_missing"] = true.into();
        assert!(PackageReceipt::parse(&serde_json::to_vec(&unknown).unwrap()).is_err());
        let text = serde_json::to_string(&receipt()).unwrap();
        let duplicate = text.replace(
            "\"runtime_files\":{",
            "\"runtime_files\":{\"nvrtc64_120_0.dll\":\"duplicate\",",
        );
        assert!(PackageReceipt::parse(duplicate.as_bytes()).is_err());
        let mut large = text.into_bytes();
        large.resize(16385, b' ');
        assert!(PackageReceipt::parse(&large).is_err());
    }
    #[test]
    fn support_warnings_preserve_opaque_tracing_and_raw_layer_animation() {
        let mesh = crate::mesh::Mesh {
            positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 1.]],
            triangles: vec![[0, 1, 2]],
        };
        let mut scene=snapshot_scene(std::sync::Arc::new(mesh),serde_json::json!({"schema_version":1,"mesh_path":"D:/fixture/mesh.bin","color":[0.2,0.4,0.6],"roughness":0.45,"geometry_kind":"original","layer_height_mm":0.2,"display_rotation_degrees":[0.,0.,0.],"warnings":[]})).unwrap();
        scene.appearance.material_preset = "resin".into();
        scene.appearance.layer_appearance = true;
        let denoise = BackendQuality::LuxCore {
            samples: 64,
            denoise: true,
        };
        let raw = BackendQuality::LuxCore {
            samples: 64,
            denoise: false,
        };
        assert!(warnings(&scene, ExportKind::Still, false, &denoise).is_empty());
        assert!(warnings(&scene, ExportKind::Gif, false, &raw).is_empty());
        assert_eq!(
            warnings(&scene, ExportKind::Still, true, &denoise),
            vec![RESIN_ALPHA_APPROXIMATION]
        );
        assert_eq!(
            warnings(&scene, ExportKind::Gif, true, &denoise),
            vec![LAYER_DENOISE_APPROXIMATION]
        );
        assert_eq!(
            warnings(&scene, ExportKind::Diagram, false, &denoise),
            vec![RESIN_ALPHA_APPROXIMATION]
        );
        assert_eq!(
            warnings(&scene, ExportKind::Gif, false, &denoise),
            vec![LAYER_DENOISE_APPROXIMATION]
        );
    }
    #[test]
    fn compiled_package_contract_matches_the_qualified_sdk_source() {
        let qualified: serde_json::Value = serde_json::from_str(include_str!(
            "../../../native/patches/luxcore/qualification.json"
        ))
        .unwrap();
        let c = contract();
        assert_eq!(c.sdk_source_commit, qualified["source_commit"]);
        assert_eq!(
            c.sdk_source_archive_sha256,
            qualified["source_archive_sha256"]
        );
        assert_eq!(c.runtime_files["luxcore.dll"], qualified["sdk_identity"]);
        assert_eq!(c.runtime_files.len(), 10);
        assert_eq!(c.driver_artifacts.len(), 2);
    }
}
