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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InspectionMode {
    #[default]
    Opaque,
    Ghost,
    Xray,
}
#[derive(Clone, Copy, Debug)]
pub struct SectionPlane {
    pub axis: u8,
    pub position_mm: f32,
    pub keep_positive: bool,
}

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
    pub section: Option<SectionPlane>,
    pub buildup_height_mm: Option<f32>,
    pub buildup_complete_layers: bool,
    pub inspection_mode: InspectionMode,
    pub opacity: f32,
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
            section: None,
            buildup_height_mm: None,
            buildup_complete_layers: false,
            inspection_mode: InspectionMode::Opaque,
            opacity: 0.25,
        }
    }
}
impl ViewportSettings {
    pub(crate) fn effective_buildup_height(&self) -> Option<f32> {
        self.buildup_height_mm.map(|height| {
            if !self.buildup_complete_layers {
                height
            } else if height + 1e-6 < self.first_layer_height_mm {
                0.
            } else {
                self.first_layer_height_mm
                    + ((height - self.first_layer_height_mm + 1e-6) / self.layer_height_mm).floor()
                        * self.layer_height_mm
            }
        })
    }
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
            || self.section.is_some_and(|p| {
                p.axis > 2 || !p.position_mm.is_finite() || p.position_mm.abs() > 1e7
            })
            || self
                .buildup_height_mm
                .is_some_and(|h| !h.is_finite() || !(0.0..=1e7).contains(&h))
            || !self.opacity.is_finite()
            || !(0.0..=1.0).contains(&self.opacity)
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
    section: [f32; 4],
    inspection: [f32; 4],
}
struct Geometry {
    vertices: wgpu::Buffer,
    colors: wgpu::Buffer,
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
    oit: Option<OitTarget>,
}
struct OitTarget {
    _accumulation: wgpu::Texture,
    accumulation: wgpu::TextureView,
    _revealage: wgpu::Texture,
    revealage: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}
