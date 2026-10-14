# Native viewport implementation — final-plan Task 4

Scope: native GPU viewport, physical camera, layer shader and viewport tests. Root owns Cargo, library/UI wiring, worker integration and final runtime launch. No other agent is spawned.

Shared camera convention: rotate source positions by Euler XYZ, center the actual rotated XY extents and place minimum rotated Z at zero; spin around studio Z afterward. Camera target/direction/up use this centered displayed frame, matching the Blender worker. `Viewport::display_bounds` supplies bounds for fitting; raw source bounds must not be used after display centering. Custom camera JSON carries target_mm, direction (target to camera), up, projection, distance_mm, vertical_fov_degrees and ortho_span_mm.

## API and implementation

`OrbitCamera` is Clone/Serialize/Deserialize and carries yaw/pitch degrees, distance/target millimeters, projection mode, vertical FOV and vertical orthographic span. `CanonicalView::{Front,Right,Top,Iso}`, `fit`, `fit_aspect`, `orbit`, `pan`, `zoom`, `validate`, `project`, and `camera_spec` are public. Front is -Y/+Z up, Right is +X/+Z up, Top is +Z/+Y up. Fit uses a conservative bounding sphere with 15% margin, so arbitrary turntable angles fit. Use `fit_aspect` for portrait viewports; the default fit assumes square composition.

`Viewport::{new,set_mesh,show,display_bounds,clear_mesh,texture_size,triangle_count}` and `ViewportSettings` match the parent interface. Geometry kinds currently accepted are original/beads/stepped. The revision identifier is the caller's immutable mesh generation: reusing it skips buffer upload. Upload validates source indices/counts, finite f32 coordinates, triangles surviving f32 conversion and adapter buffer limits before replacing existing buffers. An invalid replacement preserves the previous GPU mesh. Unload releases geometry buffers and CPU positions.

The viewport uses persistent indexed position/triangle/triangle-edge buffers. It renders into an egui-registered RGBA8 texture at the viewport's physical pixel dimensions, capped to 8192 or adapter limits, with a matching Depth32Float target. Resizing replaces color/depth targets and updates the existing texture registration. Drop releases that registration. Native drawing uses queue submission, with no readback, blocking poll or CPU triangle painting. Test-only readback is separate.

The WGSL shader uses original build Z minus source minimum Z, independent of display orientation and turntable spin. It applies the first interval height separately from subsequent physical pitch, the same cosine groove derivative as the Blender material, geometric cap attenuation and a smooth 0-to-1 footprint weight over 2-to-4 pixels per layer. Layer settings follow the supported 0.02–2 mm interval and <=25% depth bounds. Layer shading is enabled only for original geometry; real bead surfaces remain geometric. Orthographic specular viewing direction is parallel. Initial normals are triangle-flat; this is an intentional first native implementation, without crease smoothing or Blender material parity.

Triangle edges are an explicit debug mesh overlay. They are not inferred design-feature edges and are unsuitable as final four-view design edges. The first native viewport is opaque, single-sample and neutrally lit. OIT/resin inspection, section caps, buildup and comparison remain subsequent plan tasks. No 1M-triangle/60fps performance claim is made from the correctness test.

## Test-first and runtime evidence

