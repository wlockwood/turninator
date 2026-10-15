# Turninator: Final Design and Implementation Plan

**Goal:** Build a local Windows print-model studio that produces looping GIFs, quality stills, and four-view sheets, with optional physical layer appearance and genuine extrusion-bead geometry from supported G-code.

**Architecture:** One Rust package owns geometry, print settings, G-code interpretation, scene state, native interaction, and jobs. One application-owned Blender worker renders authoritative previews and exports with EEVEE or Cycles/OptiX. FFmpeg encodes the initial GIF/MP4 outputs. The same validated geometric mesh reaches native rendering and Blender.

**Execution:** The user explicitly authorized merging the two-agent review and immediately beginning implementation. No additional design approval is required. Begin with the runnable geometry/rendering foundation described in Tasks 1–3; later tasks describe the complete application. Record implemented scope and measured evidence in `docs/progress.md`; do not describe the whole application as finished when only the foundation works.

**Date:** October 14, 2026. This file incorporates `idea2-opus.md`, `idea2-codex.md`, and the two reviews in `reviews/geometry-review.md` and `reviews/architecture-review.md`. Original proposals remain unchanged.

## 1. Decisions from the reconciliation

| Topic | Final decision |
| --- | --- |
| Renderer | Both proposals now agree on Rust plus Blender. Use the established renderer; defer a native path tracer. |
| Real layer geometry | Adopt Opus's geometric emphasis. Prove rounded deposited beads early instead of making a general STL slicer a prerequisite. |
| STL preview | Keep clean STL and shader-based Layer Appearance. It is useful for unsliced/broken models and does not pretend to reproduce toolpaths. |
| Stepped STL | Preserve a bounded sliced-envelope experiment after the bead proof. Full contour booleans/repair/adaptive slicing are separate work with thin-feature tests. |
| G-code | Bring a constrained plain-text importer forward. Grow supported commands/flavors deliberately, rather than waiting until every inspection feature exists. |
| Auto detail | Select/filter detail once per job; never replace bead/stepped geometry with the original STL as the camera rotates. A display approximation must be explicit. |
| GIF encoding | Start with installed FFmpeg and verified timing/palette behavior. Compare gifski later if evidence justifies the dependency and distribution choice. |
| Jobs | Immutable snapshots, retained verified frames, encode-only retry and owned cancellation early. Cross-session resume UI and sophisticated retention later. |
| Inspection | Basic orbit/edges early; OIT, capped sections and comparison follow primary outputs. Bead scenes require actual infill geometry to display infill. |
| Accuracy | G-code rendering represents commanded deposition with a bead model, not a guaranteed photograph of the eventual print. |

This adopts Opus's strongest product direction while keeping Codex's physical-coordinate and lifecycle contracts. It also corrects several assumptions: all slicers do not share one sampling convention; polygon union does not repair arbitrary defective solids; every G-code Z move is not a layer change; path parsing is stateful; normal thresholds alone cannot reliably remove geometric layer-step edges; and large-mesh GPU fit/performance must be measured.

## 2. Input modes and scope

**STL Studio:** read-only ASCII/binary STL, visible assumed units, explicit print orientation, optional Layer Appearance, independent presentation pose. First settings support arbitrary uniform pitch (default 0.20 mm) and a separate first-layer height.

**G-code Print:** supported depositing moves become physical rounded beads with width/height provenance, ordered extrusion data, optional feature/tool/layer labels, and warnings. Top patterns, walls, support and infill emerge from the actual paths rather than painted stripes. Begin with a documented planar single-tool Marlin-style subset. Unsupported motion/geometry-affecting commands must fail clearly; unknown feature tags remain visible.

