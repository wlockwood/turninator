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
