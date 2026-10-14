# Turninator: Revised Design and Implementation Plan

> **For implementation:** Use `superpowers:executing-plans` to work through the tasks sequentially. Use subagents only if the user requests that execution method. This document is the requested planning deliverable; application implementation starts with a separate request.

**Goal:** Make a local Windows application that turns print models into attractive looping GIFs, high-resolution stills, and four-view sheets, with an optional print appearance that uses the actual chosen layer height.

**Architecture:** Rust owns the desktop UI, canonical geometry, physical units, print setup, cameras, and export jobs. A modest wgpu viewport provides immediate interaction. Blender provides authoritative rendered previews and exports; FFmpeg encodes animations. Print-surface parameters belong to the shared scene description, so they work across materials, output modes, and saved projects.

**Tech stack:** One Rust package using eframe/egui, its compatible wgpu types, glam, serde/serde_json, image, rayon, rfd, tracing, and Windows process APIs; application-owned Python scripts running in Blender; FFmpeg/ffprobe. Resolve and lock exact Rust dependencies during implementation.

**Spec:** The design and acceptance requirements are embedded below. Reviewed against `idea-opus.md` and `idea-codex.md` on October 14, 2026. No application, layer shader, render benchmark, or GPU-capability probe has been implemented as part of this review.

## 1. Recommended path

**Keep Codex's Rust application plus Blender renderer, and bring Opus's physically scaled layer appearance into the first useful release.** Layer rendering is a central feature, rather than something deferred behind comparison tools or packaging polish.

This is a print-model photo studio with an inspection view. Its distinguishing feature should be that choosing “PLA, 0.20 mm layers” produces a plausible printed surface without requiring Blender knowledge. That does not require owning a path tracer.

| Approach | Strength | Main cost | Decision |
| --- | --- | --- | --- |
| Rust UI and Blender worker | Established material/rendering capabilities, immediate native interaction, controlled workflow | Two preview paths, external tool setup | Recommended |
| Fully native path tracer | One appearance renderer and tighter distribution | Integrator, acceleration structures, materials, denoising, transparency, animation stability, driver support | Reconsider after the product works and measurements justify it |
| Blender add-on | Fastest route to a rendering prototype | User works inside Blender | Use Blender scripts for the initial proof, not the final application UI |

