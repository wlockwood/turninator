# Turninator architecture reconciliation

Reviewed `idea2-opus.md` and `idea2-codex.md` in full. This review concerns application architecture, worker lifecycle, export correctness, and delivery order. It incorporates the user's interest in realistic G-code deposition rendering. It is a design review; it does not claim any renderer or GPU benchmark passed.

## Recommendation

Keep the common foundation: one Rust desktop package, a modest native viewport, and one application-owned Blender worker for authoritative previews and exports. Adopt Codex's explicit physical coordinates and immutable export snapshots. Adopt Opus's emphasis on actual deposited material, but implement a shared extrusion-bead representation before attempting a general STL-to-stepped-solid reconstructor.

Expose two honest input workflows:

1. **STL studio:** clean geometry or optional Layer Appearance. Uniform pitch and first-layer height are physical inputs, while bump mapping approximates a printed surface without changing silhouette.
2. **G-code print:** actual commanded extrusion paths converted into bead geometry, retaining perimeters, top patterns, seams, supports, and infill where supplied. This simulates commanded deposition; it cannot promise physical defects, exact printed dimensions, or material behavior.

Both workflows produce the same scene/render/export contracts. A later STL slicer could produce `BeadPath` records if justified. A generic stepped envelope should remain optional because robust contour booleans, degeneracies, thin walls, and topology consume effort without reproducing nozzle paths.

## Reconciliation decisions

| Concern | Recommendation and reason |
| --- | --- |
| Geometry ownership | Rust owns validated source data and physical transforms. Start with a narrow Python proof using explicit shared JSON, then port CPU import/path construction behind the same contract. Do not build throwaway scripts that hide units or dimensions. |
| Layer rendering | Keep simple STL bump appearance; prioritize genuine G-code bead meshes for silhouette/top-pattern realism. Distinguish them in labels and acceptance criteria. |
| Schedule | Uniform plus separate first layer for STL. G-code carries actual extrusion heights and layer metadata; do not extract layers from every Z move because Z-hop is travel. |
| G-code parsing | Sequential state machine first. Chunk-parallel parsing requires correct carried modal state and is unjustified initially. Supported commands/flavors must be explicit. |
| Worker | Main-thread bpy execution, prefixed events, explicit worker epoch, tracked process tree. A basic blocking main-thread request loop is enough; a command-reader thread is optional until needed for coalescing. Cancellation terminates the owned process rather than waiting for a queued command. |
| Frame reuse | Immutable scene snapshot, job-local frame directory, verified frame completion records, encode-only retry immediately. Cross-session resume UI and cache eviction policy can follow. |
| GIF encoder | Start with installed FFmpeg after live filter probing. Test output, including final delay. Consider gifski only if measured visual improvement warrants an additional dependency/distribution decision. No encoder is assumed universally superior. |
| First materials | Matte PLA and neutral clay; simple Light studio. Add silk, PETG, dark studio and resin after the bead/animation proof. |
| Viewport | Basic opaque mesh orbit plus authoritative rendered preview. OIT, capped sections, linked comparison, and ghost overlay follow primary outputs. |
| Four-view edges | Use clean unperturbed geometry or explicit topology. Normal-angle thresholding cannot reliably suppress genuine stepped-layer normals while preserving all design creases. |
| Quality fallback | Never silently change requested engine or geometry mode. Capability failure produces a clear result; an explicitly chosen fallback becomes part of the snapshot. |
| Benchmarks | Record measured timings, tool/device identities and actual artifacts. Remove calendar promises and unverified claims that ten million triangles fit every material/scene. |

## Data and module contracts

Use one Rust package; neither a workspace nor custom rendering plugin system is required yet. Keep GPU resources, child-process handles, and UI state out of serialized scene data.

| Owner/module | Contract |
| --- | --- |
| `model/stl` | `load_stl(path) -> MeshData`: f64 rebased physical source positions, checked indices, corner normals, topology diagnostics, source hash. Read-only source files. |
| `print/gcode` | `parse_gcode(reader, ParseOptions) -> ToolpathData`: supported dialect, millimeter coordinates, tool identity, extrusion amount, feature labels/provenance, warnings and unsupported motion diagnostics. |
| `print/beads` | `build_beads(paths, BeadOptions) -> RenderMesh`: deterministic segment joins/end caps, bounded tessellation, counts/bounds and memory estimate; physical width/height are inputs. |
| `print/surface` | `LayerAppearanceSpec { schedule, depth_mm, strength, seed }`: applies only to original STL surface, not over the already generated bead profile by default. |
| `scene` | `SceneSpec { schema_version, source, build_transform, display_transform, camera, material, studio }`: source is a tagged STL surface or toolpath scene, with validated identifiers and finite values. |
| `render/protocol` | `Request { protocol_version, worker_epoch, request_id, scene_revision, job_id?, kind, payload }`; events repeat identity and include typed status/error fields. |
| `render/worker` | `start`, `submit`, `cancel`, `shutdown`: owns process, reader threads, bounded request queue, and Windows Job Object. No unrelated-process enumeration/termination. |
| `export/jobs` | `JobSnapshot { job_id, scene, render_recipe, geometry_hash, scripts_hash, tool_build, frames }`; separate mutable completion journal and output status. |
| `export/gif` | Encode a verified ordered frame list into a destination-side temporary output, decode-check timing/loop/dimensions, then publish. |
| `export/diagram` | Four orthographic cameras consume build-space bounds and a single pixels/mm scale; composition consumes decoded panel images and bundled font. |

