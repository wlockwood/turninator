# Task 6 print geometry expansion

The new APIs are opt-in and leave `build_bead_mesh` and default strict parser behavior compatible with the verified first milestone. Root owns application integration and commits.

## Joined bead contract

`build_joined_bead_mesh(&[BeadSegment]) -> Result<Mesh>` and `build_joined_bead_mesh_with_report -> Result<JoinedBeadMesh>` are available. The report contains `mesh` and half-open `smooth_face_ranges`. Scene tag is `joined_stadium_v1`, with ranges emitted as `bead_side_face_ranges`; the renderer agent implemented this contract independently.

Ordered segments join only when endpoint coordinates, tool, feature and layer match exactly. Internal joints share one 32-vertex stadium section, with mean adjacent width/height and a miter limited to four times the lateral profile distance. Width==height is supported without duplicate ring vertices. Open paths have terminal caps only; closed loops share the first ring and have no terminal caps. Exact reversal fails with an explicit split-path diagnostic.

For each path, side faces are first: two triangles per ring edge, 64 triangles per segment. Open paths append two center vertices after all section vertices and 64 alternating start/end fan triangles. The report marks side ranges explicitly so caps remain flat. This sweep is not a polygon union, pressure model or self-intersection solver: sharp corners, short segments and overlapping paths may still overlap. Mean joint profiles are a visualization interpolation, not a prediction of width transitions in molten material. The original independently capped builder remains available unchanged.

## Parser and explicit schedules

`ParseOptions` adds `dialect: GcodeDialect::{Marlin,Klipper}`, `allow_arcs`, `allow_tool_changes`, `chord_error_mm` (0.001–1mm), `homing_position_mm: Option<[f64;3]>`, and `layer_schedule: Option<LayerSchedule>`. New behavior is disabled by default except the documented Marlin dialect. Parent literals use `..Default::default()`.

Opt-in G2/G3 supports planar XY center-offset I/J, positive/negative R and I/J full circles. Adaptive subdivisions use a conservative chord-error budget, cap each angular step at π/8 and respect the existing 30,000-segment bound. Extrusion width uses true arc length, not chord length. Retraction repayment progresses along the subdivided curve, deposited volume is conserved, and final endpoints are exact commanded endpoints. Ambiguous R/full-circle combinations, conflicting center/radius words, radius mismatch beyond the error budget, helical/vase paths, other planes and unsupported geometry modes fail explicitly.

Marlin G90/G91 clears E overrides. Klipper preserves M83; its M82 allows XYZ absolute/relative mode rather than independently forcing absolute E. Klipper inch mode is rejected. This was checked against the [Klipper gcode_move source](https://github.com/Klipper3d/klipper/blob/master/klippy/extras/gcode_move.py) and [Marlin arc documentation](https://marlinfw.org/docs/gcode/G002-G003.html).

Tool opt-in explicitly means normalized shared XYZ coordinates, a global E counter and no implicit parking, offsets, purge or toolchange macros. Tool identity is retained and retraction debt is separate for tools 0–15. This is deliberately not a universal interpretation of printer-specific T macros. G28 is accepted only with an explicitly supplied physical home coordinate; selected axes reset their coordinate offsets. Homing assumptions, dialect, arcs, tool semantics and explicit schedules appear in warnings. Arbitrary firmware macros, volumetric extrusion and flow overrides remain rejected.

HEIGHT/LAYER_HEIGHT/layer_height, WIDTH/LINE_WIDTH/line_width and TYPE/FEATURE/feature comment adapters are supported. Unknown feature names are retained. `LayerSchedule::explicit` validates zero origin, strictly increasing bounded 0.02–2mm intervals and 50,000-layer budget. Explicit parser schedules require each depositing Z to match a supplied boundary and reject conflicting HEIGHT metadata. No adaptive-height inference is claimed. `clipped(top)` preserves all full intervals and ends the terminal interval at actual source height, even when the clipped remainder is smaller than the minimum normal layer.

## Bounded stepped-envelope decision

`slice_stepped(mesh, boundaries_mm) -> Result<Mesh>` and `slice_stepped_with_report -> Result<SteppedEnvelope>` implement a separate experimental sliced-solid model. The report carries mesh, actual sample heights, coordinate tolerance and warnings. Input must be closed, consistently oriented and have build minimum Z=0. Intersections stitch through exact source-edge identity; ambiguous/open/branching or intersecting/touching/unresolved contours fail. The default approximation samples interval midpoints, nudging by one ULP only when a source vertex lies exactly on the plane, then extrudes each region through its interval. The last interval clips to model height.

Nested contour hierarchy preserves holes. Adjacent-layer differences create correctly oriented treads; no interior overlapping slab caps are retained. i_overlay 9.0.0 uses one shared 48-bit coordinate grid with automatic cleaning disabled, zero area threshold and collinear points preserved. earcutr 0.5.0 triangulates cap polygons/holes. Edge conformance only subdivides existing boundary edges at existing boolean vertices; final output must pass closed, opposite edge-incidence validation. No source file is mutated, no automatic gap closure is performed and no tolerance-based feature deletion is permitted.

The bounded proof accepts cube, wedge, a stagger-triangulated dome, a through-hole solid and a 0.001mm thin wall. It rejects a latitude-aligned dome whose near-collinear sub-grid slivers cannot be triangulated consistently. That failure is retained as a regression, with original source positions unchanged. The decision is **keep experimental**: do not silently route normal STL imports here or describe this as a general slicer. Representative visual comparison and broader topology/range stress remain necessary before expansion. Features wholly between sample planes may be missed by the explicitly selected mid-layer approximation. Work is bounded to 2,000 layers, 10M triangle/plane checks and 100M contour/edge-conformance comparisons, in addition to the common mesh budgets.

## Evidence

- RED: joined corner, variable-profile closed loop and explicit-schedule tests failed against explicit implementation stubs. After implementation all three passed.
- Parser expansion tests were written before its implementation; the first run exposed parent ParseOptions initializer changes, then the completed parser passed arc length/volume/chord, full-circle/R, dialect, tool debt, explicit adaptive and diagnostic assertions.
- RED: stepped cube/wedge/hole/thin-wall test failed against a slice stub. After slicing/boolean/cap implementation it passed; dome stress then exposed unresolved slivers, which remain explicitly rejected rather than repaired. A nondegenerate triangulated dome passes.
- `cargo test --offline --test expanded_print --test gcode --test geometry`: **24 passed**, one proof writer ignored in the ordinary suite (9 expansion, 8 legacy parser, 7 legacy geometry).
- `cargo test --offline --test expanded_print write_expanded_geometry_proofs -- --ignored --nocapture`: **passed**, writing validated binary meshes and JSON scenes to `artifacts/expanded-print`.
- Proof counts: joined arcs **23,040 vertices / 46,080 triangles**; stepped cube **396/788**; wedge **752/1,500**; dome **1,728/3,452**; hole and thin wall each **799/1,598**. Each snapshot passed f32 serialization, closed opposite edge incidence and positive signed volume. Hole cap centroid assertions show the aperture remains empty. The joined source G-code contains 16 closed circular layers at 0.2mm pitch.
- Owned Rust files were formatted. Targeted strict Clippy initially found two parser formatting/range issues, corrected here; the remaining reported failures were concurrent parent app/cache/project collapsible-if issues. Root owns final whole-workspace Clippy and live Blender rendering.

These are CPU geometry/protocol proofs. Actual rendered proof images, app UI settings, feature/tool coloring and physical/photo agreement are separate integration evidence.