The native-tracer proposal understates the work needed for reliable resin, correct smooth shading, denoiser integration, and stable animation. A software BVH or raw Vulkan fallback is another substantial graphics implementation, not a small contingency. Current wgpu documentation still identifies ray queries as experimental and lists Vulkan as their supported platform; DX12 is not an equivalent documented route. This is a dependency risk, not proof that native tracing is impossible. [wgpu feature documentation](https://docs.rs/wgpu/latest/wgpu/struct.Features.html), [wgpu ray-tracing specification](https://github.com/gfx-rs/wgpu/blob/trunk/docs/api-specs/ray_tracing.md)

Blender can implement the layer surface using a procedural height function and bump mapping. Its Bump node perturbs the shading normal from a height texture; this is sufficient for the first appearance mode. The proposed node group still needs a live EEVEE/Cycles proof. [Blender Bump node](https://docs.blender.org/manual/en/4.5/render/shader_nodes/vector/bump.html)

Preserve these useful ideas from Opus:

- Layer spacing expressed in millimeters, with a user-selected value.
- Print orientation independent of display orientation and turntable rotation.
- A rounded groove/bead profile rather than painted horizontal stripes.
- Stable surface filtering when layers become smaller than a pixel.
- Camera-relative lighting for diagrams, fixed studio lights for turntables.

Preserve these safeguards from Codex:

- Canonical geometry and cameras shared between native and Blender views.
- Explicit units, crease-aware normals, and warnings instead of silent mesh repair.
- An actual rendered preview that exposes the final material appearance.
- Immutable export snapshots, tracked worker cancellation, and reusable completed frames.
- Decode-based export checks plus visual review of complete animations.

Correct both plans' optimistic assumptions: STL has no unit or printer-setting metadata; source Z is only an initial print-axis assumption; closed-looking geometry is not necessarily a valid solid; and sample-rate estimates do not establish export time or temporal stability.

## 2. Scope and verified starting point

The current folder contains the two proposals and no implementation or Git repository. Create only this revised plan now; do not initialize Git, install tools, or scaffold the app during the review.

Read-only local checks found:

| Tool | Current evidence | Initial action during implementation |
| --- | --- | --- |
| Blender | `C:\Program Files\Blender Foundation\Blender 4.5\blender.exe` reports **4.5.2 LTS** | Use this explicit path for the first proof |
| FFmpeg | `C:\Program Files\ffmpeg\bin\ffmpeg.exe`, build dated **2022-05-23** | Probe required filters/encoders before use |
| ffprobe | Present beside FFmpeg, same dated build | Verify encoded dimensions and stream timing |
| Rust | `cargo.exe` is available | Resolve compatible dependencies and record toolchain |

Blender is not on PATH. Discover conventional installation directories as well as configured paths and PATH. Existing FFmpeg help confirms Bayer and Sierra dithering, GIF infinite looping, and explicit final-frame delay. It does not prove the complete encoding pipeline works.

Begin with installed Blender 4.5.2 for the proof. Before declaring a supported release, move to a selected maintained LTS patch and rerun the same probes. The official LTS page currently lists 4.5.14 and 5.2.2; choose one tested build, rather than supporting both series immediately. Installation/updating is future implementation work. [Blender LTS releases](https://www.blender.org/download/lts/)

Initial platform: Windows x64. Runtime work is local and offline. Keep one GPU render worker. The proposals identify an RTX 4070 Ti, Ryzen 9 9950X, and 96 GB RAM as the target; verify device, driver, and memory during the proof rather than treating those details as this review's live hardware measurement.

## 3. The user workflow

1. Drop an STL or use Open. Retain the previous model while loading and show progress.
2. Review “Units: millimeters (assumed)” and set the print orientation if needed.
3. Choose a finish, color, and studio. Enable **Printed surface** to preview layer appearance.
4. Enter **Layer height**, initially **0.20 mm** for FDM. Optionally set a different first-layer height.
5. Orbit or choose a view. Use separate presentation controls to tilt the part for a better shot without changing how it was printed.
6. Request **Rendered Preview** for the authoritative appearance, then export **Looping GIF**, **Still Image**, or **Four-View Sheet**.

Keep the viewport prominent, model/print/finish controls in a compact sidebar, and output/progress below. Advanced settings remain collapsed. No Blender nodes, sample-bounce terminology, or process details in the normal workflow.

The viewport has two clear states: **Interactive View** and **Rendered Preview**. The interactive view approximates studio highlights but uses the same layer spacing, print axis, geometry, and camera. Changing settings marks the last rendered image “Out of date”; keep it visible with an Updating indicator until a replacement succeeds. Manual rendered-preview refresh is the initial default.

Keep **Printed surface**, **See-through inspection**, and **Translucent resin** separate. The first changes the visual surface; the second helps inspect geometry; the third is an optical material. Turning off layer appearance should not change the selected finish, dimensions, or print orientation.

## 4. Layer rendering: the core feature

### 4.1 Controls and defaults

| Control | Initial behavior |
| --- | --- |
| Printed surface | Off for clean CAD/clay views; on in FDM finish presets |
| Layer height | Numeric field; FDM default **0.20 mm** |
| Quick values | 0.08, 0.12, 0.16, 0.20, 0.24, 0.28, 0.32 mm; arbitrary valid values remain accepted |
| First layer | “Same as other layers” by default; optional separate height |
| Layer visibility | Subtle / Normal / Emphasized; changes groove strength, never spacing |
| Advanced groove depth | Physical millimeters; initial tuning value **0.01 mm** at 0.20 mm pitch |
| Print orientation | Source Z-up initially; axis buttons and 90-degree rotations, with a build-axis indicator |
| Presentation rotation | Separate from print setup; reset independently |

The starting validation range for layer and first-layer height is **0.02–2.00 mm**, inclusive. This is an application limit, not a printer recommendation. Accept finite values only, and prevent the scene from generating more than **50,000 layers**. Groove depth starts at 5% of the selected pitch and is bounded to **0–25% of the smallest active layer height**. These are visualization tuning constraints, not measured extrusion physics.

Preset selection must not overwrite a layer height the user has explicitly chosen. Track whether the print setup was edited. An unedited setup may adopt a material's default: FDM 0.20 mm, resin 0.05 mm, clay/metal layers disabled. Switching among finishes does not imply that the actual print process changed; an explicit Reset Print Settings action restores defaults.

### 4.2 Distinguish variable settings from adaptive layers

Support arbitrary **uniform** layer height and a different **first layer** in the first useful release. This covers the user's stated varying height without requiring a slicer.

Design the data contract to accept an explicit list of layer boundaries later. That will cover adaptive/variable-height jobs, but the first UI does not pretend to extract those settings from an STL. A future CSV or slicer integration must provide the actual schedule; do not infer it from mesh shape or generic G-code Z moves, which may include travel and Z-hop.

For example, a 0.30 mm first layer followed by 0.20 mm layers gives boundaries at `0.00, 0.30, 0.50, 0.70, ... mm`. A later adaptive schedule might explicitly supply `0.00, 0.20, 0.40, 0.52, 0.64, ... mm`. Store boundaries in physical print coordinates and require finite, strictly increasing heights starting at zero. The final interval may extend beyond the highest mesh point; geometry clips the appearance naturally.

### 4.3 Physical coordinates and transforms

Use three distinct coordinate spaces:

```text
Source coordinates
    -> unit conversion + source-origin rebasing
Physical model coordinates, millimeters
    -> PrintPose: build rotation + translation onto the bed
Build coordinates, millimeters, +Z is the print axis
    -> DisplayPose: presentation rotation/placement + turntable angle
Studio/world coordinates
    -> camera projection
Image
```

Define `PrintPose` and `DisplayPose` as rigid transforms; only unit conversion changes scale. The build bed is `z = 0`. Print setup centers XY and places the minimum build Z at zero. Presentation tilt may subsequently place the displayed part on a studio floor; that floor placement has no effect on layer phase.

Evaluate the layer profile from **build-space Z before presentation and animation transforms**. Bake the print pose into derived renderer positions, or carry the equivalent immutable build transform. Blender's local mesh coordinates should remain in that build frame, with presentation and spin applied at the object level. Never drive layers from world Z or bounding-box-normalized Generated coordinates. Generated coordinates span 0–1 over the mesh bounds, which would make the requested physical pitch depend on model height. [Blender texture coordinates](https://docs.blender.org/manual/en/latest/render/shader_nodes/input/texture_coordinate.html)

Use a fixed **0.001 meters per millimeter** conversion into Blender. Positions, groove depth, lights, camera distances, and clipping distances all respect that convention. Native rendering may rebase further for precision but must retain the physical build Z used by the material. Do not normalize each model to an independent unit cube.

Examples that must remain true:

- A 20 mm-high part at 0.20 mm pitch has 100 layer intervals whether the camera is close or far.
- Rotating the presentation 90 degrees rotates the grooves with the part.
- Changing the print orientation 90 degrees changes which surfaces carry the grooves.
- Translating a source STL or a displayed part does not shift the bed-relative layer pattern.
- Changing units changes the physical model size; the chosen 0.20 mm layer height stays 0.20 mm.

### 4.4 First implementation: shaded layer grooves

For an interval `[b_i, b_{i+1})`, define:

```text
h = b_(i+1) - b_i
u = clamp((z_build - b_i) / h, 0, 1)
height_mm = -0.5 * groove_depth_mm * (1 + cos(2*pi*u))
```

This produces a smooth valley at the boundaries and a rounded crest at the layer center. It is an initial visual profile, not a claim about a particular nozzle's bead cross-section. Separate first-layer height uses a two-branch phase calculation; uniform layers use arithmetic rather than a large shader graph.

Implement an application-owned Blender node group in `renderer/layers.py`: local build-space Z converted to millimeters -> layer phase -> rounded height -> Bump -> Principled normal. Convert height back to Blender meters before bump evaluation. Use a side-surface factor derived from the **unperturbed geometric normal** relative to build +Z: `w = 1 - abs(dot(n_build, Z))^2`. Bump should affect walls and slopes while vanishing on horizontal caps. Verify that masking does not create artificial edge seams; smooth the transition if captures require it.

Mirror this profile in the native fragment shader with the same constants. Perturb its shading normal using the surface gradient of the filtered height, in a consistent build-space basis. No layer triangles or mesh rebuild are needed when changing uniform pitch or groove strength. Unit or print-pose changes invalidate derived build geometry; presentation/camera changes do not.

Start without random layer offsets. After stable loops are proven, add a small optional variation in groove strength/roughness keyed by layer index and a saved seed. It must be identical across frames and must not move layer boundaries.

### 4.5 Resolution and animation filtering

At 960 pixels, a 200 mm-tall part filling 80% of the image provides approximately 3.84 pixels/mm. Its 0.20 mm layers are only **0.768 pixels apart** even before foreshortening. Literal bands at that frequency can shimmer or form moire patterns. More path-tracing samples or a denoiser does not automatically fix this.

In the native shader, calculate the pixel footprint from build-Z screen derivatives. Start with an amplitude fade that is zero at **two or fewer pixels per layer** and reaches full strength at **four pixels per layer**. Between those points, use a smooth transition. This conservative cutoff avoids drawing unresolved periodic detail at or below the Nyquist limit. Fade perturbation toward its mean; never enlarge the physical pitch to force visibility. Horizontal caps already have a zero side weight.

Blender Bump filtering is not automatically identical to native filtering. During the initial proof, compare the node's filter behavior and **2x linear-resolution supersampling followed by linear-light downsampling**, keeping color and alpha handling correct. For nearly periodic grooves, prototype a filtered baked normal/height texture if the node approach still aliases. Treat the validated filtering recipe as part of each quality preset and job fingerprint.

An engine that cannot show stable layers at the requested size must use a validated smoother filtered mode or a different engine. Do not silently disable the effect. The preview/export UI may explain “Layers are finer than this image can resolve; use a close-up or larger output.” Emphasized mode strengthens resolvable grooves and is labeled as an appearance exaggeration.

Inspect the raw rendered sequence **and the encoded GIF**. Palette conversion can destroy detail or introduce shimmer that is absent in the source frames. Save close-up 4K references alongside GIF-scale comparisons for 0.12, 0.20, and 0.28 mm settings.

### 4.6 What this mode represents, and the later geometry option

Name the initial mode **Layer appearance**. It changes highlights and apparent grooves while preserving the STL silhouette and dimensions. It does not recreate stair-stepped slopes, top/bottom extrusion paths, seams, supports, infill, overhang sag, or a printer's dimensional errors. A resin preset likewise gives a plausible finish, not calibrated resin behavior.

Make **Stepped geometry** a separate later feature if close-up realism needs it. Prove it on a narrow supported class first: closed, consistently oriented solids whose horizontal slices produce valid contours. At each scheduled layer, intersect triangles with a defined sample plane, assemble closed planar contours with holes, extrude that section through the layer interval, and construct the boundary of the joined layer solids. Export from a disposable derived mesh. This represents a sliced-envelope approximation, still not individual extrusion toolpaths.

That work requires robust contour stitching, degeneracy handling, hole/cavity topology, contour boolean operations, and a choice of slice sampling convention. Arbitrary adaptive layers add more cases. Dense sampling, voxel stair-stepping, or indiscriminate Blender displacement should not be advertised as the accurate answer without demonstrating thin-wall and silhouette preservation.

Do not make this geometry option a dependency of the first release. Keep its output and cache separate from the original STL, and measure its expanded memory cost before extending beyond proof fixtures.

## 5. Geometry, materials, cameras, and outputs

### Geometry and inspection

Parse ASCII/binary STL once in Rust. Distinguish formats without relying on a `solid` header. Reject invalid counts, truncation, non-finite coordinates, and empty usable geometry. Report omitted zero-area triangles. Compute bounds/rebasing in f64 before GPU f32 conversion; rebasing cannot restore precision already lost in the source.

Use exact-coordinate adjacency, not bounding-box-relative tolerance welding that could merge small gaps. Preserve winding and disconnected shells. Offer Faceted and Smooth Curves shading, initially with a 30-degree crease threshold; derive shared corner normals. Neither mode improves a coarse STL silhouette. Check boundary edges, non-manifold edges, and winding consistency; edge-use counts alone do not prove freedom from self-intersections or a valid solid.

Native controls: orbit/pan/zoom, fit/reset, front/right/top/isometric, projection switch, axis indicator, neutral shaded and edge views. Later in the same complete release: see-through transparency, an uncapped section plane, and two-model comparison with linked cameras and preserved physical scale. These inspection modes do not change studio exports. Defer mass estimation, automatic lay-flat, repair, and automated registration.

### Finishes and studios

Initial finishes: matte PLA, satin/glossy plastic, silk-style plastic, neutral clay, tinted resin. Each has Color and Gloss where meaningful; resin has Tint, Clarity, and Cloudiness. Layer parameters are independent of material parameters. Two bundled procedural studios, Light and Dark, avoid required HDRI downloads. Fix exposure and use one selected AgX/sRGB configuration for rendered outputs.

Use **EEVEE for fast opaque rendered previews/GIFs** if the layer-animation proof passes. Use **Cycles with verified OptiX for quality stills and resin**, and offer it for GIFs. Compare EEVEE with low-sample Cycles before locking the default. Blender documents OptiX support and RTX ray-tracing acceleration; actual device selection must be probed and recorded. [Blender GPU rendering](https://docs.blender.org/UATEST/manual/en/4.5/render/cycles/gpu_rendering.html)

Starting tuning values: EEVEE 32 samples for small previews, 64 for output; Cycles 32 samples for previews, 256 maximum/adaptive sampling for stills, and an initially tested 64-sample opaque GIF candidate. These are experiments, not guarantees. Benchmark resin separately, including nested surfaces, bounces, denoising, and open-shell warnings. No native resin path tracer or bundled OIDN integration.

### Turntable GIF

- Default **960 x 960, 4 seconds, 20 fps, 80 frames**; Compact 640 square and Large 1280 square.
- Angles `start + 360*i/80` for `i = 0..79`; no duplicated endpoint, holds, easing, or motion blur by default.
- Rotate the displayed object around a stable studio-Z pivot at its XY bounds center. Keep lights and camera fixed. Layers follow the object, regardless of its presentation tilt.
- Fit the entire rotational envelope with 10% margins, fixed camera/exposure, and no per-frame zoom.
- Flatten onto an opaque background; generate one palette from the entire display-ready frame sequence.
- Start with FFmpeg `palettegen` using `stats_mode=full` and `reserve_transparent=0`, then `paletteuse` with `bayer_scale=3`. Compare Bayer against `sierra2_4a` for layer retention and temporal stability before selecting the default. [FFmpeg palette filters](https://www.ffmpeg.org/ffmpeg-filters.html#palettegen)
- Set infinite looping and a **5-centisecond delay for every frame**, including the final frame. Validate decoded delays, not just a nominal fps field. [FFmpeg GIF options](https://www.ffmpeg.org/ffmpeg-formats.html#gif-2)
- Show the actual encoded animation and file size. Offer MP4 from the same final frames as a secondary full-color alternative. No transparent GIF initially.

Supersampled working frames are downsampled before palette generation, so a quality setting does not secretly change final dimensions. Encoding failure should reuse valid rendered frames.

### Still image

Default **4096 x 4096 PNG**, current composed camera, Cycles/OptiX quality preset. Also 2048 square and 3840 x 2160. Bound custom output at **64 megapixels**, with a separate memory/disk preflight that includes supersampling and denoising buffers. This pixel limit alone does not guarantee a job fits GPU memory.

Opaque or transparent background; transparent mode removes the floor initially. Keep material transmission separate from output alpha. Allow a close-up camera so actual layer details can be inspected. Compare denoised and undenoised fine grooves, small text, holes, and thin walls before choosing the final default. Export display-ready PNG first; EXR and contact sheets are later options.

### Four-view sheet

Use a labeled 2x2 shape sheet: Front/Right on the first row, Top/Isometric on the second. All four cameras are orthographic, with one shared pixels-per-millimeter scale. Canonical views refer to the **print-oriented build frame**, independent of presentation tilt. Use a Print/Presentation view-frame selector later only if needed; the initial diagram has one predictable convention.

Front looks from -Y with +Z up; Right from +X with +Z up; Top from +Z with +Y up; Isometric from `(1, -1, 1)` with projected +Z up. Center on the same build bounds. Rotate the diagram light rig with each camera. No floor, depth of field, or hidden independent rescaling.

Default **4096 x 4096**, outer margins 64 px, gutters 64 px, label bands 96 px per panel, giving **1952 x 1856 render areas**. The 2048 preset scales all layout dimensions by half. Fit projected extents with 10% margins.

The default sheet uses clean neutral shading with layer appearance disabled. An explicit **Show printed surface** option uses the selected pitch/finish. This prevents tiny simulated grooves from obscuring design edges. Silhouette/crease lines should come from unperturbed geometry normals/depth or explicit topology, not from the bumped layer normal. Otherwise a normal-based edge pass could turn every groove into a false engineering line.

Choose the edge method in the proof: test renderer passes first; if unavailable/reliable only in another engine, use a depth-tested visible-edge overlay from canonical topology, excluding coplanar triangulation edges. Do not assume Freestyle works with EEVEE.

Optional metadata: filename and oriented overall dimensions in selected units. No mass estimates, tolerance claims, or engineering-drawing standards in the first release.

## 6. Shared data and worker design

### Data contracts

Use distinct `MeshId`, `SceneRevision`, `RequestId`, and `JobId` types. Define these owning types before their consumers:

| Type | Required content |
| --- | --- |
| `MeshData` | Indexed rebased positions, triangle indices, corner normals, source f64 origin/bounds, content hash, topology warnings |
| `Units` | Finite positive millimeters per source unit; mm/inch/m/custom |
| `PrintPose` / `DisplayPose` | Separate rigid transforms; print pose includes bed placement |
| `PrintSurfaceSpec` | Enabled, `LayerScheduleSpec`, groove depth mm, visibility multiplier, optional saved variation seed, schema version |
| `LayerScheduleSpec` | `Uniform { height_mm, first_layer_height_mm: Option<f64> }`; reserved `Explicit { boundaries_mm: Vec<f64> }` for later schedule input |
| `LayerSchedule` | Validated derived boundaries covering the build bounds, with interval/depth constraints enforced |
| `ModelSpec` | Mesh reference, units, print/display poses, per-model print surface |
| `CameraSpec` | Camera basis, target, projection/span or field of view, clipping distances |
| `SceneSpec` | Version, models/selection, camera, material, studio, smoothing mode |
| `ExportSpec` | Tagged GIF/Still/Diagram/MP4 options, quality/filtering recipe, dimensions, destination |
| `ToolPaths` | Validated Blender/FFmpeg/ffprobe executable paths, exact builds, and probed capabilities |
| `WorkerRequest` / `WorkerEvent` | Versioned request/event enums with request, scene revision, job identity, and typed payloads |
| `JobManifest` | Immutable scene/mesh hashes, tool/build/device/shader versions, frame integrity, output-stage state |
| `ProjectFile` | Schema version, source paths/hashes, scene, inspection/comparison state, export presets |

The first UI accepts Uniform schedules only. Explicit is versioned for later support; a project containing it must receive a clear unsupported-feature error until that implementation exists. Do not silently substitute a uniform height.

Cache native mesh uploads by content/normal settings, derived build geometry by units/print pose, and appearance by print-surface/material revision. Export resume fingerprints include the **entire print surface, print/display transforms, filtering recipe, seed, and renderer scripts**, not merely a preset name. A different 0.20/0.24 mm pitch must invalidate cached appearance and frame reuse.

Transfer canonical geometry in a private versioned `meshbin`: checked counts, little-endian positions/indices/corner normals, exact sizes, finite values, index-range checks. Scene units/transforms/surface settings travel in versioned JSON. Rust and Python readers must agree on winding and normal mapping. Blender uses its bundled Python; normal users do not need another Python installation.

### Worker and jobs

Launch one persistent hidden Blender worker with factory settings, auto-execution disabled, and only the application's script. Rust sends versioned line-delimited requests through stdin. An input thread queues commands; the Blender main thread performs all bpy operations. Return prefixed `TURNINATOR_EVENT ` JSON events among ordinary diagnostic logs.

Probe, Preview, Export, Shutdown carry request and scene/job identity. Discard stale completions. Optional automatic preview later uses 300 ms debounce and latest-only coalescing. Final export owns the worker until completion; native interaction continues, subject to GPU contention.

The first proof must establish that a main-thread command loop works in Blender background mode with both EEVEE and Cycles on a desktop Windows session. A one-shot script proves rendering only; it does not prove the persistent protocol. If the command loop fails, resolve that boundary before integrating a GUI, using a persistent scene cache plus one worker process per job if necessary.

Use a Windows Job Object for the tracked worker/process tree. Cancellation stops that tree and joins readers; it cannot rely on a cancel command queued behind a blocking bpy render. Restart when needed. Track FFmpeg processes separately and clean them up on cancellation/exit. Never terminate unrelated Blender processes.

Each job owns immutable mesh/settings snapshots, manifest, logs, frames, and validated completion records. Write frames completely, decode/check them, then record completion. Recovery rerenders only missing/invalid frames when all fingerprints match. Retain work after cancellation or encoding failure. Keep one GPU job active and estimate frame storage, including supersampling intermediates; a nominal 20 GB job-cache budget requires explicit retention/discard choices for interrupted jobs.

Publish a decoded/validated output using a temporary file in the destination directory and a platform-correct atomic replacement/rename operation. Preserve existing output on failure. Use process argument arrays and numbered job-owned frame paths; test spaces, Unicode, brackets, and percent signs. Do not embed arbitrary paths in shell commands or FFmpeg pattern syntax.

## 7. Proof before product implementation

The first milestone is deliberately a rendering experiment, with no GUI. Prove these items using application-owned Blender scripts and generated STL fixtures:

1. Installed Blender path, actual GPU/device selection, EEVEE background rendering, and Cycles/OptiX rendering.
2. Layer grooves at 0.12, 0.20, and 0.28 mm, plus 0.30 mm first layer followed by 0.20 mm.
3. Print-axis independence from presentation tilt and turntable rotation.
4. Complete 80-frame loops through both source-frame playback and GIF encoding.
5. A 4096-square close-up, a resin scene with internal surfaces, and a correctly oriented four-view sheet with geometric edges.
6. Persistent request loop, cancellation/restart, and a second warm render without reimporting unchanged geometry.

Fixtures: known-height rectangular block, capped cylinder, curved shell/sphere, 45-degree wedge, bracket with narrow gaps/holes/text, tall eccentric part, asymmetric XYZ shape, nested shells, open/reversed variants, and 100k/1M/5M-triangle meshes. Use deterministic generated fixtures; add real user STLs when available.

The wedge demonstrates the distinction between shaded grooves and actual stair-step geometry. The cylinder checks cap masking. The XYZ fixture checks camera handedness. Narrow gaps expose harmful welding, smoothing, denoising, or future generated geometry.

Measure cold startup, warm preparation, mesh/normal work, upload, shader compilation, render median/p95, denoising, supersampling/downsampling, frame writes, palette generation, encoding, total time, file size, and peak system/GPU memory where measurable. Record tool hashes/builds, driver/device, quality settings, and other GPU load. Never replace end-to-end measurements with a samples/second estimate.

| Scenario | Initial tuning target, not a promise |
| --- | --- |
| Native orbit at 1440p, <=1M triangles, no export | Approximately 60 fps |
| 100k-triangle import to native view | Under 2 seconds excluding cold app startup |
| Warm 768-square opaque rendered preview with layers | Under 2 seconds |
| 960-square, 80-frame opaque GIF with layers | Under 2 minutes including encoding |
| 4096-square opaque Cycles still | Under 2 minutes including denoising/write |
| 4096-square four-view sheet | Under 1 minute including composition |

The proof ends with a measured recommendation: selected Blender build, default engine/filtering recipe for opaque layers, Cycles resin quality, diagram-edge implementation, real output artifacts, and observed limitations. If the targets are missed, tune resolution/sampling/filtering presets and update the plan from measurements. A missed target does not itself justify replacing Blender with an unmeasured custom path tracer.

## 8. Proposed implementation structure

Start with one package and modules with clear ownership:

```text
Cargo.toml / Cargo.lock
src/main.rs                         Application entry point
src/app.rs                          UI state, revisions, event application
src/error.rs                        Typed failures and user-facing summaries
src/model/{mod,stl,topology}.rs      Import, canonical mesh, adjacency/normals
src/scene/{mod,camera,transforms}.rs Physical scene, print/display poses, cameras
src/print/{mod,schedule,surface}.rs  Height validation, schedules, shared profile
src/viewport/{mod,gpu,inspect}.rs    Interactive shading and inspection
src/viewport/shaders/*.wgsl         Shared-profile layer shading and view passes
src/render/{mod,protocol,worker}.rs  Tool discovery, process lifetime, messages
src/export/{mod,jobs,gif,still,diagram,video}.rs
src/project.rs / src/settings.rs    Project data and machine-local tool paths
renderer/worker.py                  Main-thread command execution/events
renderer/{mesh,scene,materials,layers,cameras,render}.py
assets/presets/*.json               Versioned finishes/studios/quality presets
assets/fonts/                      Bundled labels font and notices
tools/{generate-fixtures,benchmark-render}.py
tools/check-runtime.ps1             Selected live probes and artifact checks
tests/{import,print_surface,framing,protocol,jobs,gif,diagram,project}.rs
tests/render_probe.py               Blender runtime/regression checks
docs/performance.md                 Measured results
docs/visual-validation.md           Artifact comparisons and limits
README.md                          Setup and supported tool builds
```

## 9. Review focus and global constraints

Every task inherits the design requirements above. In particular: source STLs are read-only; UI updates never block on import/render/encoding; stale work cannot replace newer state; units stay physical; jobs snapshot settings; geometry/appearance errors are reported; no silent engine/quality substitutions; and visual claims need actual captures.

These five cases need explicit validation:

1. **Layer pitch and transform mistakes:** mm/inch conversion, source offsets, bed phase, first layer, print rotation versus display rotation. Tasks 1, 3, 6.
2. **Fine-layer aliasing and detail loss:** curved/sloped surfaces, distant views, denoising, GIF palette conversion. Tasks 1, 6, 7.
3. **Damaged or ambiguous geometry:** binary `solid` headers, invalid counts, winding, open/nested shells, thin gaps, cap masking. Tasks 1, 2, 6, 9.
4. **Job/preview races and failures:** rapid edits, changed pitch, cancellation, crashes, corrupted frame, full disk, existing destination. Tasks 4, 5, 7, 10.
5. **Diagram/inspection misunderstanding:** shared scale, view orientation, presentation-independent diagram frame, layer normals mistaken for edges, effects excluded from exports. Tasks 3, 8, 9.

## 10. Implementation tasks

Implement sequentially. Each task has a checkable deliverable. Write substantive regression checks before their implementation; run the focused check to demonstrate the failure, implement, then rerun. Runtime/appearance checks supplement unit tests. Use focused commits if Git has been initialized for implementation; stage only the task's files and inspect the staged diff.

### Task 1: Prove layer rendering and tool boundaries

**Files:** `tools/generate-fixtures.py`, `tools/benchmark-render.py`, `renderer/{layers,materials,scene,cameras,render,worker}.py`, `tests/render_probe.py`, `docs/{performance,visual-validation}.md`.

**Interface:** `benchmark-render.py --blender <exe> --ffmpeg <exe> --output <directory>`; machine-readable results record settings, tool/device details, per-stage timings, and artifact paths. Run bpy scripts through the selected Blender executable.

- [ ] Generate deterministic fixtures with known dimensions and asymmetric orientation.
- [ ] Add runtime assertions for image sizes, 100 intervals across a 20 mm build height at 0.20 mm pitch, first-layer phase, transformed build coordinates, and 80 decoded GIF delays totaling 4 seconds.
- [ ] Implement the minimal owned node group and render scripts required by Section 7.
- [ ] Render/encode both engine candidates; inspect full loops, close-ups, slope/cap behavior, and diagram edges. Record the selected filtering/engine settings and timings.
- [ ] Prove persistent main-thread request handling and tracked cancellation/restart, rather than testing only one-shot rendering.
- [ ] Stop GUI integration until a supported rendering/filtering path is demonstrated. Revise this plan if the proof changes the chosen recipe.

**Deliverable:** Actual GIF, 4K close-up, resin image, four-view sheet, and evidence-backed backend/quality choice. Initial Python prototypes become maintained worker modules only after their inputs are brought into the shared schema.

### Task 2: Implement canonical STL import

**Files:** `Cargo.toml`, `src/error.rs`, `src/model/{mod,stl,topology}.rs`, `tests/import.rs`.

**Interfaces:** `load_stl(path: &Path) -> Result<MeshData, AppError>`; `derive_corner_normals(mesh: &MeshData, crease_degrees: f32) -> Result<Vec<[f32; 3]>, AppError>`; `write_mesh_snapshot(mesh: &MeshData, destination: &Path) -> Result<(), AppError>`.

- [ ] Add ASCII/binary equivalence, binary `solid` header, truncated/overflow counts, empty/non-finite geometry, zero-area reporting, disconnected shells, winding warnings, and huge-offset/tiny-gap fixtures.
- [ ] Run `cargo test --test import`; establish failures before implementing the behavior.
- [ ] Implement checked parsing, f64 bounds/rebasing, content hashing, exact-coordinate adjacency, shared corner normals, and checked meshbin writing/reading.
- [ ] Rerun import checks and measure representative dense imports; verify no source files change.

**Deliverable:** Reliable shared mesh with physical metadata and visible warnings.

### Task 3: Implement print setup, layers, and camera math

**Files:** `src/print/{mod,schedule,surface}.rs`, `src/scene/{mod,transforms,camera}.rs`, `tests/{print_surface,framing}.rs`.

**Interfaces:** `build_layer_schedule(spec: &LayerScheduleSpec, build_height_mm: f64) -> Result<LayerSchedule, AppError>`; `sample_layer_profile(schedule: &LayerSchedule, surface: &PrintSurfaceSpec, z_build_mm: f64) -> LayerSample`; `build_model_transform(model: &ModelSpec) -> Result<BuildTransform, AppError>`; `fit_turntable(bounds: &ModelBounds, aspect: f64, margin: f64) -> CameraSpec`; `diagram_cameras(bounds: &ModelBounds, panel_size: [u32; 2], margin: f64) -> [CameraSpec; 4]`.

`LayerSample` contains layer index, interval height, local phase, height perturbation, and derivative. `BuildTransform` maps rebased source coordinates into bed-relative millimeters. `ModelBounds` carries build and displayed bounds. Diagram cameras consume build bounds; turntable fitting consumes displayed bounds.

- [ ] Assert uniform boundaries `0, 0.2, 0.4`, first-layer boundaries `0, 0.3, 0.5`, 100 intervals at 20 mm, inclusive height limits, rejected NaN/zero and excessive layer counts, groove-depth bounds, and deterministic profile values.
- [ ] Assert unit conversion, source-translation invariance, independent print/display rotations, shared diagram scale, canonical axes, and no clipping through a full rotation of an elongated tilted part.
- [ ] Run `cargo test --test print_surface` and `cargo test --test framing` to expose incorrect behavior; implement the exact contracts and rerun.
- [ ] Add schema behavior for reserved Explicit schedules: clear unsupported-feature error in the first release, never substitution.

**Deliverable:** Tested physical print setup and view math, ready for both render paths.

### Task 4: Build the interactive application

**Files:** `src/{main,app,settings}.rs`, `src/viewport/{mod,gpu}.rs`, `src/viewport/shaders/*.wgsl`, `tests/framing.rs`.

**Consumes:** `MeshData`, `ModelSpec`, scene/camera transforms from Task 3. **Produces:** revision-tagged scene updates and an interactive viewport; GPU resources remain separate from serializable state.

- [ ] Implement drop/Open, asynchronous import, units/print-pose controls, independent presentation controls, orbit/pan/zoom, view presets, fit/reset, and axis indicators.
- [ ] Render neutral shading with shared normals and physical camera conventions using eframe's compatible wgpu device.
- [ ] Add deterministic delayed-import/event checks showing an older completion cannot replace a newer selection.
- [ ] Launch `cargo run --release`; validate behavior visibly, including the previous model remaining visible during import. Measure 100k/1M/5M fixtures at 1440p without an export.

**Deliverable:** Responsive model setup/inspection application. Studio fidelity remains the Blender preview's responsibility.

### Task 5: Integrate the worker and durable render jobs

**Files:** `src/render/{mod,protocol,worker}.rs`, `src/export/{mod,jobs}.rs`, `renderer/{worker,mesh,scene,cameras,render}.py`, `tests/{protocol,jobs}.rs`, `tests/render_probe.py`.

**Interfaces:** `WorkerClient::start(tools: &ToolPaths) -> Result<WorkerClient, AppError>`; `submit(&mut self, request: WorkerRequest) -> Result<(), AppError>`; `cancel_active(&mut self) -> Result<(), AppError>`; `create_job(scene: &SceneSpec, export: &ExportSpec, root: &Path) -> Result<JobManifest, AppError>`.

- [ ] Add fake-worker checks for protocol mismatch, mixed logs/events, stale revisions, partial messages/writes, crashes, cancellation, and fingerprint changes when pitch/pose/shader changes.
- [ ] Run `cargo test --test protocol` and `cargo test --test jobs`; implement checked mesh transfer, owned process-tree lifetime, immutable jobs, frame completion records, and stream readers.
- [ ] Add the manual Rendered Preview action with stale/update status, preserving the last successful image on error.
- [ ] Run live requests against the validated build; cancel during preparation and rendering, restart, verify no unrelated process is stopped, and compare native/Blender camera silhouettes on the XYZ fixture.

**Deliverable:** Authoritative Blender preview and recoverable job foundation.

### Task 6: Ship finishes and configurable print appearance

**Files:** `assets/presets/*.json`, `renderer/{materials,layers,scene}.py`, `src/print/surface.rs`, `src/viewport/shaders/*.wgsl`, `src/app.rs`, `tests/print_surface.rs`, `tests/render_probe.py`, `docs/visual-validation.md`.

**Consumes:** `PrintSurfaceSpec`, Uniform schedules, rigid transforms, worker schema. **Produces:** the validated Blender layer group and matching native physical pitch/profile; material selection preserves edited print settings.

- [ ] Add tests for edited-height preservation, reset/default behavior, disabled surface preserving other settings, uniform/first-layer phase parity with Rust, and non-frame-dependent seeds.
- [ ] Implement layer controls and the proven filtering recipe in both paths, with correct millimeter-to-meter bump scaling and unperturbed-normal cap masking.
- [ ] Implement the five finishes/two studios, rendered thumbnails, fixed color management, and explicit device/engine reporting. Add resin warnings for unsuitable meshes.
- [ ] Capture 0.12/0.20/0.28 mm close-ups and full rotations, including print/display tilt, caps, first-layer transitions, and a narrow-gap bracket. Compare denoising/filtering candidates using the actual final sequence.
- [ ] Run `cargo test --test print_surface` and the live Blender probes; record any native highlight approximation separately from coordinate/profile agreement.

**Deliverable:** Printed appearance is usable before export polish, comparison, or advanced formats.

### Task 7: Deliver GIF and high-resolution still exports

**Files:** `src/export/{gif,still,video,jobs}.rs`, `src/app.rs`, `renderer/render.py`, `tests/{gif,jobs}.rs`, `tests/render_probe.py`.

**Interfaces:** `turntable_angles(frame_count: u32, start_degrees: f64) -> Vec<f64>`; `encode_gif(job: &JobManifest, tools: &ToolPaths) -> Result<EncodedOutput, AppError>`; `export_still(job: &JobManifest, worker: &mut WorkerClient) -> Result<EncodedOutput, AppError>`; `encode_mp4(job: &JobManifest, tools: &ToolPaths) -> Result<EncodedOutput, AppError>`. `EncodedOutput` carries path, dimensions, size, and animation frame/duration metadata where applicable.

- [ ] Assert 80 angles ending at 355.5 degrees, all decoded GIF delays equal to 5 centiseconds, infinite loop metadata, expected opaque dimensions, and 4-second duration. Use a synthetic encoding fixture as well as real renders.
- [ ] Add job checks for changed layer pitch blocking resume, missing/corrupt frame recovery, encode-only retry, insufficient disk, cancellation, destination preservation, still size limits, current-camera snapshots, and transparent PNG alpha.
- [ ] Run `cargo test --test gif` and `cargo test --test jobs`; implement durable sequential rendering, the validated downsampling/palette pipeline, safe publication, and actual output playback.
- [ ] Export/inspect representative layer GIFs repeatedly and 4K stills at light/dark backgrounds; record complete timings/memory. Add MP4 only by reusing the same validated final frames, with ffprobe timing checks.

**Deliverable:** The first useful product: open a model, choose its print appearance, export a stable GIF or quality still.

### Task 8: Add four-view sheet export

**Files:** `src/export/diagram.rs`, `src/app.rs`, `renderer/{cameras,render}.py`, `assets/fonts/`, `tests/diagram.rs`, `tests/render_probe.py`.

**Interfaces:** Consume `diagram_cameras` from Task 3; `compose_diagram(panels: &[PathBuf; 4], spec: &DiagramSpec, destination: &Path) -> Result<EncodedOutput, AppError>`. `DiagramSpec` includes sheet dimensions, proportional layout, metadata, and printed-surface toggle.

- [ ] Assert 4096-sheet/1952x1856 panel geometry, proportional 2048 layout, Front/Right/Top/Isometric order, shared physical scale, and build-frame orientation independent of presentation tilt.
- [ ] Assert edge classification uses geometry rather than bumped normals and excludes coplanar triangle edges.
- [ ] Run `cargo test --test diagram`; implement the edge method proven in Task 1, camera-relative lighting, neutral/default appearance, printed option, font labels, and lossless composition.
- [ ] Decode/inspect the final sheets on asymmetric/tall/holey fixtures with printed surface both off/on. Verify no blank/clipped panel, false groove edges, or hidden independent rescaling.

**Deliverable:** All three primary output modes work.

### Task 9: Add see-through inspection and two-model comparison

**Files:** `src/viewport/inspect.rs`, `src/viewport/shaders/*.wgsl`, `src/scene/mod.rs`, `src/app.rs`, `tests/framing.rs`, `docs/visual-validation.md`.

**Interfaces:** `InspectionSpec` defines edge display, opacity, and optional axis section plane. `ComparisonSpec` defines two models, relative rigid placement, and SideBySide/GhostOverlay; it is separate from export material state.

- [ ] Assert mixed mm/inch models keep relative physical size, centering never rescales, linked cameras agree, and section-plane direction/position matches physical coordinates.
- [ ] Implement weighted blended inspection transparency, edges, uncapped fragment clipping, linked panes, and explicit ghost alignment/reset.
- [ ] Capture nested/intersecting/open-shell meshes across rotations; inspect ordering stability, cap absence, and true size differences. Confirm active export snapshots remain unchanged.
- [ ] Verify export UI identifies enabled inspection effects that will not be included. Rerun affected framing/runtime probes.

**Deliverable:** Useful visual inspection/comparison without extending the studio renderer into CAD analysis.

### Task 10: Persist projects and package the supported build

**Files:** `src/{project,settings,app}.rs`, `tests/{project,jobs}.rs`, `tools/check-runtime.ps1`, `README.md`, `docs/{performance,visual-validation}.md`.

**Interfaces:** `save_project(path: &Path, project: &ProjectFile) -> Result<(), AppError>`; `load_project(path: &Path) -> Result<ProjectFile, AppError>`. Machine-local executable paths remain outside portable project files.

- [ ] Add round-trip/version checks for units, layer pitch/first layer/depth, print/display poses, camera, comparison, relative/Unicode paths, changed/missing sources, and reserved unsupported Explicit schedules.
- [ ] Run `cargo test --test project`; implement failure-preserving save/load, source relocation, settings, cache retention, job resume/discard, and exit cleanup.
- [ ] Select the maintained LTS patch and FFmpeg build to support; rerun all live probes before recording their exact versions/hashes. Bundle app scripts/presets/font notices, with separately installed Blender/FFmpeg initially.
- [ ] Smoke-test fresh settings, missing/wrong tools, filenames with spaces/Unicode/brackets/percent, outputs, layer edits, cancellation/restart, corrupted-frame recovery, full disk, and launch without console windows.
- [ ] Run `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets`, and `pwsh -NoProfile -File tools/check-runtime.ps1`; also `git diff --check` if Git exists. Document any runtime exclusions.
- [ ] Review actual final GIFs/stills/sheets and interactive captures after the last relevant changes; publish measured performance and known appearance limits in the local documentation.

**Deliverable:** Reproducible local release with saved print setups and verified artifacts.

## 11. Milestones, deferrals, and acceptance

**Milestone A: Task 1.** Rendering proof with layer appearance and actual output artifacts. This settles the high-risk product premise before UI investment.

**Milestone B: Tasks 2–7.** Native interaction, independent print/display setup, editable layers, rendered preview, GIF, and 4K still. This is the earliest useful application.

**Milestone C: Tasks 8–10.** Four-view sheets, complete inspection/comparison, projects, recovery UI, and packaging. Some project/job schema work happens earlier because reproducibility depends on it.

After these milestones, consider explicit adaptive-layer inputs, stepped geometry, OBJ/3MF, file watching, animated WebP, batch CLI, print-bed backdrop, top-surface toolpath appearance, and additional studio presets. Automatic repair, printer infill, physical calibration, engineering dimensioning, and a native path tracer need their own requirements and proof; they are not implicit release obligations.

Avoid calendar estimates until Task 1 provides evidence. A small prototype can arrive quickly, while robust geometry, material tuning, animation filtering, and Windows lifecycle handling account for much of the remaining work.

The implementation is complete when:

- A first-time user can open an STL, choose a finish, set optional layers at 0.20 mm or another valid pitch, and export without operating Blender.
- Units and print orientation are visible; presentation/turntable changes leave bed-relative spacing attached to the part.
- First-layer height works; switching finishes preserves user-edited print settings; projects reproduce them.
- Printed appearance looks plausible in representative close-ups and remains stable at GIF resolution. Subpixel layers filter naturally rather than changing pitch.
- The default GIF decodes to an infinite four-second loop with 80 uniformly delayed frames, stable framing, and no boundary pause.
- Quality stills retain small details and support independent background alpha; resin is validated separately.
- Four-view sheets have correct build-frame orientation/shared scale and useful geometric edges, with an explicit optional printed finish.
- Native inspection/comparison remains responsive on normal fixtures and preserves relative model sizes.
- Cancellation, crashes, stale completions, and encoding failures preserve valid work and existing outputs; source STLs and unrelated processes remain untouched.
- Supported builds, actual end-to-end performance, captures, and the distinction between layer appearance and generated layer geometry are documented.

The immediate next implementation action is the layer-rendering proof in Task 1. Its result determines the rendering/filtering defaults; the user-facing architecture and physical print-setting contract stay focused on the requested workflow.
