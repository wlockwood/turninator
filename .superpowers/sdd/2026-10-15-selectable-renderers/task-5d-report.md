# Task 5d — corrected reproducible LuxCore SDK

Scope: private Windows CUDA/OptiX SDK prerequisite, not a production renderer
adapter. Worktree `D:/code/turninator/.worktrees/luxcore-sdk`, branch
`codex/luxcore-sdk`, base `bf6b4c8`. No Cargo, ABI, core, host, pipeline, UI,
sampling/jitter/material/transport source changes. No subagents. Original
downloaded archives, extractions and probe caches remain immutable.

## Inputs and owned artifacts

`Q` is the original
`D:/code/turninator/.worktrees/selectable-renderers/artifacts/renderers/qualification/2026-10-15-sdk-probes`.
`B` is this worktree's `artifacts/luxcore-sdk`. Full native outputs and logs are
ignored under `B`; reviewable patches/tests/tool locks are under
`native/patches/luxcore`. The build recipe is `tools/build-luxcore-sdk.ps1` and the
consumer recipe is `native/patches/luxcore/qualify.ps1`.

- Upstream source commit `b872186d2fe995a762f73708c606847eed55843e`, version2.11.2;
  archive SHA256 `31a82d87c1f912864e7b039620747507d2b0ce9db70648656d41d0ebc4636cda`.
- LuxCoreDeps2.2.3 archive SHA256
  `069a1eace7e8f192b07629d6318eca9e0ad970f8c7db39a0919b10eddc2345d0`.
- Exact official Conan2.18.1 tooling and fifteen support archives were downloaded
  into B/tool-wheels; every artifact SHA256 was checked against its official
  PyPI release JSON. `tools-lock.json` retains immutable URLs and digests.
- CMake4.2.3, Python3.10.11, MSVC19.44.35215.0, link14.44.35215.0 are recorded by
  executable SHA256 in `host-tools.json`. SDK and consumer use C++20/Release/MD/
  iterator-debug0, exact Boost1.88. Official dependency metadata identifies
  MSVC19.44.35228; this patch-level difference within ABI family194 is explicit.

## Dependency graph correction

The official cache restored 53 entries successfully. Offline graph resolution
first failed on absent build tools (b2), then on absent header-only
`getopt-for-visual-studio/20200201` referenced by the precompiled giflib package.
The attempted platform-requirement omission was rejected because it changed
binary package IDs; it was removed. No compiled dependency was rebuilt or
silently substituted.

Coordinator explicitly authorized restoring the missing exact header-only
package from the official Conan remote. Retrieved recipe revision
`32ee360e2552fe65ad8bec52edcd64fd`, package
`da39a3ee5e6b4b0d3255bfef95601890afd80709`, package revision
`26eb4a9cdd6e4c7b9d2c892b970f14d3`. A local cache export with SHA256 in
`getopt-archive.sha256` supports entirely offline replay.

Successful graph command:

```powershell
$env:CONAN_HOME = "$B/conan"
& "$B/venv/Scripts/conan.exe" install "$B/source/build-system/conan" `
  -pr:h "$B/profile" -pr:b "$B/profile" --build=never --no-remote `
  -c tools.graph:skip_build=True
```

`skip_build` omits irrelevant build-tool expansion, while official full runtime/
build-generator dependencies remain restored. `tools.system.package_manager:mode`
is `check`, overriding upstream `install`. Python tool installation uses a
private venv and pip cache. An initial pip attempt to write its default cache was
denied; rerun used the owned cache. No system install or HOME/TEMP/TMP changes.

## Source corrections

- Per-context UTF8 root, compiler SHA256 and SDK SHA256 enter via existing
  `context.*` forwarding. Constructor rejection is explicit; no global fallback.
- CNG SHA256 key length-frames actual source/program/architecture/effective
  options. The exact same option-producing function feeds compilation and key
  calculation. Schema1 header binds compiler, SDK, key, size and PTX digest.
