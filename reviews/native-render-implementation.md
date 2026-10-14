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

Full application-level visible UI behavior is root's validation task. MP4 defaults remain H.264. Layer filtering is now covered by the measured checkpoint below; no claim of calibrated bead joins or optical resin accuracy is made.

## Multi-material and build-up checkpoint

`ExportRequest.animation` is the defaulted `AnimationMode::{Turntable,BuildUp}`. BuildUp supports GIF or MP4 and explicitly rejects section combinations and nonanimated export kinds. The render fingerprint includes animation mode, while output codec remains outside it. The worker protocol uses `mode: "buildup"` and reports `visible_layer` with each frame and `build_layers` at completion.

Scene `face_groups` entries contain `start_triangle`, `end_triangle` (half-open), linear `color`, integer `tool` and `label`. Ranges must be ordered/disjoint/in-bounds; gaps retain the base material. At most 256 unique tool/color material combinations are accepted. Copies preserve the selected finish and resin absorption settings while assigning each tool's color. Validation happens before Blender scene allocation.

Scene `layer_face_ranges` entries contain `layer_index`, `start_triangle`, `end_triangle` and `z_top_mm`. These must cover all faces in increasing build order; repeated layer indices must agree on height. Build-up requires real bead geometry. A face-domain integer attribute and Geometry Nodes deletion reveal complete deposited layers cumulatively. No envelope or infill is fabricated. Camera, lights and object pose remain fixed at the full source's framing. When the frame budget is lower than the layer count, each frame adds a group of complete layers; the final frame always contains the full mesh.

Executed evidence:

- Pure range/schedule tests first failed for missing contracts, then passed. The full Python suite is now 30 passing tests; native defaults/encoder/protocol checks remain seven passing tests.
- `cargo test --offline --test worker native_buildup -- --ignored --nocapture` passed against actual Blender/FFmpeg in 20.56 s. It rendered eight joined stadium layers: first-frame pixels contain the red first tool and no future blue layer; final pixels contain both tools. All eight frames were then reused for AV1 encoding, with unchanged file modification times. Unsupported build-up plus section was rejected before rendering.
- The same runtime test rendered a Cycles/OptiX resin specimen with both tool mappings and verified independent PNG alpha plus an explicit internal-interface limitation warning. First/final build-up PNGs and the resin PNG were visually inspected: fixed framing, increasing deposited geometry and both colors are present.
- A subsequent actual overlapping-shell resin extension passed in 4.40 s with prior frames reused. The fixture offsets a second complete bead stack by 0.45 mm, retains all 2048 input triangles and produces a distinct image. Its warning explicitly states that this is not a fused-resin simulation. Both resin images were inspected; this verifies retained-interface rendering and disclosure, not physical optical accuracy.
- Artifacts are `artifacts/native-buildup-probe/buildup.gif`, `buildup-av1.mp4`, retained job frames, and `resin/still.png` / `resin-overlap/still.png`. The build-up integration proof uses 256-pixel frames and eight frames, not a full-resolution performance benchmark.

## Larger outputs and layer-footprint filtering

Actual acceptance test `native_large_sheet_and_complete_section_animation` passed in 157.76 s against the supplied model-0 STL snapshot. The 4096×4096 shared-scale sheet took 23.876 s. An 80-frame section animation with 960-pixel panels produced a decoded 1920×960 GIF, 4.000 s / 20 fps, in 133.782 s. The latter includes 160 Blender panel renders plus debug-build CPU composition, PNG encoding and full GIF validation; it is not a render-only GPU benchmark. Artifacts and exact results are under `artifacts/final-large-outputs/`. The full sheet and quarter-turn frames 0/20/40/60 were inspected: consistent panel scale/framing, cut surfaces distinct from source, preserved holes, captions outside the image, no coplanar triangle wireframe. Source pitch is 0.2 mm and presentation rotation is [20,15,0] degrees. No supplied photograph was available for comparison.

