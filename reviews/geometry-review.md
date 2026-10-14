# Turninator geometry and rendering review

Reviewed `idea2-opus.md` and `idea2-codex.md` in full. This review recommends the merged architecture and a first implementation slice; no application code or final plan is changed here.

## Recommendation

Use Rust for canonical geometry, physical print settings, generated bead/layer geometry, the native viewport, and immutable export jobs; use Blender for authoritative materials and image rendering. Bring **real bead geometry and a constrained G-code proof into the first implementation**, reflecting the user's interest in actual geometry and the eventual toolpath renderer. Codex's decision to postpone all geometry until after the complete release is too conservative for that priority. Opus's promise of broadly reliable slicing, adaptive scheduling, and G-code matching in one early phase is too broad.

Support explicit representations: Original, Layer Appearance, Bead Paths, and experimental Stepped Envelope. Initially support uniform height (default 0.20 mm) and an independent first layer. The internal schedule should support validated explicit boundaries so imported schedules do not force a schema rewrite. Put path data behind a shared `BeadPath` contract that synthetic fixtures, the constrained G-code importer, and later STL-derived contour paths can all produce. A renderer can still show the original or appearance representation when an STL geometry request fails; it must identify the substitution.

The first proof should sweep known planar bead paths, import a constrained synthetic G-code fixture, and produce an actual still and turntable with measured geometry expansion. Run a bounded stepped-envelope experiment separately on closed cube, wedge, hollow prism, and dome fixtures; use its robustness and cost as the decision gate for general STL-derived geometry. Robust arbitrary-STL reconstruction is not a prerequisite for G-code or the shared bead renderer. Neither proof should wait for user STLs/photos: generate deterministic fixtures now and add real samples when available. A missing photograph limits claims about realism; it does not block implementation.

## Decisions to adopt, change, and defer

| Proposal | Decision | Reason |
| --- | --- | --- |
| Blender worker plus modest native wgpu preview | Adopt both plans | Concentrates original work on print geometry rather than a new path tracer. |
| Millimeters, source rebasing, independent build/display transforms | Adopt Codex detail | Layer spacing and phase must stay attached to the build frame. |
| Sliced geometry with wall/top/bottom attributes | Bounded early experiment, separate from bead-path proof | Produces staircase silhouettes, but robust solid reconstruction must not block direct toolpath geometry. |
| Uniform and distinct first-layer height | Adopt immediately | Directly serves the user's requested actual height. |
| Automatic adaptive height generation | Defer | A visualization-generated schedule is not the user's slicer schedule. Explicit boundaries are more valuable initially. |
| G-code layer reader before a modal parser | Reject | Comments and raw Z moves cannot reliably establish deposition layers. |
| G-code bead rendering | Prove a constrained planar subset immediately; expand dialect/features later | Shared geometry/material/export interfaces support real geometry without first building a general slicer. |
| Auto switches between original and stepped geometry by px/layer | Replace with filtering plus a fixed representation | Camera changes must not alter topology or silhouettes during a loop. |
| 0.01 mm unconditional contour simplification / tiny-gap repair | Reject as default | Can erase thin ribs, close holes, or merge shells. Simplification must preserve topology and have a reported error budget. |
| Immutable manifests and retained frames | Adopt a small version immediately | Re-encode and failure recovery need provenance even without a full resume UI. |
| Gifski as mandatory in-process dependency | Defer selection pending a measured comparison | Quality and distribution choice need evidence; an encoder interface keeps this reversible. |
| Two-model comparison, many finishes, contact sheets, automatic lay-flat | Defer behind useful geometry and primary exports | These do not prove the defining feature. |

## Geometry findings

### The solid construction is mathematically sound, with implementation conditions

Let layer solid `S_k = R_k × [b_k, b_(k+1)]`, and let the output represent the boundary of the union of all `S_k`. The exposed upward area at `b_(k+1)` is `R_k \\ R_(k+1)`; the exposed downward area at `b_k` is `R_k \\ R_(k-1)`. Opus has these signs correct. Empty neighbor regions yield the bottom and top caps. Both expansion and contraction need tests, including holes that appear, disappear, split, and merge.

Emit only exposed horizontal surfaces, with consistent winding, outward hole-wall normals, and conforming boundary subdivisions. Merely triangulating each Boolean result independently and welding later can leave T-junctions where horizontal polygons acquire extra vertices along wall edges. A visually closed raster image is not proof of a watertight mesh. Validate edge incidence after position welding and confirm no duplicate internal caps. Attribute seams can duplicate rendering vertices while retaining a separate topological identity.

