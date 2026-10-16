# Initial measured performance

This report preserves the first renderer proof timings. Later measurements and ownership checks are in [progress.md](progress.md), [native viewport evidence](../reviews/viewport-implementation.md) and [timeline/export evidence](../reviews/timeline-implementation.md). The later desktop uses suspended process creation, Job assignment before resume and owned encoder cleanup; the missing launcher described at the end of this initial report has been implemented and verified.

Measured October 14, 2026 on NVIDIA RTX4070Ti (12282MiB, driver595.79), Ryzen9950X, Blender4.5.2 LTS (`ab25eae04993`), Rust1.95.0, and the installed FFmpeg2022-05-23 build. Cycles device selection explicitly enabled OptiX and disabled CPU/CUDA alternatives. Peak host/GPU memory and other desktop GPU load were not instrumented, so these are single observed runs rather than a statistical benchmark.

| Actual job | Still request | Animation request | Median animation frame | Encoding | End-to-end proof |
| --- | --- | --- | --- | --- | --- |
| Synthetic bead mesh, EEVEE32 samples,4096 still plus80x960 GIF | 21.53s | 26.39s | 0.255s | 1.89s | 51.60s |
| User spiral egg, Cycles/OptiX64 samples,4096 still plus80x960 GIF | 28.45s | 107.73s | 1.326s | 1.97s | 139.96s |
| Synthetic bead mesh, Cycles/OptiX16 samples,512 still plus4x256 GIF | 6.16s | 6.41s | 0.182s | 0.14s | 13.41s |

Request timings include scene rebuilding, rendering, writing and worker frame checks; end-to-end includes worker startup/probe and Pillow decode/encoding/validation. The full EEVEE bead GIF is33,064,535 bytes; the egg GIF is20,269,510 bytes. These large sizes motivate later palette/backdrop/detail tuning. No silent size reduction was applied.

The first two rows use different geometry/engines/quality settings and are **not** an engine speed comparison. The small Cycles proof confirms the same bead mesh works under OptiX; a matched full-size engine experiment remains future work. Completed proof metrics are preserved rather than overwritten by warm frame-reuse runs.

Mesh sizes:

- Synthetic beads:15,360 depositing segments,829,440 vertices,1,597,440 triangles; physical height24 mm,120x0.20 mm layers.
- Spiral egg:53,987 vertices,107,970 triangles; imported bounds39.4307x39.4310x46.5433 mm.

All nine supplied STLs were prepared successfully after exact-zero omission handling was added. Release CLI process/import/serialization times were approximately0.01–0.04s per file in this warm local run. The largest input was the5.4 MB spiral egg. This does not measure GUI latency or dense-million-triangle throughput. All source SHA256 hashes remained unchanged; [per-file results](../artifacts/test-model-results.json) record dimensions, counts, warnings and timing.

## Evidence

- [EEVEE full bead metrics](../artifacts/bead-demo-v2/proof-eevee-74150454cd15/metrics.json)
- [Cycles full spiral egg metrics](../artifacts/spiral-egg-layers/proof-cycles-40bba5171218/metrics.json)
- [Cycles small bead metrics](../artifacts/bead-demo-v2/proof-cycles-15f3e9512089/metrics.json)
- [Live cancellation/restart/recovery results](../artifacts/bead-demo-v2/runtime-ffa4efcfd8/result.json)

Encode-only retry was also run on the complete EEVEE80-frame sequence without launching Blender:80 frames at50ms,960x960,infinite loop revalidated, about1.54s encoding. The frame manifest was matched against geometry, scene, engine, samples, angles and renderer-script identities before publication.

Persistent requests work, but no geometry cache is implemented yet. Scene preparation is repeated per request. Cancellation was exercised during preparation, not after a confirmed GPU sample had begun; a separate idle Blender sentinel remained alive and responded, a replacement rendered successfully, a corrupted PNG was rerendered, and a matching valid PNG was reused.

The Windows proof controller assigns a kill-on-close Job Object immediately after process creation. A suspended-create/assign/resume launcher is still required before claiming the later desktop guarantee of ownership from the first instruction. FFmpeg direct-process timeout handling is present; broader native encoder lifetime ownership follows desktop integration.

## Render pipeline follow-up