**Stepped Envelope, experimental:** horizontal section regions extruded through each layer interval as independently indexed closed layer solids. Real-model validation replaced Boolean joins between adjacent layers: those joins produced microscopic ribbons that failed contour, manifold and renderer-precision checks even on valid STLs. The stacked solids have the same sampled exterior and additive volume; internal interfaces remain and may show through transparent materials. Whole-layer face ranges drive build-up in native and exported output; physical sections cut layers independently. This estimates a sliced solid, not nozzle beads. Source topology is validated before slicing, and renderer precision must preserve sampled holes and gaps. Local self-crossing folds in generated slice outlines can be resolved only when their enclosed triangle has zero area at renderer f32 precision and lies within four reported coordinate tolerances. The import warning reports the count and maximum per-fold displacement; original contour conflicts and hole nesting are checked before and after this correction. No automatic source repair or gap closure is performed. This remains an explicit experimental option.

The initial implementation is a CLI and rendering proof, not a desktop release. Generated fixtures allow immediate progress. Real user G-code/STL/photo comparisons remain useful future validation; their absence does not block development.

## 3. Physical layer and bead model

STL physical conversion is `source -> rebased millimeters -> PrintPose -> build coordinates -> DisplayPose/turntable -> camera`. PrintPose places minimum build Z at zero; DisplayPose changes presentation only. G-code units are normalized to millimeters once, with source build placement retained. Blender receives meters through a fixed 0.001 m/mm conversion. Do not independently resize models to fit a unit cube.

Uniform layer boundaries cover the model: `0, h, 2h, ...`; with first layer `h0`, boundaries are `0, h0, h0+h, ...`. Validate finite heights in 0.02–2.00 mm and at most 50,000 layers. An explicit boundary list is the eventual common schedule for adaptive inputs; strictly increasing from zero. In the first STL UI, arbitrary adaptive schedules are not inferred.

Layer Appearance uses a smooth groove profile in build Z, with depth initially 0.01 mm at 0.20 mm pitch, bounded to 25% of the smallest active height. Suppress it on horizontal caps using unperturbed geometric normals. Filter away unresolved periodic shading; a conservative initial native fade is zero below two pixels/layer and full at four. Pitch never changes to make lines visible. Do not apply this groove shader over an already rounded bead mesh by default.

For supported planar G-code, extrusion Z denotes the bead's upper surface in the nominal layer model: center the bead at `Z - height/2`, not at nozzle Z. Cross-section is a rounded rectangle/stadium of width w and height h. For w>=h its area is `h*(w-h) + pi*h*h/4`. Metadata may supply width/height; otherwise estimate width from deposited volume per travel length and a known/assumed height. Record filament diameter and every fallback. [Slic3r flow model](https://manual.slic3r.org/advanced/flow-math)

