# Turninator

A local print-model studio under development. The first implementation milestone is runnable: STL import, a constrained G-code interpreter, physical extrusion-bead meshes, Blender rendering, and verified PNG/GIF output. The desktop interface and four-view sheets are still planned.

The merged design is [final-plan.md](final-plan.md). Evidence and current scope are in [docs/progress.md](docs/progress.md), [docs/performance.md](docs/performance.md), and [docs/visual-validation.md](docs/visual-validation.md).

## Requirements

- Windows x64 and Rust; this implementation was built with Rust 1.95.0.
- Blender, tested here at `C:\Program Files\Blender Foundation\Blender 4.5\blender.exe` (4.5.2 LTS).
- FFmpeg, tested at `C:\Program Files\ffmpeg\bin\ffmpeg.exe`.
- Python 3.10+ and Pillow for development proof tools. Blender uses its own Python for the worker.

Cycles proof rendering currently requires an available OptiX GPU. EEVEE is also supported. The proof scripts accept `--blender` and `--ffmpeg` paths; they do not install or update tools.

## Try the geometric layer demo

Run from this directory, choosing fresh output directories:

```powershell
cargo run --offline --release -- fixture --out artifacts/my-fixtures
cargo run --offline --release -- prepare artifacts/my-fixtures/layer-demo.gcode --out artifacts/my-bead-job
python tools/prove-render.py --job artifacts/my-bead-job --engine eevee --size 960 --frames 80 --still-size 4096 --samples 32
```

The proof prints paths to the GIF, still, logs, and metrics. It sends two rendering requests to the same Blender process and decodes the output. An 80-frame animation uses 50 ms per frame and loops indefinitely.

## Try an STL with layer appearance

```powershell
cargo run --offline --release -- prepare "test-models/spiral egg - A.stl" --out artifacts/my-egg-job --surface layers --layer-height 0.20 --first-layer-height 0.20
python tools/prove-render.py --job artifacts/my-egg-job --engine cycles --size 960 --frames 80 --still-size 4096 --samples 64
```

STL Layer Appearance changes shading and preserves the source silhouette. G-code bead geometry creates actual rounded extrusion surfaces. Both are approximations of material appearance; neither predicts printer defects. STL defaults to assumed millimeters and source Z-up. `--millimeters-per-unit 25.4` interprets an inches STL; G-code uses its own declared physical units.

`--surface clean` disables the STL layer shader. First-layer height may differ from later layers. Source files remain read-only. Exact zero-area STL faces are omitted with a visible count, while nonzero tiny faces are retained; no tolerance-based repair is performed.

## Current G-code scope

Plain text, planar, monotonically increasing deposition layers, one tool (T0), Marlin-style modal behavior. Supported geometry commands: G0/G1, G20/G21, G90/G91, G92, M82/M83. A small documented temperature/fan/query/dwell whitelist is harmless to geometry. HEIGHT/WIDTH/TYPE comments are recognized. Width without metadata is estimated from E, configured filament diameter (default 1.75 mm), and configured layer height. Retraction debt is repaid before a move creates a bead.

Homing, arcs, firmware retraction, volumetric extrusion, flow-changing commands, other tools, macros, checksums, binary/compressed containers, and nonplanar/vase deposition fail explicitly. Common full slicer files can therefore be unsupported today. The generated fixture exercises the supported subset; broad slicer compatibility is a later milestone.

The initial bead builder caps individual segments; corners overlap and are not a fused continuous extrusion. Limits: 30,000 bead segments, 2 million vertices, 4 million triangles, and 128 MiB STL/G-code input. Larger production prints need a more efficient joined/chunked representation. Original STL rendering is currently faceted; crease-aware shading remains planned.

## Recovery and checks

Frames and immutable recipes remain in the proof directory after an error. Rerunning an incomplete matching recipe reuses validated frames and repairs damaged ones. Completed proof timing evidence is preserved. Re-encode without launching Blender:

```powershell
python tools/prove-render.py --job artifacts/my-bead-job/proof-eevee-<fingerprint> --encode-only
```

Encoding verifies the frame manifest and source hashes before publication. A failed encode preserves existing GIF output and rendered frames. Use a new preparation directory to change geometry/settings; preparation refuses existing jobs.

```powershell
cargo test --offline --all-targets
cargo fmt --all -- --check
cargo clippy --offline --all-targets -- -D warnings
python -m unittest discover -s tests -p "test_*.py" -v
python tools/check-runtime.py --job artifacts/my-bead-job
```

The runtime check starts owned Blender workers to exercise cancellation, restart, an unrelated idle-worker sentinel, corrupt-frame repair, and verified reuse. It saves its result and logs under the job. Full GUI behavior, native rendering, four-view output, robust STL stepped-envelope reconstruction, general G-code coverage, and calibrated physical realism are not implemented yet.