- 64MiB maximum; bounded read/allocation; required terminating NUL; corrupt/
  foreign/old-format records miss. Metadata and PTX share one atomically
  published file. Publication uses a unique owned temporary file, flush and
  MoveFileEx with bounded sharing-race retries. Reparse destinations fail.
- The optional global PathOCL `.ck` readiness hint is disabled; actual validated
  CUDA cache use remains enabled. Global caches are never consulted or cleaned.
- Narrow existing export annotations are activated on Properties and Matrix
  utility translation units, plus a Matrix class export annotation. The final
  DLL links system bcrypt. No consumer utility translation-unit workaround.
- Native-only CMake option excludes Python runtime/console/UI targets while
  retaining build-time interpreter generation. NVRTC program/PTX ownership is
  reclaimed on compile/cache/driver-loading exceptions; compile algorithms and
  actual options remain unchanged.

## Red/green and recipe evidence

`B/red-consumer.log` reproduces 12 unresolved utility symbols against the
official import library. The first consumer attempt had a missing Matrix header;
that test error was corrected before recording the actual linker RED.

The native cache suite tests cold/warm roundtrip; different compiler/SDK
namespaces; foreign copied headers; old13.1 PTX; corruption; oversized files and
writes; publication failure; and parallel independent cache objects. An early
fixture string length error was corrected. Extended-length Windows paths were
necessary for long owned namespaces. Windows concurrent rename error5 required
a bounded retry. Sandbox execution also denies otherwise-authorized atomic
publication; successful native tests used scoped escalation in unique owned
directories.

Mutation RED: remove only compiler-header comparison from the ignored test copy,
build and run; exit1 `foreign compiler header accepted` (`B/cache-red.log`).
Restore the real header and force its timestamp invalidation before rebuilding;
a stale incremental binary was detected and discarded. Final real fixture run:

```powershell
& "$B/red-build/Release/cache_tests.exe" "$B/test-cache-actual-foreign" `
  "$Q/luxcore-probe/cache/luxcorerender.org/cuda_kernel_cache/LUXRAYS_2_11_2/3c36c5a9-74c6f1d1_compute_89.ptx"
```

Exit0: `actual_foreign_ptx_rejected=...` then `OWNED_CACHE_PASS`; log
`B/cache-actual-foreign.log`. Original fixture is only read and copied into owned
test storage. Its known whole-file SHA256 is
`c9449aad6ac965cebbb5d93c6da46cfef6f924777cfd3ad389ecfcfd796ab166`.

Clean scripted configuration rehearsals succeeded under separate fresh owned
`artifacts/luxcore-sdk-recipe-check` and `artifacts/luxcore-sdk-recipe-final` roots:

```powershell
pwsh -NoProfile -File tools/build-luxcore-sdk.ps1 `
  -QualificationRoot $Q -OutputRoot artifacts/luxcore-sdk-recipe-final -ConfigureOnly
```

The recipe extracts regular source files only and records skipping upstream's
dangling editor `.clang_complete` symlink. Original Copy-Item exploration hit that
link; the final recipe uses verified archive extraction. Git autocrlf differences
in patched text are normalized to LF. Source/configuration reproduction is
tested; bit-identical PE/PDB output across paths/machines is not asserted.

## Native build and GPU qualification

The complete native build passed (`B/build-final.log`), followed by two narrow
incremental corrections (`build-config-correction.log`, `build-loader-correction.log`).
Direct actual-importlib consumer passes Properties/Matrix operations, caught
DLL exception/destruction, and exact UTF8 private property roundtrip while
unrelated context filtering remains unchanged (`B/direct-consumer.log`).

The first GPU attempt failed before compiler/cache use because upstream
`RenderConfig::ToProperties` dropped all private context properties. The valid
complete-camera regression reproduced `CONFIG_FORWARD_FAIL` exit5; the approved
fix preserves only the three required fields. An initial incomplete camera
fixture crashed (C0000005); replacing it with the existing complete valid scene
was a test setup correction, not a claim of fixing invalid-scene engine safety.