Retractions create an extrusion debt. Positive E repays that debt before adding material; E-only unretraction does not create a line. G92 resets coordinate counters, not this debt. Travel and Z-hop do not create deposited layers. Maintain independent XYZ/E modes with explicitly documented dialect behavior. The first parser is sequential. Unsupported depositing arcs, volumetric extrusion, firmware macros and nonplanar/vase paths require explicit diagnostics until implemented. [Marlin command reference](https://marlinfw.org/meta/gcode/)

`BeadSegment` retains start/end mm, width/height mm, deposited volume, tool, feature, layer/height provenance and source line. Reject inferred width smaller than height for this stadium model rather than silently clamping it. The first geometry proof may use independently capped segments with documented overlapping corner joins. Joined paths/miter limits then replace corner approximations after straight/corner/reversal/closed-loop tests. No whole-print mesh union is necessary for opaque rendering.

Geometry budgets must reject overly large jobs before allocation. Bounds include bead extent, not centerlines alone. Retain all paths initially; do not unconditionally cull inner walls/infill, since cavities, holes, cutaways, transmission and unknown labels can expose them. Later optimization uses deterministic chunks, bounded tessellation and conservative visibility checks.

## 4. Scene, mesh and worker contracts

Start as one Rust package:

```text
src/lib.rs, src/main.rs              Shared library and initial CLI
src/model/{mod,stl}.rs               Canonical read-only import
src/print/{mod,schedule,gcode,beads}.rs Physical schedule, parser, mesh generation
src/scene.rs                        Versioned render request/camera/material data
src/mesh.rs                         Checked mesh data, bounds, binary serialization
src/render/{mod,worker}.rs           Later native worker controller/ownership
src/app.rs, src/viewport/            Later desktop interaction
src/export/                         Later job integration/composition
renderer/worker.py                  Persistent main-thread request loop
renderer/scene.py                   Mesh import, physical camera/studio/material
tools/prove-render.py               Worker/encoder integration and artifact checks
tests/                              Rust behavior checks and Python runtime tests
fixtures/                           Deterministic small STL/G-code examples
docs/progress.md                     Status, decisions and verification evidence
docs/performance.md                  Actual measurements
docs/visual-validation.md            Captures and interpretation
README.md                           Runnable commands and current limits
```

Initial `Mesh` contains physical mm positions and checked u32 triangle indices. Use a versioned binary snapshot: eight-byte magic `TNMESH01`, u32 format version (1), u32 vertex count, u32 triangle count, then little-endian f32 positions and u32 indices. Reject overflow, excessive budgets, non-finite values, exact-size mismatch, or invalid indices on both readers. Corners/named attributes can extend a new version after their semantic/domain/count schema is specified; do not reinterpret version 1.

Initial Scene JSON fields are `schema_version` (1), absolute `mesh_path`, `geometry_kind` (`beads` or `original`), `layer_height_mm`, `color` (three linear RGB values), `roughness`, `display_rotation_degrees` (XYZ degrees), and `warnings` (strings). Optional fields added during the first proof are `first_layer_height_mm`, `layer_appearance`, `groove_depth_mm`, and `bead_layout` (`independent_stadium_v1`, validated before cap classification). The worker derives bounds from the checked mesh; render recipe/engine/dimensions/alpha are snapshotted in the job request. Later scene versions add explicit camera/material/surface contracts without silently reinterpreting existing fields. Camera fit uses a conservative full-rotation envelope with a 10% margin. A canonical angle never duplicates the 360-degree endpoint.

Requests/events use protocol version 1, request ID and scene revision; desktop integration adds a worker epoch so restarts cannot accept stale events. Initial request kinds: `probe`, `render`, `shutdown`. Render options support still or numbered turntable frames, resolution, sample count, engine, and job directory. Emit prefixed `TURNINATOR_EVENT ` JSON on stdout and flush it; ordinary output remains diagnostic text. All bpy work stays on the main thread. Drain stdout/stderr continuously.

Use factory startup and disabled autoexec. The first controller owns the Blender process and performs a bounded probe plus two sequential renders in the same worker. Desktop cancellation must use a kill-on-close Windows Job Object and hidden processes. Process arguments are arrays. Never enumerate/kill unrelated Blender processes. A probe/exception must return a typed error, not leave the controller waiting indefinitely.

## 5. Materials, cameras and output

First proof: Matte PLA, neutral clay, Light Studio. Add silk/glossy plastic, Dark Studio and resin after opaque layers work. Resin uses Cycles by default; fine optical/internal-interface correctness gets separate tests. Native appearance approximates studio lighting, but geometry, units, layers, poses and camera must agree.

Probe EEVEE and Cycles/OptiX on the installed Blender 4.5.2 LTS. Live read-only discovery verified an RTX4070Ti with 12282MiB, driver595.79, Ryzen9950X, and Blender OPTIX device availability. Availability does not establish render speed. Before a supported release, select one maintained LTS patch, record build/hash and rerun probes; do not auto-update.

**GIF:** default 960x960, four seconds, 20fps, 80 angles `360*i/80` for i0..79; five centiseconds including final frame, infinite looping. GIF/MP4 framerate is configurable from 1 to 100 fps with saved preferences and a tooltip explaining cost and timing. GIF delays round absolute boundaries to centiseconds; MP4 retains the selected rate. Fixed lights/camera/exposure, opaque matte background, motion blur off. Render verified PNGs to a job directory. Shared FFmpeg palette; compare Bayer and Sierra dithering on complete animations. Keep actual encoded playback and size visible. MP4 is secondary from the same frames.

**Still:** 4096x4096 PNG, current composed camera, Cycles/OptiX quality preset, optional independent transparent background with floor removed. Bound custom dimensions at64MP with separate full working-memory/disk preflight. First proof can begin at smaller sizes before completing a 4K artifact.

**Four-view:** orthographic Front(-Y,+Zup), Right(+X,+Zup), Top(+Z,+Yup), Iso(1,-1,1). Use print/build frame, shared pixels/mm and camera-relative lights. 4096 square,64px margins/gutters,96px label bands gives1952x1856 image regions;2048 scales layout byhalf. Clean geometry/neutral finish default; printed option explicit. Derive edges from original/unperturbed design geometry or explicit topology; do not use bead shading normals to manufacture design lines.

Save immutable geometry/scene/render recipes before starting jobs. Completed frames require successful decoding. Encoding retries reuse matching verified frames. Use a destination-side temporary file, verify dimensions/timing/loop/alpha, then platform-correct publication; failed output preserves any existing destination. Supersampling/downsampling recipe belongs in the fingerprint; a different physical pitch, geometry, camera, script build or engine invalidates frame reuse.

## 6. Review focus and release constraints

- Sources remain read-only; all jobs/transforms/derived meshes belong to app-owned files.
- Units and inferred settings stay visible. No claim of slicer or physical ground truth from STL layers alone.
- No silent geometry/engine changes; deterministic job snapshots and generation/epoch checks.
- UI import/render/encoding runs asynchronously; previous successful model/preview remains visible.
- Stateful G-code interpretation handles reset/retraction/travel before creating geometry; unsupported commands cannot silently produce a misleading print.
- Measure complete CPU/GPU/disk workflow, not only engine timers. Inspect raw-frame and encoded animation separately.
- Tests cover finite inputs, budgets, topology/width assumptions, mode changes, first layer, transforms, corner bounds, corruption and destination preservation.

Initial performance targets remain tuning goals: native1M triangles around60fps at1440p; warm768 preview under2s; default opaque80-frame GIF and4K still under2min each. Resin and dense full-infill scenes are stress cases. No assertion that millions of bead triangles automatically fit12GB, no calendar promise until real measurements exist.

## 7. Implementation tasks

Use meaningful tests before substantive implementation, watch their expected failure, implement, rerun the whole relevant suite. Work in place and preserve proposal files. The user's subsequent instruction requires Conventional Commits at every completed phase, with additional feature commits welcome. Track evidence durably in `docs/progress.md`.

### Task 1: Rust geometry and import foundation

**Files:** Cargo package, `src/{lib,mesh,scene}.rs`, `src/print/{mod,schedule,beads}.rs`, `src/model/{mod,stl}.rs`, `tests/{geometry,import}.rs`.

**Interfaces:** `LayerSchedule::uniform(height_mm, first_mm, build_height_mm) -> Result<LayerSchedule>`; `build_bead_mesh(segments: &[BeadSegment]) -> Result<Mesh>`; `load_stl(path: &Path, millimeters_per_unit: f64) -> Result<Mesh>`; `Mesh::write_binary(path: &Path) -> Result<()>`; `Mesh::bounds() -> Result<Bounds>`.

- [x] Tests:20mm/.20 gives100 intervals; .30first/.20later gives0,.30,.50; invalid/nonfinite/budget rejection; known stadium cross-section bounds/area; bead Z extends from nozzleZ-height to nozzleZ; correct outward winding; corners preserve extents; binary/ASCII STL including `solid` binary header; invalid counts/NaN/empty; binary snapshot overflow/index checks.
- [x] Implement finite/bounded canonical geometry, physical schedules and independently capped stadium beads; document corner approximation. STL keeps original silhouette and source-unit assumption; omit only exact zero-area faces with a visible report, never tolerance-based topology repair.
- [x] Run `cargo test`, format and strict Clippy. Record actual test counts.

**Deliverable:** Tested geometry that can feed the rendering proof and later native view.

### Task 2: Narrow G-code importer and executable CLI

**Files:** `src/print/gcode.rs`, `src/main.rs`, `tests/gcode.rs`, `fixtures/`, `README.md`.

**Interfaces:** `parse_gcode(text: &str, options: &ParseOptions) -> Result<ToolpathData>`; CLI `turninator prepare <input.stl|input.gcode> --out <job-directory> [--layer-height <mm>] [--first-layer-height <mm>] [--filament-diameter <mm>]` writes meshbin and scene JSON. CLI `turninator fixture --out <directory>` generates a deterministic layered toolpath fixture.

- [x] Tests: absolute/relative XYZ/E, dialect mode-reset behavior, G92, extrusion debt/partial repayment, E-only moves, travel/Z-hop, first/variable layer metadata, width-volume inference, malformed compact words, unsupported arcs/volumetric/nonplanar paths, no empty output or guessed support exclusion.
- [x] Implement a sequential planar single-tool subset with explicit assumptions/provenance, bounded parsing and clear unsupported errors. Retain feature labels; do not advertise full slicer coverage.
- [x] Run the fixture through the actual CLI and inspect physical bounds/triangle counts; malformed input creates no publishable job. Run `cargo test` and strict checks.

**Deliverable:** Runnable real geometric layer input, with a documented route for supported user G-code.

### Task 3: Persistent Blender proof and real exports

**Files:** `renderer/{worker,scene}.py`, `tools/prove-render.py`, `tests/test_renderer.py`, `docs/{performance,visual-validation,progress}.md`.

**Interfaces:** `worker.py` implements protocol1; `prove-render.py --job <directory> --blender <exe> --ffmpeg <exe> --engine <eevee|cycles> [--size <pixels>] [--frames <count>] [--still-size <pixels>]`.

- [x] Test binary reader corruption/index validation and actual CLI-generated scene loading. Run parser/reader tests before implementation.
- [x] Probe exact tool/device identity; render a still and another request in the same Blender process, produce events/progress, exercise failure/shutdown. Own cleanup in a finally block with finite probe waits.
- [x] Render real bead fixtures with both engines; produce a full80-frame960 GIF and4K still on the selected recipe. Verify GIF with decoded delays/loop/dimensions, inspect actual artifacts, record timings/counts and shader/build identities.
- [x] Exercise encoder failure/damaged frame/occupied destination handling; retain frames. Verify no unrelated Blender processes are touched.

**Deliverable:** Visible geometric layers in actual PNG/GIF artifacts, not an untested renderer stub. This is the intended first implementation milestone.

### Task 4: Desktop vertical slice

**Files:** `src/app.rs`, `src/viewport/`, `src/render/`, `src/export/jobs.rs`, native event tests.

- [x] Add eframe/egui with compatible wgpu device, drop/Open supported inputs, async revision-safe loading, neutral opaque orbit/pan/zoom and physical settings.
- [x] Integrate worker epochs/hidden Job Object ownership, rendered preview, immutable output requests, progress/cancel, verified encoding and output playback.
- [x] Render original STL Layer Appearance in both paths; matching physical pitch/camera/pose with explicit approximation status.
- [x] Validate the launched desktop flow, rapid edits/stale import, cancellation/restart and native GPU contention while exporting. Compilation alone does not prove desktop behavior.

**Deliverable:** First usable application with G-code beads and STL studio modes.

### Task 5: Complete outputs and projects

- [x] Four-view shared-scale camera/edge/layout tests and real sheets: TOP / ISO above FRONT / RIGHT, with one largest visible projected dimension in mm per view; independent PNG alpha, optional MP4 from matching frames.
- [x] Versioned projects/settings, edited pitch preservation, missing/changed source relocation, encode retry/recovery UI, cache ownership and bounded disk policy.
- [x] Supported tool-build documentation, font/dependency notices, fresh-settings smoke and runtime checks.

**Deliverable:** All primary outputs and reproducible local saved setups.

### Task 6: Expand real printing and inspection

- [x] Joined bead paths, variable widths, supported arcs/firmware flavors, feature/tool display, metadata adapters and explicit adaptive schedules, each with real fixtures and false-assumption diagnostics.
- [x] Compare a bounded STL stepped-envelope experiment on cube/wedge/dome/holes/thin walls against original/bead/photo references. Choose whether to extend, keep experimental, or obtain toolpaths from an external slicer. Midlayer slicing is an explicitly selected approximation.
- [x] OIT/section inspection with actual infill, optional reliable caps on supported solid meshes, build-up animation and multi-material mapping. Add resin after testing overlapping bead/internal-interface limitations.

**Deliverable:** Broader realistic print visualization without asserting unmeasured physical defects or universal file support.

### Task 7: Additional TODO motion and model actions

- [x] Explicit H.264/AV1 output controls with actual codec/timing verification and verified-frame reuse.
- [x] Paired full-model/section PNG and turntable outputs with actual deposited gaps, bounded caps and honest geometric metrics.
- [x] Click a visible triangle to align its normal down; original STL/G-code retain their physical build frame. Enabling stepped STL uses current down, active stepped face alignment rebuilds slices, and Use current down for layers applies manual pose changes. Commit the baked pose only after successful import, preserve intervening display edits and project roundtrips, and use explicit Build rotation for fixed sheets. Validate actual picking, slicing and rendered surfaces.
- [x] Authored camera keyframes with transition times, orbit/pan, signed whole-turn spirals, easing, duration and loop closure; shared native/export sampling and saved projects.
- [x] Basic turntable speed and duration controls with total turns, partial rotations, saved preferences and exact-pose frame reuse across repeated turns. Matching camera/rotation/deposited geometry renders once; all ordered frames retain the selected GIF/MP4 timing (default 20 fps).
- [x] Framerate control with tooltip, visible only for animation outputs; saved preferences, rate-aware counts/turns/timelines/build-up, paired sections, cache identities and validated GIF/MP4 timing from 1 to 100 fps.
- [x] Retain matching render frames across separate exports and app restarts; share GIF/MP4/codec variants, validate PNGs before bypassing scene preparation, repair corrupt frames, invalidate visual/sampling/runtime changes, and manage storage without reserving duplicate frames.
- [x] Saved output-format selection with contextual controls: MP4-only codec, PNG-only alpha, per-format quality and layout previews, animation-only motion options, source-specific physical settings and controls for active sections/layers/palettes. Fixed sheet presentation excludes hidden pose/material/palette state.
- [x] Contextual hover help across active and disabled controls, including fields, dropdown choices, motion, geometry, viewport interaction, export, tools, recovery and retained jobs; verify actual tooltip text for enabled and disabled widgets.
- [x] Native inspection and rendered/encoded previews use aligned image rectangles with shared size and aspect ratio; headers and playback controls preserve alignment, and window resizing scales both without changing the saved camera or render cache.
- [x] Complete-layer G-code build-up combined with materials/studios/palettes, resin/Cycles, camera motion, sections and paired layouts. Selected static stages also support PNG/sheets/fixed-stage clips; sampled angles/stages persist in projects.

These items use the same bounded source/import contract as the main plan. Build-up reconstructs deposited layers, not printer elapsed time, nozzle travel or filament dynamics. No printer photographs were supplied; stepped STL remains an explicitly selected experiment after original/stepped/bead comparisons. Physical stability and fused-resin optical behavior are outside the implemented normal-alignment and mesh-illustration contracts.

## 8. Completion evidence

The full release is complete only after: supported sources load with visible assumptions; layers remain attached through presentation/animation; real G-code deposition paths render with intended width/height; all three output modes decode correctly; complete loops visibly retain detail without objectionable shimmer; runtime cancellation/races preserve valid work and destinations; projects reproduce print setup; and measured hardware/tool/artifact evidence is recorded.

The user subsequently authorized completing the entire plan and todo.md. All task checklists now reflect the implemented bounded contracts and recorded checks. Release/package identities, actual desktop results, geometry/rendering comparisons and remaining physical-fidelity limitations are recorded in docs/progress.md and the linked review/evidence files. Future work can extend the documented printer/slicer subset and material models without repeating completed tasks.
