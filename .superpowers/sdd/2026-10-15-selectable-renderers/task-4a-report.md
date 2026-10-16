# Task 4a pure scene helper report

Implementation commit: `5f379d89d100c0780d2fac368d13b3c3102000ab` on `codex/render-scene-core`, based on `c95aca1`.
Worktree: `D:/code/turninator/.worktrees/render-scene-core`.
This report is a separate documentation commit so it can cite the exact implementation SHA.

Task 4a is implemented and CPU verified. This does not complete Task 4: prepared Blender geometry, legacy comparison, pipeline integration and image qualification remain Task 4b work.

## Scope and interfaces

Owned changes are `src/render/{geometry,materials,color}.rs`, the three requested integration test files and renderer fixtures, plus eight lines in `src/print/slice.rs`. The coordinator explicitly approved committing the three minimal `src/render/mod.rs` declarations so the commit compiles independently. No dependencies, frozen contract types, Python, Blender adapter, pipeline, sequence planner, UI or SDK files changed.

- `realize_frame(&RenderSceneSnapshot, &FrameSpec) -> Result<RealizedScene>` preserves accepted source mesh bytes and full-source reference bounds. Mesh positions and normals remain build-space millimeters/Z up; aligned f32 build coordinates never receive display/turntable transforms. Source/generated vertex identities plus final normal bits share vertices where allowed. Independent bead shells and stepped layer interfaces retain separate identities. Material IDs follow source faces through selection and clipping; generated build caps use base material and section faces use the reserved cut material.
- Original STL uses exact-coordinate welding only in a disposable topology view, allowing duplicated STL corners to qualify as closed. No nearby vertices are merged. Beads are partitioned by actual indexed connectivity; stepped ranges are processed independently by layer. Closed original hollow shells retain their holes; disconnected strands retain gaps. Nonmanifold/open sources remain uncapped with unavailable measurements and explicit warnings. Ambiguous cap contours fail rather than being repaired.
- `BuildUpSolid::cap_precise` exposes the existing robust cap operation at f64 build precision. Existing `cap` still applies its original native f32 quantization and removal of exactly collapsed generated ears. General axis sections use right-handed cyclic axis permutations and an orientation-preserving two-axis sign change for reversed keep direction. Successive build/section caps are clipped independently from the original topology, avoiding fabricated closure of open inputs and avoiding duplicate coincident caps.
- Whole-layer counts handle zero, complete, adaptive original tops and nonconsecutive deposited layer IDs. Original heights are relative to the source build minimum. Empty realized arrays are legal even though calling `Mesh::bounds()` on that empty result is not. Empty section analysis retains the historical empty-stage shape.
- Section analysis uses the frozen `SectionAnalysis::from_legacy` validator and existing field names/units. Signed volume arithmetic is rebased to full-source minimum coordinates to reduce cancellation at large/negative coordinates. Bead analysis retains the algebraic-volume/overlap warning. Stepped analysis identifies additive independently closed layer volumes.
- `resolve_display_transform(scene, angle_degrees)` returns row-major affine rows matching `spinZ(angle) * (Rz*Ry*Rx*p - [fullRotatedCenterX, fullRotatedCenterY, fullRotatedMinZ])`. It scans all full-source vertices. Adapters must use it instead of centering or flooring a clipped stage.
- `layer_surface_normal(appearance, build_z_above_min_mm, build_normal, gain)` and `layer_filter_gain(pitch_mm, pixel_footprint_mm, view_cosine)` provide the shared analytic physical bump/legacy filter. Adapters must evaluate the bump per shading sample in build coordinates before display transforms. Realized base normals alone do not bake subpixel grooves into mesh vertices.
- Sheet design edges use exact-coordinate topology and the legacy 35 degree crease threshold, limited to 200,000 edges. They are produced for `Top`, `Iso`, `Front`, `Right` panels. `Single`, `Full`, `Section` frames do not acquire a sheet-only capacity limit.
- `resolve_materials(scene)` yields base, first-use-order unique tool/color materials, then one reserved `SectionCutSurface`. Linear colors and all legacy physical preset formulas remain unchanged, including resin absorption per millimeter and zero preset scattering. A synthetic physical-material test exercises nonzero scattering serialization without changing legacy defaults. The cut surface matches Blender linear `[0.5, 0.18, 0.065]` and roughness `0.65`.
- `normalize_rgba(NativePixels)` validates exact checked input/stride/output lengths and 1..=8000 dimensions, strips padding, changes BGRA/RGBA and row origin, unpremultiplies in the declared producer transfer domain, encodes linear values to IEC sRGB once, and zeros RGB when alpha is zero. Output is tightly packed top-first sRGB RGBA8 straight alpha. HDR/tone mapping is upstream and intentionally absent from this 8-bit boundary, as agreed with the coordinator. No copied AgX/GPL implementation was introduced.

