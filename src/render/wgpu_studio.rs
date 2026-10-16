//! Headless wgpu studio. No egui texture ownership or external renderer process.
//! All positions are physical millimeters; only the canonical display affine is applied.
use crate::{
    Result,
    render::{
        color::{AGX_WGSL, AgxRecipe},
        contract::*,
        geometry::resolve_display_transform,
        selection::{AaQuality, BackendQuality, ShadowQuality},
    },
};
use eframe::wgpu::{self, util::DeviceExt};
use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    time::{Duration, Instant},
};

pub const STUDIO_SHADER: &str = include_str!("shaders/studio.wgsl");
pub const OUTPUT_SHADER: &str = include_str!("shaders/tonemap.wgsl");
pub const STUDIO_RECIPE: &str = "wgpu-studio-v1;ggx-smith-schlick-anisotropic;analytic-gaussian-softboxes-v1;nearest-surface-resin-thickness=0.35*extent;ssao-radius=0.10*extent;aa=1,4,8;shadow=1024,2048,4096;pcf=3,5,7-receiver-plane;ao=8,16,32;agx=0ev;rgba16f-linear-premult-accum;rgba8unorm-srgb-straight";

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct StudioQuality {
    pub samples: u32,
    pub shadow_size: u32,
    pub shadow_radius: u32,
    pub ao_samples: u32,
}
impl StudioQuality {
    pub fn resolve(quality: &BackendQuality) -> Result<Self> {
        let BackendQuality::Wgpu { aa, shadows } = quality else {
            return Err("wgpu quality required".into());
        };
        let samples = match aa {
            AaQuality::Low => 1,
            AaQuality::Medium => 4,
            AaQuality::High => 8,
        };
        let (shadow_size, shadow_radius, ao_samples) = match shadows {
            ShadowQuality::Low => (1024, 1, 8),
            ShadowQuality::Medium => (2048, 2, 16),
            ShadowQuality::High => (4096, 3, 32),
        };
        Ok(Self {
            samples,
            shadow_size,
            shadow_radius,
            ao_samples,
        })
    }
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    eye: [f32; 4],
    projection: [f32; 4],
    clipping: [f32; 4],
    rows: [[f32; 4]; 3],
    light_right: [f32; 4],
    light_up: [f32; 4],
    light_back: [f32; 4],
    center: [f32; 4],
    screen: [f32; 4],
    layers: [f32; 4],
    build: [f32; 4],
    sample: [f32; 4],
    quality: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    build: [f32; 3],
    material: u32,
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Material {
    base: [f32; 4],
    surface: [f32; 4],
    volume: [f32; 4],
    absorption: [f32; 4],
    flags: [f32; 4],
}

/// Immutable GPU upload. The caller owns its lifetime and budget accounting.
pub struct UploadedScene {
    vertices: wgpu::Buffer,
    materials: wgpu::Buffer,
    count: u32,
    pub bytes: u64,
}
struct Texture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}
struct Target {
    size: [u32; 2],
    sample: Texture,
    accum: Texture,
    depth: Texture,
    output: Texture,
    readback: wgpu::Buffer,
    row: u32,
}

#[cfg(test)]
type SubmissionHook = Box<dyn Fn(&wgpu::Queue) + Send + Sync>;

pub struct WgpuStudio {
    device: wgpu::Device,
    queue: wgpu::Queue,
    info: wgpu::AdapterInfo,
    quality: StudioQuality,
    limits: RenderLimits,
    geometry_layout: wgpu::BindGroupLayout,
    lighting_layout: wgpu::BindGroupLayout,
    output_layout: wgpu::BindGroupLayout,
    shadow_pipeline: wgpu::RenderPipeline,
    depth_pipeline: wgpu::RenderPipeline,
    floor_depth_pipeline: wgpu::RenderPipeline,
    surface_pipeline: wgpu::RenderPipeline,
    floor_pipeline: wgpu::RenderPipeline,
    background_pipeline: wgpu::RenderPipeline,
    accumulate_pipeline: wgpu::RenderPipeline,
    tonemap_pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    output_uniform: wgpu::Buffer,
    shadow: Texture,
    target: Option<Target>,
    uploads: u64,
    in_flight: AtomicBool,
    unusable: AtomicBool,
    completion_timeout: Duration,
    #[cfg(test)]
    after_sample_submit: Option<SubmissionHook>,
}

pub(crate) fn block_on<T>(future: impl Future<Output = T>) -> T {
    struct ThreadWake(std::thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut f = std::pin::pin!(future);
    loop {
        match f.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::park_timeout(Duration::from_millis(10)),
        }
    }
}
fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err("cancelled".into())
    } else {
        Ok(())
    }
}
fn tex(
    device: &wgpu::Device,
    size: [u32; 2],
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("studio offscreen"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    Texture { texture, view }
}
fn buffer(
    device: &wgpu::Device,
    label: &str,
    data: &[u8],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: if data.is_empty() { &[0; 4] } else { data },
        usage,
    })
}
pub fn adapter_name(info: &wgpu::AdapterInfo) -> String {
    format!(
        "{} [{:?} {:04x}:{:04x}]",
        info.name, info.backend, info.vendor, info.device
    )
}

