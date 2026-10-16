#ifndef TURNINATOR_RENDERER_H
#define TURNINATOR_RENDERER_H
#include <stdint.h>
#include <stddef.h>
#if defined(_WIN32) && defined(TN_BUILDING_BRIDGE)
#define TN_EXPORT __declspec(dllexport)
#elif defined(_WIN32)
#define TN_EXPORT __declspec(dllimport)
#else
#define TN_EXPORT __attribute__((visibility("default")))
#endif
#ifdef __cplusplus
extern "C" {
#endif

/* ABI1: x86_64, native C alignment, cdecl; no packing pragmas. Every struct
 * starts with exact sizeof(struct), version=1. No C++ objects, exceptions,
 * callbacks or allocator ownership cross this boundary. All inputs are copied
 * before return; pointers may be NULL only for zero-length spans. Counts count
 * elements, not bytes (except explicitly named byte fields). UTF-8 JSON is
 * length-delimited, never NUL-dependent, maximum 1 MiB. Error text is UTF-8,
 * truncated to capacity-1 and NUL terminated; text_capacity=0 permits NULL.
 * All calls including destroy are on the creating thread. Destroy may block
 * to drain SDK workers/readbacks; it must catch exceptions and never unwind.
 *
 * create -> Idle; upload in Idle copies/replaces a numbered scene revision.
 * begin in Idle requires that revision, copies resolved frame data -> Rendering.
 * poll never blocks or calls Rust: Rendering -> Ready after SDK/readback drain.
 * Ready read copies complete top-first tightly-packed display-sRGB RGBA8,
 * straight alpha (zero RGB where alpha=0), then -> Idle. Insufficient capacity
 * writes NO pixels, reports required bytes, leaves Ready. Begin/upload while
 * busy, read before Ready, bad input and capacity errors preserve prior state.
 * SDK errors/exceptions -> Failed, only poll diagnostics/destroy thereafter.
 * cancel is nonblocking/idempotent; active Rendering/Ready -> Cancelling.
 * poll emits Cancelled once ONLY after full SDK/readback drain, simultaneously
 * restoring Idle. cancel in Idle is a no-op. No hard thread-kill guarantee:
 * ordinary native calls blocked inside a driver/compiler are not interruptible.
 * No DLL may interpret timelines or silently change renderer/device/quality.
 *
 * JSON schema1 shapes are frozen in docs/renderers/sdk-qualification.md:
 * config {schema_version,selection,identity,limits};
 * scene {schema_version,appearance,materials,reference_bounds_mm,geometry_kind,
 *        layer_filter,color}; quality {schema_version,quality}.
 * PhysicalMaterial/RenderAppearance retain shared Rust serde field names.
 * SDK bridges validate schema1 and consume these exact fields, not private
 * aliases/defaults. scene appearance display_rotation is provenance only:
 * model_to_world already contains display pose; never apply it twice.
 * Producer performs versioned HDR tone mapping before this 8-bit boundary.
 * output_transfer=srgb/output_alpha=straight; no second transfer encoding.
 */
enum { TN_ABI_VERSION=1, TN_BACKEND_FILAMENT=1, TN_BACKEND_LUXCORE=2, TN_FLAG_TEST_FIXTURE=1 };
#define TN_BACKEND_FIXTURE UINT32_C(0xffff0001)
enum { TN_OK=0, TN_INVALID_ARGUMENT=1, TN_INVALID_STATE=2,
       TN_CAPACITY=3, TN_FAILURE=4 };
enum { TN_IDLE=0, TN_RENDERING=1, TN_READY=2, TN_CANCELLING=3,
       TN_CANCELLED=4, TN_FAILED=5 };
typedef struct tn_error {
    uint32_t size, version;
    uint8_t *text;
    uint64_t text_capacity;
} tn_error;
typedef struct tn_abi_info {
    uint32_t size, version, backend, flags;
    uint32_t error_size, config_size, scene_size, frame_size, poll_size, image_size;
} tn_abi_info;
typedef struct tn_config {
    uint32_t size, version;
    const uint8_t *json;
    uint64_t json_bytes, host_bytes, gpu_bytes;
    uint32_t max_output_side, reserved;
} tn_config;
/* Aligned arrays: positions_mm, normals and build_positions_mm each have
 * vertex_count float[3] elements. Normals are dimensionless build-space unit
 * directions; positions/build coordinates are millimeters, right-handed Z up.
 * Build coordinates are retained for physical layer shaders after display pose.
 * triangles is triangle_count uint32_t[3], triangle_materials uint32_t[1].
 * All indices refer to aligned vertex arrays; empty scenes are valid. JSON holds
 * versioned material/appearance/studio metadata, not duplicated mesh arrays.
 * Native bridges perform their SDK's unit/axis conversion exactly once.
 */
typedef struct tn_scene {
    uint32_t size, version;
    uint64_t revision;
    const float *positions_mm, *normals, *build_positions_mm;
    const uint32_t *triangles, *triangle_materials;
    uint32_t vertex_count, triangle_count;
    const uint8_t *json;
    uint64_t json_bytes;
} tn_scene;
typedef struct tn_frame {
    uint32_t size, version;
    uint64_t geometry_revision;
    uint32_t width, height;
    double camera_position_mm[3], camera_target_mm[3], camera_up[3];
    /* projection: 0 perspective, 1 orthographic; vertical = degrees or mm.
     * clip planes are host-resolved millimeters. Model matrix is row-major,
     * maps build mm to world mm, affine; camera is already world-resolved. */
    uint32_t projection, transparent; /* transparent is 0 or 1 */
    uint32_t sample_seed, reserved; /* deterministic seed; never inferred from time */
    double vertical, near_mm, far_mm, model_to_world[16];
    const uint8_t *quality_json;
    uint64_t quality_json_bytes;
} tn_frame;
typedef struct tn_poll {
    uint32_t size, version, state, reserved;
    uint64_t completed_work, total_work; /* total_work=0 means unknown */
} tn_poll;
typedef struct tn_image {
    uint32_t size, version, width, height;
    uint64_t required_bytes;
} tn_image;
typedef void *tn_context;

TN_EXPORT int32_t tn_renderer_abi_version(tn_abi_info *, tn_error *);
TN_EXPORT int32_t tn_renderer_create(const tn_config *, tn_context *, tn_error *);
/* Describe actual initialized SDK/device, not requested identity. JSON shape v1:
 * {schema_version:1, backend:string, sdk_version:string, device_id:string,
 * device_name:string, api:string, driver:string, compiler?:
 * {name:string,version:string,sha256:string,path:absolute-string}}.
 * Optional diagnostics is schema1 {schema_version:1,counters:{key:u64},
 * timings_ms:{key:finite-nonnegative-number},settings:{key:bounded-text},
 * artifacts:{key:{path:absolute-Windows-UTF8-path,sha256:64-hex}}}.
 * Each map <=64 keys matching [a-z][a-z0-9_.-]* (<=128 bytes); settings
 * values are nonempty <=1024 UTF8 bytes without controls; artifact paths
 * are <=4096 bytes, lexical drive/UNC/verbatim paths (no filesystem IO).
 * The dedicated SDK worker refreshes a validated snapshot after drained work,
 * including cancellation, before Ready/Cancelled. Other fields remain stable.
 * Diagnostics are observational and are not runtime/pixel identity.
 * required is byte length without NUL, <=1MiB. Query with NULL/0 or retry with
 * short capacity returns TN_CAPACITY, sets required and writes NO buffer bytes.
 * A complete copy does not change render state. Allowed for Failed diagnostics.
 * Snapshot length may grow/shrink between query and copy; successful copy sets
 * actual required length. Retry TN_CAPACITY with its new bounded required size.
 */
TN_EXPORT int32_t tn_renderer_describe(tn_context, uint8_t *, uint64_t, uint64_t *required, tn_error *);
TN_EXPORT int32_t tn_renderer_upload_scene(tn_context, const tn_scene *, tn_error *);
TN_EXPORT int32_t tn_renderer_begin_frame(tn_context, const tn_frame *, tn_error *);
TN_EXPORT int32_t tn_renderer_poll(tn_context, tn_poll *, tn_error *);
TN_EXPORT int32_t tn_renderer_read_rgba8(tn_context, uint8_t *, uint64_t, tn_image *, tn_error *);
TN_EXPORT int32_t tn_renderer_cancel(tn_context, tn_error *);
TN_EXPORT void tn_renderer_destroy(tn_context);
#ifdef __cplusplus
}
static_assert(sizeof(void*) == 8, "ABI1 requires x86_64");
static_assert(sizeof(tn_error)==24 && sizeof(tn_abi_info)==40 && sizeof(tn_config)==48);
static_assert(sizeof(tn_scene)==80 && sizeof(tn_frame)==280);
static_assert(sizeof(tn_poll)==32 && sizeof(tn_image)==24);
static_assert(offsetof(tn_scene,positions_mm)==16 && offsetof(tn_scene,json)==64);
static_assert(offsetof(tn_frame,model_to_world)==136 && offsetof(tn_frame,quality_json)==264);
#endif
#endif