## Frozen conformance artifacts

`tests/fixtures/renderers/conformance.json` copies the approved design thresholds exactly:

- Geometry relative error <= 0.00001, absolute floors 0.0001 mm and 0.0001 square mm.
- Explicit large-coordinate ULP fixture: at 1,000,000 mm, the next f32 value is 0.0625 mm away. No tolerance topology repair.
- Cube and hollow 512x512 masks: boundary changes within one pixel only, IoU >= 0.995, no lost hole or disconnected strand.
- CPU transfer error <= one 8-bit code value; exact byte/index/range/frame/layer counts.
- Physically unchanged layer pitch; fixed-camera closeup crest spacing within one pixel.

The fixture folder includes deterministic 10 mm cube and closed hollow square-tube TNMESH01 files, analytical grayscale mask PNGs, and a fixed orthographic camera. Outer projected coverage is `[96,416)` square; the hollow tube has an empty `[192,320)` square. Foreground counts are 102,400 and 86,016 respectively. These masks/assets were authored procedurally for this task. CPU tests decode them, validate closed input/cap areas/volumes and retain the hole. No backend was qualified against the masks in this subtask.

## TDD evidence

The initial `normalize_rgba` and `resolve_materials` stubs returned `not implemented`. Focused runs showed five color behavior failures and three material behavior failures at the expected unwrap points; implementing the conversions/formulas turned those suites green. Invalid-input tests and static conformance assertions are accompanying boundary/asset checks, not claimed as new behavior failures.

All fourteen first geometry regressions failed against the geometry stubs. After the initial implementation they passed. Further focused regressions demonstrated and then fixed:

- Coincident build/section caps: measured area `200 != 100` before the duplicate-cap fix.
- Zero-layer stage without a schedule: `original build stage requires solid layer tops` before the unconditional empty-stage handling.
- f32-collapsed clipping: unexpectedly returned a realized mesh before the precision rejection.
- Flat-face vertex sharing: cube produced 36 vertices instead of the required 24 before exact identity/normal indexing.
- Independent/legacy joined bead declarations: malformed index layouts were accepted before matching the existing producer contract validation.
- Shared physical pitch filter: test failed with `not implemented` before extracting the legacy formula.
- Ordinary frame with 200,001 design edges: failed with the sheet budget error before restricting ink generation to sheet panels.

The valid-source capacity regression builds an accepted immutable source whose normal splitting exceeds the derived vertex limit, verifies an explicit `derived render capacity budget exceeded` error, and compares the source hash before/after. The final geometry suite has 24 tests; the exact tests are listed below.

## Verification

All commands used `CARGO_TARGET_DIR=D:/code/turninator/target`; Cargo serialized shared-target builds. No GPU, native SDK or Blender acceptance commands were run.

- `cargo test --offline --test render_scene_contract --test render_geometry --test render_color`: **35 passed**, 0 failed (24 geometry, 6 transfer, 5 materials/contract).
- Final `cargo test --offline`: **241 passed**, 0 failed, **34 existing opt-in tests ignored**, across 40 result groups including doc tests. A previous full run passed 239 tests before adding final mask qualification and ordinary-frame edge-budget regressions; the full suite was repeated after the edge behavior change.
- `cargo clippy --offline --all-targets -- -D warnings`: passed. Two initial local collapsible-if style diagnostics were fixed before the final pass.
- `rustfmt --edition 2024 --check` on the seven authored Rust helper/test/fixture modules: passed.
- `git diff --check` and path-limited cached whitespace checks: passed.
- Full final CPU output is retained locally at `.superpowers/sdd/2026-10-15-selectable-renderers/task-4a-cpu-tests.log` (not committed).

