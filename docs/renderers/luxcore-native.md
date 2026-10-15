# LuxCore native bridge

Windows x64 CUDA/OptiX path tracing runs through the shared ABI1 facade in-process. No Blender, LuxCore CLI, helper process, runtime downloads or CPU fallback are used. The production bridge is `renderers/luxcore/tn_renderer_luxcore.dll`.

The pinned corrected LuxCore 2.11.2 SDK is source commit `b872186d2fe995a762f73708c606847eed55843e`, SDK DLL SHA256 `63e96b108e5007c6d378e2ac0b575d5db6b9e9e8b7b9a69a5b7cef3856f457b5`. `native/patches/luxcore/qualification.json` and the closed source build recipe describe its inputs. NVRTC 12.8.93 and its matching builtins, OIDN 2.5.0 CPU, Embree and TBB are verified by absolute owned path and file hash before SDK initialization. Driver identity reports the actual Windows driver file version and loaded CUDA/OptiX module hashes; CUDA compatibility version alone is insufficient pixel identity.

## Building

Run `native/luxcore/build.ps1 -SdkRoot <qualified-sdk-root> -OpenMpRuntime <licensed-x64-vcomp140.dll>`. The OpenMP input is Visual Studio 14.44.35112 x64 redistributable SHA256 `55aba23cdcd6484fbb06f4155b8ca75adfce7a881f10afd0c49457165e677164`. The caller must have redistribution rights under Microsoft's terms; packaging records the originating redistributable. The script writes only its artifacts directory and records actual build inputs. Core Rust builds do not require these SDK files.

## Ownership and quality

The application must retain an idle native context and its verified library on the export actor until shutdown. It has no scene or GPU context. Actual job contexts retain independent renderers on one dedicated SDK thread through `OwnedRenderer`; their destruction releases GPU resources while the idle context leases the owner and runtime libraries. The last proxy joins the SDK owner before library unload. No OS module pin or DLL static-destructor thread join is used. SDK work is serialized; the application has one active LuxCore export and no LuxCore interactive viewport. A changed package requires restarting its owning actor/application.

This lifetime is required by measured SDK behavior: full load/unload alone retained approximately 25 MB of live heap allocations per cycle, and separate SDK owner threads retained a timer/semaphore pair per context. Holding only OpenCL, NVRTC, or the SDK DLL did not close both issues. Twenty fresh-scene jobs on one owner were stable. The final leased-owner test destroys twenty independent GPU contexts with constant handle counts and zero WDDM GPU residency after every job; releasing the idle lease removes all SDK workers before actual DLL unload. Temporary SDK cleanup experiments were discarded; production uses the original qualified SDK hash.

Requested samples are a minimum for every output pixel. The versioned AA dimension is `ceil(sqrt(samples)) + 1`; actual sample-count film values are checked before publishing. For request 64, the qualified run used AA9 with actual minimum80/maximum82. Distinct poses reset film accumulation; explicit instance transforms prevent cumulative source-mesh transforms. The Rust adapter must supply a positive seed derived from the complete immutable pixel recipe, including its resolved frame plan.

Source geometry, physical normals, build coordinates and absorption coefficients remain in millimeters. Shared prepared geometry supplies caps, material mapping and sheet ink. Canonical light/dark/diagram studios use fixed 64-segment disks and the common world recipe. HDR output uses the shared AgX transform; camera alpha is normalized separately and linear radiance is unpremultiplied before AgX. OIDN High RT runs with separately prefiltered albedo/normal guides, then clean-guide beauty filtering; alpha is preserved independently.

## Qualified limits

Opaque transmission/absorption/scattering and raw physical-layer normals are supported. Transparent resin is a compositing approximation: use an opaque studio background for consistent refraction. One RGBA image cannot encode background-independent spectral transmission and refraction.

Denoised fine-layer animation is also approximate. Canonical-lighting captures retain over99% of layer contrast, correlation over.9999 and mean profile error below.47 byte, but fail the stricter temporal residual criterion. Disable denoising when stable fine-layer contrast is essential. The failed motion checker is retained; its threshold was not relaxed and no shimmer-free denoised-layer guarantee is made. Still-image denoising and raw layer phase/normals have separate tests.

Native functional qualification includes canonical worlds/shadows, opaque Beer-Lambert slabs at1/10 mm, scattering, 259 materials, exact unlit replacement, empty geometry, independent alpha, resin, A/B/A, progressive cancellation and restart, pending destruction and unavailable devices. Sampled lighting replacement uses the same two-code numerical tolerance as A/B/A; exact unlit replacement isolates cache invalidation without stochastic lighting or AgX highlight-ratio assumptions.

A 4096-square,524288-triangle,64-requested-sample OIDN render completed with actual minimum80 samples, approximately4.58 GB peak private memory and661 MB measured WDDM residency. It used explicit16 GiB host/GPU allowances; those allowances are not physical VRAM claims. Large-to-small release, final CPU RGBA handoff and actual DLL unload passed. These are functional diagnostics, not controlled benchmark timings. The conservative host reservation for4096 OIDN requires a higher host allowance than the application's default8 GiB.

The Rust adapter, common-format exports, application UI and final extracted-package/process/benchmark qualification are downstream work; native C ABI proof alone does not establish those gates.
