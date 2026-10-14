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

## Completed plan checkpoint before the expanded TODO

Tasks4-6 now have implemented native inspection/output/project paths and retained acceptance evidence. Root full suite:64 Rust integration tests passed;37 Python tests passed; strict all-target Clippy and format checks passed. Explicit GPU tests cover layer filtering, first layer/caps, physical transforms, per-face adaptive build-up, palette persistence, weighted transparency order independence, section gaps, resize/unload and checked buffers. Actual million-triangle1440p offscreen measurements are in reviews/viewport-implementation.md; they are not desktop FPS claims.

The 4096 shared-scale sheet and complete80-frame1920x960 side-by-side section GIF passed decode/timing checks. Original-versus-stepped cube/wedge/dome/hole/thin-wall comparisons plus joined-bead reference were actually rendered and inspected; the experiment remains bounded and opt-in. Complete Bayer/Sierra comparison selected Sierra2_4a based a9.94percent file-size reduction and75.37percent lower RGB error on the matched fixture, with1.29percent higher temporal residual. Native and proof encoding recipes now identify the selection separately from render identity.

Project persistence needed exact float round-tripping: the original smoke's project check failed because JSON moved a small camera coordinate one ULP. A failing regression reproduced it; enabling serde_json float_roundtrip corrected it. Later source review found five state-flow issues; all were fixed and confirmed in the packaged GUI: section position restore, missing-project dismissal, recovery/export bookkeeping, queued source/project intent and coherent saving. The actor/recovery readers enforce byte budgets before allocation; compact serialized scenes obey the actual1MiB limit. Background decoding wakes the UI, stale decode IDs are rejected, and newer preview display intent wins.

Maintained Blender4.5.14 official archive/checksum and both EEVEE/OptiX qualification passed. Portable release checkpoint artifacts/releases/turninator-0.1.0-windows-x64-FINAL-20261014-155450.zip is6,599,160bytes. A relocated executable launched from an unrelated directory with fresh preferences passed11Boolean desktop checks (including real cancellation/restart/cache recovery), null error and matching scene/preview revision11 on the107,970-triangle supplied egg. Payload/runtime hashes and ZIP contents matched. Exact176 dependency/font notices accompany the app; external tool binaries are not bundled. CIM process inspection was denied and is not claimed as verification.

The latest todo.md now additionally requests settle-on-face, camera keyframes/spiral/pan transitions and build-up combined with all scene options. Existing AV1 and paired sections are implemented; complete-layer build-up exists, but these newly requested combinations and controls remain active work. The checkpoint filename's FINAL predates those additions and does not mark the overall goal complete. User-required Conventional Commits override the initial plan's no-Git default.

## Expanded motion and model actions

The additional TODO implementation now includes native BVH face picking and normal-to-down presentation settling; authored camera keyframes with time, orbit/pan, signed whole-turn spirals, easing, loop closure and native playback; project persistence for timelines, sampled object angles and selected deposited stages; and immutable GIF/MP4 timeline export. Physical build coordinates remain unchanged by face settling. Whole-layer G-code build-up combines with studio/material/palette settings, resin/OptiX, moving cameras and paired capped sections. PNG/sheets and fixed-stage animations also accept selected deposited layers, including an empty build plate. This is complete-layer reconstruction, not machine-time/nozzle/flow simulation.

Current full checks:87 Rust tests passed with15 explicitly ignored runtime/proof/performance tests;42 Python tests passed; formatting and strict all-target Clippy passed. All four GPU correctness tests additionally ran and passed on RTX4070Ti/Vulkan. Serial real Blender timeline tests passed in31.39s, covering resin/tool colors/dark studio, spiral/pan-to-top, paired capped build-up, unchanged AV1 reuse timestamps, selected-stage PNG/shared-scale sheet and empty/partial sections. EEVEE moving-camera physical-layer filtering and composed resin sections also passed. Evidence: reviews/timeline-implementation.md, reviews/face-settle-implementation.md and artifacts/{timeline-section-proof,static-build-stage-proof,timeline-camera-proof}.