Layer Appearance now applies a conservative image-footprint filter to bump **Strength**, preserving the millimeter height field, phase, first-layer schedule and cap-normal mask. Perspective footprint uses per-shading-point camera distance and sensor/lens/pixel scale; orthographic footprint uses view span/pixel count. The surface bound divides by absolute geometric-normal/view cosine and includes diagonal pixel support. Detail fades smoothly when the lower bound on projected physical pitch is between two and four pixels; unresolved/grazing detail fades out. This is an analytical conservative bound, not a claim that Blender exposes shader derivatives. `layer_filtering:false` exists for controlled comparisons.

Pure filter checks first failed for the missing contract, then passed; the complete Python suite is 31 tests. Actual `native_layer_filter_reduces_subpixel_temporal_residuals` passed in 43.36 s: unfiltered, filtered and matched smooth-reference **complete 80-frame loops** at 256 pixels, 0.02 mm pitch, and [23,17,0] display rotation. Mean absolute cyclic temporal RGB residual relative to the smooth reference fell from **0.0875633 to 0.0** on a 0–255 channel scale. The fully unresolved filtered fixture equals the reference; this intentionally measures removal of unresolvable shading variation, not all kinds of renderer noise or universal temporal stability. A separate resolved 2 mm pitch image retains five visible grooves and a smooth cap; it was inspected. `artifacts/layer-filter-proof/measurements.json`, three GIFs and `resolved-pitch.png` retain the evidence. Per-clip timings include different shader-cache states and are not an optimization speed comparison.

`docs/runtime-support.md` records installed executable hashes and official maintained-version candidates separately. Current artifact proof uses the tested pinned runtimes; a newer downloaded build must pass qualification before that identity is promoted.

## Shared CLI preparation and maintained-runtime qualification

`prepare` now calls the same `presentation::load_model` / `model_scene` pipeline as the desktop. Legacy defaults remain independent beads and clean STL. Explicit options select joined beads, bounded XY arcs, Marlin/Klipper command profile, tool changes, arc chord error, G28 home coordinates, explicit physical layer boundaries and clean/layer-appearance/experimental-stepped STL. The emitted scene includes the original source path and supported face/layer metadata. Invalid option combinations fail before geometry output. Valid geometry and a scene no larger than 1 MiB are prepared in an owned sibling staging directory; only a completed mesh/scene pair is renamed into the requested new job directory. Float32 precision failures and oversized scenes remove staging and leave no published job. Existing jobs and source files remain untouched.

Three new-contract CLI tests first failed on unsupported flags/missing source metadata. After implementation, all eight CLI tests pass, including joined arcs/tool changes/home/schedule, stepped STL, malformed options, 14,000-layer scene budget rejection and float32 triangle collapse. The fresh combined run of `cargo test --offline --test cli --test export --test worker` passed 15 ordinary tests; seven GPU acceptance tests remain opt-in and their earlier actual runs are documented above.

Portable Blender 4.5.14 LTS was downloaded from Blender's official versioned mirror and its ZIP SHA-256 matched the official manifest. Build `62c1db4208e8` and executable hash are recorded in `docs/runtime-support.md`. Actual EEVEE and Cycles/OptiX each rendered a 512 px still plus four 256 px turntable frames and encoded a fully decoded 200 ms GIF with four 50 ms delays and infinite loop. Evidence is `artifacts/verified-models/model-0/proof-eevee-478715a51367` (15.446 s) and `proof-cycles-0c67c3ea2a1c` (3.191 s). The Cycles still was inspected. This qualifies those limited checks on the maintained patch; the larger acceptance suite remains evidence for installed Blender 4.5.2. Installed defaults and external binaries were not replaced.

The initial candidate render exposed a strict proof-manifest mismatch: derived `fit_turntable` flags had been added to the recorded source scene. A regression first failed for the missing immutable-mode helper, then passed after mode-derived flags were applied only to a copied render specification. The manifest retains the original scene plus separately fingerprinted render mode. Fresh full Python discovery passes 37 tests. The failed earlier proof directory is retained rather than relabeled as equivalent evidence.

## Complete-animation palette comparison

