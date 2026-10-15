# Task5c shared native lifecycle core

Worktree: `D:/code/turninator/.worktrees/renderer-native-core`.
Branch: `codex/renderer-native-core`; base `c19a1d855fdd2d43f08df681560b3da2b8009393`.
Scope: common production-ready lifecycle implementation and CPU transport tests;
no production SDK bridge, registration, GPU test or rendering qualification.

## Result and integration boundary

`native/common/{backend,facade}.hpp`, `facade.cpp`, `validation.cpp` and
`exports.cpp` provide the existing nine ABI1 exports through one implementation.
The public C header and Rust host/public contracts are unchanged. The fixture
is now a small fake `tn::Backend` implementation using the same facade. Wrong
ABI/struct/backend/export/exception/description/dependency variants remain.
Fixture flags/ID remain explicit; production loading still rejects them.

The internal API and worker7/8 integration steps are documented in the Task5c
appendix of `docs/renderers/sdk-qualification.md`. Each production backend calls
`tn_add_bridge(target, backend.cpp)` in CMake, implements constant
`tn::backend_id()` / `backend_flags()` and `make_backend(const Config&)`, then
implements `Backend::{describe,upload,render,~Backend}`. SDK headers/link flags
belong to its own target. No SDK C++ object crosses the ABI.

The facade constructs, describes, uploads, renders and destroys each backend
on one dedicated context worker. Create waits for actual SDK initialization and
validated runtime description. Subsequent describe calls use bounded cached
bytes. Owner-thread-only opaque handles and Rust DLL ownership remain intact.

Config/scene/frame data is strictly checked and copied before returning.
Latest Idle scene replacement retains one immutable CPU snapshot. Lazy SDK upload
uses an internal generation; a replacement using the same numeric revision still
invalidates upload, while transform-only frames reuse the SDK scene. One optional
queued task and one in-flight frame prevent queue growth. Interrupted upload is
not marked reusable. Upload/render cancellation/progress uses atomics; poll and
cancel call no SDK, take no mutex and perform no join/readback. Worker completion
publishes Ready/Cancelled only after drain. Ready cancellation may acknowledge
immediately because readback is already complete. First Cancelled poll restores
Idle. Busy/invalid/short-buffer errors retain prior state; short retries write no
caller bytes and report required size.

All C entry paths delegate to noexcept guarded code. Worker exceptions preserve
a bounded diagnostic, destroy the backend on its worker, then publish Failed.
Active owned inputs remain alive through exception cleanup and backend drain.
Failed retains cached describe/poll diagnostics and permits destruction only.
Destroy requests cancellation, joins the worker and returns only after its SDK
destructor, before Rust can unload the module. Zero-alpha RGB is normalized and
the returned image byte count is checked; actual tone mapping/color conversion
remain the SDK backend's responsibility.

## Parser provenance

One parser serves both future bridges: official unmodified `nlohmann/json`
v3.12.0 single header, complete MIT notice retained. Tag `v3.12.0` is annotated:
tag object `65ee68451d8eb2b5f3a30b410476ab83deb3289b` dereferences to commit
`55f93686c01528224f448c19128836e7df245f72`, verified with
`git ls-remote https://github.com/nlohmann/json.git 'refs/tags/v3.12.0*'`.
Both tagged files and independent immutable-commit downloads had identical hashes:

- Header: `https://raw.githubusercontent.com/nlohmann/json/55f93686c01528224f448c19128836e7df245f72/single_include/nlohmann/json.hpp`;
  SHA-256 `aaf127c04cb31c406e5b04a63f1ae89369fccde6d8fa7cdda1ed4f32dfc5de63`.
- MIT notice: `https://raw.githubusercontent.com/nlohmann/json/55f93686c01528224f448c19128836e7df245f72/LICENSE.MIT`;
  SHA-256 `46a65cffd1ea955132d95a8dd921640714a8d6b537d2e4e482d31145ae95b603`.

These are pinned in `native/sdk-lock.json.common_parser`; every CMake configure
checks both vendored file hashes. Local vendor `.gitattributes` preserves exact
upstream bytes through Windows line-ending conversions. No runtime or build download. Distribution
must carry `native/vendor/nlohmann/LICENSE.MIT`; README records provenance.
The official notice's first-line trailing space is retained; its local whitespace
attribute excludes that upstream trailing-space issue from repository lint.

The guards enforce documented schema1 required/optional keys and field types,
UTF-8, duplicate-key rejection, JSON byte limits and a 64-level nesting bound;
finite physical coefficients and aligned spans, material/triangle indices,
camera/clipping/affine transforms, versioned layer/color/quality recipes and
duplicated C/config budgets. Hard derived capacities remain 2M vertices/4M
triangles. Configurable source limits are never reapplied downstream.

