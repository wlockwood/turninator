# Native SDK boundary and qualification

The shared ABI and CPU fixture are implemented. Production Filament and LuxCore
bridges are unfinished; no fixture is registered as a renderer. Missing optional
DLLs return an availability error on explicit load. This code does not initialize
an SDK at application startup, download anything, invoke a compiler, embed Python,
or launch a render process.

The authoritative SDK feasibility evidence is Task5a's
`.superpowers/sdd/2026-10-15-selectable-renderers/task-5a-report.md` and
`artifacts/renderers/qualification/2026-10-15-sdk-probes/` in the coordinator
worktree. Those local artifacts are not distributed with the product. Task5b
consumes them; its fixture checks are CPU-only and do not repeat GPU probes.

## Pinned inputs and controlled builds

`native/sdk-lock.json` pins official Filament 1.78.0, commit
`c1dc4e444784850d6dc891a1fb38747c30b85463`, and LuxCore 2.11.2, commit
`b872186d2fe995a762f73708c606847eed55843e`, plus LuxCoreDeps 2.2.3.
It includes four immutable archive hashes, individual runtime/import/static
library/tool/support-source hashes and SHA-256 of complete header inventories.
The latter pin Filament, matching LuxCore source and Boost 1.88 headers from the
dependency archive. Exploratory Boost 1.86 is deliberately excluded. Release
asset provenance is retained as source URLs; hashes came from Task5a's downloaded
archives and inventory, rather than guessed SDK versions.

Build requirements are CMake 4.2.3, Visual Studio 2022 v143 toolset 14.44.35207
(MSVC 19.44.35215.0), Windows SDK 10.0.26100.0, C++20, Release dynamic CRT `/MD`,
and `_ITERATOR_DEBUG_LEVEL=0`. Filament's matching `matc` reports version 78;
material packages must be compiled during controlled builds and hashed into
backend identity. No compiler executable belongs in the render-time path.

```powershell
# Verify an existing unpacked evidence/input tree, including complete headers.
./tools/qualify-renderers.ps1 -InputRoot '<absolute SDK probe/input directory>'
# Explicitly build CPU test DLLs; never emits production bridges.
./tools/build-renderers.ps1 -Fixture
$env:CARGO_TARGET_DIR='D:/code/turninator/target'
./tools/qualify-renderers.ps1 -RunFixtureTests -FixtureDirectory './artifacts/native-fixtures/fixture/Release'
# These fail clearly until the corresponding production bridge exists.
./tools/build-renderers.ps1 -Backend filament -InputRoot '<absolute pinned input directory>'
./tools/build-renderers.ps1 -Backend luxcore -InputRoot '<absolute pinned input directory>'
```

The input directory uses the relative paths recorded in the lock and carries the
three pinned header manifests. Input verification reads local files only. A
successful hash check proves integrity, not GPU or redistribution qualification.
Do not package the fixture directory with production renderer payloads.

## ABI1 ownership, layout and state protocol

The common header `native/include/turninator_renderer.h` defines every exported
signature and status. Windows x64 uses native C alignment and cdecl. Each struct
starts with `size` and `version`; the initial descriptor reports every struct's
size, backend number and flags. The Rust host rejects any mismatch before context
creation and checks all nine required exports. Define `TN_BUILDING_BRIDGE` when
compiling a DLL, otherwise the header declares imports. C++ and Rust have compile-time
size assertions; the live fixture exchanges the same layouts.

| Struct | Bytes |
| --- | ---: |
| `tn_error` | 24 |
| `tn_abi_info` | 40 |
| `tn_config` | 48 |
| `tn_scene` | 80 |
| `tn_frame` | 280 |
| `tn_poll` | 32 |
| `tn_image` | 24 |

Geometry spans contain contiguous aligned `float[3]` millimeter positions,
dimensionless normals, preserved millimeter build coordinates, `u32[3]` triangle
indices and one `u32` material index per triangle. Empty geometry is valid.
Meshes remain right-handed Z up; each SDK bridge converts units/axes exactly
once. Versioned material/appearance/studio JSON is bounded to 1 MiB; schema1 is
frozen below, while material behavior still requires Task4 qualification. Config JSON contains
the serialized selection, effective identity and limits. Begin copies a known
geometry revision, resolved camera/projection/clip planes, row-major affine
model-to-world transform, dimensions, explicit transparency/sample seed and selected quality; DLLs never parse a
timeline. Rust passes no allocation ownership, callbacks, SDK objects or STL.