pub struct Viewport {
    state: RenderState,
    pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    oit_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    geometry: Option<Geometry>,
    target: Option<Target>,
    positions: Vec<[f32; 3]>,
    source_triangles: Vec<[u32; 3]>,
    face_colors: Vec<[f32; 3]>,
    face_layer_tops: Vec<f32>,
    has_face_colors: bool,
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
        let additive = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let reveal = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::OneMinusSrc,
            operation: wgpu::BlendOperation::Add,
        };
        let opaque_targets = [Some(wgpu::ColorTargetState {
            format: wgpu::TextureFormat::Rgba8Unorm,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let oit_targets = [
            Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba16Float,
                blend: Some(wgpu::BlendState {
                    color: additive,
                    alpha: additive,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            }),
            Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::R16Float,
                blend: Some(wgpu::BlendState {
                    color: reveal,
                    alpha: reveal,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            }),
        ];
        let make_pipeline = |edges: bool, transparent: bool| {
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
                    buffers: &[
                        Some(wgpu::VertexBufferLayout {
                            array_stride: 12,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                        }),
                        Some(wgpu::VertexBufferLayout {
                            array_stride: 16,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &wgpu::vertex_attr_array![1=>Float32x4],
                        }),
                    ],
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
                    depth_write_enabled: Some(!edges && !transparent),
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
                    entry_point: Some(if transparent {
                        "fs_oit"
                    } else if edges {
                        "fs_edge"
                    } else {
                        "fs_main"
                    }),
                    compilation_options: Default::default(),
                    targets: if transparent {
                        &oit_targets
                    } else {
                        &opaque_targets
                    },
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OIT composite textures"),
            entries: &[0, 1].map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }),
        });
        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("weighted OIT composite"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/composite.wgsl").into()),
        });
        let composite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("OIT composite layout"),
                bind_group_layouts: &[Some(&composite_layout)],
                immediate_size: 0,
            });
        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("OIT composite"),
            layout: Some(&composite_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &composite_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &composite_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &opaque_targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            state: state.clone(),
            pipeline: make_pipeline(false, false),
            edge_pipeline: make_pipeline(true, false),
            oit_pipeline: make_pipeline(false, true),
            composite_pipeline,
            composite_layout,
            uniform,
            bind_group,
            geometry: None,
            target: None,
            positions: Vec::new(),
            source_triangles: Vec::new(),
            face_colors: Vec::new(),
            face_layer_tops: Vec::new(),
            has_face_colors: false,
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
        self.geometry = Some(self.upload_geometry(
            &positions,
            &mesh.triangles,
            &vec![[1., 1., 1., -1.]; positions.len()],
        )?);
        self.positions = positions;
        self.source_triangles = mesh.triangles.clone();
        self.face_colors.clear();
        self.face_layer_tops.clear();
        self.has_face_colors = false;
        self.revision = Some(mesh_revision);
        self.geometry_kind = geometry_kind.to_owned();
        self.source_min_z = bounds.min[2] as f32;
        self.display_pose = None;
        self.bounds = None;
        Ok(())
    }
    fn upload_geometry(
        &self,
        positions: &[[f32; 3]],
        triangles: &[[u32; 3]],
        colors: &[[f32; 4]],
    ) -> Result<Geometry> {
        let indices: Vec<u32> = triangles.iter().flatten().copied().collect();
        let edges: Vec<u32> = triangles
            .iter()
            .flat_map(|t| [t[0], t[1], t[1], t[2], t[2], t[0]])
            .collect();
        let max = self.state.device.limits().max_buffer_size;
        if positions.len() as u64 * 16 > max
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
        Ok(Geometry {
            vertices: upload(
                "viewport positions",
                bytemuck::cast_slice(positions),
                wgpu::BufferUsages::VERTEX,
            ),
            colors: upload(
                "viewport face colors and layer tops",
                bytemuck::cast_slice(colors),
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
        })
    }
    /// Sets linear RGB colors by original triangle order. Empty input restores
    /// the scene color. Call only when palette/grouping changes, not each frame.
    pub fn set_triangle_colors(&mut self, colors: &[[f32; 3]]) -> Result<()> {
        if self.geometry.is_none() {
            return Err("load a mesh before assigning colors".into());
        }
        if !colors.is_empty()
            && (colors.len() != self.source_triangles.len()
                || colors
                    .iter()
                    .flatten()
                    .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)))
        {
            return Err(
                "face colors must match triangle count and be finite linear RGB in 0..1".into(),
            );
        }
        let geometry = self.face_geometry(colors, &self.face_layer_tops)?;
        self.geometry = Some(geometry);
        self.face_colors = colors.to_vec();
        self.has_face_colors = !colors.is_empty();
        Ok(())
    }
    /// Assigns the actual deposited layer top to each original triangle.
    /// Build-up then hides whole future-layer faces, including their bottom caps.
    /// Empty input restores build-Z plane clipping. Palette metadata is retained.
    pub fn set_triangle_layer_tops(&mut self, tops: &[f32]) -> Result<()> {
        if self.geometry.is_none() {
            return Err("load a mesh before assigning layer tops".into());
        }
        if !tops.is_empty()
            && (tops.len() != self.source_triangles.len()
                || tops
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=1e7).contains(v)))
        {
            return Err(
                "layer tops must match triangle count and be finite build heights in 0..1e7 mm"
                    .into(),
            );
        }
        let geometry = self.face_geometry(&self.face_colors, tops)?;
        self.geometry = Some(geometry);
        self.face_layer_tops = tops.to_vec();
        Ok(())
    }
    fn face_geometry(&self, colors: &[[f32; 3]], tops: &[f32]) -> Result<Geometry> {
        if colors.is_empty() && tops.is_empty() {
            return self.upload_geometry(
                &self.positions,
                &self.source_triangles,
                &vec![[1., 1., 1., -1.]; self.positions.len()],
            );
        }
        let mut remap = std::collections::HashMap::new();
        let mut positions = Vec::new();
        let mut attributes = Vec::new();
        let mut triangles = Vec::with_capacity(self.source_triangles.len());
        let maximum_vertices =
            (self.state.device.limits().max_buffer_size / 16).min(6_000_000) as usize;
        for (face, triangle) in self.source_triangles.iter().enumerate() {
            let color = colors.get(face).copied().unwrap_or([1.; 3]);
            let top = tops.get(face).copied().unwrap_or(-1.);
            let mut mapped = [0; 3];
            for (corner, &source) in triangle.iter().enumerate() {
                let key = [
                    source,
                    color[0].to_bits(),
                    color[1].to_bits(),
                    color[2].to_bits(),
                    top.to_bits(),
                ];
                mapped[corner] = if let Some(&index) = remap.get(&key) {
                    index
                } else {
                    if positions.len() >= maximum_vertices {
                        return Err(
                            "face attribute boundaries exceed GPU vertex expansion budget".into(),
                        );
                    }
                    let index = positions.len() as u32;
                    positions.push(self.positions[source as usize]);
                    attributes.push([color[0], color[1], color[2], top]);
                    remap.insert(key, index);
                    index
                };
            }
            triangles.push(mapped);
        }
        self.upload_geometry(&positions, &triangles, &attributes)
    }
    pub fn clear_mesh(&mut self) {
        self.geometry = None;
        self.positions = Vec::new();
        self.source_triangles = Vec::new();
        self.face_colors = Vec::new();
        self.face_layer_tops = Vec::new();
        self.has_face_colors = false;
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
    /// Explicit buffer/texture allocation estimate, excluding driver bookkeeping.
    pub fn resource_bytes(&self) -> u64 {
        let geometry = self.geometry.as_ref().map_or(0, |g| {
            g.vertices.size() + g.colors.size() + g.indices.size() + g.edges.size()
        });
        let target = self.target.as_ref().map_or(0, |t| {
            t.size[0] as u64 * t.size[1] as u64 * (8 + if t.oit.is_some() { 10 } else { 0 })
        });
        geometry + target
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
    /// Pick a retained triangle at a pointer in the response rectangle returned
    /// by `show`. Uses the same cached display center and actual render aspect.
    pub fn pick_at(
        &mut self,
        picker: &super::PickingMesh,
        camera: &OrbitCamera,
        settings: &ViewportSettings,
        rect: egui::Rect,
        pointer: egui::Pos2,
    ) -> Option<super::FaceHit> {
        self.display_bounds(settings)?;
        let (ndc, logical_aspect) = super::picking::pointer_coordinates(rect, pointer)?;
        let aspect = self.target.as_ref().map_or(logical_aspect, |target| {
            target.size[0] as f64 / target.size[1] as f64
        });
        picker.pick_with_shift(camera, settings, ndc, aspect, self.shift)
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        camera: &mut OrbitCamera,
        settings: &ViewportSettings,
    ) -> egui::Response {
        let size = ui.available_size().max(egui::vec2(1., 1.));
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
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
            oit: None,
        });
    }
    fn ensure_oit(&mut self) {
        let Some(target) = self.target.as_mut() else {
            return;
        };
        if target.oit.is_some() {
            return;
        }
        let texture = |label, format| {
            self.state.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: target.size[0],
                    height: target.size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let accumulation = texture("OIT accumulation", wgpu::TextureFormat::Rgba16Float);
        let revealage = texture("OIT revealage", wgpu::TextureFormat::R16Float);
        let accumulation_view = accumulation.create_view(&Default::default());
        let revealage_view = revealage.create_view(&Default::default());
        let bind_group = self
            .state
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("OIT composite textures"),
                layout: &self.composite_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&accumulation_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&revealage_view),
                    },
                ],
            });
        target.oit = Some(OitTarget {
            _accumulation: accumulation,
            accumulation: accumulation_view,
            _revealage: revealage,
            revealage: revealage_view,
            bind_group,
        });
    }
    fn render(&mut self, camera: &OrbitCamera, settings: &ViewportSettings) {
        if settings.inspection_mode != InspectionMode::Opaque {
            self.ensure_oit();
        }
        let Some(target) = &self.target else {
            return;
        };
        let vec4 = |v: [f64; 3], w: f32| [v[0] as f32, v[1] as f32, v[2] as f32, w];
        let (right, up, back) = camera.basis();
        let rows = rotation_rows(settings.display_rotation_degrees);
        let build_height = settings.effective_buildup_height().unwrap_or(0.);
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
            section: settings.section.map_or([-1., 0., 0., build_height], |p| {
                [
                    p.axis as f32,
                    p.position_mm,
                    if p.keep_positive { 1. } else { -1. },
                    build_height,
                ]
            }),
            inspection: [
                u32::from(settings.buildup_height_mm.is_some()) as f32,
                settings.opacity,
                match settings.inspection_mode {
                    InspectionMode::Opaque => 0.,
                    InspectionMode::Ghost => 1.,
                    InspectionMode::Xray => 2.,
                },
                u32::from(self.has_face_colors) as f32,
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
        if settings.inspection_mode != InspectionMode::Opaque {
            self.render_oit(&mut encoder, target, settings);
        } else {
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
                pass.set_vertex_buffer(1, geometry.colors.slice(..));
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
    fn render_oit(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &Target,
        settings: &ViewportSettings,
    ) {
        let oit = target
            .oit
            .as_ref()
            .expect("OIT targets allocated before rendering");
        let attachment = |view, clear| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("weighted transparent inspection"),
                color_attachments: &[
                    attachment(&oit.accumulation, wgpu::Color::TRANSPARENT),
                    attachment(&oit.revealage, wgpu::Color::WHITE),
                ],
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
                pass.set_pipeline(&self.oit_pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, geometry.vertices.slice(..));
                pass.set_vertex_buffer(1, geometry.colors.slice(..));
                pass.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..geometry.index_count, 0, 0..1);
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("weighted transparency composite"),
                color_attachments: &[attachment(&target.color_view, wgpu::Color::TRANSPARENT)],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.composite_pipeline);
            pass.set_bind_group(0, &oit.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        if settings.show_edges {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("transparent inspection triangle edges"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.color_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
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
                pass.set_pipeline(&self.edge_pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, geometry.vertices.slice(..));
                pass.set_vertex_buffer(1, geometry.colors.slice(..));
                pass.set_index_buffer(geometry.edges.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..geometry.edge_count, 0, 0..1);
            }
        }
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

    #[test]
    #[ignore = "requires an actual GPU adapter; validates section/build-up/OIT pixels"]
    fn gpu_inspection_clips_build_frame_and_blends_independently_of_order() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let state = block_on(RenderState::create(
            &Default::default(),
            &instance,
            None,
            Default::default(),
        ))
        .unwrap();
        let scope = state.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut viewport = Viewport::new(&state);
        let mut mesh = Mesh::default();
        for y in [-0.5, 0., 0.5] {
            let b = mesh.positions.len() as u32;
            mesh.positions
                .extend([[-1., y, 0.], [1., y, 0.], [1., y, 2.], [-1., y, 2.]]);
            mesh.triangles
                .extend([[b, b + 1, b + 2], [b, b + 2, b + 3]]);
        }
        let colors = [
            [1., 0.03, 0.03],
            [1., 0.03, 0.03],
            [0.03, 1., 0.03],
            [0.03, 1., 0.03],
            [0.03, 0.03, 1.],
            [0.03, 0.03, 1.],
        ];
        viewport.set_mesh(&mesh, "original", 1).unwrap();
        viewport.set_triangle_colors(&colors).unwrap();
        let mut settings = ViewportSettings::default();
        let mut camera = OrbitCamera::default();
        camera.canonical(super::super::CanonicalView::Front);
        camera.orthographic = true;
        camera.fit(&viewport.display_bounds(&settings).unwrap());
        viewport.resize([256, 256]);
        viewport.render(&camera, &settings);
        let opaque = capture(&viewport);
        settings.inspection_mode = InspectionMode::Ghost;
        viewport.render(&camera, &settings);
        let ghost = capture(&viewport);
        assert!(
            opaque != ghost,
            "ghost inspection must expose rear surfaces"
        );
        let center = (128 * 256 + 128) * 4;
        assert!(
            ghost[center + 1] > opaque[center + 1] && ghost[center + 2] > opaque[center + 2],
            "rear green/blue groups must contribute"
        );
        mesh.triangles.reverse();
        let mut reversed = colors;
        reversed.reverse();
        viewport.set_mesh(&mesh, "original", 2).unwrap();
        viewport.set_triangle_colors(&reversed).unwrap();
        viewport.display_bounds(&settings);
        viewport.render(&camera, &settings);
        let reversed_pixels = capture(&viewport);
        assert!(
            ghost
                .iter()
                .zip(&reversed_pixels)
                .all(|(&a, &b)| a.abs_diff(b) <= 2),
            "weighted OIT changes with submission order"
        );
        settings.inspection_mode = InspectionMode::Opaque;
        settings.section = Some(SectionPlane {
            axis: 0,
            position_mm: 0.,
            keep_positive: true,
        });
        viewport.render(&camera, &settings);
        let cut = capture(&viewport);
        let occupied = |bytes: &[u8]| {
            bytes
                .chunks_exact(4)
                .filter(|p| p[0] > 50 || p[1] > 50 || p[2] > 50)
                .count()
        };
        let ratio = occupied(&cut) as f64 / occupied(&opaque) as f64;
        assert!((0.48..0.52).contains(&ratio));
        settings.display_rotation_degrees = [0., 0., 90.];
        viewport.display_bounds(&settings);
        camera.canonical(super::super::CanonicalView::Right);
        viewport.render(&camera, &settings);
        let rotated = capture(&viewport);
        assert!(
            (occupied(&rotated) as f64 / occupied(&opaque) as f64 - 0.5).abs() < 0.03,
            "section plane detached from build pose"
        );
        settings.display_rotation_degrees = [0.; 3];
        settings.section = None;
        settings.buildup_height_mm = Some(0.3);
        viewport.display_bounds(&settings);
        camera.canonical(super::super::CanonicalView::Front);
        viewport.render(&camera, &settings);
        let partial = capture(&viewport);
        settings.buildup_complete_layers = true;
        viewport.render(&camera, &settings);
        let complete = capture(&viewport);
        assert!((occupied(&complete) as f64 / occupied(&partial) as f64 - 2. / 3.).abs() < 0.08);
        settings.buildup_height_mm = Some(0.1);
        viewport.render(&camera, &settings);
        let empty = capture(&viewport);
        assert!(empty.chunks_exact(4).all(|p| p == &empty[..4]));
        // A cutaway of separate actual beads retains the air gap; no envelope or
        // synthetic infill/cap can appear in the native inspection path.
        use crate::print::beads::{BeadSegment, build_bead_mesh};
        let beads: Vec<_> = [-0.6, 0.6]
            .into_iter()
            .map(|y| BeadSegment {
                start: [-1., y, 0.2],
                end: [1., y, 0.2],
                width_mm: 0.3,
                height_mm: 0.2,
                volume_mm3: 0.1,
                source_line: 1,
                feature: "fixture".into(),
                tool: 0,
                layer_index: 0,
                width_source: "fixture".into(),
                height_source: "fixture".into(),
            })
            .collect();
        viewport
            .set_mesh(&build_bead_mesh(&beads).unwrap(), "beads", 3)
            .unwrap();
        settings = ViewportSettings {
            section: Some(SectionPlane {
                axis: 0,
                position_mm: 0.,
                keep_positive: true,
            }),
            ..Default::default()
        };
        camera.canonical(super::super::CanonicalView::Top);
        camera.fit(&viewport.display_bounds(&settings).unwrap());
        for mode in [
            InspectionMode::Opaque,
            InspectionMode::Ghost,
            InspectionMode::Xray,
        ] {
            settings.inspection_mode = mode;
            viewport.render(&camera, &settings);
            let pixels = capture(&viewport);
            assert_eq!(
                &pixels[center..center + 4],
                &pixels[..4],
                "section filled the air gap between beads"
            );
            let p = camera.project([0.5, -0.6, 0.2], 1.).unwrap();
            let x = ((p[0] + 1.) * 128.) as usize;
            let y = ((1. - p[1]) * 128.) as usize;
            let index = (y * 256 + x) * 4;
            assert!(
                pixels[index..index + 3].iter().any(|&v| v > 50),
                "section removed a retained bead"
            );
        }
        settings.show_edges = true;
        viewport.resize([192, 128]);
        viewport.render(&camera, &settings);
        let _ = capture(&viewport);
        let error = block_on(scope.pop());
        assert!(error.is_none(), "inspection GPU validation: {error:?}");
    }

    #[test]
    #[ignore = "requires an actual GPU adapter; validates complete deposited layer faces"]
    fn gpu_layer_faces_preserve_caps_and_palette() {
        use crate::print::beads::{BeadSegment, build_bead_mesh};
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let state = block_on(RenderState::create(
            &Default::default(),
            &instance,
            None,
            Default::default(),
        ))
        .unwrap();
        let scope = state.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let segments: Vec<_> = [(0.2, 0.2, -0.6), (0.5, 0.3, 0.6)]
            .into_iter()
            .enumerate()
            .map(|(i, (top, height, y))| BeadSegment {
                start: [-1., y, top],
                end: [1., y, top],
                width_mm: 0.4,
                height_mm: height,
                volume_mm3: 0.1,
                source_line: i + 1,
                feature: "fixture".into(),
                tool: i as u32,
                layer_index: i as u32,
                width_source: "fixture".into(),
                height_source: "fixture".into(),
            })
            .collect();
        let mesh = build_bead_mesh(&segments).unwrap();
        let first_faces = build_bead_mesh(&segments[..1]).unwrap().triangles.len();
        let tops: Vec<_> = (0..mesh.triangles.len())
            .map(|i| if i < first_faces { 0.2 } else { 0.5 })
            .collect();
        let colors: Vec<_> = (0..mesh.triangles.len())
            .map(|i| {
                if i < first_faces {
                    [0.8, 0.05, 0.02]
                } else {
                    [0.02, 0.8, 0.05]
                }
            })
            .collect();
        let mut reference_mesh = mesh.clone();
        reference_mesh.triangles.truncate(first_faces);
        // Retain all positions so reference and tested mesh use identical display transforms.
        let mut reference = Viewport::new(&state);
        reference.set_mesh(&reference_mesh, "beads", 1).unwrap();
        reference
            .set_triangle_colors(&colors[..first_faces])
            .unwrap();
        let mut viewport = Viewport::new(&state);
        viewport.set_mesh(&mesh, "beads", 1).unwrap();
        viewport.set_triangle_colors(&colors).unwrap();
        viewport.set_triangle_layer_tops(&tops).unwrap();
        assert!(viewport.set_triangle_layer_tops(&[0.2]).is_err());
        let mut invalid = tops.clone();
        invalid[0] = f32::NAN;
        assert!(viewport.set_triangle_layer_tops(&invalid).is_err());
        let mut settings = ViewportSettings {
            buildup_height_mm: Some(0.2),
            ..Default::default()
        };
        let mut camera = OrbitCamera::default();
        camera.canonical(super::super::CanonicalView::Top);
        camera.orthographic = true;
        camera.fit(&viewport.display_bounds(&settings).unwrap());
        reference.resize([256, 256]);
        viewport.resize([256, 256]);
        for rotation in [[0.; 3], [20., 0., 35.]] {
            settings.display_rotation_degrees = rotation;
            viewport.display_bounds(&settings);
            reference.display_bounds(&settings);
            for mode in [
                InspectionMode::Opaque,
                InspectionMode::Ghost,
                InspectionMode::Xray,
            ] {
                settings.inspection_mode = mode;
                for edges in [false, true] {
                    settings.show_edges = edges;
                    viewport.render(&camera, &settings);
                    let mut expected_settings = settings.clone();
                    expected_settings.buildup_height_mm = None;
                    reference.render(&camera, &expected_settings);
                    assert_eq!(
                        capture(&viewport),
                        capture(&reference),
                        "future-layer bottom cap leaked or printed cap vanished: {mode:?}, edges={edges}"
                    );
                }
            }
        }
        viewport.set_triangle_colors(&[]).unwrap();
        reference.set_triangle_colors(&[]).unwrap();
        viewport.render(&camera, &settings);
        let mut expected_settings = settings.clone();
        expected_settings.buildup_height_mm = None;
        reference.render(&camera, &expected_settings);
        assert_eq!(
            capture(&viewport),
            capture(&reference),
            "resetting colors lost layer metadata"
        );
        viewport.set_triangle_colors(&colors).unwrap();
        viewport.set_triangle_layer_tops(&[]).unwrap();
        viewport.set_triangle_layer_tops(&tops).unwrap();
        reference
            .set_triangle_colors(&colors[..first_faces])
            .unwrap();
        viewport.render(&camera, &settings);
        reference.render(&camera, &expected_settings);
        assert_eq!(
            capture(&viewport),
            capture(&reference),
            "resetting layer metadata lost palette"
        );
        viewport.clear_mesh();
        assert!(viewport.set_triangle_layer_tops(&tops).is_err());
        assert!(block_on(scope.pop()).is_none());
    }

    #[test]
    #[ignore = "requires an actual GPU adapter; matches click hits to rendered face colors"]
    fn gpu_click_pick_and_settle_match_rendered_surface() {
        use super::super::PickingMesh;
        use crate::model::orientation::{apply, rotation_matrix, settled_display_rotation};
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let state = block_on(RenderState::create(
            &Default::default(),
            &instance,
            None,
            Default::default(),
        ))
        .unwrap();
        let scope = state.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mesh = Mesh {
            positions: vec![
                [10., 20., 30.],
                [14., 20., 30.],
                [14., 24., 30.],
                [10., 24., 30.],
                [10., 20., 34.],
                [14., 20., 34.],
                [14., 24., 34.],
                [10., 24., 34.],
            ],
            triangles: vec![
                [0, 2, 1],
                [0, 3, 2],
                [4, 5, 6],
                [4, 6, 7],
                [0, 1, 5],
                [0, 5, 4],
                [1, 2, 6],
                [1, 6, 5],
                [2, 3, 7],
                [2, 7, 6],
                [3, 0, 4],
                [3, 4, 7],
            ],
        };
        let picker = PickingMesh::new(&mesh).unwrap();
        let mut viewport = Viewport::new(&state);
        viewport.set_mesh(&mesh, "original", 1).unwrap();
        let colors: Vec<_> = (0..12)
            .map(|i| {
                let mut color = [0.01; 3];
                color[(i / 2) % 3] = 0.9;
                color
            })
            .collect();
        viewport.set_triangle_colors(&colors).unwrap();
        viewport.resize([320, 240]);
        let mut camera = OrbitCamera::default();
        camera.canonical(super::super::CanonicalView::Front);
        let rect = egui::Rect::from_min_size(egui::pos2(20., 50.), egui::vec2(320., 240.));
        let mut settings = ViewportSettings::default();
        for orthographic in [false, true] {
            camera.orthographic = orthographic;
            for pose in [[17., -32., 28.], [-75., 40., 110.], [0., 90., 0.]] {
                settings.display_rotation_degrees = pose;
                camera.fit_aspect(&viewport.display_bounds(&settings).unwrap(), 320. / 240.);
                viewport.render(&camera, &settings);
                let hit = viewport
                    .pick_at(
                        &picker,
                        &camera,
                        &settings,
                        rect,
                        rect.center() + egui::vec2(0.5, 0.5),
                    )
                    .unwrap();
                let pixels = capture(&viewport);
                let pixel = &pixels[(120 * 320 + 160) * 4..(120 * 320 + 160) * 4 + 3];
                let channel = (hit.triangle_index / 2) % 3;
                assert!(
                    (0..3).all(|i| i == channel || pixel[channel] > pixel[i]),
                    "picked face disagrees with actual pixel {pixel:?} at pose{pose:?}"
                );
                let next = settled_display_rotation(pose.map(f64::from), hit.normal_mm).unwrap();
                let normal = apply(rotation_matrix(next).unwrap(), hit.normal_mm);
                assert!(
                    normal[0].abs() < 1e-8
                        && normal[1].abs() < 1e-8
                        && (normal[2] + 1.).abs() < 1e-8
                );
                settings.display_rotation_degrees = next.map(|v| v as f32);
                viewport.display_bounds(&settings);
                let rows =
                    rotation_rows(settings.display_rotation_degrees).map(|r| r.map(f64::from));
                for &index in &mesh.triangles[hit.triangle_index] {
                    let z =
                        apply(rows, mesh.positions[index as usize])[2] - viewport.shift[2] as f64;
                    assert!(
                        z.abs() < 1e-5,
                        "selected convex face does not sit on floor: {z}"
                    );
                }
                settings.layer_appearance = true;
                viewport.render(&camera, &settings);
                let _ = capture(&viewport);
            }
        }
        assert!(block_on(scope.pop()).is_none());
    }

    #[test]
    #[ignore = "GPU throughput benchmark; run separately from correctness checks"]
    fn dense_bead_viewport_benchmark() {
        use crate::print::beads::{BeadSegment, build_bead_mesh};
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let state = block_on(RenderState::create(
            &Default::default(),
            &instance,
            None,
            Default::default(),
        ))
        .unwrap();
        let segments: Vec<_> = (0..100)
            .flat_map(|layer| {
                (0..100).map(move |line| BeadSegment {
                    start: [0., line as f64 * 0.5, (layer + 1) as f64 * 0.2],
                    end: [50., line as f64 * 0.5, (layer + 1) as f64 * 0.2],
                    width_mm: 0.42,
                    height_mm: 0.2,
                    volume_mm3: 4.,
                    source_line: layer * 100 + line + 1,
                    feature: "benchmark infill".into(),
                    tool: 0,
                    layer_index: layer as u32,
                    width_source: "fixture".into(),
                    height_source: "fixture".into(),
                })
            })
            .collect();
        let mesh = build_bead_mesh(&segments).unwrap();
        let mut viewport = Viewport::new(&state);
        let upload = std::time::Instant::now();
        viewport.set_mesh(&mesh, "beads", 1).unwrap();
        state
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let upload_ms = upload.elapsed().as_secs_f64() * 1000.;
        let mut settings = ViewportSettings::default();
        let mut camera = OrbitCamera::default();
        camera.fit_aspect(&viewport.display_bounds(&settings).unwrap(), 2560. / 1440.);
        viewport.resize([2560, 1440]);
        for mode in [
            InspectionMode::Opaque,
            InspectionMode::Ghost,
            InspectionMode::Xray,
        ] {
            settings.inspection_mode = mode;
            viewport.render(&camera, &settings);
            state
                .device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            let mut milliseconds = Vec::new();
            for frame in 0..30 {
                settings.turntable_angle_degrees = frame as f32 * 12.;
                let started = std::time::Instant::now();
                viewport.render(&camera, &settings);
                state
                    .device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
                milliseconds.push(started.elapsed().as_secs_f64() * 1000.);
            }
            milliseconds.sort_by(f64::total_cmp);
            eprintln!(
                "viewport benchmark mode={mode:?} triangles={} size=2560x1440 frames=30 upload_ms={upload_ms:.2} median_ms={:.3} p95_ms={:.3} allocated_mib={:.2}",
                mesh.triangles.len(),
                milliseconds[15],
                milliseconds[28],
                viewport.resource_bytes() as f64 / 1048576.
            );
        }
    }
}