impl WgpuStudio {
    pub fn new(
        device_name: Option<&str>,
        quality: &BackendQuality,
        limits: &RenderLimits,
    ) -> Result<Self> {
        let quality = StudioQuality::resolve(quality)?;
        if u64::from(quality.shadow_size).pow(2) * 4 + 65536 > limits.gpu_bytes {
            return Err("studio shadow target exceeds GPU budget".into());
        }
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let mut adapters = block_on(instance.enumerate_adapters(wgpu::Backends::PRIMARY));
        adapters.sort_by_key(|a| {
            let i = a.get_info();
            (
                match i.device_type {
                    wgpu::DeviceType::DiscreteGpu => 0,
                    wgpu::DeviceType::IntegratedGpu => 1,
                    _ => 2,
                },
                match i.backend {
                    wgpu::Backend::Vulkan => 0,
                    wgpu::Backend::Dx12 => 1,
                    _ => 2,
                },
                i.name,
            )
        });
        let adapter = adapters
            .into_iter()
            .find(|a| {
                let i = a.get_info();
                device_name.map_or(
                    matches!(
                        i.device_type,
                        wgpu::DeviceType::DiscreteGpu | wgpu::DeviceType::IntegratedGpu
                    ),
                    |s| s == adapter_name(&i),
                )
            })
            .ok_or("requested native hardware adapter is unavailable; no fallback was used")?;
        let info = adapter.get_info();
        if matches!(info.device_type, wgpu::DeviceType::Cpu) {
            return Err("CPU raster adapter is not qualified".into());
        }
        let requested = wgpu::Limits {
            max_texture_dimension_2d: limits.max_output_side.max(quality.shadow_size),
            max_buffer_size: adapter.limits().max_buffer_size,
            ..Default::default()
        };
        if requested.max_texture_dimension_2d > adapter.limits().max_texture_dimension_2d {
            return Err("adapter cannot satisfy requested output dimensions".into());
        }
        for format in [
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureFormat::Rgba8Unorm,
        ] {
            let features = adapter.get_texture_format_features(format);
            if !features.allowed_usages.contains(
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            ) {
                return Err("adapter lacks required studio target formats".into());
            }
        }
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Turninator native studio"),
            required_limits: requested,
            ..Default::default()
        }))?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let uniform = buffer(
            &device,
            "studio uniforms",
            bytemuck::bytes_of(&<Uniforms as bytemuck::Zeroable>::zeroed()),
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let output_uniform = buffer(
            &device,
            "output parameters",
            bytemuck::cast_slice(&[1f32, 0., 0., 0.]),
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let geometry_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("studio scene"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let lighting_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("studio depth lighting"),
            entries: &[0, 1].map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }),
        });
        let output_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("studio color resolve"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("studio PBR"),
            source: wgpu::ShaderSource::Wgsl(format!("{AGX_WGSL}\n{STUDIO_SHADER}").into()),
        });
        let output_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("studio AgX resolve"),
            source: wgpu::ShaderSource::Wgsl(format!("{AGX_WGSL}\n{OUTPUT_SHADER}").into()),
        });
        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("studio prepass"),
            bind_group_layouts: &[Some(&geometry_layout)],
            immediate_size: 0,
        });
        let lit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("studio lit"),
            bind_group_layouts: &[Some(&geometry_layout), Some(&lighting_layout)],
            immediate_size: 0,
        });
        let resolve_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("studio output"),
            bind_group_layouts: &[Some(&output_layout)],
            immediate_size: 0,
        });
        let make = |label: &str,
                    module: &wgpu::ShaderModule,
                    layout: &wgpu::PipelineLayout,
                    vs: &str,
                    fs: Option<&str>,
                    vertices: bool,
                    depth: bool,
                    format: wgpu::TextureFormat,
                    blend: Option<wgpu::BlendState>| {
            let attributes =
                wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x3,3=>Uint32];
            let buffers = [Some(wgpu::VertexBufferLayout {
                array_stride: 40,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            })];
            let targets = [Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: if vertices { &buffers } else { &[] },
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: depth.then_some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: fs.map(|name| wgpu::FragmentState {
                    module,
                    entry_point: Some(name),
                    compilation_options: Default::default(),
                    targets: &targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let hdr = wgpu::TextureFormat::Rgba16Float;
        let shadow_pipeline = make(
            "studio shadow",
            &shader,
            &scene_layout,
            "vs_shadow",
            None,
            true,
            true,
            hdr,
            None,
        );
        let depth_pipeline = make(
            "studio depth",
            &shader,
            &scene_layout,
            "vs_main",
            None,
            true,
            true,
            hdr,
            None,
        );
        let floor_depth_pipeline = make(
            "studio floor depth",
            &shader,
            &scene_layout,
            "vs_floor",
            None,
            false,
            true,
            hdr,
            None,
        );
        let surface_pipeline = make(
            "studio material",
            &shader,
            &lit_layout,
            "vs_main",
            Some("fs_main"),
            true,
            false,
            hdr,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        let floor_pipeline = make(
            "studio floor",
            &shader,
            &lit_layout,
            "vs_floor",
            Some("fs_main"),
            false,
            false,
            hdr,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        let background_pipeline = make(
            "studio background",
            &shader,
            &scene_layout,
            "vs_fullscreen",
            Some("fs_background"),
            false,
            false,
            hdr,
            None,
        );
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let accumulate_pipeline = make(
            "studio accumulation",
            &output_shader,
            &resolve_layout,
            "vs_fullscreen",
            Some("fs_accumulate"),
            false,
            false,
            hdr,
            Some(wgpu::BlendState {
                color: add,
                alpha: add,
            }),
        );
        let tonemap_pipeline = make(
            "studio output",
            &output_shader,
            &resolve_layout,
            "vs_fullscreen",
            Some("fs_tonemap"),
            false,
            false,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
        );
        let shadow = tex(
            &device,
            [quality.shadow_size; 2],
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        if let Some(e) = block_on(scope.pop()) {
            return Err(format!("studio pipeline validation: {e}").into());
        }
        Ok(Self {
            device,
            queue,
            info,
            quality,
            limits: limits.clone(),
            geometry_layout,
            lighting_layout,
            output_layout,
            shadow_pipeline,
            depth_pipeline,
            floor_depth_pipeline,
            surface_pipeline,
            floor_pipeline,
            background_pipeline,
            accumulate_pipeline,
            tonemap_pipeline,
            uniform,
            output_uniform,
            shadow,
            target: None,
            uploads: 0,
            in_flight: AtomicBool::new(false),
            unusable: AtomicBool::new(false),
            completion_timeout: Duration::from_secs(120),
            #[cfg(test)]
            after_sample_submit: None,
        })
    }
    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.info
    }
    pub fn quality(&self) -> StudioQuality {
        self.quality
    }
    pub fn upload_count(&self) -> u64 {
        self.uploads
    }
    pub fn upload(
        &mut self,
        geometry: &RealizedScene,
        materials: &MaterialSet,
        limits: &RenderLimits,
    ) -> Result<UploadedScene> {
        self.ensure_idle()?;
        let nt = geometry.mesh.triangles.len();
        let nv = geometry.mesh.positions.len();
        let bytes = (nt as u64)
            .checked_mul(120)
            .and_then(|v| v.checked_add(materials.materials.len() as u64 * 80))
            .ok_or("studio geometry overflow")?;
        let host_peak = bytes
            .checked_add(nv as u64 * 48)
            .and_then(|v| v.checked_add(nt as u64 * 16))
            .and_then(|v| {
                v.checked_add(
                    self.target
                        .as_ref()
                        .map_or(0, |t| u64::from(t.row) * u64::from(t.size[1])),
                )
            })
            .ok_or("studio host geometry overflow")?;
        if bytes > limits.gpu_bytes.min(self.limits.gpu_bytes)
            || host_peak > limits.host_bytes.min(self.limits.host_bytes)
            || nt > u32::MAX as usize / 3
            || nt as u64 * 120 > self.device.limits().max_buffer_size
            || materials.materials.is_empty()
            || materials.materials.len() > 259
            || geometry.normals.len() != nv
            || geometry.build_positions_mm.len() != nv
            || geometry.triangle_materials.len() != nt
        {
            return Err("studio geometry exceeds budget or has invalid attributes".into());
        }
        let mut vertices = Vec::new();
        vertices.try_reserve_exact(nt.checked_mul(3).ok_or("vertex expansion overflow")?)?;
        for (triangle, &material) in geometry
            .mesh
            .triangles
            .iter()
            .zip(&geometry.triangle_materials)
        {
            if material as usize >= materials.materials.len() {
                return Err("invalid studio material index".into());
            }
            for &i in triangle {
                let i = i as usize;
                if i >= nv {
                    return Err("invalid studio vertex index".into());
                }
                let vertex = Vertex {
                    position: geometry.mesh.positions[i].map(|v| v as f32),
                    normal: geometry.normals[i],
                    build: geometry.build_positions_mm[i],
                    material,
                };
                if vertex
                    .position
                    .iter()
                    .chain(&vertex.normal)
                    .chain(&vertex.build)
                    .any(|v| !v.is_finite())
                {
                    return Err("nonfinite studio geometry".into());
                }
                vertices.push(vertex);
            }
        }
        let materials: Vec<_> = materials
            .materials
            .iter()
            .map(|m| Material {
                base: [
                    m.base_color_linear[0],
                    m.base_color_linear[1],
                    m.base_color_linear[2],
                    m.roughness,
                ],
                surface: [m.metallic, m.anisotropy, m.coat, m.coat_roughness],
                volume: [
                    m.transmission,
                    m.ior,
                    m.absorption_density_per_mm,
                    m.scattering_per_mm,
                ],
                absorption: [
                    m.absorption_color_linear[0],
                    m.absorption_color_linear[1],
                    m.absorption_color_linear[2],
                    m.scattering_anisotropy,
                ],
                flags: [m.unlit as u8 as f32, 0., 0., 0.],
            })
            .collect();
        if bytemuck::cast_slice::<Material, f32>(&materials)
            .iter()
            .any(|v| !v.is_finite())
        {
            return Err("nonfinite studio material".into());
        }
        let vertices = buffer(
            &self.device,
            "studio immutable geometry",
            bytemuck::cast_slice(&vertices),
            wgpu::BufferUsages::VERTEX,
        );
        let materials = buffer(
            &self.device,
            "studio immutable materials",
            bytemuck::cast_slice(&materials),
            wgpu::BufferUsages::STORAGE,
        );
        self.uploads += 1;
        Ok(UploadedScene {
            vertices,
            materials,
            count: (nt * 3) as u32,
            bytes,
        })
    }
    /// Checked peak storage before allocation. Includes readback on both host/GPU,
    /// two HDR surfaces, depth/output, shadow map and caller's resident geometry.
    pub fn check_budget(
        &self,
        width: u32,
        height: u32,
        resident: u64,
        limits: &RenderLimits,
    ) -> Result<(u32, u64)> {
        if width == 0
            || height == 0
            || width > 8000
            || height > 8000
            || width > limits.max_output_side.min(self.limits.max_output_side)
            || height > limits.max_output_side.min(self.limits.max_output_side)
        {
            return Err("studio dimensions exceed output limits".into());
        }
        let row = width
            .checked_mul(4)
            .and_then(|v| v.checked_add(255))
            .map(|v| v / 256 * 256)
            .ok_or("readback stride overflow")?;
        let readback = u64::from(row) * u64::from(height);
        let pixels = u64::from(width) * u64::from(height);
        let surfaces =
            pixels * 24 + readback + u64::from(self.quality.shadow_size).pow(2) * 4 + 65536;
        let gpu = surfaces
            .checked_add(resident)
            .ok_or("studio GPU budget overflow")?;
        if gpu > limits.gpu_bytes.min(self.limits.gpu_bytes)
            || readback + pixels * 4 > limits.host_bytes.min(self.limits.host_bytes)
            || readback > self.device.limits().max_buffer_size
        {
            return Err("studio frame exceeds checked GPU/host budget".into());
        }
        Ok((row, gpu))
    }
    fn ensure_target(
        &mut self,
        width: u32,
        height: u32,
        resident: u64,
        limits: &RenderLimits,
    ) -> Result<()> {
        self.ensure_idle()?;
        let (row, _) = self.check_budget(width, height, resident, limits)?;
        if self
            .target
            .as_ref()
            .is_some_and(|t| t.size == [width, height])
        {
            return Ok(());
        }
        // Retire the previous output before allocating a new size.
        self.target = None;
        self.wait(&AtomicBool::new(false))?;
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let size = [width, height];
        let sample = tex(&self.device, size, wgpu::TextureFormat::Rgba16Float, usage);
        let accum = tex(&self.device, size, wgpu::TextureFormat::Rgba16Float, usage);
        let depth = tex(&self.device, size, wgpu::TextureFormat::Depth32Float, usage);
        let output = tex(
            &self.device,
            size,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("studio checked readback"),
            size: u64::from(row) * u64::from(height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.target = Some(Target {
            size,
            sample,
            accum,
            depth,
            output,
            readback,
            row,
        });
        Ok(())
    }
    /// Retire a differently sized previous target before allocating a new mesh.
    pub fn prepare_frame(
        &mut self,
        width: u32,
        height: u32,
        resident: u64,
        limits: &RenderLimits,
    ) -> Result<()> {
        self.ensure_target(width, height, resident, limits)
    }
    fn output_group(&self, input: &wgpu::TextureView) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("studio resolve inputs"),
            layout: &self.output_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(input),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.output_uniform.as_entire_binding(),
                },
            ],
        })
    }
    fn fullscreen(
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
        group: &wgpu::BindGroup,
        clear: bool,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("studio fullscreen"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
    // Explicit scene, frame and resource-budget inputs keep the offscreen renderer
    // independent of either the export actor or egui's device/texture ownership.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        scene: &RenderSceneSnapshot,
        frame: &FrameSpec,
        geometry: &UploadedScene,
        transparent: bool,
        resident: u64,
        limits: &RenderLimits,
        cancel: &AtomicBool,
    ) -> Result<(RgbaFrame, Duration, Duration)> {
        self.ensure_idle()?;
        let result = self.render_inner(
            scene,
            frame,
            geometry,
            transparent,
            resident,
            limits,
            cancel,
        );
        // Cancellation can arrive between AA submissions. Retire every submitted
        // allocation before the caller drops this job's upload or starts another.
        self.wait(&AtomicBool::new(false))?;
        result
    }
    #[allow(clippy::too_many_arguments)]
    fn render_inner(
        &mut self,
        scene: &RenderSceneSnapshot,
        frame: &FrameSpec,
        geometry: &UploadedScene,
        transparent: bool,
        resident: u64,
        limits: &RenderLimits,
        cancel: &AtomicBool,
    ) -> Result<(RgbaFrame, Duration, Duration)> {
        check(cancel)?;
        self.ensure_target(frame.width, frame.height, resident, limits)?;
        let mut uniforms = uniforms(scene, frame, transparent, self.quality)?;
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("studio scene inputs"),
            layout: &self.geometry_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: geometry.materials.as_entire_binding(),
                },
            ],
        });
        let target = self.target.as_ref().unwrap();
        let lighting = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("studio light inputs"),
            layout: &self.lighting_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.shadow.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&target.depth.view),
                },
            ],
        });
        let sample_group = self.output_group(&target.sample.view);
        let accum_group = self.output_group(&target.accum.view);
        let start = Instant::now();
        let floor = !transparent && scene.appearance.studio_preset != "diagram";
        self.queue.write_buffer(
            &self.output_uniform,
            0,
            bytemuck::cast_slice(&[1. / self.quality.samples as f32, 0., 0., 0.]),
        );
        for sample in 0..self.quality.samples {
            check(cancel)?;
            let jitter = jitter(sample, self.quality.samples);
            uniforms.sample[0] = jitter[0];
            uniforms.sample[1] = jitter[1];
            self.queue
                .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniforms));
            let mut encoder = self.device.create_command_encoder(&Default::default());
            if sample == 0 {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("studio shadow map"),
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.shadow.view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                pass.set_pipeline(&self.shadow_pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.set_vertex_buffer(0, geometry.vertices.slice(..));
                pass.draw(0..geometry.count, 0..1);
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("studio camera prepass"),
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &target.depth.view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                pass.set_bind_group(0, &group, &[]);
                if floor {
                    pass.set_pipeline(&self.floor_depth_pipeline);
                    pass.draw(0..6, 0..1);
                }
                pass.set_pipeline(&self.depth_pipeline);
                pass.set_vertex_buffer(0, geometry.vertices.slice(..));
                pass.draw(0..geometry.count, 0..1);
            }
            Self::fullscreen(
                &mut encoder,
                &target.sample.view,
                &self.background_pipeline,
                &group,
                true,
            );
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("studio PBR lighting"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.sample.view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_bind_group(0, &group, &[]);
                pass.set_bind_group(1, &lighting, &[]);
                if floor {
                    pass.set_pipeline(&self.floor_pipeline);
                    pass.draw(0..6, 0..1);
                }
                pass.set_pipeline(&self.surface_pipeline);
                pass.set_vertex_buffer(0, geometry.vertices.slice(..));
                pass.draw(0..geometry.count, 0..1);
            }
            Self::fullscreen(
                &mut encoder,
                &target.accum.view,
                &self.accumulate_pipeline,
                &sample_group,
                sample == 0,
            );
            self.submit(encoder.finish());
            #[cfg(test)]
            if let Some(hook) = &self.after_sample_submit {
                hook(&self.queue);
            }
        }
        let mut encoder = self.device.create_command_encoder(&Default::default());
        Self::fullscreen(
            &mut encoder,
            &target.output.view,
            &self.tonemap_pipeline,
            &accum_group,
            true,
        );
        self.submit(encoder.finish());
        // Complete queued render before measuring readback. Polling is bounded so
        // cancellation can suppress publication while submitted work drains safely.
        self.wait(cancel)?;
        let render_time = start.elapsed();
        let start = Instant::now();
        let image = self.readback(cancel)?;
        Ok((image, render_time, start.elapsed()))
    }
    fn submit(&self, command: wgpu::CommandBuffer) {
        self.in_flight.store(true, Ordering::Release);
        self.queue.submit([command]);
    }
    fn ensure_idle(&self) -> Result<()> {
        if self.unusable.load(Ordering::Acquire) {
            return Err("studio context is unusable after GPU retirement failure".into());
        }
        self.wait(&AtomicBool::new(false))
    }
    fn poll(&self) -> Result<()> {
        if let Err(error) = self.device.poll(wgpu::PollType::Poll) {
            self.unusable.store(true, Ordering::Release);
            return Err(error.into());
        }
        Ok(())
    }
    fn wait(&self, cancel: &AtomicBool) -> Result<()> {
        if self.unusable.load(Ordering::Acquire) {
            return Err("studio context is unusable after GPU retirement failure".into());
        }
        if !self.in_flight.load(Ordering::Acquire) {
            return check(cancel);
        }
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.queue.on_submitted_work_done(move || {
            let _ = tx.send(());
        });
        let start = Instant::now();
        loop {
            if start.elapsed() >= self.completion_timeout {
                self.unusable.store(true, Ordering::Release);
                return Err("GPU completion timeout; studio context is unusable".into());
            }
            self.poll()?;
            if rx.try_recv().is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        self.in_flight.store(false, Ordering::Release);
        check(cancel)
    }
    fn readback(&self, cancel: &AtomicBool) -> Result<RgbaFrame> {
        check(cancel)?;
        let target = self.target.as_ref().ok_or("studio target missing")?;
        // Allocate before mapping so allocation failure cannot leave a live mapping.
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(target.size[0] as usize * target.size[1] as usize * 4)?;
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            target.output.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &target.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(target.row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: target.size[0],
                height: target.size[1],
                depth_or_array_layers: 1,
            },
        );
        self.submit(encoder.finish());
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        target
            .readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
        let start = Instant::now();
        loop {
            self.poll()?;
            match rx.try_recv() {
                Ok(r) => {
                    r?;
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.unusable.store(true, Ordering::Release);
                    return Err("GPU map callback disconnected".into());
                }
                Err(_) => {
                    if start.elapsed() > Duration::from_secs(120) {
                        self.unusable.store(true, Ordering::Release);
                        target.readback.unmap();
                        return Err("GPU readback timeout".into());
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
        if cancel.load(Ordering::Acquire) {
            target.readback.unmap();
            return Err("cancelled".into());
        }
        let mapped = target.readback.slice(..).get_mapped_range()?;
        for row in mapped.chunks_exact(target.row as usize) {
            bytes.extend_from_slice(&row[..target.size[0] as usize * 4]);
        }
        drop(mapped);
        target.readback.unmap();
        for p in bytes.chunks_exact_mut(4) {
            if p[3] == 0 {
                p[..3].fill(0);
            }
        }
        Ok(RgbaFrame {
            width: target.size[0],
            height: target.size[1],
            bytes,
        })
    }
    /// Shared HDR output boundary, also usable by a caller supplying a linear
    /// radiance image. Exercises the same output pipeline as studio rendering.
    pub fn encode_hdr(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[[f32; 4]],
        recipe: AgxRecipe,
        cancel: &AtomicBool,
    ) -> Result<RgbaFrame> {
        check(cancel)?;
        if rgba.len() != width as usize * height as usize {
            return Err("HDR image length mismatch".into());
        }
        for p in rgba {
            recipe.map_linear_rgba(*p)?;
        }
        let limits = self.limits.clone();
        // Caller input, temporary premultiplied upload and host readback/output.
        // Account for the upload alongside the target, before allocating either.
        let (row, _) = self.check_budget(width, height, rgba.len() as u64 * 16, &limits)?;
        let host_peak = (rgba.len() as u64)
            .checked_mul(36)
            .and_then(|v| v.checked_add(u64::from(row) * u64::from(height)))
            .ok_or("HDR host budget overflow")?;
        if host_peak > limits.host_bytes {
            return Err("HDR upload exceeds checked host budget".into());
        }
        self.ensure_target(width, height, rgba.len() as u64 * 16, &limits)?;
        let image = tex(
            &self.device,
            [width, height],
            wgpu::TextureFormat::Rgba32Float,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let premult: Vec<[f32; 4]> = rgba
            .iter()
            .map(|p| [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]])
            .collect();
        self.queue.write_texture(
            image.texture.as_image_copy(),
            bytemuck::cast_slice(&premult),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 16),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.write_buffer(
            &self.output_uniform,
            0,
            bytemuck::cast_slice(&[1., recipe.exposure_ev, 0., 0.]),
        );
        let group = self.output_group(&image.view);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        Self::fullscreen(
            &mut encoder,
            &self.target.as_ref().unwrap().output.view,
            &self.tonemap_pipeline,
            &group,
            true,
        );
        self.submit(encoder.finish());
        let result = self.readback(cancel);
        self.wait(&AtomicBool::new(false))?;
        result
    }
}

fn unit(a: [f64; 3]) -> Result<[f64; 3]> {
    let length = a.iter().map(|v| v * v).sum::<f64>().sqrt();
    if !length.is_finite() || length < 1e-12 {
        return Err("invalid studio camera vector".into());
    }
    Ok(a.map(|v| v / length))
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn vec4(v: [f64; 3], w: f32) -> [f32; 4] {
    [v[0] as f32, v[1] as f32, v[2] as f32, w]
}
fn uniforms(
    scene: &RenderSceneSnapshot,
    frame: &FrameSpec,
    transparent: bool,
    quality: StudioQuality,
) -> Result<Uniforms> {
    let c = &frame.camera;
    let forward = unit(std::array::from_fn(|i| c.target_mm[i] - c.position_mm[i]))?;
    let right = unit(cross(forward, c.up))?;
    let up = unit(cross(right, forward))?;
    let (sy, ortho) = match c.projection {
        RenderProjection::Perspective {
            vertical_fov_degrees,
        } if (1.0..=150.).contains(&vertical_fov_degrees) => {
            ((vertical_fov_degrees.to_radians() / 2.).tan(), 0.)
        }
        RenderProjection::Orthographic { vertical_span_mm }
            if vertical_span_mm.is_finite() && vertical_span_mm > 0. =>
        {
            (vertical_span_mm / 2., 1.)
        }
        _ => return Err("invalid studio projection".into()),
    };
    let (near, far) = c.clip_planes_mm();
    let rows = resolve_display_transform(scene, frame.angle_degrees)?;
    let b = &scene.reference_bounds_mm;
    let mut top = 0f64;
    let mut radial = 0f64;
    for x in [b.min[0], b.max[0]] {
        for y in [b.min[1], b.max[1]] {
            for z in [b.min[2], b.max[2]] {
                let p = [x, y, z];
                let q: [f64; 3] = std::array::from_fn(|i| {
                    rows[i][3] + (0..3).map(|j| rows[i][j] * p[j]).sum::<f64>()
                });
                top = top.max(q[2]);
                radial = radial.max(q[0].hypot(q[1]));
            }
        }
    }
    let extent = (radial * 2.).max(top).max(0.001);
    let a = &scene.appearance;
    let light_back = unit([-2.5, -3., 4.])?;
    let light_right = unit(cross([0., 0., 1.], light_back))?;
    let light_up = cross(light_back, light_right);
    let studio = match a.studio_preset.as_str() {
        "light" => 0.,
        "dark" => 1.,
        "diagram" => 2.,
        _ => return Err("unknown studio preset".into()),
    };
    let u = Uniforms {
        right: vec4(right, 0.),
        up: vec4(up, 0.),
        forward: vec4(forward, 0.),
        eye: vec4(c.position_mm, 0.),
        projection: [
            (sy * f64::from(frame.width) / f64::from(frame.height)) as f32,
            sy as f32,
            ortho,
            0.,
        ],
        clipping: [near as f32, far as f32, 0., 0.],
        rows: std::array::from_fn(|i| rows[i].map(|v| v as f32)),
        light_right: vec4(light_right, 0.),
        light_up: vec4(light_up, 0.),
        light_back: vec4(light_back, 0.),
        center: [0., 0., top as f32 * 0.5, extent as f32],
        screen: [
            frame.width as f32,
            frame.height as f32,
            transparent as u8 as f32,
            studio,
        ],
        layers: [
            a.layer_height_mm as f32,
            a.first_layer_height_mm as f32,
            a.groove_depth_mm as f32,
            a.layer_appearance as u8 as f32,
        ],
        build: [
            b.min[2] as f32,
            0.,
            scene.legacy_scene["layer_filtering"]
                .as_bool()
                .unwrap_or(true) as u8 as f32,
            quality.ao_samples as f32,
        ],
        sample: [
            0.,
            0.,
            1. / quality.samples as f32,
            quality.shadow_radius as f32,
        ],
        quality: [quality.shadow_size as f32, 0., 0., 0.],
    };
    if bytemuck::cast_slice::<Uniforms, f32>(&[u])
        .iter()
        .any(|v| !v.is_finite())
    {
        return Err("nonfinite studio uniforms".into());
    }
    Ok(u)
}
fn jitter(index: u32, samples: u32) -> [f32; 2] {
    const POINTS: [[f32; 2]; 8] = [
        [-0.375, -0.125],
        [0.125, -0.375],
        [0.375, 0.125],
        [-0.125, 0.375],
        [-0.375, 0.375],
        [0.375, -0.375],
        [-0.125, -0.125],
        [0.125, 0.125],
    ];
    if samples == 1 {
        [0., 0.]
    } else {
        POINTS[index as usize]
    }
}

#[cfg(test)]
mod lifecycle_regressions {
    use super::*;
    use crate::render::{
        backends::wgpu::WgpuConfig, geometry::realize_frame, materials::resolve_materials,
    };
    use std::sync::atomic::AtomicUsize;

    #[test]
    #[ignore = "exclusive GPU; deterministic cancellation after an actual AA submission"]
    fn active_aa_cancellation_retires_before_same_size_reuse() {
        let config = WgpuConfig::default();
        let mut studio = WgpuStudio::new(None, &config.quality, &config.limits).unwrap();
        let source = Arc::new(crate::mesh::Mesh {
            positions: vec![[0.; 3], [10., 0., 0.], [0., 0., 10.]],
            triangles: vec![[0, 1, 2]],
        });
        let scene=snapshot_scene(source,serde_json::json!({"schema_version":1,"mesh_path":"D:/fixture/mesh.bin","color":[0.2,0.4,0.6],"roughness":0.45,"geometry_kind":"original","layer_height_mm":0.2,"display_rotation_degrees":[0.,0.,0.],"warnings":[]})).unwrap();
        let frame = FrameSpec {
            key: FrameKey {
                index: 0,
                panel: RenderPanel::Single,
            },
            camera: RenderCamera {
                position_mm: [0., -40., 5.],
                target_mm: [0., 0., 5.],
                up: [0., 0., 1.],
                projection: RenderProjection::Orthographic {
                    vertical_span_mm: 20.,
                },
            },
            angle_degrees: 0.,
            visible_layers: None,
            section: None,
            width: 256,
            height: 256,
            time_seconds: 0.,
        };
        let geometry = realize_frame(&scene, &frame).unwrap();
        let materials = resolve_materials(&scene).unwrap();
        let upload = studio
            .upload(&geometry, &materials, &config.limits)
            .unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let completed = Arc::new(AtomicBool::new(false));
        let submissions = Arc::new(AtomicUsize::new(0));
        let token = cancel.clone();
        let retired = completed.clone();
        let count = submissions.clone();
        studio.after_sample_submit = Some(Box::new(move |queue| {
            count.fetch_add(1, Ordering::AcqRel);
            let retired = retired.clone();
            queue.on_submitted_work_done(move || retired.store(true, Ordering::Release));
            token.store(true, Ordering::Release);
        }));
        let result = studio.render(
            &scene,
            &frame,
            &upload,
            true,
            upload.bytes,
            &config.limits,
            &cancel,
        );
        assert!(result.unwrap_err().to_string().contains("cancelled"));
        println!(
            "actual AA submissions={}, completion observed at cancellation return={}",
            submissions.load(Ordering::Acquire),
            completed.load(Ordering::Acquire)
        );
        assert_eq!(submissions.load(Ordering::Acquire), 1);
        assert!(
            completed.load(Ordering::Acquire),
            "submitted geometry must retire before cancellation returns and same-size reuse allocates"
        );
        drop(upload);
        studio.after_sample_submit = None;
        cancel.store(false, Ordering::Release);
        let fresh = studio
            .upload(&geometry, &materials, &config.limits)
            .unwrap();
        let (image, _, _) = studio
            .render(
                &scene,
                &frame,
                &fresh,
                true,
                fresh.bytes,
                &config.limits,
                &cancel,
            )
            .unwrap();
        assert_eq!(image.bytes[(160 * 256 + 96) * 4 + 3], 255);
        assert!(!studio.in_flight.load(Ordering::Acquire));
        assert_eq!(studio.completion_timeout, Duration::from_secs(120));
        // Exercise the exact timeout branch without waiting two minutes or
        // manufacturing a device loss. Even an empty real submission is tracked.
        studio.completion_timeout = Duration::ZERO;
        let encoder = studio.device.create_command_encoder(&Default::default());
        studio.submit(encoder.finish());
        assert!(
            studio
                .wait(&cancel)
                .unwrap_err()
                .to_string()
                .contains("timeout")
        );
        assert!(studio.unusable.load(Ordering::Acquire));
        assert!(studio.in_flight.load(Ordering::Acquire));
        let uploads = studio.upload_count();
        assert!(
            studio
                .upload(&geometry, &materials, &config.limits)
                .is_err()
        );
        assert!(
            studio
                .prepare_frame(256, 256, fresh.bytes, &config.limits)
                .is_err()
        );
        assert!(
            studio
                .render(
                    &scene,
                    &frame,
                    &fresh,
                    true,
                    fresh.bytes,
                    &config.limits,
                    &cancel
                )
                .is_err()
        );
        assert!(
            studio
                .encode_hdr(1, 1, &[[0.18, 0.18, 0.18, 1.]], Default::default(), &cancel)
                .is_err()
        );
        assert_eq!(
            studio.upload_count(),
            uploads,
            "failed retirement must forbid fresh GPU allocations"
        );
        println!(
            "same-size reuse succeeded after drain; forced retirement timeout forbids upload/target/render/HDR reuse"
        );
    }
}