`ToolpathData` should retain ordered moves and grouped `BeadPath` records. A path contains points in build-space millimeters, width/height or per-segment values, tool, feature, optional layer index, and source-line spans. Unknown feature classification remains `Unknown`; it does not authorize removal. Bounds must include the bead radius rather than only the centerline.

Track width/height provenance: slicer metadata, E-derived estimate, or explicit fallback. E-derived width requires filament diameter and accounting for retraction repayment. Report missing assumptions. Start with a supported planar G0/G1 subset and absolute/relative XYZ/E modes, units, resets, tools, and retraction. Unsupported depositing arcs/nonplanar paths must be rejected or clearly omitted with diagnostics until implemented; never draw an inaccurate straight chord as if exact.

Keep bead geometry generation common to native and Blender paths. A versioned mesh file with named attributes is suitable once bulk transfer matters. The first proof may use a compact checked JSON mesh/path fixture; large production geometry needs a bounded binary format. Validate schema, byte lengths, finite values and index ranges before allocation or upload. An attribute schema needs type, domain (point/corner/face), count, and semantic name, not just an untyped bag.

Physical spaces remain source -> unit conversion/rebase -> build frame -> display pose/turntable -> camera. Blender receives meters at a fixed 0.001 m/mm. Build coordinates or attributes remain attached to the geometry; presentation rotation must not change layer phase. G-code coordinates already have physical units after parser normalization; never additionally apply STL's assumed-unit scale.

## Worker and job state machines

Worker states: `Stopped -> Starting -> Ready -> Busy -> Ready`, with `Starting/Busy -> Failed`, and `Starting/Ready/Busy -> Stopping -> Stopped`. Every launch receives a new worker epoch. Request identity alone is insufficient if ids restart after a crash. Attach the process to a kill-on-close Job Object before allowing unrestricted child execution; fail visibly if ownership cannot be established.

The application continuously drains stdout and stderr. Protocol lines have a bounded size; ordinary logs go to a bounded log stream/file, and invalid prefixed JSON fails the request rather than freezing the reader. All responses flush promptly. Probe returns exact Blender build, protocol version, device capabilities and supported engines before a scene request. Set finite startup/probe timeouts and report silence; avoid a hard timeout for long renders unless user configured it.

Render work runs on Blender's main thread. Final export owns the worker; the UI can orbit/edit independently and request a pending latest preview. It cannot mutate the export scene. On cancellation, kill the tracked tree, drain/join readers with bounded waits, mark the active request cancelled and preserve verified frames. A new epoch starts lazily. Late events from the prior epoch or old scene revision cannot replace current preview state.

Job stages: `Preparing -> Rendering -> Encoding -> Validating -> Publishing -> Complete`; any active stage can become `Cancelled` or `Failed`. Record stage, failing operation, tool output, frame count and reusable artifacts. Destination selection does not enter the render fingerprint; camera, appearance, bead filters, quality, renderer scripts and tool build do. This allows retrying encoding or publishing to another path without rerendering.

Write each frame to a temporary name, decode/check it, then promote and record completion. Before reuse, confirm dimensions, expected file identity and matching render fingerprint. A manifest saying complete does not establish a readable frame. Keep enough disk preflight for frames, supersampling and temporary output; distinguish estimate from a guarantee. No automatic cleanup should delete an active job's frames.

## Export correctness

