# Prepared geometry and renderer recipe compatibility

Blender adapter recipe 2 consumes the host's shared realization. Recipe 1 remains
available as a legacy reference and for verified retained-output recovery. The
recipe mapping is centralized in `render::prepared::recipe_versions`: Blender 1
uses `legacy-realization-v1` and frame-plan recipe 1; Blender 2 and native adapter
1 use `shared-realization-v1` and frame-plan recipe 2. Unknown combinations fail.
New rendering must not use old provenance to label newly generated pixels.

For scenes without an explicit camera, frame-plan recipe 2 fits the transformed
full-source envelope using the historical Blender 70 mm lens, 36 mm horizontal
sensor and 10% margin, accounting for output aspect. Explicit application cameras
and keyed timeline cameras retain their values. Recipe 1 preserves its original
provisional host camera plan so complete historical v3 artifacts remain recoverable
without Blender, FFmpeg, or ffprobe.

## Blender upload

`geometry_mode=prepared-realization-v1` requires an absolute manifest path in the
owned render directory. Absence of the mode selects `legacy-realization-v1`.
Manifest version 1 contains a material count, SHA-256-identified revision files,
canonical section/build analysis payloads, and one revision index per frame.
The JSON limit is 8 MiB. Revision files must resolve within the manifest directory.

Each revision uses little-endian `TNPRP001` binary data:

| Field | Layout |
| --- | --- |
| Header | 8-byte magic, u32 version 1, u32 vertex/triangle/edge counts |
| Vertex | f32 position[3], normal[3], build_position[3] |
| Triangle | u32 vertex_indices[3], material_index |
| Design edge | f32 endpoint_a[3], endpoint_b[3] |

Positions and edges are in build-space millimeters. Normals are finite unit
vectors. Material and vertex indices must be in range. Exact byte length, digest,
finite attributes and nondegenerate f32 triangles are checked before Blender
uploads. Empty stages have zero vertices and triangles. Fixed maxima are 2 million
vertices, 4 million triangles and 200,000 edges; host/scratch limits also apply.
Blender's table has at most 258 materials (base, 256 resolved tool/color entries,
cap); its separate curve object supplies ink.
Prepared tool colors canonicalize to f32 before deduplication, treating signed
zeros as equal and retaining first-use names/order, exactly as the host resolver
does. The actual Blender table count must equal the manifest's canonical count.

Static turntables upload once and retain Blender's native animation batch.
Changing build stages replace geometry only when the revision changes. Camera or
spin changes do not recompute build-space meshes. The worker preserves supplied
normals, triangle material indices and build coordinates; the immutable source
envelope supplies placement, framing and studio bounds. BuildAnalysis describes
the pre-section stage; SectionAnalysis describes the final section. The adapter
emits canonical metadata before each PNG notification.
Original-solid `solid_layer_tops_mm` entries are absolute BUILD-Z planes. They are
not offset by the source minimum; the schedule and analytical `position_mm` use
the same physical coordinates even when the source extends below Z=0.
Empty original-solid sections retain unavailable area/original-volume values;
empty layered sections retain their historical zero-valued empty-stage payload.

Cycles accepts an explicit boolean `denoise`; omission still means true. Seed 0
and disabled animated seed remain unchanged. Denoising is part of quality identity.

## Native sheet ink

Native adapters call `geometry::expand_design_edges` after `realize_frame` and
material resolution. The helper consumes the shared design-edge list and appends
ordinary depth-tested geometry: eight vertices and twelve triangles per edge.
It makes no additional crease decisions. The camera-facing tube uses a 0.4-pixel
radius (0.8-pixel diameter), with Blender's minimum physical radius of 0.00001 mm.
Pose, output dimensions and camera projection participate in sheet revision keys.
Use `frame_geometry_key(frame, true)` for expanded native ink and `false` for
Blender curves. Cache keys are scoped to an immutable scene/material snapshot.

Expansion preserves source/base geometry and analytical results. Derived limits
are the fixed mesh maxima and checked host/GPU byte budgets; configurable source
quotas do not apply again to generated ink. Capacity errors are explicit. Native
material JSON now permits up to 259 entries and an optional boolean `unlit`.
Absent/false retains physical shading. True bypasses lighting and layer shading,
using constant scene-linear base color through the same exposure/tone mapping.
Ink uses `[0.018, 0.022, 0.03]`. The metadata limit remains 1 MiB; C ABI layouts and
schema version are unchanged.

## Qualification

`tests/render_prepared.rs` generates input fixtures; the ignored worker test in
`tests/blender_adapter.rs` exercises actual batching, stage revisions and Cycles
flags. `tests/blender_prepared_qualification.py` compares actual legacy/prepared
images, geometry measurements and analysis invariants at frozen tolerances.
`tests/blender_prepared_layers.py` checks visible first/later pitch, posed build Z
and subpixel fade. Tests retain raw PNGs and JSON metrics under ignored artifacts.

The October 15 qualification used Blender 4.5 and an RTX 4070 Ti: 22 paired cases,
three layer diagnostics, and four actual worker scenarios passed. All paired
silhouette IoUs were 1.0. The nonbinary f32 section/build closeup had no transparent
pixels at the T-junction. A 12-edge orthographic sheet preserved spline count and
0.4-pixel radius. These fixtures qualify the tested cases; native SDK execution
of expanded unlit ink remains each native adapter's runtime qualification duty.