1. Camera axes, finite controls and full-rotation fit tests were written against failing method stubs. The initial temporary rustc harness failed all three while root dependencies were still building. Once worker module placeholders existed, `cargo test --offline --test viewport` also failed all three against the same stubs. After camera implementation, all three passed.
2. Added physical layer-setting contract test first. `cargo test --offline --test viewport layer_settings_follow` failed because excessive groove depth was accepted. After supported height/depth constraints, it passed. Final `cargo test --offline --test viewport`: **4/4 passed**.
3. The actual GPU test initially exposed a WGSL reserved `target` identifier and unsupported LineList depth bias, neither visible to Rust compilation. Renamed the WGSL field and biased triangle depth instead. The GPU pipeline then validated and rendered real pixels.
4. Added f32-collapse replacement regression before its guard. The actual GPU test failed on the expected precision-collapse assertion. After per-triangle f32 representability validation, it passed while preserving the prior two-triangle mesh.
5. `cargo test --offline --lib gpu_render_layers_resize_and_unload -- --ignored --nocapture`: **1/1 passed**, explicitly executed on **NVIDIA GeForce RTX 4070 Ti, Vulkan, NVIDIA driver 595.79**. Latest observed test time 0.76s (correctness workload, not an FPS measurement). Validated actual shader/pipeline creation, >10,000 model pixels, resolved 0.2mm layer shading, pixel-identical suppression below resolution, independently changing first height, display-rotated build-layer shading, depth occlusion independent of submission order, unchanged horizontal/reversed caps, edge pipeline, target resize, invalid replacement preservation, and empty output after unload. Zero captured wgpu validation errors. This hardware test is ignored by the ordinary suite so CPU-only environments remain usable; it must be run explicitly for runtime verification.
6. `cargo clippy --offline --lib --test viewport -- -D warnings`: **passed**. `rustfmt --edition 2024 src/viewport/mod.rs src/viewport/camera.rs src/viewport/gpu.rs tests/viewport.rs` completed. Root owns the final whole-workspace checks and live native window smoke.

No renderer/root/UI files or Cargo metadata were edited for this viewport slice. Hardware-rendered pixel assertions establish the offscreen renderer's behavior; they do not establish desktop input behavior, end-to-end worker integration, sustained GPU memory usage or throughput. Those remain root integration/runtime checks.

## Task 6 native inspection follow-up

The baseline was held unchanged until root captured and staged the desktop milestone. The following additions preserve all default rendering behavior:

- `SectionPlane { axis:u8, position_mm:f32, keep_positive:bool }` and `ViewportSettings.section: Option<SectionPlane>` clip original build coordinates. The plane follows both display pose and turntable rotation automatically. The native view is an **uncapped cutaway** of existing faces; it does not construct a solid envelope, generate infill or close air gaps. Renderer-side exact Boolean caps are a separate capability.
- `buildup_height_mm: Option<f32>` clips at build Z. `buildup_complete_layers` snaps to configured first/uniform subsequent boundaries. Adaptive schedules should supply their already-selected physical boundary with uniform snapping disabled; the viewport does not infer adaptive layers.
- `InspectionMode::{Opaque,Ghost,Xray}` and `opacity` select explicit inspection transparency. Ghost uses a bounded depth-weighted sum; Xray uses a constant weight and half the selected opacity. Neither is a resin material/refraction/physical absorption prediction.
- Transparent rendering uses additive RGBA16Float accumulation plus multiplicative R16Float revealage, then a fullscreen linear-light composite. Triangle sorting and CPU rasterization are unnecessary. OIT textures allocate lazily and resize with the native targets. Opaque depth behavior is unchanged. In transparent mode the optional triangle-wire overlay includes internal edges explicitly.
- `set_triangle_colors(&[[f32;3]])` assigns linear RGB by original source face order; empty input restores the single scene color. Only vertices crossing color boundaries are duplicated, with at most six million colored vertices and adapter buffer-size guards. Invalid color count/components or expansion budgets preserve the previous GPU geometry. The root must call this on palette/mesh changes, not every frame.
- `resource_bytes()` estimates explicitly allocated geometry buffers and viewport textures, excluding driver overhead. Geometry unload drops both source face metadata and GPU color/position/index/edge buffers.

Inspection-setting tests were written first and failed for invalid axes/planes/opacity; validation then passed. The actual GPU inspection test was written before OIT and failed because Ghost pixels equaled opaque output. After OIT implementation, it passes: rear green/blue groups contribute through a red front surface; reversing all source triangle/color order changes no channel by more than two 8-bit values; sections remove the expected half of a model before and after display rotation; complete-layer build-up correctly differs from a partial layer; sub-first-layer output is empty; gaps between separate actual stadium beads remain background in Opaque/Ghost/Xray; the retained bead remains visible; resized OIT and its edge overlay validate with no GPU errors.

Follow-up commands:

- `cargo test --offline --test viewport`: **5/5 passed**.
- `cargo test --offline --lib gpu_ -- --ignored --nocapture`: **2/2 actual-GPU correctness tests passed**, 0.80s observed total on RTX 4070 Ti/Vulkan/595.79.
- Owned viewport files are rustfmt formatted. Last targeted Clippy attempt reported only root's concurrent app.rs debounce `collapsible_if`, with no viewport diagnostics; root owns final full-tree strict Clippy.

### Measured dense-bead throughput

`cargo test --offline --lib dense_bead_viewport_benchmark -- --ignored --nocapture` passed. The generated workload is 10,000 independent infill beads across 100 layers, **1,040,000 triangles**, rendered at **2560×1440** for 30 distinct angles per mode. It uses the real persistent-buffer renderer. Each timed frame includes CPU command encoding/submission and waiting for GPU completion. One warm frame precedes each mode. The Rust test uses the unoptimized test profile.

| Mode | Median frame | P95 frame | Explicit GPU allocation |
| --- | ---: | ---: | ---: |
| Opaque | 4.787 ms | 5.204 ms | 76.19 MiB |
| Ghost | 5.389 ms | 6.519 ms | 111.35 MiB |
| Xray | 5.085 ms | 6.110 ms | 111.35 MiB |

Initial validation/buffer upload took **390.28 ms**. These offscreen results support the native geometry throughput target on this adapter. They exclude egui painting/compositing, presentation, desktop input latency, source import, shader initialization, driver bookkeeping and long-duration memory behavior; they are not an observed desktop FPS claim. Weighted transparency is approximate for dense overlapping/internal bead surfaces, and RGBA16 accumulation introduces bounded order-dependent rounding.
## Whole deposited-layer face visibility follow-up

Added `Viewport::set_triangle_layer_tops(&[f32]) -> Result<()>`. Values follow the original mesh triangle order and specify the actual top of each face's deposited layer in build millimeters. Empty input restores the earlier continuous build-Z plane clip. Values must match the face count and be finite in `0..1e7`; failed assignments preserve existing GPU resources. Mesh replacement/unload clears both color and layer metadata.

The persistent per-vertex attribute stream now packs linear RGB plus layer top (16 bytes). Vertex remapping keys include original vertex index, all three color channels, and layer top, so shared vertices split only at differing face attributes. The existing six-million expanded-vertex cap and adapter buffer limits remain enforced. Color changes retain layer metadata and layer changes retain colors. Build-up discards the entire future-layer triangle in opaque, edge, and OIT passes, preventing a future bead's bottom cap from surviving exactly at the previous layer boundary. Section clipping remains independent in build coordinates. Call the setter when imported geometry changes; the selected height remains a uniform-only frame update.

TDD evidence:

- RED: `cargo test --offline --lib gpu_layer_faces_preserve_caps_and_palette -- --ignored --nocapture` failed with missing `set_triangle_layer_tops` before implementation.
- GREEN: the same command passed on the actual GPU. It compares rendered pixels against an independently reduced mesh containing only the deposited first-layer triangles, while retaining source positions for identical camera/display transforms. It covers adaptive bead heights `.2` then `.3`, a selected top of `.2`, the future bead's boundary bottom cap, Opaque/Ghost/X-ray, display rotation, triangle edges, palette and metadata reset/reapplication, invalid lengths/NaN, and unload.
- `cargo test --offline --lib gpu_ -- --ignored --nocapture`: all three GPU regressions passed, zero validation errors; RTX 4070 Ti / Vulkan, NVIDIA 595.79; 0.99 seconds test runtime.
- Owned `gpu.rs` formatting completed. Full strict Clippy reached only the concurrent root-owned `app.rs` large `Message` enum warning after `ImportedModel` was expanded; root notified. No viewport warning was reported.

Earlier dense-scene timings above describe the preceding 12-byte color attribute stream. This follow-up adds four GPU bytes per expanded vertex plus retained CPU face metadata; those historical allocation/timing figures are not a new benchmark of this revision.
