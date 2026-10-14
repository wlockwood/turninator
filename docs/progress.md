# Turninator implementation progress — final-plan.md

The user authorized plan synthesis and beginning implementation on October 14, 2026.

## Review and setup

- Two independent reviews completed: `reviews/geometry-review.md` and `reviews/architecture-review.md`.
- Synthesized `final-plan.md` adopts shared bead geometry, early constrained G-code import, Rust + Blender, and explicit approximation labels.
- Read-only device probe: RTX4070Ti, driver595.79,12282MiB; Blender4.5.2 LTS exposes OPTIX; Ryzen9950X also reported. These are capability observations, not render performance measurements.
- This directory has no Git repository or existing implementation. Work stays in the authorized workspace; no Git initialization/worktree/commits are required.
- Immediate scope: Tasks1–3 foundation and live artifact proof. Later desktop/release work remains on the plan.

## Interface preflight

| Producer / consumer | Shared contract | Outcome |
| --- | --- | --- |
| Tasks1/2 -> Task3 | TNMESH01 version1:8-byte magic,u32 version/vertex count/triangle count, f32 XYZ positions,u32 triangles | One checked binary contract |
| Task2 -> Task3 | Scene schema1: absolute mesh_path,geometry_kind,layer_height_mm,color,roughness,display_rotation_degrees,warnings | Agreed schema |
| Task1 -> Task2 | BeadSegment positions/width/height in millimeters; commanded Z is layer top | G-code parser feeds the same bead builder |
| Task3 -> Task4 | Versioned prefixed main-thread worker protocol | Initial proof precedes native lifetime/epoch integration |

## Decisions

- Ruling: independently capped bead segments are the first geometry representation — this makes actual ridges and toolpath patterns renderable immediately — cost if insufficient: replace corner joins with grouped continuous paths.
- Ruling: no general STL slicing, full GUI, or printer-physics prediction is claimed in the first milestone — these are later plan tasks — cost: those features remain unavailable until implemented.

## Active work

- Task1/2: Rust geometry, STL/G-code parsing and executable preparation CLI.
- Task3: Python worker, checked mesh loading, studio rendering, export proof and decode validation.

Verification evidence and artifact paths will be appended as commands complete.

## Implemented first milestone

- Rust library and runnable preparation/fixture CLI: physical uniform/first-layer schedules; binary/ASCII STL import; constrained sequential G-code state machine; rounded stadium bead geometry; checked binary mesh snapshots.
- Persistent Blender worker: checked JSON/mesh input, explicit OptiX selection, studio materials, fixed framing, still/turntable requests, typed errors, real frame validation and manifest-checked reuse.
- Proof controller: Windows Job Object ownership, FFmpeg shared palette, literal paths including percent sequences, decoded frame timing/loop checks, immutable recipe/tool hashes, encode-only retry and preserved output on failure.
- STL Layer Appearance was brought forward for the supplied models: physical build-Z groove shading, separate first layer, cap mask and preserved silhouette. Native matching shading/filtering remains later work.
- All9 supplied STLs imported with source hashes unchanged. Technics reports6 exactly zero-area omissions while preserving3,438 valid triangles and tiny nonzero features.

### Verification

- `cargo test --offline --all-targets`:25 passing integration tests (4CLI,8G-code,7geometry,6import).
- `cargo fmt --all -- --check`:pass.
- `cargo clippy --offline --all-targets -- -D warnings`:pass.
- `python -m unittest discover -s tests -p 'test_*.py' -v`:23 passing tests.
- Live full EEVEE bead proof:4096 PNG and80-frame960 GIF,51.60s combined.
- Live full Cycles/OptiX spiral-egg proof:4096 PNG and80-frame960 GIF,139.96s combined.
- Live small Cycles bead proof:two render requests in one worker;4-frame256 GIF,13.41s combined.
- Both full GIFs decode80x50ms=4s,960x960,infinite looping; final delay matches all others.
- Live cancel-during-preparation, independent Blender sentinel preservation, restart render, corrupt-frame repair and matching-frame reuse:pass, `artifacts/bead-demo-v2/runtime-ffa4efcfd8/result.json`.
- Live encode-only retry revalidated all80 frames without Blender startup.
- Independent cross-reviews found/corrected overexposure, cap smoothing, reader consistency, literal-percent paths, retry provenance and frame-manifest checks. Reports remain under `reviews/`.

### Additional decisions and limits

- Ruling: omit only provably zero-area STL faces and show their count — actual Technics input has6 duplicate-vertex faces and very small valid faces, so an epsilon would destroy geometry — cost if wrong: import boundary requires a new exact-predicate regression; originals remain untouched.
- Ruling: preserve segmented bead geometry with flat caps for this milestone — a real Blender normal probe exposed axial contamination from smooth caps — cost: visible corner divisions remain until joined-path generation.
- Ruling: bring optional Blender STL layer shading into the proof — user supplied real models to validate immediately — cost: native equivalent and subpixel filtering remain unimplemented and are explicitly documented.
- No desktop GUI, four-view compositor, broad real-slicer G-code support, joined beads, full adaptive/envelope slicing, resin/section/compare or native GPU performance claim yet.
- Persistent process is verified; warm geometry caching is not implemented. Actual GIFs are larger than desired. No physical-print-photo or subjective prolonged-playback comparison yet.

Tasks1–3 are implemented to the first-milestone scope above; some expanded proof cases in the plan (multiple pitches, tilted appearance capture, dense-memory matrix) remain scheduled validation. The next development step is Task4's desktop vertical slice, alongside joined-path geometry and filtering improvements supported by these artifacts.

## Native desktop and output checkpoint

Implemented the wgpu native viewport, asynchronous generation-gated imports, physical first/later layer controls, separate build/display poses, camera controls, and a persistent hidden Windows Job-owned Blender controller. The native studio displays a rendered preview and actual encoded GIF playback; immutable jobs report progress and support cancellation/retry. Projects retain edited print settings and report missing/changed source files. Settings retain machine paths and recent sources. Marked job storage rejects escape/reparse paths, retains failed frames, and enforces a configurable storage budget.

Native exports include verified PNG/alpha, shared-scale four-view sheets, GIF and MP4. Explicit AV1 output used the installed libaom encoder and ffprobe confirmed codec/timing; it does not silently substitute H.264. Section exports retain actual deposited voids, can show full/section views beside each other, and report limited geometric metrics. The rotated cube proof measured 120 mm2 cut area and 1200 mm3 remaining of 2400 mm3. Separate-bead proof retained the 6 mm gap; open geometry remains uncapped with unknown volume.

Validation: actual launched desktop camera-mount import/render/screenshots in artifacts/ui-smoke. Native GPU shader tests explicitly ran on RTX/Vulkan and cover caps, first layer, transforms, resize and unload. Rust whole suite passed; Python checkpoint 28 tests; strict all-target Clippy passed. Native live worker tests additionally exercised warm worker/error recovery, five materials, alpha, MP4 and busy cancellation/restart. These are separate from full desktop race/recovery and performance validation still in progress. Native materials approximate Blender lighting; the full expanded inspection/packaging phase is not complete.