## Capacity and integration limits

The coordinator ruled that this first helper keeps `Mesh`'s existing 2,000,000 vertex / 4,000,000 triangle hard limits for derived geometry too. Splitting can therefore exhaust derived render capacity even for a structurally accepted source; it returns an explicit capacity error and never drops source geometry. Exact indexing reduces needless splitting but does not weld independent bead/layer shells to avoid the limit. Arithmetic and derived face/array capacities are checked; measured end-to-end memory/performance and stricter per-backend host/GPU budgets still belong to adapter qualification.

Generated triangles that become exactly degenerate at f32 adapter precision produce an explicit error. This preserves topology honesty at large coordinates or extremely shallow cut planes; no tolerance repair or silent loss of a hole/strand is introduced. Open or inconsistently wound input does not gain cap area or invented volume. Ambiguous intersecting/branching contours remain an explicit cap error rather than a repair implementation.

No prepared-mode protocol or legacy-vs-prepared image comparison exists in this commit. No animation sequence reuse, camera planning, dimensions overlay placement, projected crest/mask image measurement or backend rendering was added. The stored camera/reference bounds and physical bump inputs are provided for those later integration checks.

Task 4b must wire these helpers into prepared Blender geometry, disable duplicate realization/material/placement operations in prepared mode, preserve existing legacy mode, retain static batched uploads and update only changing geometry, attach frame/panel analysis provenance using the coordinator's refined report/build-analysis contract, use the shared placement/bump/filter helpers, and qualify legacy vs prepared solid/bead/stepped/cutaway/color/sheet renders against the frozen invariants before enabling prepared mode. Camera annotation and timeline/frame reuse checks stay in the pipeline/sequence work. Backend workers must not independently relax the frozen tolerances.

## Exact focused tests

### tests/render_geometry.rs (24)

- `complete_cube_sheet_has_flat_normals_edges_and_immutable_build_coordinates`
- `cube_sections_all_axes_both_directions_have_physical_area_volume_and_outward_caps`
- `closed_hollow_shell_keeps_its_hole_on_all_axes`
- `open_input_remains_uncapped_with_unavailable_measurements`
- `exact_stl_corner_duplicates_are_welded_without_merging_nearby_shells`
- `adaptive_original_build_stages_are_relative_to_source_minimum`
- `section_after_partial_build_preserves_build_cap`
- `stepped_layers_keep_independent_touching_interfaces_and_materials`
- `disconnected_bead_strands_keep_a_submicron_gap_without_envelope_fill`
- `large_negative_coordinates_obey_analytic_area_and_volume`
- `outside_sections_are_empty_or_complete_and_boundary_faces_are_not_duplicated`
- `display_transform_uses_full_source_floor_center_and_build_coordinates`
- `physical_layer_pitch_and_normal_gradient_use_build_z`
- `derived_allocation_counts_are_checked_before_expansion`
- `coincident_build_and_section_caps_are_not_duplicated`
- `zero_layers_always_means_empty_and_preserves_legacy_empty_analysis`
- `clipped_geometry_that_collapses_at_adapter_precision_is_rejected`
- `real_independent_bead_mesh_has_smooth_sides_flat_ends_and_preserved_gap`
- `vertex_indexing_reuses_flat_face_corners_but_preserves_cube_creases`
- `declared_legacy_bead_smoothing_rejects_incompatible_indices`
- `build_zero_filter_and_layer_height_do_not_change_physical_pitch`
- `joined_ring_layout_validates_declared_quad_pairs`
- `frozen_diagnostic_meshes_and_masks_preserve_closed_hole_geometry`
- `single_frames_do_not_require_sheet_design_edge_capacity`

### tests/render_color.rs (6)