`artifacts/dither-comparison/compare.py` encoded the same retained 80×960 bead frames with one shared full-loop 256-color palette, comparing Bayer scale 3 and Sierra2_4a on the pinned FFmpeg. Source PNGs were read-only. Both GIFs were fully decoded: 80 frames, 960×960, exactly 50 ms per frame including the last, 4000 ms total, loop=0. Bayer reproduced the original 33,064,535-byte GIF. Sierra produced 29,777,875 bytes (9.94% smaller).

Average absolute decoded RGB error against the source fell from 2.024397 to 0.498533 on a 0–255 channel scale (75.37% lower). Mean absolute cyclic change in quantization residual, including last-to-first, was 0.492634 for Bayer and 0.499013 for Sierra (1.29% higher). These are measured display-RGB errors, not perceptual scores or universal temporal-quality guarantees. The matched still and enlarged bead crop were inspected in `visual-comparison.png`: Sierra better retains the source gradients/highlights without the Bayer grid bias. Recommendation: choose Sierra2_4a as the default based on substantially lower error and size for this representative full loop, accepting the small measured temporal-residual increase. This does not claim every scene benefits equally. Old artifact identities remain Bayer; changing the default must not relabel them.

`metrics.json` retains complete delays, dimensions, timings, hashes, palette/tool identities and both error metrics; the two complete GIFs and reproducible script remain beside it.

Sierra2_4a is now the selected default in both native and development-proof encoders. New encoding manifests record `full,opaque,sierra2_4a`; native job manifests keep a separate encoding recipe including encoder hashes, leaving the render fingerprint independent so verified PNGs remain reusable. Tests first failed for missing encoding identity and then passed after the change. The actual native GIF/AV1 and preservation tests pass; full Python discovery passes 37 tests. Existing Bayer files and their earlier identities were retained unchanged.

## Original versus experimental stepped comparison

The ignored `write_expanded_geometry_proofs` fixture writer now saves the original mesh as well as stepped output, measured bounds, signed volume and actual slice sample heights for cube, wedge, dome, square aperture and thin wall. Production slicing code is unchanged. The explicitly run writer passed closed outward topology and float32 serialization for all originals and outputs.

`artifacts/stepped-visual-comparison/render.py` used one persistent Blender 4.5.14 worker to render eleven actual 768 px EEVEE stills in 15.138 s: five matched original/stepped pairs plus a separate joined-bead reference. Each pair uses the same orthographic camera, pixels/mm, neutral material and lights; Layer Appearance is off, so every shown tread is geometry. Retained source/mesh/script/runtime hashes and alpha-silhouette comparisons are in `metrics.json`. `comparison-sheet.png` was visually inspected. The cube, aperture and thin-wall pairs preserve the broad shape and opening; the wedge acquires explicit treads, and the triangulated dome acquires visible terraces. The 0.04 mm tube wall remains present, including the empty aperture. Some inner-corner shading artifacts occur in both original and stepped thin-wall renders; this image is not evidence of calibrated surface normals.

The joined-bead reference clearly differs: rounded deposited cross-sections with a hollow path-built interior, rather than full solid bands. It is a separate commanded-path specimen, not a slicer output matched to the five solids. No physical-print photos were supplied. Decision: keep the mid-layer stepped mode experimental, bounded to the supported closed-contour cases, with explicit sampling/sliver-rejection warnings. These images support that scope and do not justify general STL toolpath prediction, gap repair or photographic-accuracy claims.

| Fixture | Original volume mm³ | Stepped volume mm³ | Matched rendered alpha silhouette IoU |
|---|---:|---:|---:|
| Cube | 256.0 | 256.0 | 0.999992 |
| Wedge | 384.0 | 384.0 | 0.992874 |
| Dome | 126.221176 | 126.119729 | 0.978796 |
| Square aperture | 192.0 | 192.0 | 0.999992 |
| 0.04 mm wall | 5.0944 | 5.0944 | 0.999991 |

Volume agreement on linear/vertical specimens is consistent with midpoint integration; it does not imply identical silhouettes. IoU uses alpha >127 at the single shared 768 px camera and is not a multiview Hausdorff-distance bound. Strict all-target Clippy passed after the proof-writer and encoding updates.
