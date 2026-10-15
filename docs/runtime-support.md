# Runtime support and pinned identities

Verified on 2026-10-14. Turninator uses explicitly configured local executables. It does not download, replace or automatically update Blender or FFmpeg. A newer version is a compatibility candidate until the same application/runtime checks have passed against its actual binaries.

## Tested configuration

| Component | Exact tested identity | Evidence |
|---|---|---|
| Blender | 4.5.2 LTS, build `ab25eae04993`, Windows x64 | Persistent worker, EEVEE, Cycles OptiX, PNG/GIF/MP4 jobs, sections, joined beads, material mappings, build-up, cancellation and replacement worker |
| Maintained Blender patch | 4.5.14 LTS, build `62c1db4208e8`, Windows x64, portable workspace copy | Actual persistent-worker probe, EEVEE and Cycles OptiX: each a 512 px PNG plus four 256 px GIF frames; full GIF decode verifies 50 ms/frame and infinite loop. The broader 4.5.2 acceptance suite has not been repeated on this patch. |
| FFmpeg and ffprobe | `2022-05-23-git-6076dbcb55-full_build-www.gyan.dev`, GCC 11.3.0 | Actual palette GIF, H.264 and libaom AV1 encode/decode checks, exact 20 fps timing, Unicode/percent paths, retained-frame reuse and output-preservation failures |
| GPU | NVIDIA GeForce RTX 4070 Ti, 12,282 MiB, driver 595.79 | Actual Cycles OptiX device selection and rendering; no silent CPU fallback |

The tested FFmpeg contains `libaom-av1` and lacks `av1_nvenc`. AV1 output therefore uses software encoding on this tested configuration. A configured build that advertises AV1 NVENC is selected explicitly; encoder initialization errors are reported rather than silently changing codecs.

Installed executable SHA-256 values, read from disk:

```text
blender.exe FE6665E08D21721832E7DDC03BED315A6CE137022E2FB27E92804B77CF43EEBB
ffmpeg.exe  38E2FAF8EF2874E34BC2CF49548F3E6E524ADA22E97869BC0C6B55875031DB41
ffprobe.exe F0FA56BAEEB5A65386BB4FB9BC87F304F5B996F6D1CC3CEE4DF16201C8B22172
```

Local paths are `C:/Program Files/Blender Foundation/Blender 4.5/blender.exe` and `C:/Program Files/ffmpeg/bin/{ffmpeg,ffprobe}.exe`. These are observed test-machine paths, not required installation locations. The application settings allow different executable paths.

## Maintained patch qualification and encoder candidate

Blender's official release/LTS listings identify **4.5.14 LTS**, dated September 15, 2026, as the current 4.5 maintenance patch. The 4.5 line receives two years of support into 2027. That exact version was downloaded from the official mirror, verified against its published SHA-256 manifest, extracted inside the workspace and qualified as described above. No installed executable or configured default was changed. [Blender releases](https://www.blender.org/releases/), [4.5 LTS release notes](https://www.blender.org/releases/4-5/), [official versioned archives](https://download.blender.org/release/Blender4.5/), [4.5 LTS announcement](https://www.blender.org/press/blender-4-5-lts-release/).