The launched maintained-Blender desktop smoke on the supplied107970-triangle spiral egg passed all16Boolean checks, including normal-to-down picking through the actual viewport rectangle, timeline project restoration, decoded four-frame GIF and sampled180degree object-angle preview with matching camera snapshot. All original cancellation/stale-import/recovery checks remained passing; result had null error and revision=preview_revision13. Evidence: artifacts/ui-timeline-smoke/smoke/{result.json,timeline-ui.png}. The smoke calls the same pointer handler as clicks; GPU picking tests independently compare hit triangles to rendered face colors. This is not an OS input-injection claim.

Review found stale stage/section previews, stranded build-up flags after STL imports, sample-angle/project state loss, omitted mesh/metadata cache overhead and pretty derivative scenes exceeding the compact1MiB worker limit. Fixes preserve authored motion on failed new-source imports, pause manual camera changes, invalidate effective stage edits/playback, clear unsupported build-up on accepted STL, persist sampled stages/angles, enforce checked snapshot reservations before writes and atomically write bounded compact derivative scenes. A red actor quota test initially reached the missing Blender executable; green rejects before writing the480020-byte mesh under200000-byte quota and preserves the destination. A derivative-scene regression covers12000layer ranges that fit compact but exceed pretty JSON. Expanded project tests pass8/8, including old schema compatibility and invalid atomic-save preservation.

The final composed single-tool G-code desktop workflow passed17Boolean checks with null error and revision=preview_revision14. It exported a four-frame resin/Cycles spiral/build-up alongside a capped section, then rendered the sampled180degree camera pose with exactly three of four deposited layers in both panels. The actual1536x768 output and desktop screenshot were inspected. Evidence: artifacts/ui-composed-gcode-smoke-single-tool/smoke/{result.json,camera-timeline.gif,timeline-stage-preview.png,timeline-ui.png}. The initial multi-tool fixture was correctly rejected by the default tool-change policy; the explicit multi-tool runtime fixture is separately qualified with that option enabled. The new GUI fixture removes tool-selection commands from a generated proof copy; no user model was modified.

## Final expanded portable release

Implemented plan tasks1–7 and all five current todo.md items are complete to their documented bounded contracts. Conventional feature commit7083714 contains the motion/model-action phase; earlier core/desktop/print/studio/package phases have their own Conventional Commits. No supplied physical-print photo or printer-specific G-code was available, so those broader fidelity claims remain unverified. Experimental midlayer STL stays opt-in; whole-layer animation is not machine-time playback.

Final archive: artifacts/releases/turninator-0.1.0-windows-x64-expanded-20261014.zip,6870624bytes,SHA256526a0ebca93066699b666b287ba5ce943b282557573477a0a6a60c7c9ce6c75a. Executable SHA2560f32677e77ca9e023b81c2b1085ff9e8e5cb8a06ab66bae643c51391353b08f9. The optimized package was rebuilt from7083714; the manifest correctly reports the shared checkout's unrelated dirty/untracked files and pins41actual source-file hashes. Seven payload files were verified against the inventory, and the archive contains no external tool executables. Exact176 dependency/font notices accompany it.

The ZIP was unpacked into a directory with spaces and launched from an unrelated working directory. With isolated settings and Blender4.5.14, the release passed16/16Boolean checks on the supplied spiral egg and17/17on the composed G-code workflow, including sampled-angle preview and exactly three visible deposited layers beside a capped section. Both had null errors and matching current/preview revisions. Actual screenshots/paired output were inspected; every retained runtime job's Blender/worker/scene hashes match the package. Verification: artifacts/packaged-expanded-smoke/{verification.json,verify.py}, smoke/timeline-ui.png and gcode smoke/timeline-ui.png. Current optimized CLI additionally prepared all nine supplied STLs with source hashes unchanged: artifacts/final-model-matrix/result.json.

Final verification remains87Rust tests plus four separately run GPU correctness tests,42Python tests, formatting and strict all-target Clippy. Packaging tests also reran6/6after generation/relocation. The15normally ignored tests are explicit GPU/runtime/performance/proof cases; relevant GPU/rendering cases were separately exercised as recorded above and in the earlier checkpoint reports. Historical initial-proof reports now point to current evidence rather than presenting resolved implementation gaps as current limitations.