The next cold render produced correct RGBA/sample counts but FAILED compiler
qualification: emitted PTX explicitly identified CUDA13.1, despite preloaded
verified12.8 DLLs. CUEW preferred ambient `nvrtc64_130_0.dll` through PATH. That
owned namespace is retained as RED evidence, never promoted or reused by the
final corrected SDK. `render-cold-loader-red.log`, `loader-red.log` (exit5,
wrong actual function binding13.1), and `ptx-red.log` (actual compiler12.8
assertion fails) record the failure. No global cache was modified by this test.

Coordinator explicitly approved the additional Windows CUEW correction: bind
only the already-loaded120/128 pair; retain owned module references through SDK
teardown; check NVRTC12.8; no filesystem/PATH search or fallback. The caller still
must hash/path-verify both modules before Init; names/version are not attestation.
CPU loader GREEN covers missing compiler, missing128 builtins, wrong131 builtins,
correct12.8 pair, and both13.1+12.8 pairs simultaneously preloaded with ambient
PATH intact. Exact nvrtcVersion function pointer equals the pinned module export.

Final source proof (`source-verification.json`) verifies2036 unchanged regular
archive members byte-for-byte, all10 patched files against a fresh archive+patch
application, and the owned helper header. Dumpbin evidence has been refreshed
for the final DLL. All corrected GPU cases now pass their specified checks;
`native/sdk-lock.json` adds only `source_builds.luxcore`, preserving every existing
record unchanged. `native/patches/luxcore/qualification.json` retains output,
source/tool input and evidence hashes plus the explicit qualification limits.

Final DLL SHA256:
`63e96b108e5007c6d378e2ac0b575d5db6b9e9e8b7b9a69a5b7cef3856f457b5`.
Final import library SHA256:
`ce8e2f0e786d78a4b77673f63fd5f1578e03a7c2a4f07e6079eb4e77b9f18c67`.
NVRTC12.8 SHA256:
`02d2d7ef4690bf1a55dda1d3ac94c86ce8d16920071c87bcd3e70e94ef346e93`;
builtins128 SHA256:
`a7afa2a40cbd3f922a33d8bf71c13cb26ca707c7016a9bc5fcde36759ba668f5`.
Compiler-pair identity:
`4cbf85c6afdf0092be563772ec2cd142336d933d4c57b18fba584bb73514ea9c`.

The retained aa50b35d/09274 intermediate SDKs are superseded and unqualified.
No production bridge may select those identities.

Final commands (PowerShell7, from this assigned worktree; Q/B as above):

```powershell
cmake --build "$B/build" --config Release --target luxcore --parallel 1
pwsh -NoProfile -File native/patches/luxcore/qualify.ps1 `
  -BuildRoot $B -QualificationRoot $Q -Mode prepare
