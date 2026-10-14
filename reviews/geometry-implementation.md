# Geometry implementation ledger — final-plan.md Tasks 1–2

Scope: library geometry, STL input, physical schedules, and constrained G-code importer. Root agent owns CLI, renderer, fixtures, and user documentation.

Skills: executing-plans and test-driven-development. Ruling: the approved plan and delegated ownership override Git/worktree/commit scaffolding; this is a new directory without Git and the ledger is retained here.

Pre-flight: Task 1 produces physical mesh/binary data consumed by Task 2 and renderer Task 3. Shared format fixed at TNMESH01/version 1, f32 on disk, f64 in Rust. BeadSegment start/end are nominal nozzle coordinates; mesh bead center lies height/2 below them. Task 2 produces ToolpathData.segments and warnings for root's CLI. No interface conflict.

Public API: `turninator::Result<T>` is boxed-error Result; `mesh::{Mesh,Bounds}`; `model::stl::load_stl(&Path, f64)`; `print::schedule::LayerSchedule::uniform(height_mm, first_mm, build_height_mm)` with `boundaries_mm`; `print::beads::{BeadSegment,build_bead_mesh}`; `print::gcode::{ParseOptions,ToolpathData,parse_gcode}`. All requested fields are public. ParseOptions defaults are .20/.20/1.75 mm. Dependency pins are serde 1.0.229 / serde_json 1.0.151.

Initial tests written with intentionally failing implementation stubs. Commands/results follow below.

- RED: `cargo test --offline --test geometry --test import --test gcode` compiled and failed 5/6 G-code tests on the intentional `not implemented` error. `cargo test --offline --test geometry --test import --no-fail-fast` failed all 5 geometry tests and the successful-input STL test on the same stub. Error-only tests naturally passed against the rejecting stub and were rechecked after implementation.
- GREEN foundation: `cargo test --offline --test geometry --test import` passed 5 geometry and 2 import tests.
- GREEN parser: `cargo test --offline --test gcode` passed all 6 initial parser tests.
- Expanded regressions cover circular bead closure, segment budgets, full XYZ/E resets, malformed metadata, backward deposition, and ASCII keyword whitespace. RED: tab-separated STL keywords failed the successful-input assertion; source inspection found exact-space checks for `solid`/`outer loop`. Fixed token recognition to use whitespace splitting. The newly added source-line assertion expected 10 for a 9-line fixture; corrected the test after counting the actual lines, with no parser change.
- Ruling: STL degenerates are rejected with a repair diagnostic, rather than silently dropped, because the initial Mesh API has no warning channel. STL rebases by source minimum XYZ before unit conversion; source files remain unchanged. Later source metadata can preserve the original offset without burdening the proof format.
- Ruling: prototype mesh generation is capped at 30,000 segments, with 2,000,000 vertices / 4,000,000 triangles as general mesh budgets. These are explicit conservative limits, not measured maximum hardware capacity. Independently capped beads use 12 subdivisions per semicircle and overlap at joins as documented in the approved plan.
- RED precision regression: translating a 0.42 mm bead 10,000,000 mm along Y caused f32 snapshot coordinates to collapse its cross-section, yet the writer succeeded. `cargo test --offline --test geometry snapshot_rejects_f32_collapsed_geometry_before_touching_destination` failed its expected-error assertion. Root cause: finite casts do not establish geometric representability. Added triangle-area validation after the exact f32 conversion, before opening the destination; the reader enforces the same degeneracy check.
- Strict Clippy initially reported `nonminimal_bool` in the STL `solid` predicate; replaced the negated `is_some_and` with equivalent `is_none_or`. No behavioral change.
- `cargo test --offline --all-targets` temporarily failed only root's concurrently written CLI test while main.rs deliberately returned `Turninator CLI is being implemented`; the isolated core suite was green. Root was notified; the CLI is outside this subtask's ownership.

## Completion evidence

