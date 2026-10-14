use super::{OrbitCamera, camera::rotation_rows};
use crate::{
    Result,
    mesh::{Bounds, Mesh},
};
use eframe::{
    egui,
    egui_wgpu::{RenderState, wgpu},
};
use wgpu::util::DeviceExt;

#[derive(Clone, Debug)]
pub struct ViewportSettings {
    pub color: [f32; 3],
    pub roughness: f32,
    pub layer_appearance: bool,
    pub layer_height_mm: f32,
    pub first_layer_height_mm: f32,
    pub groove_depth_mm: f32,
    pub display_rotation_degrees: [f32; 3],
    pub turntable_angle_degrees: f32,
    pub show_edges: bool,
}
impl Default for ViewportSettings {
    fn default() -> Self {
        Self {
            color: [0.55, 0.65, 0.8],
            roughness: 0.65,
            layer_appearance: false,
            layer_height_mm: 0.2,
            first_layer_height_mm: 0.2,
            groove_depth_mm: 0.02,
            display_rotation_degrees: [0.; 3],
            turntable_angle_degrees: 0.,
            show_edges: false,
        }
    }
}
impl ViewportSettings {
    pub fn validate(&self) -> Result<()> {
        if self
            .color
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || !self.roughness.is_finite()
            || !(0.0..=1.0).contains(&self.roughness)
            || !self.layer_height_mm.is_finite()
            || !(0.02..=2.0).contains(&self.layer_height_mm)
            || !self.first_layer_height_mm.is_finite()
            || !(0.02..=2.0).contains(&self.first_layer_height_mm)
            || !self.groove_depth_mm.is_finite()
            || self.groove_depth_mm < 0.
            || self.groove_depth_mm > 0.25 * self.layer_height_mm.min(self.first_layer_height_mm)
            || self.display_rotation_degrees.iter().any(|v| !v.is_finite())
            || !self.turntable_angle_degrees.is_finite()
        {
            return Err("invalid viewport settings".into());
        }
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    right: [f32; 4],
    up: [f32; 4],
    back: [f32; 4],
    target: [f32; 4],
    projection: [f32; 4],
    clipping: [f32; 4],
    row0: [f32; 4],
    row1: [f32; 4],
    row2: [f32; 4],
    shift_spin: [f32; 4],
    color_roughness: [f32; 4],
    layers: [f32; 4],
}
struct Geometry {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    edges: wgpu::Buffer,
    index_count: u32,
    edge_count: u32,
}
struct Target {
    _color: wgpu::Texture,
    color_view: wgpu::TextureView,
    _depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    size: [u32; 2],
    id: egui::TextureId,
}
pub struct Viewport {
    state: RenderState,
    pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    geometry: Option<Geometry>,
    target: Option<Target>,
    positions: Vec<[f32; 3]>,
    revision: Option<u64>,
    geometry_kind: String,
    source_min_z: f32,
    display_pose: Option<[f32; 3]>,
    shift: [f32; 3],
    bounds: Option<Bounds>,
}
impl Viewport {
    pub fn new(state: &RenderState) -> Self {
        let device = &state.device;
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport parameters"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport parameters"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport parameters"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("physical viewport"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/viewport.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make_pipeline = |edges: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(if edges {
                    "viewport triangle edges"
                } else {
                    "viewport solid"
                }),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: 12,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: if edges {
                        wgpu::PrimitiveTopology::LineList
                    } else {
                        wgpu::PrimitiveTopology::TriangleList
                    },
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(!edges),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState {
                        constant: if edges { 0 } else { 1 },
                        ..Default::default()
                    },
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(if edges { "fs_edge" } else { "fs_main" }),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            state: state.clone(),
            pipeline: make_pipeline(false),
            edge_pipeline: make_pipeline(true),
            uniform,
            bind_group,
            geometry: None,
            target: None,
            positions: Vec::new(),
            revision: None,
            geometry_kind: String::new(),
            source_min_z: 0.,
            display_pose: None,
            shift: [0.; 3],
            bounds: None,
        }
    }
    pub fn set_mesh(&mut self, mesh: &Mesh, geometry_kind: &str, mesh_revision: u64) -> Result<()> {
        if self.revision == Some(mesh_revision) && self.geometry_kind == geometry_kind {
            return Ok(());
        }
        let bounds = mesh.bounds()?;
        if !["original", "beads", "stepped"].contains(&geometry_kind) {
            return Err("unknown viewport geometry kind".into());
        }
        let positions: Vec<[f32; 3]> = mesh.positions.iter().map(|p| p.map(|v| v as f32)).collect();
        if positions.iter().flatten().any(|v| !v.is_finite()) {
            return Err("mesh exceeds viewport precision range".into());
        }
        for triangle in &mesh.triangles {
            let [a, b, c] = triangle.map(|i| positions[i as usize].map(f64::from));
            let ab: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
            let ac: [f64; 3] = std::array::from_fn(|i| c[i] - a[i]);
            let normal = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            if normal == [0.; 3] {
                return Err("mesh triangle collapses at GPU f32 precision".into());
            }
        }
        let indices: Vec<u32> = mesh.triangles.iter().flatten().copied().collect();
        let edges: Vec<u32> = mesh
            .triangles
            .iter()
            .flat_map(|t| [t[0], t[1], t[1], t[2], t[2], t[0]])
            .collect();
        let max = self.state.device.limits().max_buffer_size;
        if positions.len() as u64 * 12 > max
            || edges.len() as u64 * 4 > max
            || edges.len() > u32::MAX as usize
        {
            return Err("mesh exceeds GPU buffer budget".into());
        }
        let upload = |label, bytes: &[u8], usage| {
            self.state
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytes,
                    usage,
                })
        };
        self.geometry = Some(Geometry {
            vertices: upload(
                "viewport positions",
                bytemuck::cast_slice(&positions),
                wgpu::BufferUsages::VERTEX,
            ),
            indices: upload(
                "viewport triangles",
                bytemuck::cast_slice(&indices),
                wgpu::BufferUsages::INDEX,
            ),
            edges: upload(
                "viewport triangle edges",
                bytemuck::cast_slice(&edges),
                wgpu::BufferUsages::INDEX,
            ),
            index_count: indices.len() as u32,
            edge_count: edges.len() as u32,
        });
        self.positions = positions;
        self.revision = Some(mesh_revision);
        self.geometry_kind = geometry_kind.to_owned();
        self.source_min_z = bounds.min[2] as f32;
        self.display_pose = None;
        self.bounds = None;
        Ok(())
    }
    pub fn clear_mesh(&mut self) {
        self.geometry = None;
        self.positions = Vec::new();
        self.revision = None;
        self.bounds = None;
        self.display_pose = None;
    }
    pub fn triangle_count(&self) -> usize {
        self.geometry
            .as_ref()
            .map_or(0, |g| g.index_count as usize / 3)
    }
    pub fn texture_size(&self) -> Option<[u32; 2]> {
        self.target.as_ref().map(|t| t.size)
    }
    pub fn display_bounds(&mut self, settings: &ViewportSettings) -> Option<Bounds> {
        if settings.validate().is_err() || self.positions.is_empty() {
            return None;
        }
        if self.display_pose != Some(settings.display_rotation_degrees) {
            let rows = rotation_rows(settings.display_rotation_degrees);
            let mut min = [f64::INFINITY; 3];
            let mut max = [f64::NEG_INFINITY; 3];
            for p in &self.positions {
                for i in 0..3 {
                    let value = (0..3).map(|j| rows[i][j] as f64 * p[j] as f64).sum::<f64>();
                    min[i] = min[i].min(value);
                    max[i] = max[i].max(value);
                }
            }
            self.shift = [
                ((min[0] + max[0]) * 0.5) as f32,
                ((min[1] + max[1]) * 0.5) as f32,
                min[2] as f32,
            ];
            self.bounds = Some(Bounds {
                min: std::array::from_fn(|i| min[i] - self.shift[i] as f64),
                max: std::array::from_fn(|i| max[i] - self.shift[i] as f64),
            });
            self.display_pose = Some(settings.display_rotation_degrees);
        }
        self.bounds
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        camera: &mut OrbitCamera,
        settings: &ViewportSettings,
    ) -> egui::Response {
        let size = ui.available_size().max(egui::vec2(1., 1.));
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::drag());
        let delta = response.drag_delta();
        if response.dragged() {
            if response.dragged_by(egui::PointerButton::Secondary)
                || response.dragged_by(egui::PointerButton::Middle)
                || ui.input(|i| i.modifiers.shift)
            {
                camera.pan(delta.x as f64, delta.y as f64, rect.height() as f64);
            } else {
                camera.orbit(delta.x as f64, delta.y as f64);
            }
            ui.ctx().request_repaint();
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0. {
                camera.zoom(scroll as f64);
                ui.ctx().request_repaint();
            }
        }
        if camera.validate().is_err() {
            *camera = OrbitCamera::default();
        }
        if let Err(error) = settings.validate() {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                error.to_string(),
                egui::FontId::proportional(16.),
                egui::Color32::LIGHT_RED,
            );
            return response;
        }
        self.display_bounds(settings);
        let limit = self
            .state
            .device
            .limits()
            .max_texture_dimension_2d
            .min(8192);
        let pixels = (rect.size() * ui.ctx().pixels_per_point()).ceil();
        let dimensions = [
            (pixels.x as u32).clamp(1, limit),
            (pixels.y as u32).clamp(1, limit),
        ];
        self.resize(dimensions);
        self.render(camera, settings);
        if let Some(target) = &self.target {
            ui.painter().image(
                target.id,
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                egui::Color32::WHITE,
            );
        }
        response
    }
    fn resize(&mut self, size: [u32; 2]) {
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let texture = |label, format, usage| {
            self.state.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
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
            })
        };
        let color = texture(
            "viewport color",
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = texture(
            "viewport depth",
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let color_view = color.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        let mut renderer = self.state.renderer.write();
        let id = if let Some(old) = &self.target {
            renderer.update_egui_texture_from_wgpu_texture(
                &self.state.device,
                &color_view,
                wgpu::FilterMode::Linear,
                old.id,
            );
            old.id
        } else {
            renderer.register_native_texture(
                &self.state.device,
                &color_view,
                wgpu::FilterMode::Linear,
            )
        };
        self.target = Some(Target {
            _color: color,
            color_view,
            _depth: depth,
            depth_view,
            size,
            id,
        });
    }
    fn render(&self, camera: &OrbitCamera, settings: &ViewportSettings) {
        let Some(target) = &self.target else {
            return;
        };
        let vec4 = |v: [f64; 3], w: f32| [v[0] as f32, v[1] as f32, v[2] as f32, w];
        let (right, up, back) = camera.basis();
        let rows = rotation_rows(settings.display_rotation_degrees);
        let parameters = Uniforms {
            right: vec4(right, 0.),
            up: vec4(up, 0.),
            back: vec4(back, 0.),
            target: vec4(camera.target_mm, camera.distance_mm as f32),
            projection: [
                (camera.vertical_fov_degrees * 0.5).to_radians().tan() as f32,
                camera.ortho_span_mm as f32,
                target.size[0] as f32 / target.size[1] as f32,
                u32::from(camera.orthographic) as f32,
            ],
            clipping: [
                (camera.distance_mm * 0.0001).max(1e-5) as f32,
                (camera.distance_mm * 100. + 100.) as f32,
                self.source_min_z,
                0.,
            ],
            row0: [rows[0][0], rows[0][1], rows[0][2], 0.],
            row1: [rows[1][0], rows[1][1], rows[1][2], 0.],
            row2: [rows[2][0], rows[2][1], rows[2][2], 0.],
            shift_spin: [
                self.shift[0],
                self.shift[1],
                self.shift[2],
                settings.turntable_angle_degrees.to_radians(),
            ],
            color_roughness: [
                settings.color[0],
                settings.color[1],
                settings.color[2],
                settings.roughness,
            ],
            layers: [
                settings.layer_height_mm,
                settings.first_layer_height_mm,
                settings.groove_depth_mm,
                u32::from(settings.layer_appearance && self.geometry_kind == "original") as f32,
            ],
        };
        self.state
            .queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&parameters));
        let mut encoder =
            self.state
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("native viewport"),
                });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("native viewport"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.color_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.095,
                            g: 0.11,
                            b: 0.14,
                            a: 1.,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(geometry) = &self.geometry {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, geometry.vertices.slice(..));
                pass.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..geometry.index_count, 0, 0..1);
                if settings.show_edges {
                    pass.set_pipeline(&self.edge_pipeline);
                    pass.set_index_buffer(geometry.edges.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..geometry.edge_count, 0, 0..1);
                }
            }
        }
        self.state.queue.submit([encoder.finish()]);
    }
}
impl Drop for Viewport {
    fn drop(&mut self) {
        if let Some(target) = &self.target {
            self.state.renderer.write().free_texture(&target.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        future::Future,
        sync::Arc,
        task::{Context, Poll, Wake, Waker},
    };

    fn block_on<T>(future: impl Future<Output = T>) -> T {
        struct ThreadWake(std::thread::Thread);
        impl Wake for ThreadWake {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::park_timeout(std::time::Duration::from_millis(100)),
            }
        }
    }
    fn capture(viewport: &Viewport) -> Vec<u8> {
        let target = viewport.target.as_ref().unwrap();
        let row = (target.size[0] * 4).div_ceil(256) * 256;
        let buffer = viewport
            .state
            .device
            .create_buffer(&wgpu::BufferDescriptor {
                label: Some("viewport test readback"),
                size: row as u64 * target.size[1] as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
        let mut encoder = viewport
            .state
            .device
            .create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            target._color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: target.size[0],
                height: target.size[1],
                depth_or_array_layers: 1,
            },
        );
        viewport.state.queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                sender.send(result).unwrap()
            });
        viewport
            .state
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        receiver.recv().unwrap().unwrap();
        let mapped = buffer.slice(..).get_mapped_range().unwrap();
        let result = mapped
            .chunks(row as usize)
            .flat_map(|r| r[..target.size[0] as usize * 4].iter().copied())
            .collect();
        drop(mapped);
        buffer.unmap();
        result
    }
    #[test]
    #[ignore = "requires an actual GPU adapter; run explicitly for viewport validation"]
    fn gpu_render_layers_resize_and_unload() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let state = block_on(RenderState::create(
            &Default::default(),
            &instance,
            None,
            Default::default(),
        ))
        .unwrap();
        eprintln!(
            "viewport validation adapter: {:?}",
            state.adapter.get_info()
        );
        let scope = state.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut viewport = Viewport::new(&state);
        let error = block_on(scope.pop());
        assert!(error.is_none(), "GPU pipeline validation: {error:?}");
        let scope = state.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mesh = Mesh {
            positions: vec![
                [-10., 0., 0.],
                [10., 0., 0.],
                [10., 0., 20.],
                [-10., 0., 20.],
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
        };
        viewport.set_mesh(&mesh, "original", 1).unwrap();
        let mut settings = ViewportSettings::default();
        let bounds = viewport.display_bounds(&settings).unwrap();
        let mut camera = OrbitCamera::default();
        camera.canonical(super::super::CanonicalView::Front);
        camera.fit(&bounds);
        camera.orthographic = true;
        viewport.resize([512, 512]);
        viewport.render(&camera, &settings);
        let plain = capture(&viewport);
        let error = block_on(scope.pop());
        assert!(error.is_none(), "GPU render validation: {error:?}");
        let scope = state.device.push_error_scope(wgpu::ErrorFilter::Validation);
        assert!(
            plain.chunks_exact(4).filter(|pixel| pixel[0] > 50).count() > 10_000,
            "GPU output has no model"
        );
        settings.layer_appearance = true;
        viewport.render(&camera, &settings);
        let layered = capture(&viewport);
        assert!(
            plain.iter().zip(&layered).filter(|(a, b)| a != b).count() > 10_000,
            "resolved physical layer shading has no effect"
        );
        // Subpixel layers must converge to the original shading instead of moire.
        settings.layer_height_mm = 0.02;
        settings.first_layer_height_mm = 0.02;
        settings.groove_depth_mm = 0.001;
        viewport.render(&camera, &settings);
        assert_eq!(plain, capture(&viewport));
        // The first interval is independent of the subsequent pitch.
        settings.layer_height_mm = 0.2;
        settings.first_layer_height_mm = 0.3;
        settings.groove_depth_mm = 0.02;
        viewport.render(&camera, &settings);
        assert_ne!(layered, capture(&viewport));
        // A rotated model keeps its layer axis in build coordinates.
        settings.display_rotation_degrees = [90., 0., 0.];
        camera.canonical(super::super::CanonicalView::Top);
        camera.fit(&viewport.display_bounds(&settings).unwrap());
        settings.layer_appearance = false;
        viewport.render(&camera, &settings);
        let rotated_plain = capture(&viewport);
        settings.layer_appearance = true;
        viewport.render(&camera, &settings);
        assert_ne!(rotated_plain, capture(&viewport));
        // Later triangles behind the front surface must fail the depth test.
        let mut occlusion = mesh.clone();
        occlusion.positions.extend([
            [-10., 3., 0.],
            [10., 7., 0.],
            [10., 7., 20.],
            [-10., 3., 20.],
        ]);
        occlusion.triangles.extend([[4, 5, 6], [4, 6, 7]]);
        settings = ViewportSettings::default();
        camera.canonical(super::super::CanonicalView::Front);
        camera.fit(&bounds);
        viewport.set_mesh(&occlusion, "original", 2).unwrap();
        viewport.display_bounds(&settings);
        viewport.render(&camera, &settings);
        assert_eq!(plain, capture(&viewport));
        // Horizontal caps have no periodic shading, including reversed faces.
        let cap = Mesh {
            positions: vec![
                [-10., -10., 0.],
                [10., -10., 0.],
                [10., 10., 0.],
                [-10., 10., 0.],
            ],
            triangles: vec![[0, 2, 1], [0, 3, 2]],
        };
        viewport.set_mesh(&cap, "original", 3).unwrap();
        camera.canonical(super::super::CanonicalView::Top);
        camera.fit(&viewport.display_bounds(&settings).unwrap());
        viewport.render(&camera, &settings);
        let cap_plain = capture(&viewport);
        settings.layer_appearance = true;
        viewport.render(&camera, &settings);
        assert_eq!(cap_plain, capture(&viewport));
        viewport.resize([320, 192]);
        settings.show_edges = true;
        viewport.render(&camera, &settings);
        assert_eq!(viewport.texture_size(), Some([320, 192]));
        assert_eq!(viewport.triangle_count(), 2);
        let collapsed = Mesh {
            positions: vec![[1e8, 0., 0.], [1e8 + 1., 0., 0.], [1e8, 1., 0.]],
            triangles: vec![[0, 1, 2]],
        };
        assert!(
            viewport.set_mesh(&collapsed, "original", 4).is_err(),
            "precision collapse must preserve the previous mesh and return an error"
        );
        assert_eq!(viewport.triangle_count(), 2);
        viewport.clear_mesh();
        viewport.render(&camera, &settings);
        let empty = capture(&viewport);
        assert!(empty.chunks_exact(4).all(|p| p == &empty[..4]));
        assert_eq!(viewport.triangle_count(), 0);
        assert!(
            block_on(scope.pop()).is_none(),
            "GPU validation reported an error"
        );
    }
}