- `srgb_patch_is_not_encoded_twice`
- `bgra_bottom_up_padding_is_removed`
- `unpremultiplies_in_declared_producer_domain`
- `transparent_rgb_is_zero_for_both_alpha_modes`
- `linear_gray_encodes_once`
- `invalid_dimensions_stride_and_checked_lengths_are_rejected`

### tests/render_scene_contract.rs (5)

- `versioned_presets_retain_legacy_physics`
- `resin_absorption_is_per_mm_and_cloudiness_does_not_add_scattering`
- `face_groups_share_tool_color_material_and_leave_unassigned_faces_base`
- `conformance_thresholds_are_frozen_from_design`
- `cut_material_matches_legacy_blender_cut_surface`

## Existing ignored tests from the full CPU run

- test viewport::gpu::buildup_gpu_tests::gpu_real_model_buildup_renders_reported_height_without_warning ... ignored, requires GPU and TURNINATOR_BUILDUP_MODEL; checks the reported 28.4 mm cap
- test viewport::gpu::buildup_gpu_tests::gpu_solid_buildup_complete_inward_source_matches_uncut_pixels_and_picking ... ignored, requires an actual GPU adapter; checks completed inward-wound solids
- test viewport::gpu::buildup_gpu_tests::gpu_solid_buildup_matches_closed_truncated_cube ... ignored, requires an actual GPU adapter; checks solid build-up cap pixels
- test viewport::gpu::buildup_gpu_tests::gpu_solid_buildup_omits_future_component_bottom_at_exact_height ... ignored, requires an actual GPU adapter; checks exact-stage disconnected bottoms
- test viewport::gpu::buildup_gpu_tests::gpu_solid_buildup_picks_visible_cap_and_zero_layers_pick_nothing ... ignored, requires an actual GPU adapter; checks picking rendered build-up caps
- test viewport::gpu::buildup_gpu_tests::gpu_stepped_stages_match_closed_layer_mesh_and_picking ... ignored, requires an actual GPU adapter; checks closed stepped layer stages
- test viewport::gpu::tests::dense_bead_viewport_benchmark ... ignored, GPU throughput benchmark; run separately from correctness checks
- test viewport::gpu::tests::gpu_click_pick_and_settle_match_rendered_surface ... ignored, requires an actual GPU adapter; matches click hits to rendered face colors
- test viewport::gpu::tests::gpu_inspection_clips_build_frame_and_blends_independently_of_order ... ignored, requires an actual GPU adapter; validates section/build-up/OIT pixels
- test viewport::gpu::tests::gpu_layer_faces_preserve_caps_and_palette ... ignored, requires an actual GPU adapter; validates complete deposited layer faces
- test viewport::gpu::tests::gpu_render_layers_resize_and_unload ... ignored, requires an actual GPU adapter; run explicitly for viewport validation
- test viewport::gpu::tests::gpu_static_frames_reuse_target_and_scene_changes_refresh_it ... ignored, requires an actual GPU adapter; run explicitly for viewport cache validation
- test real_model_caps_survive_native_source_precision ... ignored, set TURNINATOR_BUILDUP_MODEL to an original STL; writes cap diagnostics
- test write_expanded_geometry_proofs ... ignored, writes proof snapshots; run explicitly for Blender geometry verification
- test archived_slicer_files_parse ... ignored, requires TURNINATOR_GCODE_ARCHIVE containing real slicer files
- test separate_exports_reuse_frames_across_formats_and_restart ... ignored, requires Blender/GPU and FFmpeg; writes artifacts/retained-frames-proof
- test annotated_sheet_renders_requested_quadrants_at_shared_scale ... ignored, requires Blender/GPU; actual annotated four-view export and retained dimension/camera checks
- test supplied_model_exports_an_annotated_4k_sheet ... ignored, requires Blender/GPU and supplied spiral egg; actual 4096 annotated sheet
- test solid_stages_reach_final_encoded_outputs ... ignored, requires installed Blender/GPU and FFmpeg; verifies solid stages in final PNG/GIF/MP4/sheet
- test stepped_stages_reach_final_encoded_outputs ... ignored, requires installed Blender/GPU and FFmpeg; verifies stepped STL in final PNG/GIF/MP4/sheet
- test compost_booster_resolves_subprecision_slice_folds_without_changing_source ... ignored, requires Compost Booster.stl; TURNINATOR_STEPPED_MODEL can override its path
- test compost_booster_slices_sideways_build_and_preserves_snapshot_integrity ... ignored, requires Compost Booster.stl; TURNINATOR_STEPPED_MODEL can override its path
- test supplied_stls_slice_and_survive_renderer_snapshot_precision ... ignored, requires the supplied STL model directory
- test fixed_stage_exports_png_sheet_and_empty_section ... ignored, requires Blender; fixed partial PNG/sheet and empty section retain full-source framing
- test resin_buildup_sections_follow_one_timeline_and_preserve_bead_voids ... ignored, requires Blender OptiX; real resin build-up, capped sections, paired spiral timeline and AV1 reuse
- test repeated_turns_render_once_and_preserve_order_duration_and_retry ... ignored, requires Blender, GPU and FFmpeg; writes artifacts/turntable-timing-proof
- test selected_frame_rate_preserves_turns_duration_reuse_and_paired_sections ... ignored, requires Blender/GPU and FFmpeg; validates selected30fps plus repeated-frame reuse
- test native_buildup_maps_tools_reveals_geometry_and_reuses_frames_for_av1 ... ignored, requires Blender/OptiX; actual joined multi-tool build-up and AV1 frame reuse
- test native_large_sheet_and_complete_section_animation ... ignored, large GPU acceptance: actual supplied STL 4096 sheet and 80-frame 1920x960 section GIF
- test native_layer_filter_reduces_subpixel_temporal_residuals ... ignored, GPU appearance acceptance: three complete 80-frame loops and temporal residual measurement
- test native_materials_alpha_mp4_and_busy_cancellation ... ignored, requires installed Blender/OptiX; writes artifacts/native-material-probe
- test native_section_bead_voids_and_open_input_are_honest ... ignored, requires installed Blender; verifies physical bead gaps and uncapped open input
- test native_section_cube_has_exact_area_volume_and_matched_panels ... ignored, requires installed Blender; verifies actual capped section metrics and matched panels
- test native_worker_exports_diagram_and_survives_request_errors ... ignored, requires installed Blender and GPU; writes artifacts/native-backend-probe