Mid-height contour sampling is a defensible **chosen approximation**. Delete “which is what slicers do” unless verified for a named slicer/version. Nozzle offsets, wall widths, thin-wall handling, compensation, support decisions, and slicing conventions prevent an STL envelope from matching arbitrary slicer output exactly.

Define the final layer policy before coding. For an envelope that preserves source build height, clip the terminal extrusion interval to the model top and sample inside that clipped interval. Record that this shortened terminal interval is a visualization policy, not a promise about printer behavior. Test a 0.21 mm-high block with a requested 0.20 mm schedule: blindly sampling the second full interval at 0.30 mm would lose the top portion. Imported schedules will need their own explicit terminal convention.

Topological edge keys help only after correct adjacency construction. A vertex/edge exactly on the plane needs deterministic half-open intersection rules or consistent symbolic perturbation. Nudging each triangle independently by epsilon is unacceptable. A world-size-derived epsilon can erase small features; working in rebased f64 and defining a physical tolerance is safer.

Polygon union resolves defined planar overlaps, not arbitrary defective solids. Non-zero winding requires consistent ring orientation and intended cavity semantics. Reject open contours and ambiguous nesting in the initial envelope path; report the layer index and cause. Do not splice individual failed layers of shaded STL into neighboring stepped layers: that creates seams and makes the result hard to interpret. Retain the previous valid result and offer a clearly labeled whole-model representation fallback.

### Bead normals: Opus's sign is correct

For a wall displaced outward by radius profile `r(z)`, the outward normal is proportional to `N_wall - r'(z) Z`. A bead widest at its middle has `r' > 0` below the center and `r' < 0` above it; therefore its lower half faces downward and its upper half upward. Opus's `N_wall cos(phi) + Z sin(phi)` with `phi = asin(roundness * (2t-1))` has the correct sign. Do not invert it.

However, this is an artistic normal field, not automatically the derivative of the proposed semicircular displacement. Derive both optional real bead geometry and its normals from one explicit radial profile if that mode is implemented. A real bulge changes dimensions and can close narrow gaps; expose amplitude in millimeters and include expanded bounds in camera fitting. For the separate STL-envelope experiment, validate the simple staircase first before adding bulges to its walls. The immediate direct-path bead proof uses its own explicit stadium sweep.

Codex's cosine height has valleys at boundaries and a crest at the center; its derivative is `pi * depth / h * sin(2*pi*t)`. Native normal perturbation needs the negative surface gradient, not a direct positive-Z addition. On the lower half of an upright wall the resulting normal must point slightly downward. Cap masking must use the unperturbed normal. Tangent `cross(Z,N)` is undefined on horizontal caps, so choose a separate cap tangent or disable that anisotropic term there.

Generic alternating top-line bump is an optional appearance texture. It is not known infill, a perimeter layout, ironing, or the actual slicer's top pattern.

### Adaptive scheduling needs a separate metric

`h*abs(n_z)` is a plausible conservative local error surrogate for a one-sided step, not a complete adaptive slicing algorithm. Midplane sampling changes the local planar error factor; horizontal caps, curvature, feature birth/death, and triangles that enter a proposed band complicate the test. A greedy “maximum normal in this band” algorithm must test the entire proposed band and define behavior if even the minimum layer exceeds the target.

