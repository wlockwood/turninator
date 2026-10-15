# Turninator

A local Windows studio for STL and supported G-code, configurable print layers, and studio exports. The native wgpu viewport works independently of Blender. Design: [final-plan.md](final-plan.md). Evidence: [docs/progress.md](docs/progress.md) and reports under reviews/.

## Run

    cargo run --offline --release -- gui --open "test-models/spiral egg - A.stl"

Running turninator.exe without arguments opens the studio. Drag a model into the window or use Open model. Drag to orbit, right-drag to pan, and scroll to zoom. The default layer pitch is **0.20 mm**; first-layer height is independent. Build rotation changes the print frame; presentation rotation keeps layers attached to the part.

**Presentation pose → Settle on face** lets you click a visible triangle and point its face down. Layers retain their build orientation. This aligns a face; it does not calculate physical stability.

STL Layer Appearance preserves the original silhouette. G-code creates actual rounded bead surfaces, optionally joined at contiguous path corners. Source assumptions, units and tool/feature labels are visible in the sidebar. Ghost/Xray modes use approximate transparency; native sections are uncapped. G-code build-up reveals complete deposited layers, including adaptive heights, and preserves infill gaps.

**Show printed build-up** reveals either the modeled STL solid or complete G-code layers at the chosen printed height. Closed STL cuts have a filled top surface and preserve modeled holes; the surface mesh describes the boundary of the solid. The selected stage carries into previews, PNGs, sheets and ordinary turntables. **Export build-up animation** instead grows the part through its layers during GIF/MP4 export, ending at the complete model; it can follow camera motion and sections. STL layers use the configured first/later heights, with the final layer at the true model top. Open or ambiguous STL boundaries cannot define a reliable solid cap and show an explanation.

## Tools and outputs

Configure Blender, FFmpeg and ffprobe in **Tools / storage**. Missing tools leave native inspection available. No tools are installed or updated automatically. Exact tested binaries and maintained Blender LTS qualification are in [docs/runtime-support.md](docs/runtime-support.md).

Under **Export**, choose **Encoding** first: GIF animation, MP4 video, PNG image or 4-view sheet. Only settings for that output appear. MP4 codec is directly below Encoding for MP4; transparent background appears for PNG. Renderer, pixels and samples belong to the selected output, with separate animation and PNG/sheet preferences retained when switching. Use the **Export** button for that selection. **Render preview** follows its renderer/background and previews the complete layout for sheets.

Hover over any control for guidance on what it changes, units, useful limits and tradeoffs. Disabled actions also provide help, including the conditions needed to use them. The native viewport explains orbit/pan/zoom or face selection according to the current mode.

- GIF defaults: 960 square, 80 frames, 20 fps, four seconds, infinite looping. The studio plays the actual encoded GIF.
- Basic turntables: with camera timelines and build-up disabled, set **Seconds per turn** and **Animation length (s)** under Export. The studio shows total turns and the actual frame count/duration. For example, 4 seconds per turn over 12 seconds produces 3 turns. Length rounds to the nearest 20 fps frame; fractional turns are supported.
- PNG defaults: 4096 square, Cycles/OptiX, optional transparent background independent of material transmission.
- Four-view sheets: TOP / ISO above FRONT / RIGHT at a shared orthographic scale, with a fixed gray diagram style, design edges and one millimetre dimension per view. Each annotation measures the larger visible projected width or height; selected deposited stages use their visible geometry.
- MP4: explicit H.264 or AV1, capability checks and verified timing. Unavailable AV1 does not silently become H.264. Matching requests reuse verified frames.
- Section exports: full model beside a cutaway through the whole turntable; reliable caps/statistics require closed input. G-code cuts retain deposited gaps.
- Build-up exports: complete deposited layers, including adaptive heights, combined with resin/Cycles, studio lighting, tool/feature colors, camera motion and capped or paired sections. Selected stages also export to PNGs and four-view sheets; zero layers produces an empty build.
- Camera timelines: record views under **Camera keyframes and motion**, set their times and outgoing orbit/pan or spiral transitions, choose easing and duration, then export GIF/MP4. Spirals add signed whole turns while approaching the next elevation. Optional model rotation and build-up use the same timeline. Projection must stay consistent between keys.

Repeated animation poses render once, then copy the verified PNG into each matching position in the output sequence. A three-turn basic rotation at 4 seconds per turn exports 240 frames from 80 renders. Reuse requires an identical camera, rotation and visible deposited layers, so advancing build-up and moving cameras retain their distinct frames. Non-integral frames per turn reuse only phases that coincide exactly.

Controls follow the loaded source and selected features. STL units/build orientation and stepped solids appear for STL; G-code shows filament, interpretation and deposited-layer settings. Layer/groove details require layer appearance, arc precision requires enabled arcs, and section details require an enabled section. Adaptive boundaries replace numeric height defaults, and palettes show only colors used by the loaded paths. One build-up export checkbox controls both basic and camera-timeline animations. Hidden preferences are retained. Native lighting approximates Blender; Render preview provides the selected material/studio result. Resin on overlapping beads retains internal interfaces and is an illustration rather than fused-resin physics. Section volume is a mesh estimate, not calibrated deposited material or strength.