## Verification evidence

Coordinator explicitly granted native and shared Cargo slots before use. All
native builds use unique `artifacts/native-core`; Cargo uses the coordinated
`CARGO_TARGET_DIR=D:/code/turninator/target`. No GPU was used.

1. Initial RED, before replacing the old bridge: added a raw config-schema
   assertion to existing `native/fixture/contract.cpp`; old bridge accepted `{}`.
   `./tools/build-renderers.ps1 -Fixture -BuildDirectory artifacts/native-core`
   succeeded; `ctest --test-dir artifacts/native-core -C Release --output-on-failure`
   exited1, **0/1 passed**, named failure `fixture_caller_buffers`
   (`raw ABI contract failure`).
2. Added the controllably pending backend lifecycle test before implementing the
   common facade. Existing raw caller-buffer case now also checks malformed,
   mistyped, wrong-version, duplicate-key, invalid UTF-8 and oversized config;
   physical metadata/range/recipe failures, hard caps, bad indices/nonfinite
   spans and invalid frame/camera/matrix values preserve Idle. The dedicated
   lifecycle case records factory/description/upload/render/destructor thread
   IDs, owned input observations and upload revisions, holds readback behind a
   condition-variable gate, and uses a 3-second watchdog only to detect blocked
   owner calls (not a tight latency threshold). It covers 200 rejected extra
   begins, atomic progress, cancellation drain, Ready cancellation, read retry,
   replacement/reused geometry, exception cleanup and joining pending work.
3. GREEN build with pinned CMake4.2.3, MSVC19.44.35215.0/toolset14.44.35207,
   Windows SDK10.0.26100.0, C++20, dynamic Release CRT. Common/fixture/test sources
   compile with `/W4 /WX /EHsc`. Final
   `./tools/build-renderers.ps1 -Fixture -BuildDirectory artifacts/native-core`
   exited0; all12 fixture variants and dependency DLL rebuilt. Intermediate
   strict-build signedness/test-byte-conversion warnings were corrected.
4. Focused Rust host GREEN:
   `$env:CARGO_TARGET_DIR='D:/code/turninator/target'; $env:TN_RENDERER_FIXTURE_DIR='D:/code/turninator/.worktrees/renderer-native-core/artifacts/native-core/fixture/Release'; cargo test --offline --test embedded_loader`
   exited0, **9 passed / 0 failed / 0 ignored**. Only async observations changed:
   bounded state waits replace fixed two-poll assumptions, and SDK-render errors
   arrive through Failed poll. Matrix fingerprint `[129,172,163,182]`, explicit
   seed/alpha, source-vs-derived capacity, retry, dependency and unload checks
   remain. Covering
   `cargo clippy --offline --lib --test embedded_loader -- -D warnings`
   exited0, clean. Shared Cargo slot released immediately afterward.
5. Nonblocking regression sensitivity RED/GREEN after implementation: temporarily
   inserted a poll wait while state was Rendering, rebuilt only
   `tn_fixture_lifecycle` with
   `cmake --build artifacts/native-core --config Release --target tn_fixture_lifecycle`.
   `ctest --test-dir artifacts/native-core -C Release -R fixture_worker_lifecycle --output-on-failure`
   exited1, **0/1 passed**, named failure `fixture_worker_lifecycle`
   (`pending rendering`) after watchdog released the held worker at3seconds.
   Removed the mutation, rebuilt all original native targets successfully,
   then full focused CTest exited0: **2/2 passed**, no failures. This mutation
   check supplements the initial schema RED; it is not SDK cancellation evidence.
6. `rustfmt --edition 2024 --check tests/embedded_loader.rs` and
   `git diff --check` clean. Native parser/notice hashes rechecked after builds.
   Path-limited staged review and `git diff --cached --check` performed before
   commit. No full-suite rerun was needed for this focused subtask.

## Limits and remaining qualification

This establishes common ABI ownership/state/thread behavior using a CPU fake.
The backend contract still requires its render/error exits and noexcept
destructor to drain SDK operations/callbacks; an internal facade cannot prove an
unknown SDK follows that contract. Workers7/8 must qualify actual SDK/driver
startup/upload/render/readback/cancel/exception/destruction, feature/device/quality
behavior, native allocation expansion and loaded-module identity. SDK-specific
memory estimates remain additional to common supplied-span/output byte guards.
There is no hard cancellation guarantee for a blocked driver/compiler, so create
or destroy can still block. No actual GPU pixels, performance, color/alpha,
volume/layer support, production availability or redistribution qualification
is claimed. Existing Task5a/5b SDK/packaging gates remain open.