Creation returns Idle. Upload replaces a copied scene revision only in Idle.
Begin requires that revision and enters Rendering. Nonblocking poll reports
Rendering until the full image is Ready. Read in Ready writes tightly packed,
top-row-first display-sRGB RGBA8 with straight alpha and zero RGB for zero alpha.
Only a complete successful copy consumes Ready and returns Idle. A short buffer
writes no pixels and preserves Ready for retry. Argument, state and capacity
errors preserve state; SDK errors/exceptions set Failed until destruction.

Cancel is an idempotent, nonblocking request. Active work enters Cancelling;
poll emits Cancelled once after SDK workers/readback callbacks fully drain,
restoring Idle for the next frame. Destroy drains synchronously on the owner
thread. Cooperative cancellation cannot interrupt a driver/compiler-blocked
native call or provide a hard thread-kill guarantee. Production implementations
must arrange SDK ownership so blocking Stop/join calls do not occur in poll or
cancel; fixture drain simulation does not prove this for either SDK.

Rust library and context handles use `Rc` and cannot be sent/shared across
threads. Each context owns a strong library reference. Drop calls native destroy
before that reference can unload the DLL, including pending-work teardown.

`tn_renderer_describe` reports the initialized SDK/device as bounded schema1
JSON in a caller-owned buffer. Short/query buffers report required bytes without
writing payload, allowing retry. `create` validates this response before accepting
the context; `runtime_info()` can retrieve it again without changing render state.
Unknown fields, wrong schema/backend, missing/empty/oversized strings, invalid
UTF-8, malformed compiler identity and a device ID incompatible with an explicit
selection fail preparation. A selection with no device is resolved by the bridge;
the adapter must record the returned concrete identity before rendering.

## Frozen JSON schema1 shapes

All JSON is UTF-8 without a trailing NUL, at most 1 MiB. Numeric inputs must be
finite. Every top-level object includes `schema_version: 1`; schema changes
require an explicit version change, never adapter-private interpretation.

Config is `{schema_version, selection, identity, limits}`. `selection` is
`{backend, device, quality}`: backend is `filament` or `luxcore`, device is either
null or an explicit stable ID, and quality has the tagged shape below. `identity`
uses the shared `BackendIdentity` serde fields: required `backend`,
`adapter_recipe_version`, `runtime_digest`, `color_digest`, `device`, `api`,
`driver`; optional `bridge_digest`, `shader_digest`, `material_digest`,
`denoiser_digest` are omitted when absent. This is the requested/prepared identity,
not evidence of actual runtime selection. `limits` has unsigned integer fields
`max_source_vertices`, `max_source_triangles`, `max_scene_bytes`,
`max_output_side`, `host_bytes`, `gpu_bytes`, `scratch_bytes`. The duplicate C
config budget fields must match; no silent budget expansion is allowed.
`max_source_vertices` and `max_source_triangles` belong to upstream source-snapshot
validation. They must not be reapplied to already-realized upload spans, which may
expand through vertex alignment, generated surfaces or section caps. At this ABI
boundary the explicit hard derived capacities remain `Mesh::MAX_VERTICES`
(2,000,000) and `Mesh::MAX_TRIANGLES` (4,000,000), alongside checked byte budgets.

Frame quality JSON is `{schema_version:1, quality:{...}}`. Filament quality is
`{kind:"filament", aa:"low"|"medium"|"high", shadows:"low"|"medium"|"high"}`.
LuxCore quality is `{kind:"luxcore", samples:1..4096, denoise:boolean}`. These
settings are backend-specific; no translation from raster settings to samples.
Transparency and deterministic sample seed are explicit `tn_frame` integers,
not JSON defaults. Matrix translations occupy row-major indices 3, 7 and 11;
the last row is `[0,0,0,1]`.

