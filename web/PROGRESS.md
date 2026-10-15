# Turninator Web: Progress Notes (handoff document)

Plan of record: [`../idea-web-opus.md`](../idea-web-opus.md) (phases 0–10).
Shared conventions come from [`../final-plan.md`](../final-plan.md): physical mm, Z-up build frame (min Z = 0, XY centered),
print pose vs display pose, GIF defaults (960², 4 s, 20 fps, 80 frames, 5 cs, infinite loop), `360·i/N` angles without a
duplicate endpoint, four-view camera conventions, layer schedule limits (0.02–2.00 mm, ≤ 50,000 layers), bead model
(stadium section, bead top at nozzle Z, retraction debt).

**Scope rule:** this agent works only inside `web/`. Requests for work outside `web/` go to `../todo.md`, marked as from the web agent.
`web/` is **not committed** to git yet. Nothing outside `web/` was modified; `../todo.md` has no requests from this agent.

---

## HANDOFF CHECKLIST (read this first)

`[x]` done and verified in Edge on the RTX 4070 Ti · `[~]` partly done · `[ ]` not started.

### Done
**Core and viewport**
- [x] Vite + Svelte 5 + TS + three.js app; drag-drop/open; geometry worker (`src/core/*`, `src/workers/*`); 71 unit tests (`npm test`)
- [x] STL import (robust binary/ASCII detection, rebasing), exact welding, topology stats, crease normals
- [x] Studio raster viewport: orbit, views, turntable scrub/play, print vs display pose, 8 material presets, Light/Dark studios, AgX,
      procedural softbox HDR environment (shared with the path tracer), VSM soft shadows, GTAO ambient occlusion (toggle)
- [x] **Lay flat on face**: click-pick (print or display pose) or Auto (largest convex-boundary face) — `src/render/orient.ts`

**Printed layers**
- [x] **Shaded** mode (shader ribs along build Z, moiré fade by px/layer) and **Sliced** mode (real stepped geometry, `src/core/slicer.ts`)
- [x] Top-surface infill ridges (±45° alternating per layer, line width)
- [x] **Variable heights**: Uniform / Adaptive (slicer-style cusp criterion) / From a G-code file. Shader binary-searches a boundary texture; slicer takes explicit boundaries