Actual slicers use differing quality metrics: Prusa describes distance, XY-contour-spacing, and cross-section-area alternatives and smooths its adaptive profile. Therefore Turninator should call a future generated profile “automatic visualization layers,” while imported explicit heights carry source provenance. [Prusa variable layer height documentation](https://help.prusa3d.com/article/variable-layer-height-function_1750)

### Filtering must not swap geometry every frame

Global `height * pixels_per_mm` is a rough UI indicator; perspective, tilt, visible surface direction, and variable height make the actual screen frequency local. Use build-Z derivatives in the native shader and verify the Blender supersampling/filtering path. Fade unresolved normal modulation toward its mean while preserving the physical layer interval.

Select envelope versus appearance explicitly. If an Auto option is later added, resolve it once for the entire immutable job using all frames and store the resolved choice. It must not choose a new mesh per frame, per panel, or as the interactive camera crosses a threshold. Anti-alias actual silhouette steps with image sampling; changing to the original mesh changes the requested geometry. Benchmark denoising on fine grooves rather than assuming higher samples or wider filters preserve them.

## G-code milestone: keep the architecture, narrow the promise

Call the result **commanded toolpath visualization with estimated beads**. G-code provides path order, nominal extrusion, tool selections, and some feature metadata. It does not establish exact deposited seams/zits, bridge sag, extrusion width under pressure, shrinkage, adhesion, or color mixing. Avoid “ground truth” and “exactly what your printer will do.”

Start with a sequential modal interpreter for a named, tested dialect and planar `G0/G1` extrusion, followed by a colored path preview. Track units, XYZ/E modes, positions, `G92` coordinate changes, current tool and its extrusion state, retraction debt, and line/source identity. Extrusion-only priming and unretraction must not create a bead or a new layer. Preserve unsupported-command diagnostics. In Marlin, `G90/G91` clear the E override from `M82/M83`; this is a dialect behavior, so do not assume the axes' modes are globally independent. [Marlin M82](https://marlinfw.org/docs/gcode/M082.html), [Marlin M83](https://marlinfw.org/docs/gcode/M083.html)

Do not parse arbitrary chunks independently: each chunk's interpretation depends on earlier modal state. Tokenization can be parallel; interpretation needs sequential processing or explicitly calculated checkpoints. Optimize only after measuring representative files.

Infer candidate planar heights from actual deposition moves and confirm them against recognized slicer metadata. Travel hops are excluded. Spiral/vase paths, nonplanar deposition, sequential-object resets, raft/support schedules, and purge structures need classification rather than flattening into one monotonic layer list. A first import can reject unsupported schedules with a specific explanation.

For filament-length extrusion, nominal volume is `delta_E * pi*d_filament^2/4`; for volumetric E it is already a volume. Marlin's `M200` changes E units to cubic millimeters, so multiplying by filament area again is wrong. [Marlin volumetric extrusion](https://marlinfw.org/docs/gcode/M200.html)

Given known height `h`, path length `L`, and deposited volume `V`, the stadium approximation gives `A=V/L` and `w=A/h + h*(1-pi/4)`. This model requires `w>=h`; an inferred negative/too-small rectangular portion is a model failure, not a value to silently clamp. Filament diameter, firmware flow multipliers, unknown machine defaults, and retraction recovery affect confidence. Width/height comments are hints with source provenance, not universal declarations.

Add arcs only with a specified plane, IJK/radius semantics, full-circle handling, chord-error bound, and extrusion distributed by arc length. Marlin supports selected workspace planes and interpolation of Z over the arc, so G2/G3 is more than a 2D endpoint shortcut. [Marlin arc commands](https://marlinfw.org/docs/gcode/G002-G003.html)

The bead milestone can first sweep a fixed stadium profile along planar paths with bounded joins and end caps. Compare one mesh per chunk with viewport instancing. Curves are an experiment, not an assumed equivalent squashed profile. Store paths separately from their rendered representation.

Reject automatic “drop all inner perimeters/infill” as guaranteed visibility culling. Inner paths can be visible through holes, openings, overhangs, missing skins, or translucent surfaces. An explicit Exterior Preview mode can reduce selected features with a clear approximation label; the faithful commanded-path mode retains all selected features. Sectioning needs to intersect beads themselves; a cap that fills the entire part envelope would falsely fill the infill gaps.

## Memory, encoder, and edge-pass corrections

Three hundred layers times 1,000–3,000 contour edges already produce **0.6–1.8 million wall triangles**, before horizontal differences. Ten million triangles require 120 MB just for u32 indices; f32 corner normals add 360 MB. Positions, attributes, duplicated mesh copies in Rust/Python/Blender, acceleration structures, display buffers, and denoising increase that substantially. “Well under 12 GB” and “fine for raster” cannot follow from triangle count alone.

Use checked counts and preflight estimates; bound in-flight layer work and mesh buffers. A hard budget should stop with a visible diagnostic and preserve the last valid mesh. It should not silently simplify contours or replace only the excess layers. Record peak memory, transfer time, BVH/upload time, and full render/encode wall time. Avoid a claim of sub-second slicing until measured.

Keep `GifEncoder` behind an interface and initially use the available FFmpeg path for a reproducible baseline. Compare gifski against palettegen/paletteuse using the **same final frame sequence** for file size, encode time, grain/shimmer, layer retention, and decoded timing. Gifski identifies its license as AGPL-3.0-or-later and offers alternative licensing; do not choose in-process linkage solely from the assertion that the app is personal. This is a dependency choice to record, not a reason to block the initial proof. [Gifski project README](https://github.com/ImageOptim/gifski/blob/main/README.md)

Opus's normal-angle threshold cannot suppress actual layer stair edges while preserving all design creases: both can be 90-degree edges. For the default clean four-view sheet, render canonical geometry or its explicit topology edge overlay. For a printed sheet, decide explicitly whether stair edges are wanted; an original-geometry overlay can omit print features but needs depth handling against the displayed geometry.

## Interfaces to carry into the merged plan

```text
LayerScheduleSpec = Uniform { height_mm, first_height_mm }
                  | Explicit { boundaries_mm, provenance }
LayerSchedule = validated boundaries + terminal policy + source provenance
PrintRepresentation = Original | LayerAppearance | BeadPaths | SteppedEnvelope
SliceSpec = sample convention + terminal policy + topology/tolerance policy
            + optional simplification policy + resource budgets + algorithm version
PrintGeometry = mesh + corner normals + typed attributes + topology report
                + resolved representation + source/schedule/settings hash
SliceReport = counts, timings, memory estimate, warnings, failed layer/reason
BeadPath = ordered physical centerline + width/height + tool/feature/layer identity
           + open/closed flag + source/provenance
ToolpathSet = modal-interpreted BeadPaths + diagnostics
BeadSpec = cross-section model + geometry resolution + confidence/provenance
```

`build_layer_schedule`, `slice_regions`, `build_layer_boundary`, and `derive_print_geometry` should be independently testable. `meshbin` needs typed/domain-qualified attributes: point versus corner versus face must be explicit. Store layer index as flat face/corner data rather than an interpolated float. Avoid broad arbitrary attribute support in the first implementation: versioned `layer_index`, `layer_t`, and `surface_kind` cover the proof. The JSON scene records desired and resolved representation and all material/filtering settings.

## First proof tasks and acceptance

1. Scaffold a small Rust CLI/library and deterministic fixtures. Implement schedule validation, unit/build transforms, and the shared `BeadPath`/mesh contract. Test 20 mm at 0.20 mm = 100 intervals, distinct first-layer height, print versus display rotation, and non-finite/excessive schedules.
2. Sweep known planar paths into stadium-profile meshes with bounded joins and end caps. Test straight paths, a square, an open bend, a closed circle approximation, width/height bounds, winding, cross-section dimensions, layer stacking, and resource limits. For conventional planar G-code, the commanded nozzle Z is the nominal layer top; place the estimated bead center at `Z-h/2`, not at nozzle Z. Test the first layer spans bed to its stated height.
3. Implement the constrained sequential G-code importer with explicit supported dialect/commands and diagnostics. Test absolute/relative XYZ and E, omitted coordinates, G92, negative E/recovery, Z-hop, extrusion-only prime, layer comments versus deposition, and unsupported arcs/nonplanar motion. Do not silently skip a geometry-affecting command and continue with guessed state. Synthetic known toolpaths should match direct `BeadPath` fixtures.
4. Transfer the generated bead mesh and attributes into the selected installed Blender build. Render a matte-PLA still and a short rotating proof first; then the full 80-frame/960-square loop at 0.12/0.20/0.28 mm. Compare geometric ridge silhouettes, first-layer phase, gaps, and a 90-degree presentation tilt. Run both EEVEE and Cycles if available and record actual capability limits.
5. Encode the same validated frames, decode the GIF, and assert dimensions, 80 frames with 5-centisecond delays, infinite loop, and no duplicate endpoint. Inspect actual playback and 4K close-ups; record total time and peak memory. The milestone is artifacts plus evidence, not merely passing geometry tests.
6. Separately, prove STL contour extraction and stepped-envelope boundary construction on the closed supported fixtures. Cover plane degeneracies, holes, winding, expanding/contracting sections, final-layer policy, and failed open input. Assert closed edge incidence, absence of duplicate internal caps, and resource limits. Its decision gate is whether robustness/performance justify extending to general STL-derived paths; failures do not hold up the direct bead/G-code renderer.
7. Continue into the app with the proven paths. Keep job snapshots, cancellation/restart, stale-result protection, and preserved previous results from Codex. Expand G-code feature coverage only with fixture evidence; extend the STL path according to its separate experiment.

These are implementation gates inside the already authorized work. They do not require a new user approval. If Blender is unavailable, complete the CPU geometry proof and reproducible render scripts, state precisely which live render checks remain unverified, and continue independent implementation.