Scene JSON is `{schema_version, appearance, materials, reference_bounds_mm,
geometry_kind, layer_filter, color}`. `geometry_kind` is `original`, `beads` or
`stepped`; geometry itself is already realized in the typed spans.
`reference_bounds_mm` is `{min:[x,y,z], max:[x,y,z]}` in build millimeters with
ordered finite components. Studio/floor extent derives from these bounds through
the shared studio recipe, not arbitrary SDK camera fitting.

`appearance` retains every shared `RenderAppearance` serde field: `color`
([r,g,b]), `roughness`, `material_preset`, `studio_preset`, `resin_clarity`,
`resin_cloudiness`, `layer_appearance`, `first_layer_height_mm`, `layer_height_mm`,
`groove_depth_mm`, `display_rotation_degrees` ([x,y,z]). Materials below are the
authoritative physical coefficients. Display rotation is retained as provenance;
the resolved model matrix already applies it. Layer shaders use preserved build
coordinates; never repeat the import or display transform. Layer appearance is
valid only for original geometry.

`materials` is an array of at most 256 shared `PhysicalMaterial` records. Fields
are `name`, `base_color_linear` ([r,g,b]), `absorption_color_linear` ([r,g,b]),
`roughness`, `metallic`, `anisotropy`, `coat`, `coat_roughness`, `transmission`,
`ior`, `absorption_density_per_mm`, `scattering_per_mm`,
`scattering_anisotropy`. Colors/fractions are [0,1], anisotropies [-1,1], IOR is
positive, density/scattering nonnegative. Triangle material indices address this
array; they are not duplicated in JSON. Empty geometry/material arrays are valid
when no triangle refers to a material. Native allocation expansion must separately
respect budgets; host checks span bytes plus metadata against the host budget and
span bytes against the GPU budget.

`layer_filter` is `{enabled:boolean, fade_start_pixels:2.0, fade_end_pixels:4.0,
footprint_diagonal_factor:1.4142135623730951}`. These are the existing smoothstep
amplitude fade thresholds on projected layer spacing, using the conservative
diagonal/grazing footprint. Physical groove pitch/depth and cap masking remain
unchanged. These version1 constants are validated; they do not invent a physical
fade distance or authorize a visual support claim.

`color` is `{schema_version:1, exposure_ev:0.0, tone_map:string,
output_transfer:"srgb", output_alpha:"straight"}`. Filament's recipe ID is
`filament-agx-v1`; LuxCore's is `turninator-agx-v1`. The host rejects another
backend's ID and unsupported exposure/recipe values. Native producers perform
HDR tone mapping before the 8-bit boundary and encode display sRGB exactly once.
Shared AgX constants/chart, Filament's native AgX settings and producer alpha
normalization still require qualification in their respective tasks. Merely
carrying these IDs does not implement their effects.

Describe JSON requires `schema_version:1`, `backend`, `sdk_version`, `device_id`,
`device_name`, `api`, `driver` (nonempty strings up to 4096 bytes, no control
characters). Production backend IDs match the selection; only explicit fixture
loads accept `fixture`. Optional `compiler` is null/omitted or
`{name, version, sha256, path}`: name/version are nonempty <=256-byte strings,
sha256 is 64 hex digits and path is absolute. Bridges report actual loaded module
identity; the adapter must verify it against the lock and record it in effective
`BackendIdentity`. Describe is also permitted for Failed diagnostics.

An optional `diagnostics` object carries observational statistics. Older bridges
may omit it; explicit null is invalid. It requires exactly `schema_version:1`,
`counters`, `timings_ms`, `settings`, and `artifacts`. All four maps may be empty
and contain at most 64 entries each. Keys match `[a-z][a-z0-9_.-]*`, with at most
128 ASCII bytes. Counter values are exact unsigned 64-bit JSON integers (floats,
booleans and negative values are rejected). Timings are finite, nonnegative
numbers in milliseconds. Settings are nonempty UTF-8 strings up to 1024 bytes,
without control characters. Artifacts have exactly `{path, sha256}`: a path up
to 4096 UTF-8 bytes and a digest of exactly 64 hexadecimal digits. Unknown fields
in the diagnostics object or artifact records are rejected. The complete native
description, including diagnostics, remains bounded to 1 MiB. Both receivers
reject duplicate diagnostic map keys; Rust bounds each map while deserializing.