**G-code**
- [x] Interpreter (`src/core/gcode.ts`): G0–G3, modes, G92, units, tools, retraction debt, ;TYPE/;FEATURE/;WIDTH/;HEIGHT, width from volume, feature-name normalisation (Prusa/Orca/Bambu/Cura)
- [x] Containers: `.bgcode` (CRC32, deflate, heatshrink, MeatPack — verified identical to PrusaSlicer's ASCII export) and `.gcode.3mf` (zip → first plate)
- [x] Instanced bead rendering (`src/render/beads.ts`): colour by material/feature/tool/layer, feature visibility, section clipping, shadows; part bounds exclude skirt/brim/purge
- [x] Path-traced G-code: worker bakes beads into 96 layer chunks (`src/core/beadMesh.ts`); interior culled when not visible

**Outputs**
- [x] Turntable: GIF (global palette + Bayer dither) + MP4 (H.264/AV1) + WebM (VP9/AV1); Motion = Rotate / Build-up / Build-up + rotate / Camera path / Camera path + build-up; Layout = Single / Side-by-side whole|section
- [x] **Camera keyframe paths** with per-transition seconds, extra turns (spirals), easing, hold; viewport preview (`src/render/camPath.ts`)
- [x] Stills (tiled, any size, transparent), four-view sheets (STL and G-code)
- [x] **Build-up**: STL (raster: capped clip plane; path traced: real cut + cap, `src/core/cut.ts`) and G-code (bead playback; path traced: chunk reveal)
- [x] OPFS frame store for big exports (`src/export/frameStore.ts`), Screen Wake Lock, Save as… (File System Access), large-GIF hint

**Path tracing (WebGPU, `three-gpu-pathtracer/webgpu`)**
- [x] Stills + turntables with OIDN denoise (`public/oidn/rt_hdr_calb_cnrm_small.tza`, Apache-2.0); adapter buffer limits requested; GPU errors and stalls surfaced
- [x] **Raytrace viewport preview** toggle (`src/render/ptPreview.ts`)

**App**
- [x] Inspection: section plane with stencil caps, see-through X-ray (WBOIT) + feature edges
- [x] G-code inspection: section caps per bead (stencil passes run the bead shader) and density X-ray (per-surface opacity ×0.08, little rim)
- [x] Projects (`.turninator.json`) + remembered settings (`src/state/project.ts`)
- [x] PWA: manifest with File Handling, offline service worker (prod), relative base

- [x] **Export frame in the viewport**: the active export's aspect is drawn as a frame (outside dimmed, size label). `Studio.exportCamera()` = viewport camera with FOV cropped to that centred frame; all exports (stills, turntables, paths, path tracing) use it, so renders match the frame exactly (verified by crop comparison, `tools/framecheck.mjs`). Fit/views fit into the frame; turntable "Frame" button fits the full rotation at the chosen camera height (exports no longer re-frame silently).
- [x] Artifact hardening after a user report of flickering black blocks (not reproduced locally): shadows back to PCF (VSM variance maths can emit NaN/black blotches), GTAO radius capped (`min(s·0.12, 8 mm)`, thickness ≤ 1.5 mm, stronger denoise), layer shader never emits NaN normals. If it recurs, toggle Studio → Ambient occlusion to isolate.

- [x] **Sliced gaps fixed**: wall strips merge only when a layer matches the strip's *first* region (≤ 1e-6 mm²), so drift can't accumulate into walls that miss their treads; treads exist only where neighbouring layers differ and are grown 2 µm under the adjoining walls to seal T-junction cracks (sparkle dots on translucent prints gone). Egg slice ~1.6 s.
- [~] User-reported motion flicker with clear/translucent PETG: not reproduced (tiny-orbit frame-to-frame change ≈ 0.04 either way). Mitigations: AO disabled for transmissive materials, orbit damping 0.18 (settles ~3× faster). Open question for the user: was the Raytrace preview on (it restarts noisy on every move by design)?

- [x] **Path tracer runs in a worker** (`src/workers/pathtrace.worker.ts`, protocol `src/render/pathtraceProtocol.ts`, UI proxy `src/render/pathtraceProxy.ts`): WebGPU device, BVH builds, OIDN and progressive sampling are off the UI thread; geometry is sent once per content key and cached in the worker; preview frames stream back as ImageBitmaps into a `bitmaprenderer` canvas; the worker waits for GPU completion between sample batches so the WebGL viewport keeps GPU time. Raytrace preview: worst UI frame 17 ms, 0 long tasks while tracing and orbiting (`tools/ptresponsive.mjs`). `three/webgpu` is no longer in the main bundle.

### Next (priority order)
- [ ] Translucent G-code path tracing looks frosted, not clear (see limitation below) — fuse beads per layer or shell-only transmission
- [~] Shaded layer ribs in the path tracer: `src/render/layerTexturing.ts` bakes per-corner UVs/tangents + a 2-row normal-map strip (verified data), behind `PT_BAKED_RIBS = false` in `studio.ts`. **Blocked:** three-gpu-pathtracer 0.0.27 (WebGPU) barely applies it — a constant ~90° tilt changed brightness < 3%, and 3× strength via `normalScale` or texture had no effect. Investigate `AtlasTexture` / normal-map sampling in `nodes/material.wgsl.js`, or fall back to Sliced mode (real geometry) for path-traced layers.
- [ ] Auto layer mode (choose Shaded/Sliced by px/layer)
- [ ] Animated WebP (libwebp WASM) — MP4/WebM AV1 currently cover small full-colour output
- [ ] Optional: swap the TypeScript core for the shared Rust crate via WASM (needs a wasm facade outside `web/`; file a request in `../todo.md`)

### Known limitation: translucent G-code in the path tracer
Beads are thousands of overlapping closed volumes. True transmission loses light at every interface (Fresnel/TIR, bounce
budget) and renders near-black even with bead IOR ≈ 1.06. Current approach: transmissive materials make beads stochastic
alpha (`opacity = 0.45 - 0.33·clarity`, `maxTransparentBounces = 160`) — a frosted "translucent FDM" look, ~3× slower.
Lower opacity (0.03) turned foggy under the denoiser.

### Gotchas
- The user may be using `vite preview` on **http://localhost:4173**, which serves `web/dist`. **Run `npx vite build` after changes** or that tab shows a stale build (this happened: the Raytrace button and frame looked "missing").
- Dev server: `npx vite --port 5199 --strictPort` (tools assume 5199). If the port is stuck, kill the old `node … vite.js --port 5199`.
- Lazily imported deps must be listed in `optimizeDeps.include` (`vite.config.ts`), else Vite full-reloads on first use and breaks Playwright runs.
- Playwright tools use `channel: 'msedge'` + `--use-angle=d3d11` for the real GPU. Download results as base64 (number arrays OOM Node).
- Path-traced 960² exports keep the page busy; Playwright polling can look hung. Log to a file and poll the file.
- G-code fixtures in `test-results/gcode/` come from `prusa-slicer-console.exe --export-gcode [--binary-gcode] …` (PrusaSlicer 2.9.6 at `C:\Program Files\Prusa3D\PrusaSlicer`); gitignored; tests skip if missing.
- Dev builds expose `window.__turninator` (app state) for debugging and tests.
- Avoid writing `'\n'` into TS through Python heredocs: it became a literal newline twice. Use the Edit tool for such lines.

---

## How to run

```bash
cd web
npm install
npx vite --port 5199 --strictPort   # dev server
npm test                            # vitest (71 tests)
npm run check                       # svelte-check
npm run build                       # production build to web/dist
```

Browser tools (Playwright + Edge, outputs in `test-results/`, gitignored):

| Tool | Checks |
|---|---|
| `tools/e2e.mjs [stl] [--only turntable,still,fourview]` | All Studio outputs |
| `tools/smoke.mjs`, `tools/closeup.mjs`, `tools/sliced.mjs`, `tools/inspect.mjs`, `tools/layflat.mjs` | Viewport features |
| `tools/gcode.mjs [file] [tag] [--export]` | G-code preview + build-up GIF |
| `tools/pathtrace.mjs`, `tools/pt-turntable.mjs`, `tools/pt-gcode.mjs`, `tools/ptpreview.mjs` | Path tracing |
| `tools/campath.mjs`, `tools/sidebyside.mjs`, `tools/adaptive.mjs`, `tools/fourview-gcode.mjs` | Newer features |

## Architecture

| Path | Role |
|---|---|
| `src/core/stl.ts`, `mesh.ts`, `pipeline.ts` | Import, welding, topology, normals |
| `src/core/layers.ts` | Schedules: uniform, explicit, adaptive, from G-code tops |
| `src/core/slicer.ts` | Stepped-envelope slicer (contours → Clipper → walls/treads) |
| `src/core/cut.ts` | Cut + cap at a Z plane (path-traced STL build-up) |
| `src/core/gcode.ts`, `bgcode.ts`, `zip.ts` | G-code interpreter and containers |
| `src/core/beadMesh.ts` | Bake beads to indexed meshes (path tracer) |
| `src/workers/core.worker.ts`, `protocol.ts` | Geometry worker + promise client (request ids, progress) |
| `src/workers/export.worker.ts`, `src/export/*` | Encoders (GIF, video), frame store, turntable/still/four-view orchestration |
| `src/render/studio.ts` | Raster Studio: hierarchy `turntable → display → mesh(print matrix)`, softbox env, AO, shadows, tiled `renderPixels`, beads, build-up, four-view panels, change hooks |
| `src/render/layerShader.ts`, `beads.ts`, `inspect.ts` | Layer shading, bead instancing, section/see-through |
| `src/render/pathtrace.ts`, `pathtraceHost.ts`, `ptPreview.ts`, `studioEnv.ts` | WebGPU path tracer, lazy host, viewport preview, procedural HDR |
| `src/render/framing.ts`, `camPath.ts`, `orient.ts` | Camera fitting, keyframe paths, lay-flat |
| `src/state/*` | Settings/presets, app state (Svelte 5 runes), projects |
| `src/ui/*` | Sidebar, Viewport, Export panel, Four-view panel |

## Measurements (RTX 4070 Ti, Edge headless, ANGLE D3D11 / WebGPU)

| What | Result |
|---|---|
| Import spiral egg STL (108k tris) | ~90–150 ms |
| Slice egg at 0.2 mm / adaptive | 233 layers, 732k tris, 0.5–0.7 s / 580 layers, 1.8M tris, 1.7 s |
| Parse egg G-code (132k beads) / bgcode ring (14k) | ~140 ms / ~45 ms |
| Studio GIF 960², 80 frames (AO + soft shadows) + MP4 | 7.8 s total (6.4 s render) |
| Studio GIF 1280², 240 frames (OPFS) + MP4 | 37 s |
| 4096² Studio still | ~1 s; four-view 4096² ~2.4 s |
| Path-traced still 2048²: STL 32 spp + OIDN / STL 128 spp | 6.2 s / 18 s |
| Path-traced G-code still 2048², 64 spp (6.35M tris) | 60 s (opaque), ~180 s (translucent) |
| Path-traced turntable 480², 60 frames, 16–32 spp | ~25–60 s; 960²/32 spp est. ~10 min |
| Raytrace preview to 64 spp (STL / G-code) | ~10 s / ~12 s |
| Encoders available | mp4-h264, mp4-av1, webm-vp9, webm-av1 |
| Temporal stability, PT turntable 480², 16 spp (mean abs frame-to-frame change on the static floor) | OIDN on 0.13 vs off 0.75 (denoise is ~6× steadier; keep it on for animations) |

## Decisions and notes
- **TypeScript core** instead of Rust/WASM (desktop crate has no wasm facade); swap behind `CoreClient` later.
- **GIF:** `gifenc` (MIT) + own LUT/dither — avoids gifski's AGPL for a public site.
- `clipper-lib` (Clipper 1, Boost licence) instead of `clipper2-js` (pulled Angular with advisories).
- Browsers clamp canvas drawing buffers: exports tile at ≤ 4096 supersampled px via `camera.setViewOffset`.
- Layer fade thresholds scale with supersample, or sub-2 px lines alias into moiré.
- Section default side keeps the half whose cut faces the default cameras.
- Slicer/cut chaining tolerates non-manifold keys (touching shells share edges).
- Four-view edges: normal `1-dot > 0.12` (disabled for beads), depth second difference `> 8 × mm/px`.
- Path tracer requests the adapter's max storage-buffer binding size (128 MB default broke large bakes).
- Baking for path tracing must not touch `app.exporting` (it paused the raytrace preview forever).