& "$B/runtime/cache_tests.exe" "$B/cache-cpu-final" `
  "$Q/luxcore-probe/cache/luxcorerender.org/cuda_kernel_cache/LUXRAYS_2_11_2/3c36c5a9-74c6f1d1_compute_89.ptx"
& "$B/runtime/direct_consumer.exe"
# Each loader case is a separate process, no GPU rendering:
foreach ($mode in 'missing','missing-builtins','wrong-builtins','preloaded','ambient') {
  & "$B/qualification-build/Release/loader_tests.exe" $mode "$B/runtime" `
    'C:/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.1/bin/x64'
}
# Serialized GPU grants: cold/warm pair, release, then remaining warm batch.
foreach ($mode in 'cold','warm','edits','cancel','bounded64','edge-diag','parallel') {
  pwsh -NoProfile -File native/patches/luxcore/qualify.ps1 `
    -BuildRoot $B -QualificationRoot $Q -Mode $mode
}
```

All successful GPU processes exited0 except the expected raw edge diagnostic21,
which the recipe explicitly expects and handles. Every mode also ran
`python native/patches/luxcore/inspect-cache.py $B`, validating actual compiler
header12.8, full record identity/digest/size/key and terminal NUL. Runtime module
preloads use exact canonical paths and SHA checks; final module snapshots contain
only the pinned NVRTC120 and builtins128 compiler pair. Seventy actual loaded
module files were hashed in `B/loaded-modules-final.json`; complete runtime
snapshots remain in the render logs. No environment variables were changed for
compiler selection or cache isolation.

Functional results on NVIDIA RTX4070Ti:

| Case | Observed result |
| --- | --- |
| Unicode cold64x64 AA3 | Both kernels not cached; all4096 pixels9 samples; center0.830963/0/0/1, corneralpha0 |
| Warm same identity | Both kernels cached; identical image values and all9 samples; records bytehash/size/mtime unchanged |
| Persistent A/B/A | Red/green/red, all9 each;14 float values differ on reset, max error1.19209e-7 within1e-5 guard |
| Active1024 AA8 cancel then64 AA3 restart | Cancel overlaps active frame; Stop3.3017ms, restart15.9321ms; resumed pixels all9 |
|65x49 requested minimum64 using AA9 | Actual80�82, total257985 = nominal81 total; minimum guard passes |
|65x49 nominal64 AA8 diagnostic | Actual63�65, total203840; expected exit21 preserves known adjacent-pixel behavior |
| Two parallel independent SDK contexts | Both64x64 all9 with correct finite RGBA; clean teardown |

Cold Start131663ms and warm Start62.918ms are functional diagnostics only, not
controlled performance or worst-case latency claims. Warm record manifests
`cache-cold.json` and `cache-warm.json` compare exactly, including modification
nanoseconds (`warm-records-check.log`). All GPU modes reused the same two final
identity records, with no new kernels compiled after the cold pair.

Actual emitted PTX identifies CUDA12.8.93, compiler buildCL-35583870, PTX8.7,
targetsm_89. PathOCL source SHA is
`7952a163a2cf577462ba33dfe137d59a576cd6415dd5055063a71e88c4b01d8f`,
key `11e0b3d6440602efc8c0ff4ee83fbb70cb79a34b36b2c97f19a03402a9fe82a7`,
PTX SHA `d4c339339679e8c05fc8458a9d3ff8fcabe3cb8ca0e721872cff14fb62ebcf2e`,
size6148459bytes. Optix PTX SHA
`158c4e21e30969d174ad7758221e52a0110e83d6fa78fd29d8bf6f3b3c5c7ede`,
size2816bytes. Exact source/architecture/effective option list is logged for each
kernel, including all fixed-Xcudafe flags, split-compile/PCH, defines and
use_fast_math. No compiler option or rendering algorithm was altered.

The missing getopt header-only package additionally records source archive
`https://github.com/skandhurkat/Getopt-for-Visual-Studio/archive/6708172892a4d89042b743315e8a52e2d9d5defc.zip`,
source SHA `9b50026b3f10c3f6a7340e0074a898d6d1105eef068bf98d90af99770375a465`
in its immutable Conan recipe, and restored cache archive SHA
`7cc6063756c0d809baf4d7c323091ffae33dc13332b618904cd3f16338fbdcd0`.
No compiled dependency package identity changed or was rebuilt.

## Boundaries retained

The bridge must verify actual loaded compiler/builtins/SDK modules before
supplying private identities. Qualification uses compiler identity SHA256 over
`<nvrtc SHA256>\n<builtins SHA256>\n`, SDK identity equal to actual DLL SHA256,
and a real Unicode owned root `cache-é-層`. Existing `.ck` readiness returns false.
OpenCL cache isolation and driver-managed JIT/OptiX caches are separate concerns.

The cache root is an application-owned trust boundary, not an ACL sandbox against
an attacker who can replace directory ancestors concurrently. No Windows
filesystem TOCTOU immunity is claimed. Clean object destruction and /MD matching
are functional lifecycle evidence, not proof of zero allocations/leaks.

Upstream pixel-boundary float sampling, numerical non-bitwise determinism and
canonical recipe semantics remain unchanged. Full PBR/resin/layers, shared image
conversion, production cancellation/owner-thread integration and adapter
promotion remain Task8. No fake backend is marked available.


## Fix round 1 — build recipe provenance (after 7466398)

Independent review `task-5d-review.md` found two P2 recipe issues. The actual
compiled SDK and its CUDA12.8 functional evidence remained supported. This round
changes only tool-input materialization/installation, documentation, and evidence
hashes. No vendor source, compiled dependency, pinned tool version, SDK binary,
Cargo code, GPU work, or native target rebuild changed.

I1 RED used a real inert `requests-2.99.0-py3-none-any.whl` added beside the locked
archives. The old pip invocation (`--no-index --find-links <reused directory>
conan==2.18.1`) selected that unlisted compatible transitive dependency. The dry-run
report `B/fixround1/poison-red.json` records its file URL and version2.99.0; the
wheel was not installed or executed in that RED run.

The corrected recipe calls `materialize-inputs.ps1` to copy only the sixteen
locked archives plus the verified getopt archive into a fresh owned directory.
It verifies source and copied SHA256 values, emits explicit file requirements
with hashes, installs with `--isolated --no-index --no-build-isolation --no-deps
--require-hashes`, then runs `pip check`. Bootstrap setuptools/wheel and all
Conan transitives are explicit pinned files. No resolver receives the reusable
download directory or can select its extra candidates.

I2 is resolved by requiring the existing SHA256-verified `getopt-exact.tgz` for
all recipe paths. `-AllowToolDownload` now applies only to locked PyPI tools.
Missing/corrupt getopt fails before tool installation. The build recipe no longer
acquires Conan packages from a remote; the previously acquired exact offline
artifact remains unchanged. Separately acquiring a replacement export requires
its own payload qualification and pin update, since export metadata may vary.

Focused tests (`test-build-inputs.ps1`) inject a valid extra compatible wheel,
corrupt the listed requests wheel, corrupt getopt, and omit getopt while download
permission is enabled. GREEN excludes the extra wheel and rejects all invalid
inputs. A temporary ignored helper mutation removing only SHA enforcement makes
the corrupt-getopt assertion fail with `INVALID_INPUT_ACCEPTED`, exit1, proving
that test detects bypassing the payload check (`fixround1/getopt-red.log`). The
real helper is unchanged by the mutation and passes (`fixround1/inputs-green.log`).

Commands from this worktree:

```powershell
pwsh -NoProfile -File native/patches/luxcore/test-build-inputs.ps1 `
  -ArchiveDirectory artifacts/luxcore-sdk/tool-wheels `
  -OutputDirectory artifacts/luxcore-sdk/fixround1/green-fixtures
pwsh -NoProfile -File tools/build-luxcore-sdk.ps1 `
  -QualificationRoot $Q `
  -OutputRoot artifacts/luxcore-sdk-fixround1-configure `
  -ToolArchives artifacts/luxcore-sdk/fixround1/poisoned-archives `
  -ConfigureOnly
```

The offline ConfigureOnly rehearsal passed against the directory containing the
unlisted2.99.0 wheel. Installed metadata confirms all sixteen original versions,
including requests2.34.2 and Conan2.18.1; `pip check` reports no broken requirements.
The verified offline Conan restore, `--no-remote --build=never` resolution, pinned
compiler checks, and final CMake configuration all passed. No SDK native target
was built. Logs and installed versions are retained under `B/fixround1`; the full
fresh rehearsal tree is `artifacts/luxcore-sdk-fixround1-configure`.

`qualification.json` retains the original build recipe SHA and qualification
manifest SHA as historical evidence, updates current recipe input hashes, adds
these focused test/configuration evidence hashes, and states that this was a
recipe-only rehearsal. The product lock updates only that manifest SHA. All six
actual output hashes, prior GPU/CPU evidence hashes, original source/dependency
archive identities, and compiler-pair/SDK identities remain unchanged.
