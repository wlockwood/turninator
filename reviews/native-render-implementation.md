# Native render/export implementation

The desktop-facing backend now lives in `src/render/worker.rs` and `src/export/{jobs,encode}.rs`. Root owns GUI integration and phase commits. No source STL/G-code files are edited by this backend.

## API

`ToolPaths` is cloneable/serializable and supplies installed Blender/FFmpeg/ffprobe defaults plus executable-path validation. `WorkerClient::spawn` starts a persistent worker. `request(Value, &AtomicBool, callback)` runs on the caller's background thread and returns the terminal event; callbacks receive protocol events and `kind: log` diagnostics. `epoch()`, `is_running()` and `cancel()` support an actor retaining/replacing the worker.

`run_export(&mut WorkerClient, &ToolPaths, &ExportRequest, &AtomicBool, callback)` handles `Gif`, `Still`, `Diagram`, and `Mp4`. `JobEvent` exposes progress, diagnostic lines and completed frame paths. `ExportResult` reports output path, width, height, byte count, animation frame count and duration. Export request fields match the requested desktop contract; GIF defaults are 960 square, 80 frames, 32 samples and opaque background.

## Process lifetime

On Windows the launcher creates stdin/stdout pipes and calls `CreateProcessW` with `CREATE_SUSPENDED | CREATE_NO_WINDOW`. It attaches the new process to a kill-on-close Job Object **before** `ResumeThread`; failed assignment terminates the still-suspended child. This removes the startup ownership race in the Python proof controller. Handles have RAII cleanup, and only the owned tree is terminated.

The reader drains output independently, parses prefixed events and tags them with a process epoch. Requests validate response identity/revision and reject malformed protocol output. The background request loop checks cancellation every 100 ms, with probe/shutdown/render deadlines. Cancellation terminates the Job Object, then joins the pipe reader; it does not queue an ineffective cancel command behind Blender's blocking renderer. Encoder/ffprobe processes use the same hidden owned launcher, independent output draining, deadlines and cancellation.

## Jobs and publication

Render identities include scene/mesh/script hashes, Blender executable hash, engine, resolution, samples, effective alpha, frame count and render kind. GIF and MP4 share an animation render identity. Destination is outside the render fingerprint, allowing alternate encoding/publication from verified frames. Worker-written frame manifests verify reuse and corrupt/missing frames render again. Job status persists independently through rendering, encoding, completion, cancellation and failure.

A conservative Windows free-space preflight estimates frame storage. Existing source/mesh/scene paths cannot be selected as output; an optional `source_path` in the scene extends protection to the original imported file. Root should include that field when creating desktop snapshots. Scenes and meshes must remain immutable for a job; hash checks catch changed inputs before output publication.

GIF encoding uses a fixed basename pattern with the frame directory as process cwd, plus a literal palette input. Arbitrary `%04d` parent names therefore stay literal. Rust's GIF reader decodes all frames and verifies dimensions, exactly five-centisecond delays including the final frame, frame count and infinite looping. MP4 uses H.264 initially and ffprobe checks decoded count, dimensions and duration. PNGs are decoded before publication. All outputs stage in a private directory beside the destination, then use Windows replacement semantics; invalid frames and encoder failures preserve prior destinations.

## Cameras, materials and diagrams

Optional scene camera data matches the native viewport: `target_mm`, target-to-camera `direction`, `up`, `projection`, `distance_mm`, `vertical_fov_degrees`, and `ortho_span_mm`, in the centered displayed frame (XY centered, minimum displayed Z at zero). Stills retain that camera; animated exports fit the full rotational envelope while retaining view direction. Rectangular `width`/`height` render requests support diagram panels.

Scene material presets are matte PLA, clay, silk PLA, glossy plastic and resin; studios are Light and Dark. Resin uses transmission/volume absorption, separate from output alpha. Runtime probing corrected Blender 4.5's exact `Anisotropic` socket name. The root GUI should default resin to Cycles and label EEVEE refraction as approximate. These are plausible visual presets, not measured filament/resin calibration.

Four-view composition uses build-frame Front/Right/Top/Isometric cameras with one shared physical scale. A 4096-square output has 64-pixel margins/gutters, 96-pixel labels and 1952x1856 render areas; the 2048 layout scales accordingly. Camera-relative lights and neutral shading produce transparent panels composed onto white. Exact-coordinate design topology adds visible crease/boundary curves, excluding coplanar triangle diagonals, with a 200,000-edge budget. A silhouette pass comes from geometry alpha, independent of bump normals. Bead inputs retain their physical geometry and receive silhouette outlines; the app does not pretend it can recover original CAD design edges from toolpaths.

## Evidence