Artifact paths follow the reviewed lexical Windows drive/UNC/verbatim policy:
drive-absolute and server/share UNC paths are accepted, including canonical
verbatim drive/UNC paths. Relative, dot/traversal, device namespace, forbidden
Windows characters and C0 controls are rejected; verbatim paths reject forward
slashes. Legal DEL/C1 filename characters remain valid. Native and Rust receivers
perform no filesystem existence/metadata reads for these observations. Producers
report actual loaded modules, and the adapter separately verifies their identity
against the lock. Diagnostics do not establish module trust.

The dedicated SDK worker calls its existing `describe()` initially and after
each successfully drained work item, including cooperative cancellation, before
publishing Ready/Cancelled. Backends retain their static module hashes and
identity; this does not require a new SDK probe or hash pass per frame. The
facade validates each candidate, compares all stable description fields against
the initial description with only diagnostics excluded, and publishes the whole
string under a separate snapshot mutex. Owner-thread describe copies that
snapshot and never calls SDK code. Invalid or changed identity fails only after
backend drain and preserves the last valid snapshot. SDK exceptions retain their
original failure and do not trigger a refresh. Poll/cancel acquire no new locks;
the snapshot lock is never held with the task queue lock.

Description length can change between capacity query and copy. Native short
buffers remain untouched and receive the current required size; a successful
shorter snapshot reports its actual size. Rust accepts only bounded, internally
consistent lengths/statuses, retries a growing `TN_CAPACITY` at most four copy
attempts, and truncates a successful shorter snapshot before parsing. Diagnostics
are excluded from stable runtime/pixel identity. Effective quality policy still
belongs to the backend's explicit recipe, even when diagnostics report actual
sample counts, effective AA, resource reservations/reuse, or phase timings.

## DLL search and remaining production identity checks

Production load accepts only a canonical absolute
`exe_dir/renderers/{filament|luxcore}/tn_renderer_{backend}.dll`. Redirected
backend directories, traversal/symlink escapes, other names and fixture identity
are rejected. Installation owns these directories; canonicalization does not
prove an ACL or prevent an administrator from changing files concurrently.
The test-only loader accepts only the separate `renderers/fixtures` directory.
An optional missing DLL or dependency is an error affecting the selected backend.

Before the first valid-path optional load, the host permanently calls
`SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32)`. It never restores an
ambient search policy. The explicit bridge load additionally specifies
`LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32`, allowing its
initial transitive imports only beside that bridge or in System32. Later bare
SDK `LoadLibrary` calls cannot search cwd, PATH, the executable directory or user
DLL directories. This is a process-wide policy: other application dynamic loaders
must use explicit absolute paths/search flags. Existing app/desktop behavior with
production SDKs must be checked during integration.

Pinned `deps/cuew/src/cuew.cpp:625` tries bare `nvrtc64_130_0.dll` before
`nvrtc64_120_0.dll`; this Windows source does not read CUDA_PATH. Task5a's initial
probe found ambient NVRTC 13.1; final controlled probes confirmed bundled NVRTC
12.8.93. Host search flags alone do not select or verify a lazy compiler. A
production LuxCore bridge must explicitly preload the pinned compiler/builtins
from its canonical backend directory, retain those module handles through SDK
destruction, and verify actual loaded module paths/hashes. Fail on an incompatible
already-loaded module or System32 version; never silently vary runtime identity.
The host policy cannot constrain SDK-supplied absolute paths, already-loaded
same-name modules or a later component changing the process search policy.
Actual SDK module conformance remains a production qualification gate.

## LuxCore utility workaround and C++ ABI review

The official sample ZIP has a DLL/import library but no public headers; the
matching source archive supplies headers. Its import library omits eight needed
Properties utility symbols. Task5a compiled exact unmodified `properties.cpp`
and `matrix4x4.cpp` locally alongside the import library. Both source hashes,
matching header inventories, Boost version/header, `exportdefs.h` and cuew source
are pinned. Without `LUXCORE_DLL`, `CPP_API` is empty; local C++ definitions satisfy
those utilities, while import-library thunks resolve the DLL's exported SDK APIs.

