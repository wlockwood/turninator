# Final foundation geometry/schema cross-review

Reviewed the current root CLI, `renderer/scene.py`, `renderer/worker.py`, Rust mesh/bead interfaces, and Python/CLI tests after the earlier cap-normal and reader corrections. No application or renderer source was changed during this review.

## Result

No remaining important geometry or layer-coordinate defect was identified in the inspected implementation. The earlier two important renderer findings are resolved in source: reader count limits/degenerate checks now match Rust, and declared bead geometry receives smooth walls with flat cap faces. The current source supports proceeding with the foundation artifacts, with the evidence limits below.

One user-visible reporting issue remains: `prepare` stores import/units/appearance warnings in `scene.json` but does not include them in its printed JSON. For `Technics_work_support.stl`, the console therefore reports 3,438 triangles without directly reporting six omitted exactly zero-area faces. Include `warnings` in the CLI result (or print them separately) so the already implemented count is visible without opening an internal scene file. This does not change geometry or require rerendering.

## Coordinate and shader checks

- `turninator_build_z_m` is a per-point scalar derived from **source/build positions**, before the renderer bakes display rotation and floor placement. Point interpolation preserves the affine build-Z coordinate across every triangle; presentation changes cannot rescale the physical pitch or shift its phase.
- The first-layer branch uses `z / first_height` below the first boundary and `(z - first_height) / height` above it. The cosine is periodic, so the phase reset at the first boundary remains continuous. Heights are converted back to meters before driving Blender Bump.
- The normal used for cap masking comes from **True Normal**, not the already bumped normal. WORLD→OBJECT removes the turntable object rotation. The inverse Euler XYZ rotation then removes the same display rotation baked into mesh positions. This correctly returns the normal to the build basis before evaluating `1 - n_z²`.
- There is no additional normal transform on the Bump output: Blender expects that output directly at Principled's Normal input. The transformed normal is used solely to calculate the mask. The sign of the cosine-height derivative still gives the expected downward/upward perturbation on the lower/upper half of an upright bead-like groove.
- Layer Appearance is restricted to original geometry, so it is not accidentally applied on top of the real stadium bead surface. Explicit groove-depth validation respects the smaller first/regular layer interval. The CLI stores the edited first-layer and regular pitches independently.
- The swept-sphere camera fit and meter conversion remain unchanged and conservative for the current square-only render interface. No per-frame geometry switching was introduced.

These are source/math checks, not a new image comparison. A tilted presentation and a first-layer transition still deserve dedicated rendered regression images before claiming cross-pose visual validation. The ongoing Cycles proof reported by the parent uses the actual spiral-egg STL and 0.20 mm appearance; this reviewer did not take over or interrupt that job.

## Bead layout and reader checks

The CLI now tags generated beads with `bead_layout: "independent_stadium_v1"`; original STL scenes receive null. Scene validation rejects unknown layout names and use on original geometry. For the tagged layout, `bead_smooth_flags` verifies each quartet's side/side/start-cap/end-cap index relationships and distinct identities before returning `(True, True, False, False)`. This is an explicit producer contract rather than unvalidated face-number guessing.

Legacy bead scenes without the tag render faceted. This compatibility behavior is conservative and avoids reintroducing cap-normal contamination; it should be identified as a legacy/default behavior if exposed through a future UI. Optional scene fields preserve old schema-1 inputs without silently reinterpreting meshbin version 1.

Python now uses the same 2,000,000-vertex / 4,000,000-triangle limits as Rust, checks exact file size and index bounds, and rejects zero-area f32 triangles before constructing Blender geometry. The all-zero test and count preflight cover the earlier disagreement. Source STL omission happens before meshbin and is reported by the separate `StlImport` API; snapshot readers still reject geometry that collapses during f32 conversion.

## Persistence/reuse checks

The worker still reconstructs scene geometry for each request; its persistence is process persistence, not warm mesh reuse. New render manifests compare the scene, geometry hash, worker/scene script hashes, Blender build, engine, dimensions, samples, angles and alpha settings before frame reuse. Frames must decode at the requested dimensions. Post-preparation geometry hashing is checked against the recipe, improving the previous file-change handling. Immutable job inputs remain the operating assumption.

No new stale-scene bug was found. Frame content hashes and stronger immutable-input handles can follow the foundation; they are not claimed by the current decode-and-recipe verification.

## Current limits to retain in the final status

- Actual Layer Appearance has no explicit native-style screen-derivative fade or proven supersampling recipe yet. Stable animation at one tested pitch/model/size does not establish alias-free results for arbitrary fine layers or long-distance views.
- Original STL triangles retain faceted normals. The foundation does not yet implement crease-aware smoothing or a native viewport.
- Independently capped beads still overlap at joins; the flat-cap correction fixes shading normals, not join geometry or physical pressure behavior.
- Nine user models importing successfully establishes import compatibility for those files, not general manifoldness, printable-solid validity, or every slicer dialect.
- Full EEVEE/GIF timing and Cycles image claims should cite the parent's actual artifacts and metrics; this review adds source/math validation only.

No Cargo commands were run in this cross-review, avoiding interference with the parent's final build/test run.

Verification note: `python -m unittest discover -s tests -p test_*.py -v` observed **22/23 passing**. The newly present `test_encode_only_rejects_valid_images_from_a_different_render` failed because ValueError was not raised. This may be another agent's concurrent red-phase; the parent was notified immediately. The final workspace should not be described as fully passing until that encode-only regression is resolved and rerun. This failure is outside the inspected geometry/normal-transform path; no root-owned source was modified here.
