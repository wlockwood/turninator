# Initial visual evidence

These are actual renderer outputs produced on October 14, 2026. They demonstrate the first foundation milestone, not completion of the desktop product.

The measurements and limitations below describe that initial checkpoint. Current desktop, filtering, joined paths, resin, sections, adaptive geometry and camera/build-stage evidence are recorded in [progress.md](progress.md), [the print/renderer review](../reviews/native-render-implementation.md) and [the timeline review](../reviews/timeline-implementation.md). They supersede the implementation gaps listed below. Physical-print photograph agreement remains unverified.

## Real geometric beads

- [4K EEVEE still](../artifacts/bead-demo-v2/proof-eevee-74150454cd15/still/still.png)
- [960-square, 80-frame GIF](../artifacts/bead-demo-v2/proof-eevee-74150454cd15/turntable.gif)
- [Matched small Cycles bead still](../artifacts/bead-demo-v2/proof-cycles-15f3e9512089/still/still.png)

The deterministic synthetic toolpaths describe an asymmetric hollow part, 120 layers at 0.20 mm, with two perimeter paths. Mesh bounds reach from bed Z=0 to Z=24 mm. Rounded stadium cross-sections are actual triangles, rather than a groove texture. Representative opposing angles and stills were inspected: ridges, inner walls and silhouette are visible, framing stays stable, and the earlier overexposure is corrected.

Individual segments overlap at their caps; their joins produce visible vertical divisions in some highlights. Smooth wall faces and flat cap faces avoid the initial incorrect axial normal blending, but this is still a segmented-bead prototype. Joined-path geometry is the next appearance improvement.

## User-supplied spiral egg

- [4K Cycles/OptiX still with 0.20 mm Layer Appearance](../artifacts/spiral-egg-layers/proof-cycles-40bba5171218/still/still.png)
- [960-square, 80-frame GIF](../artifacts/spiral-egg-layers/proof-cycles-40bba5171218/turntable.gif)

This imports the actual `test-models/spiral egg - A.stl` (107,970 triangles). The shader evaluates preserved build Z and a cap mask before presentation/spin transforms. The inspected still shows grooves across the spiral surfaces and intact gaps/top openings. This is shaded STL appearance, not geometry generated from that model's G-code. Its silhouette remains the imported STL's silhouette.

Both full GIFs decode to exactly 80 frames, 50 ms delay on every frame including the last, 4,000 ms total, infinite looping, and 960x960 dimensions. Each generated sequence uses angles0 through355.5 degrees without duplicating the endpoint. Automated timing/decode checks and representative static-angle inspection passed; subjective long-loop playback and a comparison against a photograph of the actual printed part remain future validation.

## Limits at the initial proof checkpoint

- No real G-code was supplied; parser coverage is based on deterministic fixtures and regression cases.
- No explicit subpixel shader filtering/supersampling recipe is integrated yet. Animation filtering and palette compression still need matched visual tuning.
- GIF sizes are currently large: approximately33 MB for beads and20 MB for the egg. These are measured outputs, not a size target met.
- No measured resin, infill-section, adaptive-envelope, or native-viewport quality claim.
- Original STL normals are faceted; smooth/crease-normal equivalence between render paths is later work.
- No warm geometry cache: the process persists, but each rendering request rebuilds the Blender scene.
