//! Exact native ABI1 mirror; see native/include/turninator_renderer.h.
use std::ffi::c_void;
pub const VERSION: u32 = 1;
pub const FIXTURE: u32 = 0xffff0001;
pub const FIXTURE_FLAG: u32 = 1;
pub const MAX_JSON: usize = 1024 * 1024;
#[repr(C)]
pub struct ErrorBuffer {
    pub size: u32,
    pub version: u32,
    pub text: *mut u8,
    pub text_capacity: u64,
}
#[repr(C)]
#[derive(Default)]
pub struct Info {
    pub size: u32,
    pub version: u32,
    pub backend: u32,
    pub flags: u32,
    pub error_size: u32,
    pub config_size: u32,
    pub scene_size: u32,
    pub frame_size: u32,
    pub poll_size: u32,
    pub image_size: u32,
}
#[repr(C)]
pub struct Config {
    pub size: u32,
    pub version: u32,
    pub json: *const u8,
    pub json_bytes: u64,
    pub host_bytes: u64,
    pub gpu_bytes: u64,
    pub max_output_side: u32,
    pub reserved: u32,
}
#[repr(C)]
pub struct Scene {
    pub size: u32,
    pub version: u32,
    pub revision: u64,
    pub positions_mm: *const f32,
    pub normals: *const f32,
    pub build_positions_mm: *const f32,
    pub triangles: *const u32,
    pub triangle_materials: *const u32,
    pub vertex_count: u32,
    pub triangle_count: u32,
    pub json: *const u8,
    pub json_bytes: u64,
}
#[repr(C)]
pub struct Frame {
    pub size: u32,
    pub version: u32,
    pub geometry_revision: u64,
    pub width: u32,
    pub height: u32,
    pub camera_position_mm: [f64; 3],
    pub camera_target_mm: [f64; 3],
    pub camera_up: [f64; 3],
    pub projection: u32,
    pub transparent: u32,
    pub sample_seed: u32,
    pub reserved: u32,
    pub vertical: f64,
    pub near_mm: f64,
    pub far_mm: f64,
    pub model_to_world: [f64; 16],
    pub quality_json: *const u8,
    pub quality_json_bytes: u64,
}
#[repr(C)]
#[derive(Default)]
pub struct Poll {
    pub size: u32,
    pub version: u32,
    pub state: u32,
    pub reserved: u32,
    pub completed_work: u64,
    pub total_work: u64,
}
#[repr(C)]
#[derive(Default)]
pub struct Image {
    pub size: u32,
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub required_bytes: u64,
}
pub type Context = *mut c_void;
pub type Query = unsafe extern "C" fn(*mut Info, *mut ErrorBuffer) -> i32;
pub type Create = unsafe extern "C" fn(*const Config, *mut Context, *mut ErrorBuffer) -> i32;
pub type Describe = unsafe extern "C" fn(Context, *mut u8, u64, *mut u64, *mut ErrorBuffer) -> i32;
pub type Upload = unsafe extern "C" fn(Context, *const Scene, *mut ErrorBuffer) -> i32;
pub type Begin = unsafe extern "C" fn(Context, *const Frame, *mut ErrorBuffer) -> i32;
pub type PollFn = unsafe extern "C" fn(Context, *mut Poll, *mut ErrorBuffer) -> i32;
pub type Read = unsafe extern "C" fn(Context, *mut u8, u64, *mut Image, *mut ErrorBuffer) -> i32;
pub type Cancel = unsafe extern "C" fn(Context, *mut ErrorBuffer) -> i32;
pub type Destroy = unsafe extern "C" fn(Context);
const _: () = {
    assert!(size_of::<ErrorBuffer>() == 24 && size_of::<Info>() == 40 && size_of::<Config>() == 48);
    assert!(size_of::<Scene>() == 80 && size_of::<Frame>() == 280);
    assert!(size_of::<Poll>() == 32 && size_of::<Image>() == 24);
    assert!(
        std::mem::offset_of!(Scene, positions_mm) == 16 && std::mem::offset_of!(Scene, json) == 64
    );
    assert!(
        std::mem::offset_of!(Frame, model_to_world) == 136
            && std::mem::offset_of!(Frame, quality_json) == 264
    );
};
