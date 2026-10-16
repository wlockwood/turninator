# Native package qualification

Qualified locally on 2026-10-15: native desktop selection, real SDK rendering,
common output formats, extracted full/core packages and the forty-case matrix.
The browser application is unchanged. Qualification covers this machine and
these exact runtime bytes; other hardware and drivers remain unqualified.

| Engine | Backend evidence | Extracted package | Painted UI | Matched 960 animation / 4096 still |
| --- | --- | --- | --- | --- |
| Native wgpu | Real PBR/shadows/AA; resin approximate | Pass, no tools | Pass | All 8 cases pass |
| Filament Vulkan | Real SDK, embedded materials/IBL; resin approximate | Pass, no tools | Pass, including viewport handoff | All 8 cases pass |
| LuxCore CUDA/OptiX | Real path tracer/OIDN; limitations below | Pass, no tools | Pass for exports; no LuxCore interactive viewport | All 8 cases pass |
| Blender EEVEE | Batched external adapter; resin approximate | Pass with installed Blender | Pass | All 8 cases pass |
| Blender Cycles OptiX | External traced adapter | Pass with installed Blender | Pass | All 8 cases pass |

Only Windows x64 is in the native qualification scope. Previously observed
RTX 4070 Ti results do not qualify another device or driver. Native SDK presence,
a successful build, and test registration are not runtime qualification.

## Package inputs

`tools/package.ps1` defaults to `-Variant full`, requiring an explicit
`-NativeSdkManifest` and `-NativeRuntimeDirectory`. Full mode validates both reviewed backend receipts, exact native imports and
qualification attestations before staging. `-Variant core` omits both optional SDKs and records `not_bundled`
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

## Runtime evidence

Evidence lives under `artifacts/renderer-matrix-final`, `artifacts/luxcore-host`,
`artifacts/extracted-package-proof-v1` and `artifacts/switch-proof-final` in the
integration checkout. JSON/CSV records retain input/output/runtime hashes,
quality, actual device/driver identities, phase timings and process audits.

Extracted packages launched outside their own directory. Native preview,
transparent PNG and sheets passed with Blender/FFmpeg/ffprobe paths absent.
Nine native-format cases share one owned desktop process with cumulative process
count exactly one. The core package launches and retains unavailable Filament
and LuxCore selections. Separate actual launches rejected a missing bridge for
each SDK, missing Filament MSVC dependency, missing LuxCore Embree dependency,
saved unavailable CUDA device, and an output exceeding configured memory.
The UI clamps an 8192 input to 8000 before that memory rejection. Busy cancellation
passed. All ten desktop launches exited normally; source fixture hashes stayed
unchanged. Blender cases explicitly used the installed tool and counted its
children, including Windows console infrastructure.

Twenty complete cycles rendered and destroyed 120 contexts across wgpu,
Filament, LuxCore, Eevee, Cycles and wgpu. After the warm-up baseline, each cycle
stayed within 128 MiB private memory and 32 handles of that baseline; the retained
SDK lease is intentional. After registry destruction, private memory was
491364352 bytes and407 handles, below the warmed 502190080 bytes/689 handles.
The test uses 64-pixel Low/4-sample, denoise-off output to test lifetime rather
than quality. Native per-backend tests separately verify zero WDDM residency
after job-context destruction. The large LuxCore case used 524288 triangles,
4096 pixels and a measured4.58 GB private-memory peak, with661 MB WDDM residency.

LuxCore confirmed-active-render cancellation drained in 46.71 ms in the native
lifecycle test, with 887808 samples observed before cancellation; restart and
pending destruction passed. This is one functional observation, not a latency
guarantee. CPU SDK preparation and kernel compilation can delay cancellation,
and a hung graphics driver can require terminating the owned application.

Transparent LuxCore resin RGBA and denoised fine-layer animation remain explicitly
Approximate. Use an opaque studio still for consistent refraction; disable
denoising for stable fine-layer detail. The failed temporal-denoise criterion
was preserved rather than relaxed. Raster engines do not advertise fully traced
material volume. A 4096 LuxCore denoised still needs a 16 GiB host ceiling under
the conservative reservation, above the default 8 GiB. These are allocation
ceilings, not installed/free RAM or VRAM claims.

