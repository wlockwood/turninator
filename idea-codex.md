# Turninator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task by task. Use superpowers:subagent-driven-development only if the user selects that execution method. Steps use checkbox syntax for tracking.

**Goal:** Build a local desktop application that turns STL models intended for 3D printing into attractive looping GIFs, high-resolution stills, and labeled four-view diagrams, with a responsive viewport for inspection and comparison.

**Architecture:** A Rust desktop application owns model loading, camera controls, inspection, projects, and export jobs. A separate Blender process generates authoritative studio previews and final renders using EEVEE or Cycles. FFmpeg converts rendered frames into looping GIFs; Rust composes four-view sheets from individual rendered panels.

**Tech stack:** Rust, eframe/egui with its wgpu backend, glam, serde/serde_json, image, rayon, rfd, tracing, Blender with Python, and FFmpeg/ffprobe. Use egui-wgpu's re-exported wgpu types to avoid incompatible independently selected versions. Lock compatible Rust dependencies in Cargo.lock during implementation.

**Spec:** The design and acceptance requirements are embedded in this file. This is a planning deliverable, dated October 14, 2026; no application has been implemented or benchmarked.

## Recommendation and rationale

Build a focused "photo studio and inspection desk for 3D prints," rather than a general modeling application. Good defaults should make a first export possible with three decisions: which model, which finish, and which output.

The user's machine is an NVIDIA RTX 4070 Ti, AMD Ryzen 9 9950X, and 96 GB system RAM. This is a strong basis for the proposed application, but render times must be measured on that machine. The original 4070 Ti has 12 GB GPU memory; the 96 GB of system RAM is a separate resource. [NVIDIA specifications](https://www.nvidia.com/en-us/geforce/graphics-cards/40-series/)

