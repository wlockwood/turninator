# Corrected Windows LuxCore SDK

This recipe builds a private Windows x64 LuxCore 2.11.2 SDK from upstream commit
`b872186d2fe995a762f73708c606847eed55843e`. It corrects CUDA PTX cache provenance,
cache ownership, and missing Properties/Matrix utility exports. It does not
implement or qualify Turninator's production LuxCore adapter.

The upstream source archive SHA256 is
`31a82d87c1f912864e7b039620747507d2b0ce9db70648656d41d0ebc4636cda`.
The official LuxCoreDeps 2.2.3 ZIP SHA256 is
`069a1eace7e8f192b07629d6318eca9e0ad970f8c7db39a0919b10eddc2345d0`.
Existing source, dependency extractions and earlier probe caches remain immutable.

## Build

Use PowerShell 7, the recorded CMake 4.2.3/Python 3.10.11 binaries, and MSVC
19.44.35215 x64. `native/patches/luxcore/host-tools.json` records executable hashes.
The SDK and direct consumer use C++20, Release, `/MD`, and
`_ITERATOR_DEBUG_LEVEL=0`, with exact Boost 1.88 headers. Precompiled dependencies
record MSVC 194; their official build metadata identifies 19.44.35228. This is
the same compiler ABI family, with the patch-level difference retained explicitly.

```powershell
pwsh -NoProfile -File tools/build-luxcore-sdk.ps1 `
  -QualificationRoot '<original SDK qualification directory>' `
  -OutputRoot artifacts/luxcore-sdk-rebuild `
  -ToolArchives artifacts/luxcore-sdk/tool-wheels
```

The output directory must be fresh and below this worktree's `artifacts` directory.
The script never recursively deletes existing outputs. Add `-ConfigureOnly` for
an offline dependency/configuration rehearsal. `-Parallel 4` selects compiler
`/MP4` with one MSBuild project at a time to cap compiler concurrency. The native `luxcore` target is built; Python runtime,
console/UI programs, demos and tests are excluded by the vendor CMake option.
Build-time Python interpreter discovery and kernel generation remain enabled.

The tool archive directory contains the exact files in
`native/patches/luxcore/tools-lock.json`, plus `getopt-exact.tgz` with the SHA256
in `getopt-archive.sha256`. `-AllowToolDownload` permits downloading missing
locked public build tools and the exact missing header-only Conan package. It
never enables runtime downloads or compiled dependency rebuilds. Tool hashes
were checked against official PyPI release metadata; installation uses a private
venv and pip cache. Conan's cache is private to this output, and `CONAN_HOME` is
restored on exit. HOME/TEMP/TMP and system installation are untouched.

The original dependency archive omits one header-only dependency referenced by
its precompiled giflib package:

```text
getopt-for-visual-studio/20200201
recipe: 32ee360e2552fe65ad8bec52edcd64fd
package: da39a3ee5e6b4b0d3255bfef95601890afd80709
package revision: 26eb4a9cdd6e4c7b9d2c892b970f14d3
```

That exact package is restored separately. Dependency resolution then runs with
`--no-remote --build=never -c tools.graph:skip_build=True`. The last switch omits
build-tool dependencies for packages that cannot be rebuilt; winflexbison and
Ninja are retained as full dependencies in the official graph. Package manager
mode is overridden from `install` to `check`. No compiled dependency identities
are substituted. Missing packages fail the build.

The patch and the additional header are separate reviewable inputs. The source
extractor omits archive links, including upstream's dangling editor
`.clang_complete` link. Patched text is normalized to LF to avoid Git autocrlf
differences. Logs retain configuration, dependency resolution and native build
commands. Reproduction means the pinned source/configuration/build procedure;
bit-identical PE/PDB outputs across machines or build directories are not asserted.
Every resulting DLL must receive its own measured identity and qualification.

## Private context properties

Before creating a CUDA render context, the native adapter must supply:

| Property | Required value |
| --- | --- |
| `context.cuda.cache.directory` | Canonical absolute local Windows directory encoded as UTF8, owned by the application |
| `context.cuda.cache.compiler_identity` | Lowercase SHA256 identifying the actual verified loaded NVRTC DLL and matching builtins |
| `context.cuda.cache.sdk_identity` | Lowercase SHA256 identifying the actual corrected SDK build |

The patch preserves exactly these three fields through `RenderConfig::ToProperties`,
then LuxCore's existing `context.*` forwarding. Unrelated filtering is unchanged. Missing, relative,
invalid UTF8, traversal, device/UNC paths, alternate data streams, ambiguous path
components and reparse directories fail explicitly. Extended-length Win32 paths
are used internally. The caller must protect the owned directory from concurrent
untrusted mutation; these checks do not provide an ACL sandbox against an attacker
who can replace directory ancestors during a filesystem operation.

The qualification recipe defines compiler identity as SHA256 of the UTF8 text
`<nvrtc_sha256>\n<builtins_sha256>\n`, in that order. SDK identity is the corrected
DLL's SHA256. The probe loads compiler modules from the executable's pinned
runtime directory, checks actual loaded module paths and hashes, and verifies
both identities before rendering. Passing a hash string alone does not prove
module provenance: the future production bridge must perform its own verification.
Before SDK initialization it must preload both verified modules and retain their
handles through SDK teardown. The patched Windows CUEW loader takes its own
references to those already-loaded modules, requires NVRTC 12.8, and never searches
PATH or selects a different toolkit. Missing compiler/builtins fail CUDA setup.
The names/version checks are not hash attestation; canonical-path and hash checks
remain the caller's responsibility. A focused test covers ambient CUDA 13.1
simultaneously loaded alongside the pinned pair.

No public Turninator ABI structure or renderer configuration schema is changed
by this SDK patch. A bridge must carry its host-owned root into these properties.

## Cache format and failure behavior

The per-instance directory is
`<owned root>/turninator-luxcore-ptx-v1/<compiler SHA256>/<SDK SHA256>/`.
A SHA256 key frames source, program name, compute architecture and the complete
effective ordered NVRTC argument list with lengths, preventing concatenation
ambiguity. The fixed compiler options are generated by the same function used
for compilation and key generation. Logs record key, source SHA256, architecture
and effective options.

Each `.ptxrec` contains a 272-byte little-endian Windows header followed by PTX:
8-byte `TNPTX01\0` magic, four 64-byte lowercase hashes (compiler, SDK, key, PTX),
and a uint64 PTX byte count. Loads validate all fields, exact file size, digest,
and terminal NUL before returning a hit. PTX is limited to 64 MiB before cache
allocation/publication. Old four-byte-checksum cache files and foreign headers
are misses. A corrupt owned entry is replaced only after successful compilation.

One record contains metadata and PTX, eliminating sidecar races. Publication
writes a uniquely created temporary file in the owned namespace, flushes it,
then atomically renames it over the owned destination. Bounded retries handle
Windows rename sharing races. Persistent IO errors propagate explicitly;
temporary cleanup touches only the file this instance created. No global cache
is consulted, renamed or deleted. Optional PathOCL `.ck` readiness hints are
disabled; `HasCachedKernels` returns false while real validated PTX reuse remains
enabled. This may suppress an upstream readiness shortcut but does not disable
the CUDA PTX cache.

This correction qualifies the Windows CUDA/OptiX path only. It does not establish
owned-cache behavior for an OpenCL renderer. CUDA driver JIT and OptiX-managed
caches are separate from LuxCore's PTX records and remain driver-managed.

## Qualification

Prepare the direct consumer and pinned runtime using:

```powershell
pwsh -NoProfile -File native/patches/luxcore/qualify.ps1 `
  -BuildRoot artifacts/luxcore-sdk-rebuild `
  -QualificationRoot '<original SDK qualification directory>' -Mode prepare