## Review fix round 1: I1 closed-source winding

Implementation commit: `12038e1e8ec02a98ee6c563d48d5ef669dfdfc1b` (base `7190b00`). This section supersedes the original report where closed inward-wound source behavior was not covered.

The review identified a real acceptance mismatch: `BuildUpSolid` validates consistent closed edge incidence without requiring positive signed volume. Realization preserved inward source faces while generating outward halfspace caps, and the exact coplanar culling rule assumed outward source faces. A final absolute global signed-volume sum also allowed independently closed stepped layers of opposite orientation to cancel.

The fix normalizes negative signed orientation only in disposable **closed** components before clipping. Source faces and their normals flip together with the disposable topology view. The complete original component, including every cavity shell, flips as one group, preserving outer/cavity winding relationships. Indexed bead components and stepped layers remain independent. Measurement now sums absolute signed contributions per independent realization component, retaining signed cavity subtraction inside the original component. No accepted source mutation, tolerance welding, output type changes, source acceptance changes, broad `Mesh` refactor or renderer integration was introduced.

Five regression groups were added:

- `inward_cube_sections_normalize_disposable_winding_for_all_axes_and_keep_directions`: axis X/Y/Z and both keep directions; analytical 1000/500 mm3 volumes, positive signed output volume, outward cut normals, 100 mm2 cap and matching oriented surface boundaries; source hash unchanged.
- `inward_cube_exact_boundary_cuts_keep_complete_faces_and_empty_stages`: all axes and keep directions at exact source minima/maxima; complete stages retain all twelve source faces and volume 1000 mm3, empty stages have zero faces/volume.
- `inward_original_partial_build_caps_remain_consistently_closed_after_sectioning`: a reversed source at five millimeters build height, then all axes/both keep directions, including coincident stage/section planes; analytical 500/250/0 mm3 volumes, oriented boundaries and immutable source hash.
- `globally_inward_hollow_source_preserves_cavity_winding_relationships`: the entire outer-plus-oppositely-wound cavity source reverses together; all axes/both keep directions retain 936/468 mm3 volumes and 84 mm2 cut area, with source hash unchanged.
- `independently_closed_stepped_layers_of_mixed_winding_have_additive_volumes`: outward/inward independently closed layers retain additive original volume 16 mm3 and remaining volume 8 mm3 in all axes/both keep directions, with source hash unchanged.