## Review fix round1: idle shutdown and upload cancellation

Base: `bf6b4c8399b5b8db875c53c5ec0fe1af52f70d80`. Addresses Important I1 and
Minor M1 from `task-5c-review.md`. Only `native/common/facade.cpp`,
`native/fixture/{CMakeLists.txt,lifecycle.cpp}` and this report change. ABI1,
exports, public Rust contracts, SDK backends and parser pin are unchanged.

I1 root cause was verified in the actual wait path: the predicate was read under
`Context::mutex`, but destruction changed `stopping` without taking that mutex.
An atomic stop store did not prevent notify from occurring between the worker's
false predicate check and its wait. Destruction now sets cancellation promptly,
then acquires the same mutex to set `stopping`, releases it, notifies and joins.
No SDK work holds that mutex, and poll/cancel remain lock-free.

The lifecycle executable alone compiles an instrumented copy of the exact common
facade source with `TN_NATIVE_LIFECYCLE_TEST_HOOKS`; it links the shared core's
validation object. Ordinary fixture/production bridge facades compile without
that macro and contain no hook call or new ABI export. A single hook runs after
the idle predicate evaluates false while the worker still owns its wait mutex.
The regression arms the hook only during a completed frame, reads the drained
frame back to Idle, then destroys on the owner thread. The hook waits for prompt
atomic cancellation and checks that `stopping` has not changed while the worker
still owns the mutex. Old code fails this check; the test exits the worker through
normal exception cleanup instead of deliberately hanging after a lost notify.
Fixed code allows the worker to enter its real atomic unlock/wait transition,
after which destroy obtains the mutex, sets stop and wakes/joins it. This is a
controlled boundary regression, not a probabilistic create/destroy loop.

M1 adds an independent SDK-upload gate to the same fake backend. Cancellation
is observed while upload/drain remains held; poll must stay Cancelling and begin
must reject until the gate opens. No render starts for the interrupted upload.
After drain and Cancelled acknowledgement, retrying the same scene without a
new upload call must produce SDK upload revisions `[7,7]` and one completed
render/readback. The existing conditional upload-generation assignment already
satisfied this behavior; the new test directly protects it.

Coordinator granted the focused CPU/native/shared-Cargo slots. Exact covering
verification in the same worktree/build directory:

1. I1 RED before the destructor fix:
   `./tools/build-renderers.ps1 -Fixture -BuildDirectory artifacts/native-core`
   and `cmake --build artifacts/native-core --config Release --target tn_fixture_lifecycle`
   exited0. Then
   `ctest --test-dir artifacts/native-core -C Release -R fixture_worker_lifecycle --output-on-failure`
   exited1: **0/1 passed**, `fixture_worker_lifecycle` failed in0.03seconds with
   `stop predicate changed while worker still held wait mutex`.
2. I1 GREEN after serializing stop: the same targeted build and CTest exited0,
   **1/1 passed**. The new gated-upload coverage also passed, **1/1**.
3. M1 regression sensitivity RED: temporarily assigned `uploaded_generation`
   unconditionally after upload, rebuilt `tn_fixture_lifecycle`, and ran the same
   targeted CTest. It exited1, **0/1 passed**, named failure
   `interrupted upload generation must be uploaded again` in0.05seconds. Removed
   the mutation; the final code retains `if(!work.cancelled())`.
4. Final complete focused native build:
   `./tools/build-renderers.ps1 -Fixture -BuildDirectory artifacts/native-core`
   exited0 with `/W4 /WX /EHsc`, rebuilding all fixture bridges with the fixed
   ordinary facade and the controlled lifecycle executable. Then
   `ctest --test-dir artifacts/native-core -C Release --output-on-failure`
   exited0: **2/2 passed**, no failures.
5. `$env:CARGO_TARGET_DIR='D:/code/turninator/target'; $env:TN_RENDERER_FIXTURE_DIR='D:/code/turninator/.worktrees/renderer-native-core/artifacts/native-core/fixture/Release'; cargo test --offline --test embedded_loader`
   exited0: **9 passed / 0 failed / 0 ignored**, including repeated context/module
   destruction and all fault/dependency/matrix/capacity cases. Covering
   `cargo clippy --offline --lib --test embedded_loader -- -D warnings`
   exited0, clean; no Rust source/test changes were needed. CPU/native/shared
   Cargo slots were explicitly released after these checks.
6. `git diff --check`, path-limited staged review and
   `git diff --cached --check` clean before commit. No whole-suite or GPU tests.

These changes resolve the ordinary idle-worker missed wake. They do not alter
the documented possibility of SDK/compiler/driver blocking during teardown;
actual SDK cancellation/drain and GPU behavior remain separately unqualified.