This is a feasibility workaround, not an accepted ABI boundary. Properties holds
`std::map<std::string, unique_ptr<Property>>`; Property contains string/vector
storage and Blob owns `unique_ptr<char[]>`. Inline destructors and returned
`unique_ptr` values can free DLL-originated data inside the bridge; defining local
utilities duplicates definitions also present inside the DLL. Matching headers,
Boost 1.88, `/MD`, C++20, MSVC 194x, iterator-debug level and compile flags are
necessary but do not prove allocator/layout/ODR safety. The dependency build used
19.44.35228.0, slightly newer than the qualified probe compiler. Full allocation
and exception lifecycle qualification, a precise build-flag audit, and preferably
a corrected narrow upstream SDK export build remain required. Do not enable a
production build on the strength of successful linking alone.

Copy mesh data into `Scene::AllocVerticesBuffer` / `AllocTrianglesBuffer` buffers,
including Embree padding; never transfer Rust allocations to LuxCore. Every
bridge entry catches C++ exceptions. Avoid the pinned GPU path's
`WaitForDone()` followed by ordinary `Stop()`/destruction: Task5a reproduced a
double join and fail-fast crash. Readiness must establish full per-pixel coverage
and synchronized film state. Finite 8/64-spp probes demonstrated minimum per-pixel
coverage with overshoot, not exact sample counts or deterministic accumulation.

## Qualification still required

Filament's 64x64 Vulkan probe and LuxCore's native CUDA/OptiX probes establish
narrow feasibility only. Production bridge state/cancel/readback behavior,
cold compilation cancellation, resource bounds, geometry/material/layer/volume
conversion, alpha edges, color transfer, actual loaded module identity, selected
device failure, larger outputs and repeated switching require native SDK tests.
Task4 shared geometry/pipeline integration is also pending. Fixture images cannot
be used for quality, throughput, five-backend completion or availability claims.

Redistribution is unfinished: Filament static dependency notices (including
Abseil/Zstd/helper code), complete LuxCore/OIDN notices, Microsoft CRT rights and
exact NVIDIA NVRTC/builtins payload/license terms need a packaging audit.
`nvrtc64_120_0.alt.dll` is pinned but its packaging role remains unresolved.
CUDA/OpenCL/OptiX driver libraries are system prerequisites. No lock entry grants
redistribution rights or asserts that the official sample ZIP is a complete SDK.

## Task5c common native lifecycle integration

Workers7/8 must build against `native/common/backend.hpp` and the shared
`tn_native_core` target. In each backend CMake directory call
`tn_add_bridge(tn_renderer_filament "${CMAKE_CURRENT_SOURCE_DIR}/backend.cpp")`
(or `tn_renderer_luxcore`), then add that target's qualified SDK includes, links
and compile definitions. The helper links `tn_native_core` and compiles the same
`common/exports.cpp` nine-export facade. Do not implement a second ABI, parser,
state machine or owner-thread wrapper. Production builds return the matching
`TN_BACKEND_FILAMENT`/`TN_BACKEND_LUXCORE` and flags `0`; fixture builds alone
return `TN_BACKEND_FIXTURE` and `TN_FLAG_TEST_FIXTURE`. No production bridge is
implemented or registered by Task5c.

Implement these internal C++ functions in namespace `tn`:

```cpp
uint32_t backend_id() noexcept;
uint32_t backend_flags() noexcept;
std::unique_ptr<Backend> make_backend(const Config&);
```

`backend_id`/`backend_flags` are constant descriptors queried without creating an
SDK. The factory, `Backend::describe`, `upload`, `render` and `~Backend` all run
on one dedicated thread per context. `create` waits for factory and validated
actual description; SDK initialization may block there. The Rust-facing opaque
handle still accepts calls only on the creating Rust thread. The backend object
never crosses threads, including on factory/description/upload/render failures.
`describe` returns actual schema1 runtime JSON once; subsequent ABI describe
calls copy cached immutable bytes, including after a later Failed transition.

`Config` owns the complete validated config JSON (`document`), backend string,
and checked duplicated C host/GPU/output/metadata bounds. `Scene` owns aligned
`positions_mm`, `normals`, `build_positions_mm`, flat `triangles` and
`triangle_materials` vectors, `revision` and validated `metadata` JSON. Vertex
and triangle counts are vector sizes divided by three. `Frame::parameters` is
an owned scalar `tn_frame`; its `quality_json` is null, and `Frame::quality`
owns `{schema_version, quality}`. Consume those exact shared field names with
typed JSON access; do not substitute defaults or reapply display rotation.
These internal C++ values and SDK objects never cross ABI boundaries.