### Exact RED/GREEN commands and results

Every command set `CARGO_TARGET_DIR=D:/code/turninator/target`. Initial test execution encountered shared-target `turninator.exe` replacement failures (Windows access denied), which the coordinator attributed to another worktree's CLI test execution after Cargo released its compile lock. No process was terminated and no target tree was copied. After one retry produced the RED evidence below, the coordinator explicitly serialized the shared CPU slot for the fix and verification.

Before production changes:

```text
cargo test --offline --test render_geometry inward_
4 failed, 0 passed, 25 filtered out
  inward_cube_sections...: 166.66666666666677 != 500
  inward_cube_exact_boundary...: retained triangle count 2 != 0
  inward_original_partial_build...: -166.6666666666667 != 500
  globally_inward_hollow...: 188 != 468

cargo test --offline --test render_geometry independently_closed_stepped
1 failed, 0 passed, 28 filtered out
  independently_closed_stepped...: 0.0000000000000013322676295501878 != 16
```

After the production fix, `cargo test --offline --test render_geometry` initially passed 28 tests and failed one stronger boundary assertion on an already-existing collinear triangulation mismatch described below. The coordinator approved exact subdivision in the **test boundary comparison**, without vertex movement or tolerance. No production repair or test tolerance was added.

Final verification:

```text
cargo test --offline --test render_geometry
29 passed; 0 failed; 0 ignored

cargo test --offline --test render_scene_contract --test render_geometry --test render_color
40 passed total: 29 geometry, 5 material/contract, 6 color; 0 failed; 0 ignored

cargo clippy --offline --all-targets -- -D warnings
exit 0; Finished dev profile; no warnings

rustfmt --edition 2024 --check src/render/geometry.rs tests/render_geometry.rs
exit 0

git diff --check
exit 0

git diff --cached --check
exit 0
```

The full CPU suite was not repeated for this focused review round, as explicitly instructed by the coordinator; the original final full-suite result remains 241 passed / 34 opt-in ignored. Local readable transcripts are `task-4a-i1-red.log`, `task-4a-i1-stepped-red.log`, `task-4a-i1-green.log`, `task-4a-i1-focused.log`, and `task-4a-i1-clippy.log` in this report's directory. They are retained local evidence, not committed artifacts.

### Carry-forward: exact collinear boundary subdivisions

Independent clipping of original source faces, the build cap and the section cap can produce different exact collinear subdivisions along the same geometric boundary. For example, the partial cube's source boundary includes a vertex at `[5,0,2.5]` while the independently clipped cap can span that point. This behavior predates the winding fix; the normalized reversed cube is geometrically the same as the existing outward cube. Flat normal splits also mean raw indexed closure is not itself a contract guarantee.

The new test helper canonicalizes only exactly equal coordinates and compares oriented edge incidence after splitting at **existing exactly collinear** vertices. It moves no vertex, uses no tolerance and invents no geometry. Every resulting segment must have exactly two incidences with opposite direction. This checks consistent closed oriented surface coverage across those subdivisions instead of requiring identical triangulation.

Task 4b must still qualify those seams after f32 transport in prepared Blender and native renders. Potential raster cracks must be caught by the existing mask/physical invariants; no mask threshold was relaxed. A production conforming-edge refinement, if required by that evidence, is a separate bounded change. The coordinator was notified to carry this observation forward in the integration ledger. Prepared Blender/image conformance remains unqualified by this CPU fix.