- `cargo test --offline --test geometry --test import --test gcode`: **18/18 core tests pass** (7 geometry, 3 STL, 8 G-code), including the f32 collapse regression.
- `cargo test --offline --all-targets --no-fail-fast`: **21/21 pass**, including root's now-implemented three CLI tests. Unit-test targets contain no additional tests.
- `cargo clippy --offline --all-targets -- -D warnings`: pass.
- `rustfmt --edition 2024 --check src/lib.rs tests/geometry.rs tests/import.rs tests/gcode.rs`: pass for owned implementation/tests. `cargo fmt --all -- --check` caught only root's concurrently written main.rs formatting at the latest run; root was notified and owns the final workspace formatting pass.

Tasks 1–2 library portion is complete. The CLI/rendered artifacts belong to the root agent and renderer agent. No visual realism, GPU performance, complete slicer support, or physical deposition accuracy is claimed from these CPU tests.

Supported parser commands: G0/G1 with XYZEF, G20/G21, G90/G91, G92 with explicit XYZE, M82/M83, T0; benign G4, M104/M109, M140/M190, M106/M107, M105/M114/M115/M400 with bounded explicit parameter lists. Decimal uppercase words only; comments use semicolons. WIDTH/HEIGHT are millimeters regardless of motion units. Unknown/malformed words, homing, other tools, arcs, firmware retract, flow/volumetric changes, macros, checksums, backward layers and nonplanar deposition fail with source-line diagnostics. Initial counters/position/modes and fallback geometry assumptions are included in warnings.

Known intentional prototype limits: 30,000 bead segments; independently capped overlapping joins; no cap union, pressure/flow dynamics, arbitrary slicer dialects or support-removal reconstruction. STL import accepts one ASCII solid or a size-valid binary STL, welds exact coincident positions, retains winding, and rejects degenerate faces. It does not certify manifoldness or repair geometry. General mesh snapshots have exact sizes, finite/range/count validation and f32 representability checks.

## Follow-up: user-provided STL exact-zero omission

The user supplied nine real STLs. `Technics_work_support.stl` exposed the initial deliberate all-degenerates rejection policy. The parent authorized a reported omission API consistent with the original design: `load_stl_with_report(path, scale) -> Result<StlImport>`, with public `mesh` and `omitted_degenerate_triangles`; `load_stl` remains a mesh-only compatibility wrapper. This supersedes the earlier rejection ruling above. The CLI owner will make any omission count visible.

Read-only inspection: source is a valid 172,284-byte binary STL with 3,444 triangles; SHA-256 `cdbae31624af2afbfd0a9a0ccb5cad51cd3a399f0e7687bb90e7760c06af0ec9`. It contains exactly six zero-area triangles, all with repeated vertices, confirmed using Python Fraction arithmetic on original coordinates. The smallest nonzero face area is **1.8189894035458565e-12 mm²**, so area thresholds would delete valid features.

RED: `cargo test --offline --test import` with the report-wrapper stub failed the new mixed-valid/zero fixture and real Technics fixture with the original degenerate-face errors. Implementation classifies source triangles before rebasing or welding, omits only exact zero-area faces, excludes their unused vertices from geometry/bounds, and reports the count. There is no epsilon-based area threshold. Power-of-two edge scaling and FMA product residuals avoid falsely classifying very small or cancellation-sensitive nonzero faces as zero. Unrepresentable numerical ranges produce errors, not guessed omissions.

GREEN: the real model retains **3,438 triangles**, reports **6 omissions**, writes a validated f32 snapshot, and keeps the original source byte-for-byte unchanged. The hand fixture retains its nonzero 5e-13 mm² face and reports one omission. All-zero input still fails with an explicit no-usable-triangles message; nonfinite input still fails. Extra regression fixtures preserve nonzero source triangles whose naive area underflows or whose cross products round to equality; their eventual f32 snapshot error remains explicit rather than silently deleting geometry.

Follow-up verification: `cargo test --offline --all-targets` passes **25/25 tests** (4 CLI, 8 G-code, 7 geometry, 6 import); `cargo clippy --offline --all-targets -- -D warnings` passes; `rustfmt --edition 2024 --check src/model/stl.rs tests/import.rs` passes. No root-owned CLI/docs or renderer files changed in this follow-up.