`upload` in Idle validates and copies a complete immutable snapshot; repeated
Idle uploads retain only the latest snapshot. Actual SDK upload is lazy on the
first frame using it. An internal upload generation invalidates SDK geometry
even if a caller reuses the numeric revision. Transform-only subsequent frames
reuse uploaded SDK geometry. `begin` validates/copies frame input and enqueues
one task; it never waits for SDK upload, compilation, rendering or readback.
Rendering/Ready/Cancelling reject new upload/begin calls, so there is no task
queue growth. Invalid input preserves the current revision and state.

`Work` contains references to cancellation and completed/total atomics. Backend
work may update numeric progress and check `work.cancelled()` between bounded
operations. Do not retain `Work`, input references or SDK callbacks beyond the
operation; copy CPU spans into SDK-owned buffers when the SDK retains them.
An upload interrupted by cancellation is not marked reusable. Every successful
`render` returns a tightly packed top-first display-sRGB straight-alpha RGBA8
vector of exactly `width*height*4` bytes. The core checks that count and zeros
RGB for zero-alpha pixels; tone mapping, row/channel conversion, unpremultiplying
and sRGB encoding remain backend responsibilities. No partial image crosses ABI1.
SDK-specific allocations/expansion must independently respect the config budget;
the common guards validate supplied span bytes/metadata and output image bytes,
not undocumented SDK memory estimates.

The backend must drain SDK operations, readbacks and callbacks before `render`
returns, including cancellation/error exits. Its noexcept destructor must also
drain any remaining SDK work and catch SDK teardown exceptions. The core keeps
active owned frame/scene input alive through backend destruction on exceptions.
An SDK exception destroys the backend on its worker before publishing Failed;
the original bounded diagnostic remains readable through poll. Failed rejects
upload/begin/read/cancel; cached describe and destroy remain available.

Poll/cancel call no SDK functions, wait on no locks and join no threads; they
only inspect/update lock-free atomics and bounded context data. A pending cancel
first enters Cancelling. Only worker completion after drain publishes the
internal Cancelled acknowledgement. Cancel of an already Ready image can be
acknowledged immediately because readback has already drained. The first poll
emits Cancelled while restoring Idle; it is never inferred from elapsed time or
number of polls. Ready/Cancelled publication uses acquire/release state ordering.
Too-small image/description retries set required size, write no caller bytes,
and retain the current state. Destruction requests cancellation and joins the
worker before returning, allowing the Rust RAII owner to unload the DLL safely.
A blocked driver/compiler can still delay create/destroy indefinitely.

The single schema1 parser is official MIT `nlohmann/json` v3.12.0, vendored under
`native/vendor/nlohmann`. Its annotated tag resolves to immutable commit
`55f93686c01528224f448c19128836e7df245f72`. Exact official source/notice URLs and
SHA-256 hashes are pinned in `sdk-lock.json.common_parser`; CMake checks both
files before any fixture or production compilation. Preserve the complete
`LICENSE.MIT` in distribution notices. Builds and runtime never download it.
The common guards reject malformed/invalid UTF-8, duplicate keys, nesting beyond
64 levels, unknown/missing schema1 fields, mistyped numeric/boolean/string data,
unsupported recipes, invalid cameras/affine matrices, physical material/range
errors, indices, nonfinite spans and byte/count overruns. Source-count overrides
remain upstream only; derived caps stay 2,000,000 vertices/4,000,000 triangles.

The deterministic fixture uses this exact core, retains the fault/dependency
variants, and fingerprints all16 model-matrix double bit patterns with explicit
alpha/seed. Its controllably pending C++ test covers one worker thread, copied
input, latest snapshot, upload reuse, bounded task count, atomic progress,
nonblocking poll/cancel, drain-before-cancel acknowledgement, failure cleanup,
buffer retry and join-before-return. These CPU tests qualify the common lifecycle
and transport only. Production SDK blocking, cancellation, memory, quality,
color/alpha, device selection and module teardown remain workers7/8 gates.