Pinned download: [blender-4.5.14-windows-x64.zip](https://mirror.blender.org/release/Blender4.5/blender-4.5.14-windows-x64.zip), checked against [official SHA-256 manifest](https://mirror.blender.org/release/Blender4.5/blender-4.5.14.sha256). Local copy: `artifacts/runtime-tools/blender-4.5.14-windows-x64/blender.exe`.

```text
archive SHA256 b9533d2397ac1984db4466fb23a7a4649391cca93f6e84209f9bcc60d071c8b9
exe SHA256     57fa1d294ea76448c3bec84ca758caf4629611330abba7dcf55bb0c56a0a15ab
build          62c1db4208e8 (2026-09-15 build, 2026-09-14 source commit)
```

Retained qualification evidence is `artifacts/verified-models/model-0/proof-eevee-478715a51367/metrics.json` and `proof-cycles-0c67c3ea2a1c/metrics.json`. Each records exact runtime/script/source hashes, device probe, individual requests, frame timing and decoded GIF facts. Total job times were 15.446 s and 3.191 s respectively; these small jobs with different shader cache states are compatibility checks, not comparative benchmarks. The Cycles still was visually inspected. The maintained patch can be selected explicitly in application tool settings; the portable app does not bundle this runtime.

The official FFmpeg site lists **9.0.2**, released September 18, 2026. FFmpeg distributes source and links third-party Windows builds. The selected Windows qualification candidate is **Gyan 9.0.2 full static x64**, release tag `9.0.2`, upstream source `946fcce07b`; the publisher lists that release and checksums separately from moving development builds. This candidate is **not** the installed/tested 2022 executable. [FFmpeg downloads and release verification](https://ffmpeg.org/download.html), [Gyan release build page](https://www.gyan.dev/ffmpeg/builds/), [versioned Gyan 9.0.2 release](https://github.com/GyanD/codexffmpeg/releases/tag/9.0.2).

For broader qualification of a replacement runtime, record the versioned archive, publisher checksum/signature verification and executable hashes. Beyond the completed maintained-patch probe/PNG/GIF checks, repeat rectangular four-view composition, H.264/AV1 codec/duration checks, section/build-up, cancellation/restart and packaged resource-discovery checks. Existing extensive artifact evidence applies to the pinned 4.5.2 configuration; only the explicitly listed small jobs establish 4.5.14 compatibility so far.

## Runtime resources and capability limits

The native worker resolves `renderer/worker.py`, `renderer/scene.py` and `renderer/frame_pipeline.py` beside the running executable first; source-tree fallback supports development. Worker launches disable Blender auto-execution and use an application-owned hidden Job Object before the child resumes. Render fingerprints include the selected Blender executable and all three renderer script hashes. Changing a runtime creates a different render identity.

Fresh static turntables use one Blender animation for their unique poses. Build-up, camera timelines and retained-frame retries update and render individual frames. Both paths overlap GPU rendering with bounded CPU PNG validation and atomic publication. Worker callbacks use file bytes and protocol events; Blender scene operations remain on its main thread. Progress stays ordered, duplicates wait for their source to be published, and a failed validation prevents successful completion. Full PNG chunk/CRC/zlib/scanline checks use bounded streaming allocations. Unchanged native viewport frames reuse their existing texture during UI progress repaints. See [measured performance](performance.md#render-pipeline-follow-up).

GIF and MP4 exports support integer framerates from 1 to 100 fps, defaulting to 20 for new and legacy projects. Changing FPS updates frame counts without changing the chosen rotation period or clip length, apart from rounding to whole output frames. Jobs remain limited to 10,000 frames. GIF delays round absolute frame boundaries to centiseconds to avoid cumulative duration drift; some players slow very short delays, so MP4 is preferable for dependable high-rate playback. MP4 validation checks actual encoded FPS as well as count and duration. PNG and sheet outputs hide the framerate control while retaining its saved preference.

The EEVEE studio floor uses a coplanar grid to prevent horizontal shadow bands from an oversized single polygon. The image regression is verified on both Blender versions above. From the source checkout, run `blender --background --factory-startup --python-exit-code 1 --python tests/blender_floor.py -- --output artifacts/eevee-floor-check` with a fresh output directory. It generates a reproducing cylinder, compares the old and current floor, and checks three turntable angles plus transparent/diagram floor omission. Restart Turninator after replacing renderer scripts so its persistent worker loads the update.

EEVEE and Cycles are explicit choices. Cycles currently requires an available OptiX device; other GPU backends and CPU rendering are outside this verified configuration. Image transparency is separate from resin transmission. Resin on bead meshes retains internal and overlapping interfaces and is labeled as an illustration, not a fused-resin optical simulation. Section statistics are mesh geometry measurements, not measured print strength or calibrated material volume. No photograph comparison has been performed.

Third-party binaries are configured separately. Packaging/license notices are maintained with the application distribution; this support table does not establish redistribution rights or change upstream terms.