```

This links exact public headers and only the corrected import library. Neither
`properties.cpp` nor `matrix4x4.cpp` is compiled into the consumer. It exercises
Properties parsing/get, private cache configuration roundtrip, unrelated context
field filtering, Matrix construction/operations, a DLL-thrown exception
caught by the consumer, and destruction. The runtime copies only hash-verified
dependencies and the corrected DLL; the original official LuxCore DLL is excluded.

After reserving the GPU, run modes `cold`, `warm`, `edits`, `cancel`, `bounded64`,
`edge-diag`, and `parallel` sequentially. The owned cache directory includes
`é-層`, exercising real UTF8 configuration. `edge-diag` retains expected exit 21
for upstream's known adjacent-pixel rounding anomaly. `bounded64` requires the
actual per-pixel minimum to reach 64 with AA9/nominal81 and logs min/max/total.
It does not claim exactly 64 or exactly 81 samples in every pixel.

Use the same `qualify.ps1` command with `-Mode cold` (then each subsequent mode).
Each run writes `render-<mode>.log` and validated `cache-<mode>.json` under the
build root. Compare cold/warm record hashes and modification timestamps to
confirm reuse. Successful rendering alone does not establish compiler provenance.

The GPU-free `qualification-build/Release/loader_tests.exe` takes a mode and
the absolute pinned runtime directory. Modes `missing`, `missing-builtins`,
`wrong-builtins`, `preloaded`, and `ambient` test loader behavior. The latter two
toolkit-comparison cases (`wrong-builtins` and `ambient`) also take the absolute
directory of an existing CUDA 13.1 compiler pair. Run this executable from its
build directory, where no compiler DLL is next to the executable, so missing
preload tests cannot accidentally find a compiler through application search.

The GPU-free `cache_tests.exe` accepts a new owned directory and optionally an
existing foreign PTX file to read. It covers independent compiler/SDK namespaces,
copied foreign headers, old 13.1-format PTX, corrupt/oversized entries, atomic
write failure and concurrent independent cache instances. A mutation test that
omits compiler-header validation must fail with `foreign compiler header accepted`.
On this managed Windows workspace, atomic rename tests require the native tool
execution permission; sandbox error 5 is not evidence that a cache record passed.

Every GPU mode validates actual emitted PTX headers as CUDA 12.8 as well as record
identity, size and digest; preloading a DLL alone cannot pass this qualification.
Cold means an absent current identity namespace; retained failed-run namespaces
remain separate evidence. Retain exact DLL/importlib/source/patch/module/test/log hashes with each run.
Clean native object teardown and bounded tile drain are functional observations,
not a leak-proof allocator audit or a worst-case cancellation guarantee. Full
production PBR/resin/layers, image conversion, denoising, canonical recipe inputs,
owner-thread cancellation and adapter integration remain Task 8 work.
