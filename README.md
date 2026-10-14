# Turninator

A local Windows studio for STL and supported G-code, configurable print layers, and studio exports. The native wgpu viewport works independently of Blender. Design: [final-plan.md](final-plan.md). Evidence: [docs/progress.md](docs/progress.md) and reports under reviews/.

## Run

    cargo run --offline --release -- gui --open "test-models/spiral egg - A.stl"

Running turninator.exe without arguments opens the studio. Drag a model into the window or use Open model. Drag to orbit, right-drag to pan, and scroll to zoom. The default layer pitch is **0.20 mm**; first-layer height is independent. Build rotation changes the print frame; presentation rotation keeps layers attached to the part.

**Presentation pose → Settle on face** lets you click a visible triangle and point its face down. Layers retain their build orientation. This aligns a face; it does not calculate physical stability.

STL Layer Appearance preserves the original silhouette. G-code creates actual rounded bead surfaces, optionally joined at contiguous path corners. Source assumptions, units and tool/feature labels are visible in the sidebar. Ghost/Xray modes use approximate transparency; native sections are uncapped. G-code build-up reveals complete deposited layers, including adaptive heights.

## Tools and outputs

Configure Blender, FFmpeg and ffprobe in **Tools / storage**. Missing tools leave native inspection available. No tools are installed or updated automatically. Exact tested binaries and maintained Blender LTS qualification are in [docs/runtime-support.md](docs/runtime-support.md).

- GIF defaults: 960 square, 80 frames, 20 fps, four seconds, infinite looping. The studio plays the actual encoded GIF.
- PNG defaults: 4096 square, Cycles/OptiX, optional transparent background independent of material transmission.
- Four-view sheets: shared-scale orthographic Front, Right, Top and Iso, design edges and labels.
- MP4: explicit H.264 or AV1, capability checks and verified timing. Unavailable AV1 does not silently become H.264. Matching requests reuse verified frames.
- Section exports: full model beside a cutaway through the whole turntable; reliable caps/statistics require closed input. G-code cuts retain deposited gaps.
- Build-up exports: complete deposited layers, including adaptive heights, combined with resin/Cycles, studio lighting, tool/feature colors, camera motion and capped or paired sections. Selected stages also export to PNGs and four-view sheets; zero layers produces an empty build.
- Camera timelines: record views under **Camera keyframes and motion**, set their times and outgoing orbit/pan or spiral transitions, choose easing and duration, then export GIF/MP4. Spirals add signed whole turns while approaching the next elevation. Optional model rotation and build-up use the same timeline. Projection must stay consistent between keys.

Animation and PNG/sheet quality have separate controls. Native lighting approximates Blender; Render preview provides the selected material/studio result. Resin on overlapping beads retains internal interfaces and is an illustration rather than fused-resin physics. Section volume is a mesh estimate, not calibrated deposited material or strength.

To author a motion, set the time slider, compose a view with orbit/pan/zoom or a canonical camera, then **Record this view**. Edit each key's outgoing transition and easing; **Play** previews the path natively. Camera loops reserve a closing transition or end at the starting camera. Build-up reveals whole deposited layers rather than reproducing printer time or nozzle motion; its last exported frame is complete.

## Projects and recovery

Save .turninator projects to retain physical settings, material/palette, pose, camera, timeline, selected deposited stage and output settings. Sources are relative when possible; missing/changed files require relocation or explicit reimport. Edited pitch survives material changes. Older projects load with the new motion fields disabled.

Exports use immutable snapshots. Cancellation terminates only the owned worker/encoder tree. Failed frames remain available through **Retry retained job** or **Tools / storage → Recover / encode**. Recipes and frames are revalidated; failed encoding preserves existing output. Only marked job storage is removable, and its budget is configurable. Sources remain read-only. Defaults live under %LOCALAPPDATA%/Turninator; gui --data-dir selects another location.

## Supported printing geometry

The planar G-code subset interprets XYZ/E modes, G92, retraction debt, travel and metadata. Explicit options add XY G2/G3 arcs, Marlin/Klipper modal behavior, a known G28 home, adaptive boundaries and normalized tool changes. Tool changes assume shared XYZ and a global E counter. Implicit offsets, parking/purge macros, volumetric/flow-changing commands, binary/compressed files and nonplanar/vase paths are rejected. Universal slicer coverage is not claimed.

Joined stadium beads are a bounded sweep rather than a polygon union or pressure model; sharp/short paths can overlap. Experimental stepped STL samples closed solids at layer midpoints and changes the silhouette. Cube, wedge, dome, hole and thin-wall proofs pass; ambiguous contours and numerical slivers fail explicitly. Features between sample planes may be missed. Adaptive boundaries apply to G-code or stepped STL; original STL shading uses uniform first/later heights.

Limits: 128 MiB input, 30,000 bead segments, two million source vertices, four million triangles and 1 MiB scene/project metadata. GPU attribute expansion has additional checked limits. Only exactly zero-area STL faces are omitted, with a visible report; no tolerance repair or automatic gap closure is performed.

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