Use EEVEE by default for short opaque-material GIFs and diagram panels. Use Cycles with OptiX for high-resolution studio stills and realistic translucent materials. Cycles supports RTX hardware acceleration through OptiX and GPU denoising on supported RTX devices. [Blender GPU rendering](https://docs.blender.org/manual/en/latest/render/cycles/gpu_rendering.html)

The CPU and RAM help with dense mesh import, normal generation, scene preparation, GIF palette generation/encoding, and retaining reusable scene data. They do not eliminate GPU-memory pressure or make multiple concurrent render processes automatically faster. Start with one Blender worker and one GPU export at a time. Use parallel CPU work where profiling shows a benefit, leaving at least two logical processors available to the GUI and other applications.

A native inspection viewport is justified because orbiting, sectioning, and comparing designs must respond immediately. Keep its shading deliberately modest: neutral studio-style shading, matcap, edge overlays, and inspection transparency. Do not attempt to recreate EEVEE or Cycles inside Rust. A separate Rendered Preview action shows the actual Blender result when appearance matters.

An all-Blender add-on would be cheaper to build but would require operating inside Blender. A fully native path tracer would turn this into a much larger graphics project. Directly embedding EEVEE would also introduce considerable integration work. The process boundary is the recommended compromise.

## Global constraints

- Windows x64 is the initial target; the user should never need to operate Blender's GUI for normal use.
- Models, projects, and exports remain local. Normal operation requires no network connection.
- Read-only source STL files are never rewritten. Geometry and scene changes are stored in the project or job snapshot.
- Initial renderer compatibility target: Blender 5.2.2 LTS. Record the exact validated build, executable path, and render backend. A newer patch requires the render regression checks before being marked supported. Blender 5.2 LTS is supported until July 2028. [Blender release information](https://www.blender.org/releases/5-2/)
- Detect Blender and FFmpeg/ffprobe from configured paths or PATH; present a settings picker if missing. Inspection remains available without either tool.
- Resolve exact Rust and FFmpeg versions during the first implementation milestone, record them, and retain a lockfile/tool manifest. Do not silently update renderer dependencies.
- UI work, mesh parsing, Blender I/O, image decoding, and GIF encoding run on separate appropriate threads. Never perform blocking work inside an egui update or wgpu paint callback.
- Export jobs use immutable settings and geometry snapshots. Subsequent edits cannot change a running export.
- Import and preview results carry generation IDs. A stale completion must never overwrite a newer model or scene.
- Preserve physical scale across comparisons. Assume millimeters for STL initially, visibly label the assumption, and offer inches/meters and a custom scale factor.
- Every performance number below is a target or an illustrative calculation until measured; retain CPU/GPU/disk and cold/warm timing evidence.

## Product design

### Workflow and GUI

One window has a large viewport, a compact settings sidebar, and an export/progress strip. Keep the three primary output buttons visible: **Looping GIF**, **Still Image**, and **Four-View Sheet**.

1. Drop an STL or use Open. Show import progress while the previous model remains visible.
2. Set upright direction with X/Y/Z axis buttons and 90-degree rotations. The default is Z-up; inference must not override the user's choice.
3. Choose a material swatch and a studio preset. Default: satin blue plastic on a light neutral background.
4. Orbit, zoom, or use front/right/top/isometric presets. Turntable playback in the native viewer demonstrates framing and rotation.
5. Optionally request an actual Blender-rendered preview. Keep the last successful preview visible while updating it, with an Updating indicator.
6. Export with a concise settings dialog showing dimensions, duration/frame count where applicable, quality, output path, and an estimate once measurements exist.

Inspect and Studio controls act on the same model orientation. The viewport explicitly distinguishes **Interactive View** from **Rendered Preview** so approximation differences are understandable. Opening a still preview does not discard the interactive camera state.

The native viewer needs fit-to-model, reset, perspective/orthographic switching, mouse orbit/pan/zoom, double-sided inspection shading, and a visible axis indicator. Backgrounds and preset thumbnails should be bundled procedural assets, with no required HDRI download.

### Materials and studios

Ship five initial finishes: satin plastic, glossy plastic, matte clay, anodized metal, and translucent resin. Expose Color and Gloss for opaque finishes. Resin additionally exposes Tint, Clarity, and Cloudiness; these map to controlled shader parameters rather than exposing the full node graph.

Ship two coordinated studios: **Light Studio** and **Dark Studio**, each using large soft key/fill/rim lights, a subdued world, and a ground plane where appropriate. Diagram mode supplies its own white background and neutral material. Avoid large texture sets, displacement, actual printed-layer geometry, and default depth of field in the first release.

Starting EEVEE sample counts are 16 for reduced-resolution rendered previews and 64 for GIF/diagram frames. Keep exposure fixed and validate final renders independently of native viewport accumulation. Tune these values in Task 1, especially for temporal stability and translucent approximations.

Store presets as versioned data that both the GUI and Blender bridge consume. Use Blender Principled materials and one consistent AgX-to-sRGB output configuration across EEVEE/Cycles. Native shading approximates the chosen finish; agreement is required for geometry, camera, orientation, and base color intent, not pixel-identical highlights.

Provide **Faceted** and **Smooth Curves** shading. Smooth Curves derives normals across suitable connected faces with a 30-degree crease threshold; never smooth across disconnected shells. Native and Blender rendering consume the same derived corner normals. This alters shading only: it does not turn a low-resolution STL into a smoother silhouette. Do not apply automatic bevels, remeshing, hole filling, or geometry decimation to final exports.

### Transparency and inspection

Separate these controls:

- **See-through:** native inspection transparency without optical distortion; combines adjustable opacity with an edge overlay. Implement weighted blended order-independent transparency, with depth testing against opaque objects, rather than naive triangle sorting. It remains an approximation for many overlapping surfaces.
- **Translucent resin:** a studio material using actual transmission and, for cloudiness, a bounded scattering/absorption setup. Route final exports to Cycles by default. EEVEE's refraction/thickness approximation can omit internal information, so it is not the reference view for this material. [EEVEE limitations](https://docs.blender.org/manual/en/latest/render/eevee/limitations/limitations.html)
- **Section plane:** an interactive visual cutaway movable along X, Y, or Z, implemented by fragment clipping. The first release does not generate closed section caps or manufacture new geometry. This inspection effect is not included in studio exports; the export dialog says so when it is enabled.

Show surface/open-edge warnings for meshes that are unsuitable for reliable solid-material transmission. Inspection remains usable for open meshes. The app must not invent walls or printer infill: interior details appear only if the STL contains those surfaces. Appearance presets are visualization choices, not physical predictions of a particular filament or resin.

### Looping GIFs: the primary output

The default is a four-second, 960 x 960 GIF at 20 fps: **80 frames**, five centiseconds per frame, infinite looping, light matte background. Provide Compact (640 x 640) and Large (1280 x 1280) sizes. Duration choices are 3, 4, and 6 seconds; rate choices are 10, 20, and 25 fps, which map to integral GIF delays.

Render a constant-speed full rotation with a fixed camera and fixed studio lights. Rotate the object about its XY bounding-box center and bottom Z, including user-set orientation. Rotation frame i is `start_angle + 360 * i / N`, for `i = 0..N-1`. Never duplicate the 360-degree endpoint or add a pause to the last frame. Disable motion blur by default to preserve print details.

Framing must fit every rotation, not just the first view. Fit a conservative rotational envelope with 10% image margins and a stable camera target. No frame-by-frame zoom or exposure adaptation. Validate long parts, eccentric geometry, and portrait-like models.

Use one animation-wide palette generated after flattening to the selected background and converting rendered frames to display-ready sRGB. Start with FFmpeg `palettegen` (`stats_mode=full`, `reserve_transparent=0`) and `paletteuse` with ordered Bayer dithering (`bayer_scale=3`). Compare this against `sierra2_4a` during visual validation, and keep the choice available as Balanced versus Smooth Gradients if both prove useful. A shared palette avoids palette changes between frames; it cannot alone prevent render noise or dither shimmer. [FFmpeg palette filters](https://www.ffmpeg.org/ffmpeg-filters.html#palettegen)

Use a two-pass palette/encoding process from disk rather than retaining all full-resolution frames in memory. Encode infinite looping explicitly and set the last delay equal to the other frame delays. GIF's loop and delay options are documented by FFmpeg. [FFmpeg GIF options](https://www.ffmpeg.org/ffmpeg-formats.html#gif-2)

GIF is the default shareable output; translucent objects are composited against an opaque background. Do not offer transparent-background GIFs initially: GIF transparency cannot represent partially transparent edge pixels. Display the actual encoded GIF and its file size before calling the job complete. Include an optional MP4 export from the same frames for much smaller, full-color animation; it is secondary and must not replace GIF.

The app may offer a smaller preset after encoding if the result is large. Never silently reduce resolution, colors, or frame rate to meet an arbitrary size limit. A 10 MB default-GIF size target is desirable, not a guaranteed constraint.

### High-resolution stills

Default: 4096 x 4096 PNG, Cycles/OptiX, adaptive sampling, GPU denoising when available. Offer 2048 square and 3840 x 2160 alternatives. An advanced custom size is bounded by a 64-megapixel total pixel limit in the initial release.

Starting quality settings: Preview at 32 maximum samples and noise threshold 0.05; Final at 256 maximum samples and threshold 0.01, using denoising normal/albedo data. These are initial tuning values, not proof of sufficient quality. Retain sharper alternatives if tiny holes, text, or thin walls disappear under denoising. Transmissive materials start with enough transmission bounces to traverse multiple interfaces (8), subject to benchmark review.

Offer a transparent PNG background; remove the ground plane in this option initially so it does not accidentally leave opaque pixels or a backdrop. Keep the chosen object finish independent from background alpha. Default exports use the current composed camera; explicit Fit can restore margins.

### Four-view diagrams

Default layout is a labeled two-by-two **shape sheet**:

| Top left | Top right |
| --- | --- |
| Front | Right |
| Top | Isometric |

Use orthographic projection for all four panels. Canonical coordinates after the user's upright transform are X-right, Y-back, Z-up. Front camera looks from -Y with +Z up; Right looks from +X with +Z up; Top looks from +Z with +Y up. Isometric looks from normalized `(1, -1, 1)` toward the model, with projected +Z as image up. These conventions must match between native controls and Blender.

Default output is a 4096 x 4096 PNG with white background, 64-pixel outer margins, 64-pixel gutters, and 96 pixels reserved at the bottom of each panel for a view label. This yields four 1952 x 1856 render areas. Labels are drawn after rendering with a bundled font. Offer 2048 square by scaling the layout proportionally.

Use the same pixels-per-model-unit across all panels, selected to fit the largest projected extent with 10% margins. Center each panel on the same model bounds center; do not independently enlarge narrow views. Diagram rendering uses neutral gray shading, modest surface contrast, no floor, no depth of field, and visible silhouette/crease lines. The renderer produces EEVEE panel images and an edge pass; prototype Blender Freestyle compatibility in the first milestone. If Freestyle is unsuitable, render visible sharp edges using an explicit overlay pass from canonical topology, without drawing every STL triangle.

This is not a projection-standard-compliant engineering drawing. Do not add dimension arrows, tolerances, or arbitrary measurements. Optional metadata lists filename and overall oriented bounding-box dimensions, marked with the selected/assumed units. A later diagram preset can add drawing conventions after those requirements are settled.

### Comparing designs

The first complete release supports two loaded models with linked camera controls in side-by-side inspection panes and a ghost-overlay mode. Preserve the models' relative physical scale; offer explicit centering, manual translation/rotation, and reset. The default comparison alignment centers their bounds without rescaling them.

Comparison is visual, not a tolerance or boolean-difference analysis. Export remains focused on one selected model in the initial release. Save comparison setup in the project, but defer automated registration, deviation heatmaps, and animated comparison exports.

## Renderer and job design

### Canonical geometry and coordinates

Parse STL once in Rust. Preserve triangle winding, disconnected components, and source precision in source-space calculations. Derive bounds in f64, and translate near the origin before converting render geometry to f32. Retain the source-space origin in model metadata so placements and physical dimensions remain recoverable. Rebasing cannot recover precision already lost in an input STL. Define model-to-studio transforms explicitly; do not let each renderer decide its own scaling or axis conversion.

Both renderers consume the same indexed positions, triangle indices, and per-corner normals. Use a private versioned `meshbin` snapshot: magic/version, vertex and triangle counts, followed by little-endian f32 position triples, u32 triangle-index triples, and f32 corner-normal triples. Header parsing checks arithmetic overflow, exact buffer sizes, finite coordinates/normals, and index ranges. Counts exceeding u32 index capacity are rejected with a clear error. Source-space unit metadata and transforms travel in JSON, not in an implicit renderer assumption.

Exact-coordinate vertex welding may support adjacency; never tolerance-weld separate nearby surfaces automatically. Classify and omit zero-area triangles from render snapshots with a visible count; invalid indices/non-finite input reject the import. Reject empty geometry. Expensive topology analysis can finish after an initial usable preview, with its own generation ID.

STL format detection must not rely on a `solid` prefix alone. Test binary files whose header begins with `solid`, ASCII files, truncated data, excessive declared counts, and long paths. Preserve source file hashes for project and resume validation.

### Worker protocol and lifecycle

Use one long-lived Blender process launched with factory settings, Python auto-execution disabled, and the application-owned worker script. Disable console windows on Windows. Blender background mode on a normal desktop session must be validated for EEVEE in milestone 1; do not assume the same setup works on a display-less Windows service. [Blender EEVEE platform limitations](https://docs.blender.org/manual/en/latest/render/eevee/limitations/limitations.html)

Rust sends line-delimited JSON requests through stdin. A Python reader thread places commands in a queue; only Blender's main thread touches bpy. Messages carry protocol version, request ID, scene revision, request kind, and absolute job-directory path. Emit structured events on stdout with a unique `TURNINATOR_EVENT ` prefix; other Blender output is retained as diagnostic log text.

Request kinds: Probe, Preview, Export, and Shutdown. Responses report capabilities, preparation status, frame completion, output files, errors, and completion. There is one active render request. Preview requests are debounced by 300 ms and coalesced to the latest scene revision when automatic rendered-preview refresh is enabled; manual refresh is the initial default. Preview is suspended while a final export occupies the worker. The UI continues native inspection.

Cancellation terminates the worker process tree through a Windows Job Object and joins process/output readers. This avoids pretending a command queued behind a blocking render can interrupt it. After cancellation/crash, restart the worker when needed and restore the last accepted scene snapshot. On application exit, terminate tracked child processes and release handles. Never kill unrelated Blender instances.

Cache geometry by content hash and material/studio setup by a versioned scene fingerprint. Enable Cycles persistent data for repeated frames and previews; monitor its memory cost and restart the worker when the user explicitly unloads large scenes or cache retention exceeds a configurable ceiling. Persistent data retains render data to speed repeated/animation renders at a memory cost. [Blender performance settings](https://docs.blender.org/manual/en/latest/render/cycles/render_settings/performance.html)

### Export integrity and recovery

Each job owns a manifest, immutable mesh/settings snapshots, renderer version/device details, diagnostic log, and frames directory. Jobs write to a managed application cache directory with an explicit disk budget (20 GB default). Preflight checks estimated frame storage and available disk space; queue/resume never silently deletes interrupted jobs.

Resume only when the source/snapshot hash, renderer build, preset version, camera, engine, resolution, and sampling settings match the manifest. A completed frame is recorded only after its file is fully written and decoded successfully. Retain valid frames after cancellation; retry missing or invalid frames. Frame generation and GIF/MP4 encoding are separate stages, so an encoding failure need not rerender.

Publish the final output by an atomic rename from a temporary file in the destination directory after verifying it decodes and has expected dimensions/frame timing. Ask about replacing an existing output through the normal save dialog. Do not overwrite it with partial data. Cleanup is restricted to job directories owned by the app; interrupted jobs require explicit deletion or a configured retention policy.

Use argument arrays for Blender/FFmpeg processes, never shell-built commands. Include filenames with spaces, Unicode, brackets, and percent characters in process/path validation. Stage numbered frames in the app's job directory rather than interpolating arbitrary source filenames into FFmpeg sequence patterns.

OptiX selection must be explicit and verified by a capability probe and recorded device information. If it fails, offer CUDA, then an explicitly selected slower CPU fallback. Do not silently label a CPU render as GPU accelerated. GPU-memory failures should offer a lower-cost preset or another device while retaining completed work.

## Performance experiment and tuning targets

The first milestone is a feasibility/performance experiment, before building the full GUI. It must prove worker launch, EEVEE rendering, Cycles/OptiX rendering, GIF encoding, and diagram-edge generation on the user's machine.

Generate fixtures locally: a mechanical bracket with holes and thin ribs, a curved decorative model, a tall/long asymmetric part, an outer shell with a separate internal component, and dense curved meshes around 100k, 1M, and 5M triangles. Also include deliberately open and reversed-winding variants. Procedural fixtures avoid relying on unlicensed downloaded models; add real user STLs later if available.

Measure cold launch, warm scene loading, parsing, topology/normals, GPU upload, shader preparation, first frame, median/p95 subsequent frames, denoising, PNG writing, palette generation, GIF encoding, output size, peak host memory, and peak GPU memory where instrumentation is available. Record driver/tool versions and whether other GPU applications were running. Compute end-to-end totals; render-engine timers alone are insufficient.

| Scenario | Initial target on the user's hardware |
| --- | --- |
| Native orbit, one model up to 1M triangles, 2560 x 1440 viewport | Approximately 60 fps when no export is running |
| 100k-triangle model to usable native view | Under 2 seconds, excluding app cold startup |
| Warm 768 x 768 opaque EEVEE rendered preview | Under 2 seconds end to end |
| Default 960 x 960, 80-frame opaque GIF | Under 2 minutes including preparation and encoding |
| Opaque 4096-square Cycles still, ordinary studio | Under 2 minutes including preparation and writing |
| 4096-square four-view sheet | Under 1 minute including all panel renders and composition |

Targets are not release guarantees. Record actual results and bottlenecks. Resin, pathological geometry, and 5M-triangle fixtures are separate stress cases. A four-second/80-frame loop costs about 40 seconds at 0.5 seconds per frame or 160 seconds at 2 seconds per frame, before setup/encoding; these are arithmetic examples only.

Tune lighting, sampling, and materials before building GPU-specific optimizations. Compare opaque EEVEE and low-sample denoised Cycles loops for shimmer, small-detail retention, and time. Never claim GIF denoising is adequate from a single still. Keep full-quality final geometry; a future preview-only simplification is justified only by measured viewport bottlenecks.

## Proposed file structure

Start as one Rust package with focused modules, not a multi-crate framework.

```text
Cargo.toml
src/main.rs                     Desktop entry point
src/app.rs                      UI state and asynchronous event application
src/error.rs                    Shared typed errors and user-facing summaries
src/model/{mod,stl,topology}.rs   Canonical mesh, import, adjacency, normals
src/scene/{mod,camera,presets}.rs Serializable orientation, units, cameras, finishes
src/viewport/{mod,gpu,inspect}.rs Native shading, edges, transparency, section clipping
src/viewport/shaders/*.wgsl      Native shader passes
src/render/{mod,protocol,worker}.rs Blender discovery, process lifecycle, events
src/export/{mod,jobs,gif,still,diagram,video}.rs Output validation and composition
src/project.rs                  Versioned saved projects and source validation
src/settings.rs                 Tool paths and persistent preferences
assets/presets/*.json            Finish/studio definitions
assets/fonts/                   Font and redistribution notice for panel labels
renderer/worker.py              Command queue, main-thread bpy work, structured events
renderer/{mesh,scene,materials,cameras,render}.py Blender scene implementation
tools/generate-fixtures.py       Procedural STL fixtures
tools/benchmark-render.py        End-to-end renderer experiment
tools/check-runtime.ps1         Hardware/tool check and selected live probes
tests/{import,framing,protocol,jobs,gif,diagram,project}.rs
tests/render_probe.py            Blender runtime checks using unittest
tests/fixtures/                 Small deterministic mesh files
docs/performance.md             Actual hardware measurements and limitations
docs/visual-validation.md       Captures and assessment of exported artifacts
README.md                       Setup, workflow, supported tool builds, limitations
```

The Python worker uses Blender's bundled Python/NumPy. Avoid an additional user-installed Python dependency for normal use. Test/benchmark scripts can run through the selected Blender executable where they require bpy.

## Shared interfaces

Define these contracts when their owning tasks are implemented; subsequent tasks consume them unchanged unless the plan is revised.

- `MeshData`: indexed positions, triangle indices, corner normals, f64 source bounds, source hash, import/topology warnings. Shared by Arc in native app state.
- `MeshId`, `SceneRevision`, `RequestId`, `JobId`: distinct ID types to prevent accidental cross-use.
- `Units`: Millimeters, Inches, Meters, Custom with finite positive millimeters-per-source-unit.
- `ModelSpec`: MeshId, source-space origin, per-model Units, orientation, and placement. Each model retains its own unit interpretation so an inches STL can be compared with a millimeters STL.
- `SceneSpec`: version, ModelSpecs, selected model, camera, material, studio, shading mode. Physical model dimensions remain separate from a single shared studio normalization transform; never normalize comparison models independently.
- `CameraSpec`: target, view direction/up, projection, perspective vertical field of view or orthographic vertical span, and clipping distances.
- `ExportSpec`: tagged Gif, Still, Diagram, or Mp4 options, quality/engine, dimensions, and destination. Diagram builds its own four cameras; a studio export does not include native inspection effects.
- `WorkerRequest` / `WorkerEvent`: versioned serde-tagged protocol enums matching Python JSON definitions and carrying request/revision/job identity.
- `JobManifest`: snapshot fingerprints, settings, versions, verified completed frames, output stages, and job state.
- `AppError`: contextual operation, actionable summary, and diagnostic detail; external process errors preserve the associated job log.

## Review focus

These are the five highest-risk inputs/lifecycle conditions and the tasks that verify them:

1. STL binary headers beginning with `solid`, invalid counts, and non-finite geometry: deterministic import or clear rejection (Task 2).
2. Off-center elongated parts, huge source coordinates, and mixed units: stable framing with preserved comparison scale (Tasks 2, 3, 9).
3. Nested translucent surfaces, open shells, and thin ribs: understandable inspection, warnings, and visually validated Cycles output (Tasks 1, 5, 9).
4. Cancellation, stale completions, process crashes, or full disks: retained valid work without replacing a newer scene or damaging an existing output (Tasks 4, 6, 10).
5. GIF loop timing/palette shimmer and four-view orientation/scale mismatch: decode-based checks plus visible artifact review (Tasks 6, 8).

## Implementation tasks

Execute sequentially. Each task ends with a working deliverable and a focused commit if the workspace is a Git repository. Stage only that task's paths, check the staged diff, and preserve other work. This planning task does not initialize Git or make commits.

For substantive logic, first add the stated regression checks, run them to demonstrate a meaningful failure, implement, then rerun. Visual/rendering acceptance requires actual captures; compilation and tests alone do not establish appearance or speed.

### Task 1: Prove the rendering and output pipeline

**Files:** `tools/generate-fixtures.py`, `tools/benchmark-render.py`, `tests/render_probe.py`, `docs/performance.md`, `docs/visual-validation.md`.

**Interfaces:** Produce `benchmark-render.py --blender <path> --ffmpeg <path> --output <directory>` and a machine-readable benchmark JSON containing tool/device metadata, per-stage timings, and artifact paths. Fixture generation supports an output-directory argument and deterministic mesh parameters.

- [ ] Generate the listed fixtures, including an asymmetric XYZ-marked shape for view-orientation checks.
- [ ] Add runtime checks for an EEVEE PNG, an OptiX Cycles PNG, resin with internal geometry, a four-panel edge-render experiment, and an 80-frame GIF whose decoded delays sum to 4 seconds.
- [ ] Run using the configured Blender/FFmpeg builds on the target desktop. Verify the selected GPU and background-mode behavior from logs and device reporting.
- [ ] Compare EEVEE versus Cycles loops and Bayer versus error-diffusion GIF dithering visibly; inspect holes, thin ribs, color banding, loop boundary, and resin interiors.
- [ ] Record cold/warm timings and memory data. Establish the diagram-edge implementation and initial quality settings from evidence. If a chosen backend fails, resolve it here and amend this plan before GUI integration.

**Deliverable:** Real sample GIF, 4K still, four-view sheet, and a measured backend recommendation. No GUI required yet.

### Task 2: Load STL into a canonical mesh

**Files:** `Cargo.toml`, `src/error.rs`, `src/model/{mod,stl,topology}.rs`, `tests/import.rs`, `tests/fixtures/`.

**Interfaces:** `load_stl(path: &Path) -> Result<MeshData, AppError>`; `write_mesh_snapshot(mesh: &MeshData, destination: &Path) -> Result<(), AppError>`; `derive_corner_normals(mesh: &MeshData, crease_degrees: f32) -> Result<Vec<[f32; 3]>, AppError>`.

- [ ] Add fixtures/assertions for ASCII and binary equivalence, `solid` binary headers, truncation, count overflow, non-finite coordinates, empty input, degenerate triangles, disconnected shells, and large source offsets with small features.
- [ ] Run `cargo test --test import`; confirm checks fail on absent/incorrect parsing behavior.
- [ ] Implement bounded parsing, f64 bounds/rebasing, source hashing, indexed geometry, exact-coordinate adjacency, and crease-aware corner normals. Preserve winding and collect visible warnings.
- [ ] Define/write the meshbin format and verify count/size/index validation with a round-trip reader test; add a Python reader check later in Task 4.
- [ ] Rerun the focused checks and record import/normal timings for dense fixtures. Lock dependency versions needed by this task.

**Deliverable:** Reliable import and reusable canonical geometry without modifying source STLs.

### Task 3: Build scene/camera math and the interactive desktop view

**Files:** `src/main.rs`, `src/app.rs`, `src/scene/{mod,camera,presets}.rs`, `src/viewport/{mod,gpu}.rs`, `src/viewport/shaders/`, `tests/framing.rs`.

**Interfaces:** `fit_turntable(scene: &SceneSpec, bounds: &ModelBounds, aspect: f64, margin: f64) -> CameraSpec`; `canonical_view(view: ViewKind, bounds: &ModelBounds, aspect: f64) -> CameraSpec`. Define `ModelBounds` as oriented physical-space bounds and `ViewKind` as Front, Right, Top, Isometric. Define `TurninatorApp` as the eframe application; keep GPU resources separate from serializable scene state.

- [ ] Add projection tests for all canonical camera bases, tall/long/off-center geometry, positive unit conversion, huge-origin rebasing, and full-rotation envelope fitting with 10% margins.
- [ ] Run `cargo test --test framing`; verify the tests expose wrong orientation, clipping, or independent rescaling.
- [ ] Implement Open/drop, asynchronous import generations, native orbit/pan/zoom, fit/reset, axis indicator, projection toggle, orientation buttons, and neutral shaded viewport using eframe's compatible wgpu device.
- [ ] Render asymmetric fixtures through native and Blender camera probes; compare silhouette, screen placement, orientation, and projected axis directions. Use the same normalized scene transform in both paths.
- [ ] Launch via `cargo run --release` and measure interaction at 1440p with 100k/1M/5M triangle fixtures. Verify the previous model remains visible during loading and late imports cannot replace a newer selection.

**Deliverable:** A usable local STL inspection application with deterministic scene math.

### Task 4: Integrate the Blender worker and durable jobs

**Files:** `src/render/{mod,protocol,worker}.rs`, `src/export/{mod,jobs}.rs`, `src/settings.rs`, `renderer/{worker,mesh,scene,cameras,render}.py`, `tests/protocol.rs`, `tests/jobs.rs`, `tests/render_probe.py`.

**Interfaces:** `WorkerClient::start(tools: &ToolPaths) -> Result<WorkerClient, AppError>`; `submit(&mut self, request: WorkerRequest) -> Result<(), AppError>`; `cancel_active(&mut self) -> Result<(), AppError>`. Define `ToolPaths` for validated Blender/FFmpeg/ffprobe executables. `create_job(scene: &SceneSpec, export: &ExportSpec, root: &Path) -> Result<JobManifest, AppError>` owns immutable snapshots.

- [ ] Add protocol checks for version mismatch, prefixed events amid ordinary Blender logs, stale request/revision IDs, crashes, and incomplete writes. A fake worker makes lifecycle checks independent of installed Blender.
- [ ] Run `cargo test --test protocol` and `cargo test --test jobs`; verify failure paths before implementing them.
- [ ] Implement tool discovery/capability probing, hidden process launch, tracked Job Object lifetime, stream readers, main-thread Python command execution, immutable job directories, and validated meshbin transfer. Python/Rust readers must agree on counts, winding, normals, and bounds.
- [ ] Implement native progress reporting, manual rendered-preview requests, optional 300 ms latest-only refresh, worker caching, persisted frame completion, and termination/restart cancellation. Keep arbitrary bpy operations off the input-reader thread.
- [ ] Run live EEVEE/OptiX probes, cancel during scene preparation and mid-render, restart, and verify no orphan child or unrelated Blender termination. Verify the UI remains interactive with a busy worker.

**Deliverable:** Actual Blender previews and a recoverable export-job foundation.

### Task 5: Add attractive finishes, studios, and resin

**Files:** `assets/presets/*.json`, `renderer/materials.py`, `renderer/scene.py`, `src/scene/presets.rs`, `src/app.rs`, `tests/render_probe.py`, `docs/visual-validation.md`.

**Interfaces:** `MaterialPreset` and `StudioPreset` are versioned data consumed by `SceneSpec`; `build_material(spec: dict)` and `build_studio(spec: dict)` are worker-owned Python functions, with output attached to the current scene. Their parameters match the serialized Rust preset schema.

- [ ] Add schema checks for all five finishes/two studios and assertions for finite, bounded parameters, stable preset IDs, and independent background alpha.
- [ ] Implement finish swatches, color/gloss controls, resin tint/clarity/cloudiness, Faceted/Smooth Curves selection, and controlled EEVEE/Cycles material construction.
- [ ] Set constant exposure/color management, matching ground placement, verified corner normals, OptiX selection, and actionable fallback/device status. Route resin exports to Cycles unless the user explicitly selects approximate EEVEE.
- [ ] Capture opaque and resin fixtures at representative angles with both engines. Inspect engraving, narrow gaps, curved surfaces, separate internal components, and open-shell warnings; check for oversmoothing and denoising detail loss.
- [ ] Update visual evidence and tuning values based on complete animations as well as stills. Preserve identical geometry/cameras while permitting renderer-specific highlight differences.

**Deliverable:** A small set of visually validated finishes that work without Blender knowledge.

### Task 6: Ship seamless GIF export

**Files:** `src/export/gif.rs`, `src/export/jobs.rs`, `src/app.rs`, `renderer/render.py`, `tests/gif.rs`, `tests/jobs.rs`.

**Interfaces:** `turntable_angles(frame_count: u32, start_degrees: f64) -> Vec<f64>`; `encode_gif(job: &JobManifest, tools: &ToolPaths) -> Result<EncodedOutput, AppError>`. Define `EncodedOutput` with destination, dimensions, byte size, frame count, and duration where applicable.

- [ ] Add checks for N=80 yielding angles 0 through 355.5 degrees with no duplicate endpoint; 20 fps yielding 80 decoded frames at 5 centiseconds and infinite loop metadata; and a final frame delay equal to every other delay.
- [ ] Add a tiny synthetic-frame encoding fixture to verify actual GIF decoding, shared-palette behavior, selected opaque matte, valid file publication, and bounded error reporting. Run `cargo test --test gif`.
- [ ] Implement the GIF dialog defaults, rotational-envelope camera, fixed lighting/exposure, sequential durable frame rendering, two-pass palette encoding, and encoded playback/file-size display.
- [ ] Verify missing/damaged-frame recovery, encode-only retry, cancellation, full-disk handling, and preservation of an existing destination on failure. Use injectable filesystem/process failures in `tests/jobs.rs`.
- [ ] Export real 80-frame fixtures and inspect several repeated loops for a boundary pause, clipping, color shifts, temporal noise, and dither shimmer. Record total render/write/palette/encoding time and GIF size.

**Deliverable:** The primary use case works end to end, including interrupted-job recovery.

### Task 7: Add high-resolution PNG and optional MP4 output

**Files:** `src/export/{still,video}.rs`, `src/app.rs`, `renderer/render.py`, `tests/jobs.rs`, `tests/render_probe.py`.

**Interfaces:** `export_still(job: &JobManifest, worker: &mut WorkerClient) -> Result<EncodedOutput, AppError>`; `encode_mp4(job: &JobManifest, tools: &ToolPaths) -> Result<EncodedOutput, AppError>`. Consume the same frame sequence and camera/material snapshots as Task 6; MP4 is an explicit alternative output.

- [ ] Add checks for requested PNG dimensions, 64-megapixel limit, alpha/background independence, exact current-camera snapshot, and MP4 duration/frame rate. Exercise invalid sizes and existing destination preservation.
- [ ] Implement still presets, final sampling/denoising, transparent-background PNG without a floor, and optional MP4 encoding using a supported H.264 encoder. Offer NVENC only after detecting it; encoding choice must not change frame rendering.
- [ ] Decode exported PNGs and use ffprobe to validate MP4 stream dimensions, frame rate, and duration. Ensure completed frames are reused when switching encoding format.
- [ ] Render 4K opaque/resin fixtures and inspect small geometry and alpha edges against light/dark backgrounds; record CPU/GPU memory and end-to-end time.
- [ ] Run the focused job/runtime checks and retain captures. Do not equate a successful PNG write with acceptable detail preservation.

**Deliverable:** High-quality stills and a full-color animation alternative.

### Task 8: Add four-view diagram export

**Files:** `src/export/diagram.rs`, `src/scene/camera.rs`, `src/app.rs`, `renderer/render.py`, `assets/fonts/`, `tests/diagram.rs`, `tests/render_probe.py`.

**Interfaces:** `diagram_cameras(bounds: &ModelBounds, panel_size: [u32; 2], margin: f64) -> [CameraSpec; 4]`; `compose_diagram(panels: &[PathBuf; 4], spec: &DiagramSpec, destination: &Path) -> Result<EncodedOutput, AppError>`. Define `DiagramSpec` with sheet size, scaled layout dimensions, optional metadata, and ordered labels Front/Right/Top/Isometric.

- [ ] Add checks for the exact 4096-sheet layout, 1952 x 1856 panel render sizes, 2048 proportional layout, correct panel order, shared model-to-pixel scale, and 10% fit margins.
- [ ] Add asymmetric XYZ fixture assertions showing Front/Right/Top orientation and orthographic isometric behavior; verify a known length spans the same pixel distance in each unforeshortened orthographic view.
- [ ] Implement the edge method validated in Task 1, diagram lighting/material overrides, four panel jobs, bundled-font labels, optional assumed-unit bounding dimensions, and lossless Rust composition.
- [ ] Run `cargo test --test diagram` and the Blender panel probe; decode the final sheet and verify dimensions/label regions and all panel outputs before publication.
- [ ] Inspect bracket/tall/asymmetric fixtures visibly. Check internal holes, silhouette/crease visibility, blank panels, unintended triangle edges, scale consistency, label readability, and no clipped geometry. Record the total four-view time.

**Deliverable:** A useful print-design shape sheet that needs no manual image assembly.

### Task 9: Complete see-through inspection and model comparison

**Files:** `src/viewport/inspect.rs`, `src/viewport/shaders/`, `src/app.rs`, `src/scene/mod.rs`, `tests/framing.rs`, `docs/visual-validation.md`.

**Interfaces:** `InspectionSpec` holds opacity, edge display, optional axis section plane, and view mode. `ComparisonSpec` holds two MeshIds, explicit relative placement, and SideBySide/GhostOverlay mode. These are native-view/project settings, separate from studio export material.

- [ ] Add checks for millimeters-versus-inches conversion, bounds-center alignment without rescaling, linked camera state, manual translation reset, and section-plane sign/position in canonical coordinates.
- [ ] Implement edge display, flat/matcap shading, weighted blended transparency, uncapped section clipping, and explicit native-only-effect copy in the export dialog.
- [ ] Implement two-model asynchronous loading, side-by-side panes, synchronized orbit/zoom, and contrasting ghost overlay with explicit alignment controls.
- [ ] Capture intersecting/nested/open-shell fixtures from several views. Confirm rotating the view does not produce gross transparency ordering jumps, original surfaces remain inspectable, and clipping hides the expected side.
- [ ] Validate distinct physical-size models remain visibly different, comparison changes do not alter an active export snapshot, and memory is released when a model is unloaded. Rerun affected camera/runtime checks.

**Deliverable:** Responsive geometry inspection and honest visual comparison alongside studio rendering.

### Task 10: Save projects, finish runtime behavior, and package

**Files:** `src/project.rs`, `src/settings.rs`, `src/app.rs`, `tools/check-runtime.ps1`, `tests/project.rs`, `tests/jobs.rs`, `README.md`, `docs/performance.md`, `docs/visual-validation.md`.

**Interfaces:** `save_project(path: &Path, project: &ProjectFile) -> Result<(), AppError>`; `load_project(path: &Path) -> Result<ProjectFile, AppError>`. Define `ProjectFile` with schema version, source paths/hashes, SceneSpec (including per-model units), InspectionSpec, ComparisonSpec, and export presets; tool settings remain machine-local.

- [ ] Add project round-trip/version tests, changed/missing-source relocation checks, relative/Unicode path checks, and failure-preserving save behavior. Run `cargo test --test project`.
- [ ] Implement project save/load and source relocation, bounded caches, job history/resume/discard, tool-path settings, readable logs, and app-exit cleanup. Reimport a changed source only after clearly exposing that it differs from the saved project.
- [ ] Package an application folder containing the Rust executable, worker scripts, presets, and font notices. Initially document separately installed Blender/FFmpeg paths rather than bundling hundreds of megabytes or a second Python runtime.
- [ ] Run a clean-machine/user-settings smoke test: missing tools, configured tools, filename edge cases, all outputs, cancel/restart/resume, damaged frame, low disk, wrong renderer version, and launch without a console window.
- [ ] Run `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets`, and `pwsh -NoProfile -File tools/check-runtime.ps1`. Run `git diff --check` if Git is initialized. Document runtime exclusions honestly.
- [ ] Repeat the measured acceptance scenarios after the final relevant changes and review actual GIFs, PNGs, diagrams, and native comparison captures. Record measured values against targets and remaining limitations.

**Deliverable:** A usable local release with reproducible setup and evidence for its visual quality/performance.

## Milestones and scope control

- **Milestone A, Tasks 1-6:** Prove the renderer, build the native view, and deliver attractive looping GIFs. This is the earliest useful application.
- **Milestone B, Tasks 7-8:** Add the requested high-resolution stills and four-view sheet. All primary output modes now exist.
- **Milestone C, Tasks 9-10:** Complete inspection/comparison, project persistence, recovery, and distribution polish.

Defer OBJ/3MF imports, per-face material painting, printer infill simulation, printable layer texture, physical material calibration, automatic mesh repair, automated dimensioning, capped section exports, batch directories, renderer plugins, and a native path tracer. File watchers and drag-to-reload can be added after generation/lifecycle behavior is proven.

If the first measured experiment shows low-sample Cycles GIFs are fast and stable enough, keep the dual-engine architecture but change the recommended quality preset based on evidence. If native viewport shading grows into a renderer project, stop adding studio fidelity there and improve the authoritative Blender preview instead.

## Completion criteria

- A first-time user can open a print STL, set orientation/finish, and export a looping GIF without learning Blender.
- The default GIF decodes as an infinite four-second loop with 80 uniformly timed frames, maintains framing, and looks stable over repeated playback.
- A 4096-square still preserves representative small print features and supports opaque or transparent background.
- A four-view sheet has correctly labeled, correctly oriented, consistently scaled panels with useful silhouette/crease information.
- Resin appearance and see-through inspection remain separate, understandable choices; open meshes are reported without being silently repaired.
- Two-model comparisons preserve relative scale, and native interactions remain responsive for the agreed normal-size fixtures.
- Cancellation/crashes preserve validated frames, and no failed operation damages source geometry, saved projects, existing exports, or unrelated Blender processes.
- Performance and appearance claims are supported by measurements and visible artifacts on the user's 4070 Ti / 9950X / 96 GB system.

Implementation starts only after the user requests it. This document supplies a concrete recommended design and staged implementation plan; it does not authorize dependency installation, application implementation, or publication by itself.