The slowdown report exposed avoidable serial work, plus differences between the renderer selected in the application and an Eevee comparison. The inspected Pi Zero resin job used Cycles/OptiX, 32 samples at 960x960: 80 unique renders plus 80 copied repeats took 289.41 seconds, including 0.17 seconds of scene preparation. Two preceding matte axolotl Eevee jobs at the same resolution, samples and frame counts took 27.35/28.86 seconds. These different models/materials are diagnostic evidence, not an engine benchmark. Selecting Resin initially selects Cycles; export progress now explicitly names the active engine.

Previously each unique frame called Blender's still-render operator and then loaded the PNG into Blender before advancing. A fresh basic turntable now submits its unique prefix to one native animation render. A bounded CPU thread pool validates complete PNG bytes and atomically publishes each frame while the next GPU render proceeds. Build-up, moving cameras and recovery use individual render calls with the same CPU pipeline. Blender scene access remains on its main thread. Repeated poses still copy verified source frames in sequence. An unchanged native viewport now reuses its existing GPU texture instead of drawing the model again for every UI progress repaint.

Matched warm runs use the same 58,308-triangle axolotl scene, 960x960, Eevee at 32 samples and 24 unique angles. The latest serial still loop took 6.216 seconds; the actual animation/pipeline helper took 5.546 seconds, about 10.8% less elapsed time. CPU process time fell from 13.58 to 10.58 seconds. All 24 resulting frames have identical decoded RGB pixels to their serial references. Its 24 PNG validations used 0.404 seconds of summed task wall time, with only 0.016 seconds left to drain after rendering. Peak in-flight validation was one: the render stage produced frames more slowly than validation consumed them. Increasing CPU occupancy would add no useful throughput in that run. `frame_pipeline` statistics in each completed render distinguish worker limits, peak outstanding work, producer backpressure, final drain and validation work; their summed work time is not process CPU time.

This is a single observed comparison, not a guaranteed speedup. Earlier matched runs varied from roughly 3% to 13%; a heavily loaded run was 17.00 seconds serial versus 16.23 seconds pipelined. Aggregate GPU utilization remained near 99% even with no Blender process running during one observation, so other GPU activity makes timings variable. The latest first warm-up render took 25.62 seconds, versus 0.19 seconds after rebuilding the scene in the same process. Initial GPU/shader setup remains a substantial latency; preparation itself was 0.26–0.31 seconds. A thread pool does not eliminate GPU rendering, shader startup, Blender's main-thread scene updates or final encoding. FFmpeg still validates/encodes after the full frame sequence is ready; streaming GIF palette generation is outside this change.

Reproduce the comparison inside the configured Blender, writing to a fresh directory:

```powershell
& 'C:/Program Files/Blender Foundation/Blender 4.5/blender.exe' -b --factory-startup --python-exit-code 1 --python tools/benchmark-render.py -- --scene artifacts/render-performance/axolotl.scene.json --output artifacts/render-performance/my-fresh-run --frames 24 --cases still-loop worker-animation
```

Use any prepared scene JSON in place of the local axolotl snapshot. The tool records input/script hashes, settings, warm-up, process CPU time and retained PNGs. Evidence: [final comparison](../artifacts/render-performance/axolotl-pipeline-final/results.json), [initial native-animation comparison](../artifacts/render-performance/axolotl-baseline/results.json), [busy comparison](../artifacts/render-performance/axolotl-pipeline/results.json). The serial cases in the final tool use the new streaming PNG validator; the initial baseline used Blender image loading, so those independent runs are not a pure thread-pool comparison.


## Five-renderer native desktop qualification

The completed 40-case synthetic matrix, exact profiles, timings, hashes and
measurement limits are in [the renderer qualification report](renderers/qualification-matrix.md#observed-performance).
Native wgpu and Filament complete the 80-frame 960-square jobs in roughly 3–7 s
at their declared Medium profiles. LuxCore is a functional in-process path
tracer, but this adapter's observed ray-tracing cost exceeds Cycles on these
fixtures. Renderer settings and noise are not equivalent quality measures.
GPU jobs were serialized; some CPU verification/package work overlapped LuxCore.
Detailed cold/startup, warm, transfer, validation, encoding and whole-process
observations remain in the JSON/CSV evidence. Do not reuse older debug-build
numbers as production throughput.
