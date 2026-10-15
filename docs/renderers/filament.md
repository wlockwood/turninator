# Native Filament bridge

Task 7a implements a Windows x64 C++ bridge against Filament 1.78.0, commit
`c1dc4e444784850d6dc891a1fb38747c30b85463`. The Rust adapter, runtime registry,
interactive transport and packaging belong to subsequent integration tasks.
The existence of this target does not make the renderer available in the app.

Build with the qualified immutable SDK root:

```powershell
cmake -S native -B artifacts/renderers/task7a/build -G "Visual Studio 17 2022" -A x64 `
  -DTN_BACKEND=filament -DTN_FILAMENT_CONFORMANCE=ON -DTN_SDK_ROOT=<qualified-filament-root>
cmake --build artifacts/renderers/task7a/build --config Release --parallel 4
```

CMake validates the existing lock's matc 78/cmgen and fourteen static libraries, the
complete qualified SDK header inventory, and the additional build-only headers
in `native/filament/vendor-manifest.json`. The official SDK omits BlueVK/Vulkan
headers required by its public VulkanPlatform API; these are exact files from
the same Filament commit. Their source URLs, byte counts, SHA256 and Apache 2.0
license are retained. No system Vulkan SDK headers replace them.
The license is the exact pinned root `LICENSE`: 11,362 LF bytes, with its
upstream source URL and Git blob recorded in the vendor manifest.

Only the build launches matc/cmgen. Surface, resin and unlit material packages are
embedded in the DLL; rendering requires no shader-tool executable, subprocess,
asset-path search or download. Link inputs are filament, backend, bluegl,
bluevk, filabridge, filaflat, utils, geometry, smol-v, ibl, abseil, zstd, image and ktxreader from
the qualified `/MD` SDK. Task 10 must reconcile the actual link inputs with the
Task5a4 full transitive notice inventory, including static helper libraries.

The bridge uses the frozen common nine-export facade. That facade owns one
worker thread, copies inputs and provides nonblocking poll/cancel. Filament
objects and teardown remain on that worker. Upload and readback descriptors
retain native-owned storage until callbacks drain, including exception exits.
`flushAndWait()` plus `pumpMessageQueues()` drain the threaded SDK;
`Engine::execute()` is inappropriate for this Windows build.

Input positions, camera, clip planes and matrix translation convert from
millimetres to metres once. The SDK world remains right-handed Z up. Normals
become tangent-frame quaternions, and a separate custom vertex attribute retains
build XYZ in millimetres and original normalZ for posed layer shading. The
row-major input pose becomes a column-major SDK matrix. The bridge does not fit
cameras, reapply import/display rotations, clip geometry, derive edges, generate
caps or schedule layers.

Geometry uploads are lazy. Transform-only frames reuse buffers; an upload always
replaces the snapshot even when its numeric revision matches an earlier upload.
Triangle material indices become stable primitive ranges. The approved optional
physical-material `unlit` flag selects an SDK unlit material for prepared ink.
This flag requires the corresponding Task4b common-validator refinement.

Recipe1 uses native `AgxToneMapper(NONE)`, high quality color grading, EV 0,
Rec709/sRGB/D65 and camera exposure multiplier1. Readback is top-first RGBA8.
Filament's pinned color-grading shaders encode output and then premultiply
RGB by alpha; the bridge unpremultiplies in that display-encoded domain and
zeros RGB at alpha 0. It does not apply a second sRGB transfer.

| Selection | AA | Shadows and AO |
| --- | --- | --- |
| Low | MSAA1 | PCF1024, full-resolution SAO LOW |
| Medium | MSAA4 | PCF2048, full-resolution SAO MEDIUM |
| High | MSAA4 plus FXAA | PCF4096, full-resolution SAO HIGH |

The actual physical device's color/depth sample masks must support the requested
MSAA count, including combined sampled HDR/depth format support. The bridge
rejects an unsupported count. Custom resolve is explicitly false: this pinned
Vulkan implementation does not support frame-buffer-fetch MSAA. A qualified
High request selects 4x MSAA plus FXAA from the outset. The abandoned 8x MSAA probe
failed SDK readback despite advertised hardware support; the bridge never tries
it and then reduces quality. All profiles disable TAA,
dithering and dynamic accumulation; each pose gets a new View, with no
history shared across poses. Filtered PCF provides approximate soft studio
shadows. Studio lighting uses three fixed directional lights and a compiled
specular/diffuse environment: `studio.py` creates an analytic 256 x 128 linear HDR
with three softened rectangular studio panels, and the exact pinned SDK cmgen
prefilters a 64-pixel cube with seven roughness mips plus matching nine-coefficient SH.
The embedded KTX uses linear R11F_G11F_B10F. Rendering binds both the reflection
cubemap and its SH. There are no external photo assets or runtime environment
tools. Material sphere fixtures verify metal, roughness, silk anisotropy and
clearcoat, including a metallic-only response to changed IBL intensity while
directional lights remain fixed. The floor derives from full source bounds and
is excluded for transparent output and diagram studio.

Original geometry's layer normal perturbation uses the exact first/later
heights, physical groove depth, buildZ relative to fixed full-source minZ and cap mask. The metadata's2→4pixel
smoothstep and sqrt2 diagonal/grazing footprint attenuate amplitude without
changing pitch. Bead and stepped geometry cannot receive repeated shader
grooves.

Raster resin is approximate. The material uses Filament screen-space refraction,
transmission, IOR and absorption, with the smallest full-bounds extent as optical
thickness. Absorption converts coefficients per millimetre to per metre.
Nonzero scattering contributes extinction but does not implement directional or
multiple volume scattering. Background coverage remains independent of material
transmission. The Rust capability descriptor must retain this approximation
reason, including unavailable offscreen refraction and overlapping-interface
limitations.

Device IDs are `vulkan:<vendor-id>:<device-id>:<physical-device-uuid>`.
An explicit selection is enumerated before Engine creation, passed through the
public GPUPreference index and verified against the actual initialized physical
device. Name and driver strings come from VulkanPlatform's initialized context.
The System32 Vulkan loader is loaded by an absolute path, checked and retained
until Engine destruction. Runtime driver modules remain system prerequisites.

Checked budgets include aligned vertex/quaternion/build attributes, callback
copies, sorted index data, both scene snapshots, output/readback, MSAA color and
depth, resolved HDR/post/AO/refraction/mip targets and shadow maps. The GPU bound
charges current targets plus three distinct recent target configurations for
the pinned SDK's texture cache. Host reservation additionally charges that GPU
bound again as SDK/WDDM backing, with 256 MiB baseline; observed native commitment
is much larger than the explicit CPU pixel arrays. Conservative reservations
are not exact driver VRAM measurements. The 4096 High qualification used 8 GiB host
and 10 GiB GPU budgets; a 1 GiB GPU budget fails explicitly. There is no resolution or quality reduction and no
internal tile renderer.

One checked estimator composes these reservations before upload copies,
realization staging and frame allocations. During realization, two scene
snapshots plus staging and callback copies charge 160 bytes per vertex and
56 bytes per triangle, alongside current/retained GPU backing and baselines.
Transform-only frames omit the drained upload staging. Old scene capacities
and buffers are released before reserving a replacement scene.

CPU-only checks run independently of SDK/GPU work:

```powershell
cmake -S native/filament/tests -B artifacts/renderers/task7a/cpu -G "Visual Studio 17 2022" -A x64
cmake --build artifacts/renderers/task7a/cpu --config Release --parallel 4
ctest --test-dir artifacts/renderers/task7a/cpu -C Release --output-on-failure
```

The tests cover peak allocation arithmetic without allocating the maximum mesh
and compare production alpha normalization with an independent asymmetric
3-by-2 literal pixel oracle. A real CABI asymmetric colored-marker fixture also
checks actual SDK top-first row order and RGBA channels.

The real CABI caller qualifies frozen 512-square cube/hollow silhouettes, 4096 High,
Low/Medium/High settings, posed/negative-origin/first-layer/fade behavior,
mapped colors, studio casts, resin black/white composites, 259 unlit materials,
same-revision replacement, 64 orbit frames, cancellation/readback reuse,
explicit/missing device selection and pending destruction followed by actual
DLL unload/reload. The persistent task report records exact commands, RED/GREEN,
artifact hashes, observed memory and remaining limits. These native checks do
not establish Rust output-format integration, packaged availability or
interactive performance; those remain later tasks.