- GIF default: 960 square, 80 frames, angles `360*i/80` for i=0..79, 5 centiseconds each including final frame, infinite loop, opaque background. Inspect decoded timing and actual animated playback. Nominal ffprobe frame rate alone is insufficient.
- FFmpeg receives argument arrays. Use a generated safe job directory and numbered frame names; arbitrary user basenames must not become image-pattern syntax. Capture failure output and retain render frames.
- Keep exposure, lights, framing and material parameters stable across frames. Noise seed and denoising recipe are measured choices. Compare whole loops rather than a single successful still.
- Downsample to final dimensions before palette generation. If supersampling is used, perform correct linear-light filtering and correct premultiplication for alpha; flatten GIF backgrounds before quantization.
- Frame caches can be reused for alternate encoding settings. A request for a larger image cannot reuse lower-resolution frames as if they were newly rendered at that resolution.
- Still export uses the snapshotted current camera, with independent background alpha. Transmission and transparent background are separate properties.
- Four-view export uses Front(-Y), Right(+X), Top(+Z with +Y up), Iso(1,-1,1), build-frame orientation and shared physical scale. A 4096 sheet with 64-pixel margins/gutters and 96-pixel label bands has 1952x1856 image panels. Fit all projected extents, not four independent bounds scales.
- Publish through a verified temporary file in the destination directory and Windows-correct replacement. Encoding, validation or publication failure must leave an existing destination intact. Do not claim atomic replacement based on a cross-volume move.

## Concrete delivery order

1. **Rust CLI plus renderer/bead proof, immediately:** Rust core/CLI owns validated uniform/first-layer schedules, a constrained plain G-code parser, stadium bead mesh generation, meshbin and explicit scene JSON. Application-owned Python scripts consume that contract with matte PLA and a simple studio. Use deterministic tiny paths, one real still and a short decoded GIF. Add a persistent request/probe/render loop, render twice in the same process, cancel/restart via the controller. Log exact installed tools and selected device. This is the first runnable implementation, not a whole application claim.
2. **Complete visual proof:** 80-frame 960 GIF, 4K bead close-up, original STL Layer Appearance comparison, both candidate engines on a small matched fixture, first-layer/print-pose tests. Promote the best measured recipe; do not require resin/OIT/diagram edge work before this proves the core value.
3. **Broaden input and CLI reliability:** canonical STL import, additional supported G-code cases, transforms and rotation-envelope fitting. CLI renders supported input through the maintained worker and reuses verified frames on encoding retry.
4. **Desktop vertical slice:** drop/open supported files, async import with generations, basic orbit, physical print settings, material color, manual rendered preview, GIF/still export, visible progress/cancel and actual output playback. This is the first useful application.
5. **Primary-output completion:** shared-scale four-view sheet, quality presets, project/settings persistence, reliable publication and corruption recovery, selected tested tool build documentation.
6. **Expanded printing and inspection:** more G-code commands/flavors and fixtures, support/tool visibility, full infill sections, translucency, silk, OIT, comparison, richer caching/resume. Add only after the existing output pipeline remains validated.

The first implementation pass should finish phase 1 with runnable commands and artifacts, then continue according to the final plan. It need not pretend phase 1 satisfies every release acceptance item. Generated fixtures let work begin without asking the user for files; real STL/G-code/photo comparisons improve later realism assessment.

## Meaningful proof and failure cases

Use focused mathematical/parser tests and actual runtime artifacts. Avoid tests that only repeat constants in the implementation.

- Parser: relative/absolute E, G92 E reset, negative retraction, partial repayment, stationary extrusion, zero-length moves, unit changes, missing metadata, tool change, Z-hop, unsupported depositing arc, malformed/huge input. Verify spatial paths and positive deposited volume, not merely parsed line counts.
- Beads: straight path known bounds/cross-section, 90-degree corner, reversal, disconnected paths/end caps, variable width/height, thin gap and closed loop. Look for cracks, overlaps, collapsed tangents and inverted normals in the actual render.
- Transforms: known-size asymmetrical model; changing display pose rotates printed detail with the part; changing build pose changes print direction; source translation does not change layer phase. Check first-layer boundaries independently.
- Worker: two sequential real renders with reused scene, mixed logs, malformed event, crash, cancel during startup/render, restart, and delayed stale completion. Assert current preview survives failures and old worker processes are gone.
- Export: corrupted/missing frame, encode failure, mismatched render fingerprint, existing destination, disk error, Unicode/space/percent filenames, still alpha and exact GIF last-frame delay.
- Visual: raw-frame and encoded loops, bead silhouette, slopes/top paths, hidden-feature policy, caps, narrow gaps, camera envelope through a full revolution, diagram axis labels and shared scale. Record what was inspected versus what was only numerically checked.

Do not apply unconditional infill/inner-wall culling. Cavities, cutaways, transparent materials, missing outer skins, bridges and unknown feature tags can make those paths visible. The initial proof should retain all fixture paths; later visibility optimizations need conservative rules and before/after renders.

Avoid describing G-code as exact physical ground truth: it specifies commands and estimated extrusion volume, while pressure, cooling, shrinkage, acceleration and printer mechanics affect deposited shape. Likewise, stencil-capping a bead scene does not recreate infill; actual bead geometry must provide it.