- Initial native worker/export tests failed before modules existed, then passed after implementation.
- Four focused export checks and two protocol/quoting checks pass. The encoder check executes installed FFmpeg through the actual suspended/owned launcher in a `%04d`/Unicode directory, decodes the GIF, and confirms a corrupted frame preserves the previous destination.
- The diagram layout/camera check first failed before those contracts were implemented. It now verifies 4096 panel dimensions, canonical direction/up and shared scale.
- Python camera/crease tests failed before implementation, then passed; the renderer suite now has 16 passing checks.
- `cargo test --offline --test worker native_worker_exports -- --ignored --nocapture`: passed against real imported model 0. Initial 2048 sheet proof took 19.87 s; the final geometry-crease version took 14.69 s (different cache/startup conditions, not a comparative benchmark). Both use one worker for probe, error recovery, four renders, warm probe and cancellation. The final sheet was viewed at `artifacts/native-backend-probe/four-view.png`: four populated panels, intended axes, shared scale, visible hole/design edges and no coplanar triangulation wireframe.
- `cargo test --offline --test worker native_materials -- --ignored --nocapture`: passed in 25.74 s. It renders all five presets, verifies opaque and transparent pixels in each alpha PNG, exports and ffprobe-validates four MP4 frames (200 ms), cancels a long Cycles request, and starts/probes a new worker epoch. Artifacts are in `artifacts/native-material-probe`.
- Runtime tests are marked ignored for ordinary headless CI, but were explicitly executed here. Their fixture source is the already imported real-model snapshot under `artifacts/verified-models/model-0`; raw user files remain untouched.

## Section, AV1 and deployment checkpoint

`ExportKind` now also supports `SectionStill` and `SectionTurntable`; the latter encodes GIF. Any ordinary Still/Gif/Mp4 request can include `section: SectionSpec { axis, position_mm, keep_positive, cap }`. `layout` is Single or SideBySide; `video_codec` is H264 or Av1. New fields default to prior behavior. MP4 plus section/layout provides the AV1 section-turntable combination. `JobEvent::Analysis(Value)` delivers geometric inspection results, also retained as `analysis.json` in the render job.

Sections operate on the disposable mesh in build coordinates before turntable animation. Closed, consistently wound input uses Blender's exact Boolean intersection with a halfspace box. Open input uses an uncapped plane cut with explicit unavailable-area/volume warnings. Exact duplicate STL corners may be joined for topology assessment; proximity repair is not performed. Caps receive a distinct cut-surface material. Separate bead shells remain actual bead solids, so gaps are not filled by a whole-part envelope. Original bead volume is an algebraic closed-mesh estimate; overlaps between paths are not assessed, and the analysis explicitly says so.

Side-by-side panels retain identical full-source bounds, camera and lighting, even though one mesh is cut. A separate caption strip avoids covering rendered geometry. One 960-pixel panel request yields 1920×960 output; the per-image budget is 64 million pixels. Default camera fitting now accounts for the narrower dimension of rectangular panels. Render fingerprints include section/layout but exclude codec/destination, permitting verified-frame reuse across GIF, H.264 and AV1.

AV1 encoder selection probes the configured FFmpeg. It prefers available av1_nvenc, then libaom-av1, then libsvtav1, and reports hardware/software choice. It never substitutes H.264 for requested AV1. On the installed 2022 build, actual libaom-av1 encoding succeeded; ffprobe verified codec_name=av1, decoded frame count, dimensions and duration. Encoder errors preserve the prior destination. The low-resolution actual AV1 test is part of ordinary native export tests.

`renderer_script` first searches `renderer/` beside the running executable and then the source repository for development. Render identities hash the scripts actually selected. The Windows owned-process launcher remains unchanged.

Joined bead scenes now accept `joined_stadium_v1` plus explicit `bead_side_face_ranges`, ordered half-open triangle ranges. Every range has a multiple of 64 triangles and validated 32-vertex-ring triangle pairs. Only declared side faces are smooth; terminal cap faces stay flat. A red-to-green pure regression verifies malformed ranges/indices and the cap split.

Additional executed evidence:

- Five native export tests and two protocol/quoting tests pass; the AV1 test executes the installed encoder and verifies two frames / 100 ms.
- All 28 Python tests pass after joined-bead and rectangular-camera checks.
- Two actual section runtime tests passed serially in 36.17 s. A rotated 20×12×10 mm cube cut at build X=10 measured 120 mm² cross-section and 1200 mm³ remaining volume; the side-by-side four-frame GIF decoded as 256×128 / 200 ms.
- Two separate 0.44×0.2 mm bead paths, spaced 6 mm apart, measured 0.15811657 mm² cut area and 1.58116597 / 3.16233188 mm³ remaining/original mesh volume. This matches the polygon stadium approximation and half-volume expectation without filling the gap. Open triangle input returned uncapped geometry, null area/volume and a warning.
- Artifacts: `artifacts/native-section-probe/section.gif`, its retained panel/composite PNGs, and `artifacts/native-section-beads/` with bead/open PNGs and analysis manifests. The initial composed cube frame was visually inspected. The final narrower-dimension camera change is covered by pure geometry regression; the previous full render used the same camera orientation and a slightly tighter conservative fit.

## Remaining integration work

Full application-level visible UI behavior is root's validation task. MP4 defaults remain H.264. Multi-tool material face groups and build-up animation follow this checkpoint. Shader filtering at subpixel layer pitch remains an appearance limitation; no claim of calibrated bead joins or optical resin accuracy is made.