Native libraries load from canonical application-owned paths. Their frozen
normal-import dependencies are explicitly retained in dependency order and
released after the bridge, without PATH search or helper processes. Installed
inventory hashes/sizes and local CRT presence are checked before SDK work,
preventing System32 from masking a missing packaged file. An intermittent 126
load error reproduced on process 7 before this change; thirty fresh preparation
processes passed afterward. The exact failed nested import was not attributed.

The original Eevee cutaway run stopped during frame 59 acceptance/manifest
publication with AccessDenied 5. A controlled external file-lock test reproduced
that error class. Bounded atomic-JSON retries now tolerate a brief lock and
preserve the previous file on permanent failure; the full 80-frame cutaway rerun
passed. No antivirus or particular external file holder was proven responsible.

## Observed performance

Windows 11 Pro build 26300, Ryzen 9 9950X 16 cores/32 threads, approximately 93.6 GiB
visible RAM, RTX 4070 Ti. Filament reported Vulkan 1.4.329/NVIDIA 595.79; LuxCore
reported NVIDIA 32.0.15.9579/CUDA 13020 and recorded actual loaded driver hashes.

All scenes are deliberately small synthetic fixtures: a layered 10 mm cube,
eight actual stadium bead strands, a two-level capped cutaway, and solid resin.
Their geometry hashes match across all engines and output sizes. This does not
establish dense-model throughput or equal appearance/noise between renderers.
Animation is 960 square, 80 frames, 20 fps; stills are 4096 square. Raster quality
is Medium for animation and High for stills. Path profiles request 16/64 samples
and denoising; LuxCore guarantees a minimum using AA5/AA9 with overshoot.
These backend-specific settings are not equivalent quality scores.

The table measures fixture preparation through validated publication. The CSV
also includes whole-process/teardown time and peak job commitment where audited.
The direct LuxCore resin animation has no Job Object peak-memory/child-process
audit; its output and pipeline/SDK timings were verified, and its separate still
and actual resin-alpha desktop case have audits. No missing audit was invented.
GPU rendering was serialized; CPU checks and packaging overlapped portions of
LuxCore runs. These are single workstation observations, not CPU-quiet or
isolated GPU benchmarks. Host producer, sink, encoding/validation, startup and
native first/later-frame phases remain separate; nested OIDN timing is included
in native readback/color time. Callback intervals can include metadata and are
not per-image GPU timing. Session startup includes SDK compilation, whose
exclusive GPU time is not exposed by the frozen ABI.

| Engine | Scene | Animation, 80 × 960 | Still, 4096 |
| --- | --- | ---: | ---: |
| wgpu | opaque | 6.60 s | 0.97 s |
| wgpu | beads | 6.07 s | 0.79 s |
| wgpu | stepped-cutaway | 3.25 s | 0.79 s |
| wgpu | resin | 3.23 s | 0.94 s |
| filament | opaque | 4.05 s | 0.81 s |
| filament | beads | 3.58 s | 0.56 s |
| filament | stepped-cutaway | 6.50 s | 0.68 s |
| filament | resin | 3.85 s | 0.69 s |
| luxcore | opaque | 467.48 s | 227.19 s |
| luxcore | beads | 226.24 s | 90.36 s |
| luxcore | stepped-cutaway | 217.07 s | 95.24 s |
| luxcore | resin | 699.73 s | 402.40 s |
| blender_eevee | opaque | 20.71 s | 3.94 s |
| blender_eevee | beads | 13.09 s | 3.85 s |
| blender_eevee | stepped-cutaway | 22.34 s | 14.62 s |
| blender_eevee | resin | 20.37 s | 4.85 s |
| blender_cycles | opaque | 78.55 s | 25.60 s |
| blender_cycles | beads | 76.32 s | 23.40 s |
| blender_cycles | stepped-cutaway | 76.62 s | 24.35 s |
| blender_cycles | resin | 92.34 s | 40.26 s |

Filament is the preferred Eevee-like choice for responsive native PBR on this
machine. Native wgpu is also fast. LuxCore supplies in-process traced transmission
and volumes, with substantially higher observed cost in this adapter; removing
Blender does not guarantee faster ray tracing than Cycles. See
[host behavior](luxcore.md) and [native implementation](luxcore-native.md).

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
