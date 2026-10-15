use super::*;
use crate::{Result, render::BackendId};
use std::path::Path;

#[cfg(all(windows, target_pointer_width = "64"))]
mod platform {
    use super::*;
    use crate::render::contract::RenderProjection;
    use std::{os::windows::ffi::OsStrExt, ptr, rc::Rc, sync::OnceLock};
    use windows_sys::Win32::{
        Foundation::{FreeLibrary, HMODULE},
        System::LibraryLoader::{
            GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
            LoadLibraryExW, SetDefaultDllDirectories,
        },
    };

    struct Module(HMODULE);
    impl Drop for Module {
        fn drop(&mut self) {
            // SAFETY: this object owns the successful LoadLibrary reference.
            unsafe {
                FreeLibrary(self.0);
            }
        }
    }
    struct Api {
        _module: Module,
        create: abi::Create,
        describe: abi::Describe,
        upload: abi::Upload,
        begin: abi::Begin,
        poll: abi::PollFn,
        read: abi::Read,
        cancel: abi::Cancel,
        destroy: abi::Destroy,
        backend: u32,
    }
    /// Application-local bridge; Rc retains the module and prevents Send/Sync.
    /// ```compile_fail
    /// fn require_send<T: Send>() {}
    /// require_send::<turninator::render::embedded::EmbeddedLibrary>();
    /// ```
    pub struct EmbeddedLibrary {
        api: Rc<Api>,
    }
    /// Owner-thread RAII: native destruction drains work before module unload.
    /// ```compile_fail
    /// fn require_send_sync<T: Send + Sync>() {}
    /// require_send_sync::<turninator::render::embedded::EmbeddedContext>();
    /// ```
    pub struct EmbeddedContext {
        api: Rc<Api>,
        raw: abi::Context,
        limits: RenderLimits,
        backend: BackendId,
        selected_device: Option<String>,
        dimensions: Option<(u32, u32)>,
    }
    impl Drop for EmbeddedContext {
        fn drop(&mut self) {
            // SAFETY: unique live context; its module outlives this call.
            unsafe {
                (self.api.destroy)(self.raw);
            }
        }
    }
    fn invoke(f: impl FnOnce(*mut abi::ErrorBuffer) -> i32) -> (i32, String) {
        let mut bytes = [0u8; 1024];
        let mut error = abi::ErrorBuffer {
            size: size_of::<abi::ErrorBuffer>() as u32,
            version: abi::VERSION,
            text: bytes.as_mut_ptr(),
            text_capacity: bytes.len() as u64,
        };
        let code = f(&mut error);
        let n = bytes.iter().position(|v| *v == 0).unwrap_or(bytes.len());
        (
            code,
            format!(
                "native renderer error {code}: {}",
                String::from_utf8_lossy(&bytes[..n])
            ),
        )
    }
    fn call(f: impl FnOnce(*mut abi::ErrorBuffer) -> i32) -> Result<()> {
        let (code, message) = invoke(f);
        if code == 0 {
            Ok(())
        } else {
            Err(message.into())
        }
    }
    fn json(value: &impl serde::Serialize, limit: usize) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(value)?;
        if bytes.len() > limit.min(abi::MAX_JSON) {
            return Err("native JSON exceeds 1 MiB/configured budget".into());
        }
        Ok(bytes)
    }
    fn backend(id: BackendId) -> Result<u32> {
        match id {
            BackendId::Filament => Ok(1),
            BackendId::LuxCore => Ok(2),
            _ => Err("backend does not use the native SDK ABI".into()),
        }
    }
    // Permanent process policy, intentionally not restored after loading.
    fn restrict_default_search() -> Result<()> {
        static POLICY: OnceLock<std::result::Result<(), String>> = OnceLock::new();
        POLICY
            .get_or_init(|| {
                // SAFETY: initial and subsequent SDK loads must never use cwd/PATH.
                if unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) } == 0 {
                    Err(format!(
                        "cannot restrict DLL search: {}",
                        std::io::Error::last_os_error()
                    ))
                } else {
                    Ok(())
                }
            })
            .clone()
            .map_err(Into::into)
    }
    impl EmbeddedLibrary {
        /// Only exe_dir/renderers/{backend}/tn_renderer_{backend}.dll is accepted.
        /// First valid-path load permanently restricts process default DLL search
        /// to System32. Other dynamic loaders must use explicit paths too.
        /// Missing or invalid optional SDKs are ordinary availability errors.
        pub fn load(path: &Path, expected: BackendId) -> Result<Self> {
            Self::load_inner(path, backend(expected)?, expected.as_str(), false)
        }
        /// Explicit CPU contract fixture. Never accepted as a production renderer.
        pub fn load_fixture(path: &Path) -> Result<Self> {
            Self::load_inner(path, abi::FIXTURE, "fixtures", true)
        }
        fn load_inner(path: &Path, expected: u32, directory: &str, fixture: bool) -> Result<Self> {
            if !path.is_absolute() {
                return Err("renderer DLL path must be absolute".into());
            }
            let exe = std::env::current_exe()?.canonicalize()?;
            let root = exe
                .parent()
                .ok_or("missing executable directory")?
                .join("renderers")
                .join(directory);
            let trusted = root.canonicalize()?;
            if trusted != root {
                return Err("renderer directory is redirected outside its application path".into());
            }
            let canonical = path.canonicalize()?;
            if !canonical.starts_with(&trusted) || !canonical.is_file() {
                return Err(
                    "renderer DLL must be in its canonical application-owned directory".into(),
                );
            }
            if !fixture
                && (canonical.parent() != Some(trusted.as_path())
                    || canonical.file_name()
                        != Some(std::ffi::OsStr::new(&format!(
                            "tn_renderer_{directory}.dll"
                        ))))
            {
                return Err("unexpected selected renderer DLL name/directory".into());
            }
            restrict_default_search()?;
            let wide: Vec<u16> = canonical.as_os_str().encode_wide().chain(Some(0)).collect();
            // SAFETY: canonical absolute NUL-terminated path; transitive search restricted.
            let handle = unsafe {
                LoadLibraryExW(
                    wide.as_ptr(),
                    ptr::null_mut(),
                    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
                )
            };
            if handle.is_null() {
                return Err(format!(
                    "optional renderer unavailable (DLL/dependency): {}",
                    std::io::Error::last_os_error()
                )
                .into());
            }
            let module = Module(handle);
            macro_rules! symbol {
                ($name:literal,$ty:ty) => {{
                    // SAFETY: static NUL-terminated name, checked ABI before context use.
                    let address =
                        unsafe { GetProcAddress(module.0, concat!($name, "\0").as_ptr()) }
                            .ok_or(concat!("missing native renderer entry point: ", $name))?;
                    unsafe {
                        std::mem::transmute::<unsafe extern "system" fn() -> isize, $ty>(address)
                    }
                }};
            }
            let query = symbol!("tn_renderer_abi_version", abi::Query);
            let mut info = abi::Info {
                size: size_of::<abi::Info>() as u32,
                version: abi::VERSION,
                ..Default::default()
            };
            call(|e| unsafe { query(&mut info, e) })?;
            if info.size != size_of::<abi::Info>() as u32
                || info.version != abi::VERSION
                || info.backend != expected
                || info.flags != if fixture { abi::FIXTURE_FLAG } else { 0 }
                || info.error_size != size_of::<abi::ErrorBuffer>() as u32
                || info.config_size != size_of::<abi::Config>() as u32
                || info.scene_size != size_of::<abi::Scene>() as u32
                || info.frame_size != size_of::<abi::Frame>() as u32
                || info.poll_size != size_of::<abi::Poll>() as u32
                || info.image_size != size_of::<abi::Image>() as u32
            {
                return Err(
                    "incompatible renderer ABI version/struct size/backend/fixture identity".into(),
                );
            }
            let api = Api {
                create: symbol!("tn_renderer_create", abi::Create),
                describe: symbol!("tn_renderer_describe", abi::Describe),
                upload: symbol!("tn_renderer_upload_scene", abi::Upload),
                begin: symbol!("tn_renderer_begin_frame", abi::Begin),
                poll: symbol!("tn_renderer_poll", abi::PollFn),
                read: symbol!("tn_renderer_read_rgba8", abi::Read),
                cancel: symbol!("tn_renderer_cancel", abi::Cancel),
                destroy: symbol!("tn_renderer_destroy", abi::Destroy),
                _module: module,
                backend: expected,
            };
            Ok(Self { api: Rc::new(api) })
        }
        pub fn create(&self, config: &EmbeddedConfig) -> Result<EmbeddedContext> {
            config.selection.validate()?;
            let id = backend(config.selection.backend)?;
            if (self.api.backend != abi::FIXTURE && id != self.api.backend)
                || config.identity.backend != config.selection.backend
            {
                return Err("config/identity does not match selected native backend".into());
            }
            let l = &config.limits;
            if l.max_output_side == 0
                || l.max_output_side > 8000
                || l.max_scene_bytes == 0
                || l.host_bytes == 0
                || l.gpu_bytes == 0
            {
                return Err("invalid native renderer limits".into());
            }
            let bytes = json(
                &serde_json::json!({"schema_version":1,"selection":config.selection,"identity":config.identity,
                "limits":{"max_source_vertices":l.max_source_vertices,"max_source_triangles":l.max_source_triangles,
                    "max_scene_bytes":l.max_scene_bytes,"max_output_side":l.max_output_side,
                    "host_bytes":l.host_bytes,"gpu_bytes":l.gpu_bytes,"scratch_bytes":l.scratch_bytes}}),
                abi::MAX_JSON,
            )?;
            let cfg = abi::Config {
                size: size_of::<abi::Config>() as u32,
                version: abi::VERSION,
                json: bytes.as_ptr(),
                json_bytes: bytes.len() as u64,
                host_bytes: l.host_bytes,
                gpu_bytes: l.gpu_bytes,
                max_output_side: l.max_output_side,
                reserved: 0,
            };
            let mut raw = ptr::null_mut();
            // SAFETY: temporary config and JSON remain valid through the copying call.
            let result = call(|e| unsafe { (self.api.create)(&cfg, &mut raw, e) });
            if let Err(e) = result {
                if !raw.is_null() {
                    unsafe {
                        (self.api.destroy)(raw);
                    }
                }
                return Err(e);
            }
            if raw.is_null() {
                return Err("native create returned a null context".into());
            }
            let mut context = EmbeddedContext {
                api: Rc::clone(&self.api),
                raw,
                limits: l.clone(),
                backend: config.selection.backend,
                selected_device: config.selection.device.clone(),
                dimensions: None,
            };
            context.runtime_info()?; // Validate actual runtime before accepting preparation.
            Ok(context)
        }
    }
    impl EmbeddedContext {
        pub fn runtime_info(&mut self) -> Result<RuntimeInfo> {
            let mut required = 0;
            // SAFETY: capacity query writes only required/error and must not mutate state.
            let (code, message) = invoke(|e| unsafe {
                (self.api.describe)(self.raw, ptr::null_mut(), 0, &mut required, e)
            });
            if code != 3 {
                return Err(format!("invalid native describe capacity response: {message}").into());
            }
            if required == 0 || required > abi::MAX_JSON as u64 {
                return Err("native runtime info exceeds bounds".into());
            }
            let mut bytes = vec![0u8; required as usize];
            let capacity = required;
            // SAFETY: capacity matched to caller-owned buffer, no native allocation transfer.
            call(|e| unsafe {
                (self.api.describe)(self.raw, bytes.as_mut_ptr(), capacity, &mut required, e)
            })?;
            if required != capacity {
                return Err("native runtime info changed length during query".into());
            }
            let text = std::str::from_utf8(&bytes)?;
            let info: RuntimeInfo = serde_json::from_str(text)?;
            let expected = if self.api.backend == abi::FIXTURE {
                "fixture"
            } else {
                self.backend.as_str()
            };
            let fields = [
                &info.backend,
                &info.sdk_version,
                &info.device_id,
                &info.device_name,
                &info.api,
                &info.driver,
            ];
            if info.schema_version != 1
                || info.backend != expected
                || fields
                    .iter()
                    .any(|v| v.is_empty() || v.len() > 4096 || v.chars().any(char::is_control))
                || self
                    .selected_device
                    .as_ref()
                    .is_some_and(|id| id != &info.device_id)
            {
                return Err("native runtime identity/backend/selected device mismatch".into());
            }
            if let Some(c) = &info.compiler
                && (c.name.is_empty()
                    || c.name.len() > 256
                    || c.version.is_empty()
                    || c.version.len() > 256
                    || c.sha256.len() != 64
                    || !c.sha256.bytes().all(|v| v.is_ascii_hexdigit())
                    || !Path::new(&c.path).is_absolute()
                    || c.path.len() > 32768)
            {
                return Err("invalid native runtime compiler identity".into());
            }
            Ok(info)
        }
        pub fn upload_scene(&mut self, scene: &SceneUpload<'_>) -> Result<()> {
            let n = scene.positions_mm.len();
            let t = scene.triangles.len();
            let meta = scene.metadata;
            // These are already-realized/aligned spans. Configurable source
            // limits are enforced before realization, not on derived geometry.
            if n > crate::mesh::MAX_VERTICES
                || t > crate::mesh::MAX_TRIANGLES
                || scene.normals.len() != n
                || scene.build_positions_mm.len() != n
                || scene.triangle_materials.len() != t
                || scene.triangles.iter().flatten().any(|i| *i as usize >= n)
                || scene
                    .triangle_materials
                    .iter()
                    .any(|i| *i as usize >= meta.materials.len())
                || scene
                    .positions_mm
                    .iter()
                    .chain(scene.normals)
                    .chain(scene.build_positions_mm)
                    .flatten()
                    .any(|v| !v.is_finite())
            {
                return Err("invalid native geometry spans/indices/budget".into());
            }
            if meta.materials.len() > 256
                || meta.materials.iter().any(|m| {
                    m.name.is_empty()
                        || m.name.len() > 256
                        || m.base_color_linear
                            .iter()
                            .chain(&m.absorption_color_linear)
                            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                        || [
                            m.roughness,
                            m.metallic,
                            m.coat,
                            m.coat_roughness,
                            m.transmission,
                        ]
                        .iter()
                        .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                        || [m.anisotropy, m.scattering_anisotropy]
                            .iter()
                            .any(|v| !v.is_finite() || !(-1.0..=1.0).contains(v))
                        || !m.ior.is_finite()
                        || m.ior <= 0.
                        || [m.absorption_density_per_mm, m.scattering_per_mm]
                            .iter()
                            .any(|v| !v.is_finite() || *v < 0.)
                })
            {
                return Err("invalid native physical materials".into());
            }
            let bounds = &scene.metadata.reference_bounds_mm;
            let a = &meta.appearance;
            let filter = &meta.layer_filter;
            if meta.color.exposure_ev != 0.
                || meta.color.tone_map != EmbeddedColorConfig::for_backend(self.backend)?.tone_map
                || filter.fade_start_pixels != 2.
                || filter.fade_end_pixels != 4.
                || filter.footprint_diagonal_factor != std::f64::consts::SQRT_2
                || a.layer_appearance
                    && meta.geometry_kind != crate::render::contract::GeometryKind::Original
            {
                return Err("unsupported native color/layer recipe configuration".into());
            }
            if bounds.min.iter().chain(&bounds.max).any(|v| !v.is_finite())
                || (0..3).any(|i| bounds.min[i] > bounds.max[i])
                || a.color
                    .iter()
                    .chain([a.roughness, a.resin_clarity, a.resin_cloudiness].iter())
                    .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                || a.display_rotation_degrees.iter().any(|v| !v.is_finite())
                || [a.first_layer_height_mm, a.layer_height_mm]
                    .iter()
                    .any(|v| !v.is_finite() || *v <= 0.)
                || !a.groove_depth_mm.is_finite()
                || a.groove_depth_mm < 0.
                || a.material_preset.is_empty()
                || a.studio_preset.is_empty()
                || a.material_preset.len() > 256
                || a.studio_preset.len() > 256
            {
                return Err("invalid native appearance/bounds".into());
            }
            let bytes = json(
                &serde_json::json!({"schema_version":1,"appearance":meta.appearance,
                "materials":meta.materials,"reference_bounds_mm":{"min":bounds.min,"max":bounds.max},
                "geometry_kind":meta.geometry_kind,"layer_filter":filter,
                "color":{"schema_version":1,"exposure_ev":meta.color.exposure_ev,"tone_map":meta.color.tone_map,
                    "output_transfer":"srgb","output_alpha":"straight"}}),
                self.limits.max_scene_bytes.min(abi::MAX_JSON as u64) as usize,
            )?;
            let geometry_bytes = (n as u64)
                .checked_mul(36)
                .and_then(|v| (t as u64).checked_mul(16).and_then(|w| v.checked_add(w)))
                .ok_or("geometry size overflow")?;
            if geometry_bytes
                .checked_add(bytes.len() as u64)
                .is_none_or(|v| v > self.limits.host_bytes)
                || geometry_bytes > self.limits.gpu_bytes
            {
                return Err("native upload exceeds host/GPU byte budget".into());
            }
            let raw = abi::Scene {
                size: size_of::<abi::Scene>() as u32,
                version: abi::VERSION,
                revision: scene.revision,
                positions_mm: scene.positions_mm.as_ptr().cast(),
                normals: scene.normals.as_ptr().cast(),
                build_positions_mm: scene.build_positions_mm.as_ptr().cast(),
                triangles: scene.triangles.as_ptr().cast(),
                triangle_materials: scene.triangle_materials.as_ptr(),
                vertex_count: n as u32,
                triangle_count: t as u32,
                json: bytes.as_ptr(),
                json_bytes: bytes.len() as u64,
            };
            // SAFETY: contiguous validated slices and JSON outlive the copying call.
            call(|e| unsafe { (self.api.upload)(self.raw, &raw, e) })
        }
        pub fn begin_frame(&mut self, frame: &EmbeddedFrame) -> Result<()> {
            RenderSelection {
                backend: self.backend,
                device: self.selected_device.clone(),
                quality: frame.quality.clone(),
            }
            .validate()?;
            let bytes = u64::from(frame.width)
                .checked_mul(u64::from(frame.height))
                .and_then(|n| n.checked_mul(4))
                .ok_or("image byte overflow")?;
            if frame.width == 0
                || frame.height == 0
                || frame.width > self.limits.max_output_side
                || frame.height > self.limits.max_output_side
                || bytes > self.limits.host_bytes
                || bytes > self.limits.gpu_bytes
                || frame.quality.backend() != self.backend
            {
                return Err("invalid native frame dimensions/quality/budget".into());
            }
            let c = &frame.camera;
            let direction: [f64; 3] = std::array::from_fn(|i| c.position_mm[i] - c.target_mm[i]);
            let distance = direction.iter().map(|v| v * v).sum::<f64>().sqrt();
            let cross = [
                direction[1] * c.up[2] - direction[2] * c.up[1],
                direction[2] * c.up[0] - direction[0] * c.up[2],
                direction[0] * c.up[1] - direction[1] * c.up[0],
            ];
            let valid_projection = match c.projection {
                RenderProjection::Perspective {
                    vertical_fov_degrees,
                } => {
                    vertical_fov_degrees.is_finite()
                        && (1.0..=150.0).contains(&vertical_fov_degrees)
                }
                RenderProjection::Orthographic { vertical_span_mm } => {
                    vertical_span_mm.is_finite() && vertical_span_mm >= 0.000001
                }
            };
            if c.position_mm
                .iter()
                .chain(&c.target_mm)
                .chain(&c.up)
                .any(|v| !v.is_finite())
                || !distance.is_finite()
                || distance < 0.000001
                || cross.iter().map(|v| v * v).sum::<f64>() < 1e-15
                || !valid_projection
            {
                return Err("invalid resolved native camera".into());
            }
            if frame.model_to_world.iter().any(|v| !v.is_finite())
                || [
                    frame.model_to_world[12],
                    frame.model_to_world[13],
                    frame.model_to_world[14],
                    frame.model_to_world[15],
                ] != [0., 0., 0., 1.]
            {
                return Err("native model transform must be finite and affine".into());
            }
            let quality = json(
                &serde_json::json!({"schema_version":1,"quality":frame.quality}),
                abi::MAX_JSON,
            )?;
            let (projection, vertical) = match frame.camera.projection {
                RenderProjection::Perspective {
                    vertical_fov_degrees,
                } => (0, vertical_fov_degrees),
                RenderProjection::Orthographic { vertical_span_mm } => (1, vertical_span_mm),
            };
            let (near_mm, far_mm) = frame.camera.clip_planes_mm();
            let raw = abi::Frame {
                size: size_of::<abi::Frame>() as u32,
                version: abi::VERSION,
                geometry_revision: frame.geometry_revision,
                width: frame.width,
                height: frame.height,
                camera_position_mm: frame.camera.position_mm,
                camera_target_mm: frame.camera.target_mm,
                camera_up: frame.camera.up,
                projection,
                transparent: u32::from(frame.transparent),
                sample_seed: frame.sample_seed,
                reserved: 0,
                vertical,
                near_mm,
                far_mm,
                model_to_world: frame.model_to_world,
                quality_json: quality.as_ptr(),
                quality_json_bytes: quality.len() as u64,
            };
            // SAFETY: inputs borrowed until the native copying call returns.
            call(|e| unsafe { (self.api.begin)(self.raw, &raw, e) })?;
            self.dimensions = Some((frame.width, frame.height));
            Ok(())
        }
        pub fn poll(&mut self) -> Result<EmbeddedPoll> {
            let mut raw = abi::Poll {
                size: size_of::<abi::Poll>() as u32,
                version: abi::VERSION,
                ..Default::default()
            };
            // SAFETY: caller-owned initialized output and owner-thread live context.
            call(|e| unsafe { (self.api.poll)(self.raw, &mut raw, e) })?;
            if raw.size != size_of::<abi::Poll>() as u32
                || raw.version != abi::VERSION
                || raw.reserved != 0
                || raw.total_work != 0 && raw.completed_work > raw.total_work
            {
                return Err("invalid native poll result".into());
            }
            let state = match raw.state {
                0 => EmbeddedState::Idle,
                1 => EmbeddedState::Rendering,
                2 => EmbeddedState::Ready,
                3 => EmbeddedState::Cancelling,
                4 => EmbeddedState::Cancelled,
                5 => EmbeddedState::Failed,
                _ => return Err("unknown native renderer state".into()),
            };
            Ok(EmbeddedPoll {
                state,
                completed_work: raw.completed_work,
                total_work: (raw.total_work != 0).then_some(raw.total_work),
            })
        }
        /// Too-small buffers remain untouched and retain native Ready for retry.
        pub fn read_pixels(&mut self, pixels: &mut [u8]) -> Result<(u32, u32)> {
            let mut image = abi::Image {
                size: size_of::<abi::Image>() as u32,
                version: abi::VERSION,
                ..Default::default()
            };
            // SAFETY: pointer paired with its true capacity, no ownership transfer.
            call(|e| unsafe {
                (self.api.read)(
                    self.raw,
                    pixels.as_mut_ptr(),
                    pixels.len() as u64,
                    &mut image,
                    e,
                )
            })?;
            if image.size != size_of::<abi::Image>() as u32
                || image.version != abi::VERSION
                || self.dimensions != Some((image.width, image.height))
                || image.required_bytes != u64::from(image.width) * u64::from(image.height) * 4
                || image.required_bytes > pixels.len() as u64
            {
                return Err("invalid native readback dimensions/length".into());
            }
            self.dimensions = None;
            Ok((image.width, image.height))
        }
        pub fn cancel(&mut self) -> Result<()> {
            // SAFETY: live context; ABI requires nonblocking cooperative request.
            call(|e| unsafe { (self.api.cancel)(self.raw, e) })
        }
    }
}
#[cfg(not(all(windows, target_pointer_width = "64")))]
mod platform {
    use super::*;
    use std::{marker::PhantomData, rc::Rc};
    pub struct EmbeddedLibrary(PhantomData<Rc<()>>);
    pub struct EmbeddedContext(PhantomData<Rc<()>>);
    fn unavailable<T>() -> Result<T> {
        Err("optional native SDK bridges are qualified only on Windows x64".into())
    }
    impl EmbeddedLibrary {
        pub fn load(_: &Path, _: BackendId) -> Result<Self> {
            unavailable()
        }
        pub fn load_fixture(_: &Path) -> Result<Self> {
            unavailable()
        }
        pub fn create(&self, _: &EmbeddedConfig) -> Result<EmbeddedContext> {
            unavailable()
        }
    }
    impl EmbeddedContext {
        pub fn runtime_info(&mut self) -> Result<RuntimeInfo> {
            unavailable()
        }
        pub fn upload_scene(&mut self, _: &SceneUpload<'_>) -> Result<()> {
            unavailable()
        }
        pub fn begin_frame(&mut self, _: &EmbeddedFrame) -> Result<()> {
            unavailable()
        }
        pub fn poll(&mut self) -> Result<EmbeddedPoll> {
            unavailable()
        }
        pub fn read_pixels(&mut self, _: &mut [u8]) -> Result<(u32, u32)> {
            unavailable()
        }
        pub fn cancel(&mut self) -> Result<()> {
            unavailable()
        }
    }
}
pub use platform::{EmbeddedContext, EmbeddedLibrary};
