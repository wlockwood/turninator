# Native package qualification

This is the staged Task 10 checklist. The five-engine packaged application has
**not completed qualification**. Backend implementation evidence is separate
from extracted-package, UI and matched-performance evidence.

| Engine | Backend evidence | Extracted package | Painted UI | Matched 960 animation / 4096 still |
| --- | --- | --- | --- | --- |
| Native wgpu | Task 6 reviewed | Pending | Pending | Pending |
| Filament Vulkan | Tasks 7a/7b reviewed; raster resin approximate | Pending | Pending | Pending |
| LuxCore CUDA/OptiX | Tasks 8a/8b integration pending | Pending | Not an interactive viewport choice | Pending |
| Blender EEVEE | Shared adapter implemented | Pending | Pending | Pending |
| Blender Cycles OptiX | Shared adapter implemented | Pending | Pending | Pending |

Only Windows x64 is in the native qualification scope. Previously observed
RTX 4070 Ti results do not qualify another device or driver. Native SDK presence,
a successful build, and test registration are not runtime qualification.

## Package inputs

`tools/package.ps1` defaults to `-Variant full`, requiring an explicit
`-NativeSdkManifest` and `-NativeRuntimeDirectory`. Full mode currently fails
closed until the reviewed LuxCore receipt and final native qualification closure
are integrated. `-Variant core` omits both optional SDKs and records `not_bundled`
in `native-sdk-manifest.json`. No native executable or SDK is downloaded.

The native runtime directory is a closed payload. Its schema-1 manifest records
the exact SDK-lock hash, components, notices, and every payload file's relative
path, SHA-256, byte size, component and role. Allowed roles are bridge, runtime,
asset, notice, source and receipt. Runtime files belong under `renderers/`,
notices under `notices/`, and corresponding-source archives under `sources/`.
Build-machine absolute paths and reparse points are rejected. DLL normal and
delay imports must resolve to the same backend directory or a recognized
Windows system component; MSVC/OpenMP are not assumed to be OS components.

PE imports cannot discover all `LoadLibrary` calls, linked static objects, or
license obligations. Reviewed native build attestations, link inputs/maps,
embedded asset recipes, full notices, and actual loaded-module evidence must
close those separate requirements before full packaging is enabled. Receipt
hashes identify the actual final DLL, including embedded materials/environment;
they are packaging assertions, not signatures.

`generate-notices.py` appends the complete native notices to Cargo/font notices,
including aggregate notices and exact corresponding-source instructions. It
checks notice hashes again when reading the staged package. Notice generation
does not modify the checkout's `THIRD-PARTY-NOTICES.md`.

## Runtime evidence to retain

- Extract both variants, launch outside the package directory, and hide external
  tool availability for native preview/PNG/sheet cases. Rename each optional
  bridge and an imported dependency; verify launch, preserved saved selection,
  and a backend-specific unavailable explanation.
- Use the same opaque STL, G-code beads, stepped/cutaway and resin scenes across
  all five engines. Retain input/output/SDK hashes and concrete device, quality,
  denoiser and memory-cap settings. Use 960-square, 80 frames, 20 fps and a
  4096-square still without silently reducing quality or dimensions.
- Keep cold initialization/compile, upload, rendering, readback, validation,
  encoding and end-to-end observations distinct. Host elapsed rendering is not
  isolated GPU kernel time. Serialize GPU work and note external graphics hooks.
- Cycle engines twenty times; cancel during upload/render/readback, reject an
  oversized request, and retain memory/callback/context observations. Existing
  destinations and rejected retained-job records must remain protected.
- Inspect actual images, masks, dimensions, alpha composites, layers, motion,
  stationary regions and painted UI against the frozen shared tolerances and
  reviewed appearance baselines. Driver hangs may require terminating the owned
  application; an API timeout alone cannot guarantee recovery from a hung driver.

## Child-process audit

`tools/process_audit.py` starts the explicit executable suspended, assigns it to
an owned Job Object, then resumes it. `TotalProcesses == 1` after exit proves no
child process was launched, including short-lived helpers. Assignment failure
fails the audit. The job has no breakaway permission; timeout/cleanup affect only
that owned job. The harness records process identity, executable hash, elapsed
CPU/wall time and peak process/job commitment, which is not GPU memory usage.
Observed process-image samples are diagnostic and do not replace accounting.

The harness uses `DETACHED_PROCESS`: Windows created a `conhost.exe` for the
console fixture even with `CREATE_NO_WINDOW`. No count is subtracted to excuse
that process. The no-child and fast-child tests respectively observe totals 1
and 2. GUI applications keep their normal window behavior. Encoder cases need
separate child identity/command evidence; a count alone cannot prove that only
the requested encoder/probe ran.

Example, after obtaining the coordinator's exclusive runtime grant:

```powershell
python tools/process_audit.py --cwd <outside-directory> --output <new-audit.json> --require-no-children -- <absolute-qualified-executable> <arguments>
```

## Redistribution conditions

Local test assembly is separate from authorization to publish a distribution.
Microsoft's [VS2022 redistribution list](https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution)
identifies unmodified VC/redist files and conditions distribution on applicable
Visual Studio licensing. The [runtime EULA](https://visualstudio.microsoft.com/license-terms/vs2022-cruntime/)
alone grants installation/use, not a general redistribution right. A local
Community installation is evidence of an installed edition, not legal
certification. Retain the applicable Community/BuildTools/runtime terms and
exact delivered CRT hashes; do not copy arbitrary System32 DLLs.

LuxCore's OIDN aggregate notices, c-blosc notices, vendored header attributions,
corrected source notices, exact Eigen MPL2 source availability, and the pinned
NVRTC archive terms are separate closure items. The NVRTC compiler and builtins
must match; unused `.alt` and driver DLLs are not automatically payloads.