Export save dialogs suggest the loaded model's base name with the chosen output extension. You can edit the filename before saving. A model change invalidates a pending save choice so it cannot export a different model under the previous suggestion.

To author a motion, select GIF or MP4 and expand **Camera keyframes and motion** under Export. Set the time slider, compose a view with orbit/pan/zoom or a canonical camera, then **Record this view**. Edit each key's outgoing transition and easing; **Play** previews the path natively. Camera loops reserve a closing transition or end at the starting camera. Build-up reveals whole deposited layers rather than reproducing printer time or nozzle motion; its last exported frame is complete.

## Projects and recovery

Save .turninator projects to retain physical settings, material/palette, pose, camera, timeline, basic turntable speed/duration, selected deposited stage and output settings. Sources are relative when possible; missing/changed files require relocation or explicit reimport. Edited pitch survives material changes. Older projects load with the new motion fields disabled and the original four-second basic rotation.

Exports use immutable snapshots. Cancellation terminates only the owned worker/encoder tree. Failed frames remain available through **Retry retained job** or **Tools / storage → Recover / encode**. Recipes and frames are revalidated; failed encoding preserves existing output. Only marked job storage is removable, and its budget is configurable. Sources remain read-only. Defaults live under %LOCALAPPDATA%/Turninator; gui --data-dir selects another location.

## Supported printing geometry

The planar G-code subset interprets XYZ/E modes, G92, retraction debt, travel and metadata. Recognized Slic3r/PrusaSlicer files accept common startup commands, homing followed by absolute positioning, M221 flow percentage and adaptive depositing heights. Prusa startup purge lines are omitted. Sequential objects are grouped by physical Z for whole-layer playback. Interleaved Prusa support/model heights without per-path metadata use a visibly reported nominal-interval estimate; explicit HEIGHT metadata or boundaries take precedence. Legacy KISSlicer motor-driven files need their layer-thickness and filament-speed comments; those calibrate motor-speed changes and commanded feed rate. Firmware purge/destring transients are not simulated.

Explicit options add XY G2/G3 arcs (E-axis files), Marlin/Klipper modal behavior, a known G28 home, adaptive boundaries and normalized tool changes. Tool changes assume shared XYZ and a global E counter. Unknown geometry-changing commands, uncalibrated motor extrusion, implicit offsets, arbitrary macros, volumetric extrusion, binary/compressed files and nonplanar/vase paths are rejected. Universal slicer coverage is not claimed.

Joined stadium beads are a bounded sweep rather than a polygon union or pressure model; sharp/short paths can overlap. Experimental stepped STL samples closed solids at layer midpoints and stacks independently closed layer solids. This changes the silhouette and retains internal layer interfaces, which can show through transparent materials. It avoids fragile Boolean joins between neighboring layers. Coordinates use renderer precision; touching/intersecting contours and holes or gaps that collapse at that precision fail explicitly. Features between sample planes may be missed. Adaptive boundaries apply to G-code or stepped STL; original STL shading uses uniform first/later heights.

Limits: 128 MiB input, 250,000 bead segments, two million source vertices, four million triangles and 1 MiB scene/project metadata. Joined beads automatically use 32, 24, 16, 12 or 8 vertices per ring to fit the geometry budget, retaining every depositing segment. Independent beads have a lower mesh capacity; joined beads are recommended for real files. GPU attribute expansion has additional checked limits. Reversing paths receive separate caps. Recognized slicer low-flow paths use round strands preserving commanded volume when a flattened bead cannot fit. Only exactly zero-area STL faces and generated faces that collapse at GPU precision are omitted, with visible reports; no tolerance repair or automatic gap closure is performed.

## CLI and verification

    cargo run --offline --release -- fixture --out artifacts/my-fixtures
    cargo run --offline --release -- prepare artifacts/my-fixtures/layer-demo.gcode --out artifacts/my-job --beads joined
    python tools/prove-render.py --job artifacts/my-job --engine eevee --size 960 --frames 80 --still-size 4096 --samples 32

Run turninator --help for options. CLI defaults preserve independent beads and clean STL. Use --surface layers with --layer-height/--first-layer-height, or explicitly select --surface stepped. Preparation requires a new directory. Development proof tools need Python/Pillow; the packaged studio uses Blender's Python and native encoders.

    cargo test --offline --all-targets
    cargo fmt --all -- --check
    cargo clippy --offline --all-targets -- -D warnings
    python -m unittest discover -s tests -p "test_*.py"
    powershell -File tools/package.ps1

GPU/runtime tests are marked and have been run separately. [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) includes exact Windows dependencies and bundled font notices. Portable packages include the app and renderer resources; external binaries remain separately configured.
