# Turninator Web: Browser-Based Plan

A **static web app** that renders everything **on the user's own machine**: no uploads, no server-side rendering, and it works offline once loaded. Feature goals match the desktop plan (`idea2-opus.md`):

1. **Looping turntable GIFs** (plus MP4/WebM/WebP)
2. **High-resolution stills**
3. **Four-view sheets**
4. **Printed-layer simulation**: real layer heights, stair-stepping, top-surface infill lines
5. **Inspection**: see-through and section plane with caps
6. **Later: G-code print preview**

Reference machine: RTX 4070 Ti / Ryzen 9 9950X / 96 GB, in desktop Chrome or Edge. Other browsers are best effort (§9).

---

## 1. Decision

**Rust → WebAssembly for all geometry, three.js for rendering, a TypeScript/Svelte UI, and in-browser encoders, all shipped as an installable PWA (an installable, offline-capable web app).**

| Layer | Choice | Why |
|---|---|---|
| UI | **TypeScript + Vite + Svelte 5** | A DOM UI beats a canvas-drawn UI in the browser: native file pickers, drag-drop, text input, accessibility. Svelte keeps it small and fast. |
| Geometry core | **Rust compiled to WASM** in Web Workers, multithreaded with `wasm-bindgen-rayon` | STL import, welding, normals, **the layer slicer**, G-code parsing and GIF quantization are CPU-heavy. This is the *same crate* the desktop plan needs, so the two versions share their most distinctive code. |
| Fast renderer ("Studio", the EEVEE analogue) | **three.js** raster: `MeshPhysicalMaterial`, PMREM environment lighting, soft shadows, GTAO (N8AO), SMAA/TAA | Interactive, temporally stable, renders GIF frames in milliseconds |
| Quality renderer ("Path traced", the Cycles analogue) | **`three-gpu-pathtracer`** on the same three.js scene, plus a denoiser | Mature progressive GPU path tracer for three.js scenes: physical materials, transmission, area lights, environment importance sampling. No second scene description needed. |
| Encoders | GIF via Rust/WASM (`gifski` or `gif` + quantizer, see the license note in §6), **WebCodecs** + **Mediabunny** for MP4/WebM, libwebp WASM for animated WebP | All local and fast. WebCodecs uses the GPU's hardware video encoder. |
| Storage | **OPFS** (Origin Private File System) for frame sequences and caches; IndexedDB for projects, presets and file handles | Long animations don't sit in RAM; projects survive reloads |
| Hosting | Any static host that can send COOP/COEP headers (Cloudflare Pages/Netlify), or GitHub Pages + `coi-serviceworker` | No backend at all |

### The key design move: the print look is plain PBR data

In the desktop plan the layer look was a custom shader formula (bead normal from `layer_t`). For the web, **bake it into standard assets** that any renderer understands:

- **Walls** get UVs: `u` = contour arc length in mm, `v` = position within the layer (0 → 1, from `layer_t`). A tiny tileable **bead normal map** (the rounded extrusion profile) is applied along `v`, with tangents along the contour. Ribbing then appears without extra triangles, and variable layer heights work automatically because each layer's wall strip spans its own `v` range of 0 → 1.
- **Top treads** get UVs from the XY position rotated ±45° per `layer_index`, scaled by the line width. An **infill-line normal map** repeats along them.
- **Bottom face**: a smooth-PEI gloss material, optionally textured.
- Separate material groups for wall, top and bottom, sharing color and finish.

Because it's just geometry + UVs + normal maps + `MeshPhysicalMaterial`:
- The raster viewport and the path tracer render it **identically by construction**, with no shader porting.
- The whole printed-look model can be **exported as a GLB** and opened in Blender, any viewer, or `<model-viewer>`.
- The desktop Blender path can consume the same data, which simplifies its node group.

---

## 2. What differs from the desktop plan

| Topic | Desktop (`idea2-opus.md`) | Web |
|---|---|---|
| Renderer | Blender Cycles/EEVEE in a worker process | three.js raster ("Studio") + three-gpu-pathtracer ("Path traced") |
| Hardware ray tracing | Yes (OptiX) | **No.** WebGPU/WebGL expose no RT cores, so BVH traversal runs in shaders. Expect several times slower path tracing than Cycles/OptiX (§8). |
| Install | Rust exe + Blender install | Open a URL; optionally "Install app" (PWA) and double-click STLs via the File Handling API |
| Threads | Native rayon | rayon over Web Workers, which needs **cross-origin isolation** (COOP/COEP headers) |
| Memory | 96 GB host, 12 GB VRAM | WASM heap (4 GB in wasm32), browser GPU process limits, WebGL context loss on OOM |
| Long jobs | Background worker, unattended | The tab must stay open. Use Screen Wake Lock, render in a worker with OffscreenCanvas, and keep checkpoints in OPFS. |
| Shared code | Rust `mesh`, `print`, `gcode`, GIF | **The same Rust crates**, compiled to WASM |

---

## 3. Architecture

```
┌───────────────────────────── Browser tab (PWA) ─────────────────────────────┐
│  Svelte UI ──► app state (SceneSpec, generation IDs) ◄── IndexedDB projects  │
│      │                                                                       │
│      ▼                                                                       │
│  Core Worker (Rust/WASM + rayon pool)                                        │
│    import STL ─► weld/normals/stats ─► LAYER SLICER ─► print mesh + UVs      │
│    G-code parser (later) · GIF quantize/encode · adaptive heights            │
│      │  transferable ArrayBuffers (zero-copy)                                │
│      ▼                                                                       │
│  Render (main thread, or a Render Worker with OffscreenCanvas)               │
│    three.js scene ─┬─ Studio raster (viewport, fast exports, diagram passes) │
│                    └─ three-gpu-pathtracer (progressive, quality exports)    │
│      │  frames                                                               │
│      ▼                                                                       │
│  Export Worker: OPFS frame store ─► GIF (WASM) / WebCodecs MP4/WebM /        │
│                 WebP / PNG16 / four-view compositor (OffscreenCanvas 2D)     │
│      ▼                                                                       │
│  Save: File System Access API (save picker) · fallback: download link        │
└──────────────────────────────────────────────────────────────────────────────┘
```

**Repository layout**

```
core/                         Rust workspace (shared with desktop)
  mesh/  print/  gcode/  encode/
  web/                        wasm-bindgen facade (thin API, typed arrays in/out)
web/
  src/ui/                     Svelte components (panels, swatches, dialogs)
  src/state/                  SceneSpec store, generation IDs, persistence
  src/render/
    scene.ts                  build three.js scene from SceneSpec + print mesh
    studio.ts                 raster pipeline (shadows, GTAO, AA, tone map)
    pathtrace.ts              three-gpu-pathtracer wrapper, denoise, tiling
    inspect/                  section plane + stencil caps, WBOIT see-through
    passes/                   normal/depth/ID passes, edge detection
  src/export/                 turntable, still, diagram, encoders, OPFS frames
  src/workers/                core.worker.ts, export.worker.ts, render.worker.ts
  public/                     HDRIs (small, CC0), normal-map tiles, fonts
```

**Color**: linear workflow, **AgX tone mapping** (three.js has `AgXToneMapping`), fixed exposure, sRGB output. The same in both renderers.

---

## 4. Printed-layer simulation (web specifics)

The logic is identical to desktop §3 (modes, height profiles, slicing algorithm, px/layer Auto mode). Web-specific notes:

- **Modes**:
  - **Off**
  - **Shaded**: original mesh; layer bump computed in a three.js `onBeforeCompile` / TSL shader chunk from object-space Z. Raster only; the path tracer falls back to Sliced or Off.
  - **Sliced**: real stepped geometry + bead/infill normal maps; both renderers
  - **Auto**: chooses by pixels per layer, as on desktop
- **Height profiles**: uniform (default 0.20 mm) + first layer, adaptive (cusp-height), and **from G-code** (read layer Zs from a dropped `.gcode`).
- **Slicer runs in the Core Worker** with rayon threads. Results go to the render thread as transferable typed arrays (positions, normals, tangents, UVs, groups). Use generation IDs so a stale slice never replaces a newer one.
- **Budget**: if the print mesh exceeds a triangle cap (start at ~6M, tune in Phase 0), the slicer simplifies contours harder, or Auto falls back to Shaded with a notice.
- **Normal-map tiles** are generated procedurally at startup (bead profile with roundness and contrast, infill lines) on a small canvas, so roundness and contrast sliders just regenerate a 256 px texture.
- **Anisotropic sheen**: `MeshPhysicalMaterial.anisotropy` with tangent direction along the layers in the raster renderer. Check whether three-gpu-pathtracer supports anisotropy (Phase 0 probe); if not, the path-traced version uses an isotropic sheen fallback.
- **Build vs display frame** stays separate, exactly as on desktop.

---

## 5. Rendering

### 5.1 Studio (raster)
- `MeshPhysicalMaterial` presets: Matte PLA (default), Silk PLA, Glossy PETG, Translucent PETG, Grey resin, Clear/tinted resin, Clay, Anodized metal. Resin and translucent use `transmission`/`thickness`/`attenuationColor`. These are approximations in raster, labelled as such.
- Lighting: **Light Studio** and **Dark Studio** built from a PMREM-processed environment (`RoomEnvironment` or a small CC0 studio HDRI) plus a key light with soft shadows (PCSS or VSM), on a sweep backdrop mesh with a shadow-catcher option.
- GTAO (N8AO) for contact shadows in layer grooves, SMAA or TAA (TAA gives very clean stills when the camera is static).
- Used for the interactive viewport, **fast GIF exports** and four-view panels.

### 5.2 Path traced
- `three-gpu-pathtracer` driven from the same scene. Progressive in the viewport when the camera is idle ("Path traced" toggle), and per-frame accumulation for exports.
- **Denoising**: evaluate in Phase 0:
  - (a) A community WebGPU/TF.js port of OIDN's network, if mature
  - (b) The path tracer's own denoise or blur pass
  - (c) Simply more samples, which is often enough at GIF resolution
- **GPU watchdog safety**: Windows resets the GPU if one draw call runs longer than ~2 s, which kills the WebGL context. Render in **tiles** with a bounded samples-per-tile budget, handle `webglcontextlost` by rebuilding and resuming from the OPFS checkpoint, and never issue giant single draws.
- Used for **quality GIFs, stills, and resin/translucent** anything.

### 5.3 Engine choice in the UI
Each export dialog has **Studio (fast)** vs **Path traced (quality)** with the last measured time per frame shown, mirroring desktop EEVEE vs Cycles. Agreement standard: geometry, camera, layers and base color match; highlights may differ.

---

## 6. Outputs

### 6.1 Turntable
- Defaults: 960², 4 s, 20 fps, 80 frames, 5 cs delays, infinite loop. Angles `360·i/N` with no duplicate endpoint. Rotational-envelope framing with 10% margin. Fixed lights.
- **Frame pipeline**:
  1. Render frame (Studio: ~ms; Path traced: accumulate to the target spp)
  2. Read pixels → transfer to the Export Worker
  3. Write the frame to OPFS (enables resume and re-encoding without re-rendering)
  4. Encode: **GIF** (WASM), **MP4/WebM** via WebCodecs `VideoEncoder` (hardware H.264/VP9/AV1) + Mediabunny muxing, **animated WebP** via libwebp WASM
- **GIF license note (important for a public website):** `gifski` is **AGPL-3.0**. Serving it in a public web app means the app must be distributed under AGPL-compatible terms (open source). Options:
  - Open-source Turninator Web under AGPL and use gifski (best quality)
  - Use permissive alternatives: the `gif` crate + `color_quant` (NeuQuant, MIT), or a custom median-cut with temporal-stable ordered dithering (more work)

  Pick in Phase 0 after comparing quality. `libimagequant` is GPL/commercial, so it has the same consideration.
- Show the encoded GIF playing with its file size; offer (never silently apply) smaller presets.
- **Screen Wake Lock** during exports; progress with a frame strip; cancel at any time; frames retained in OPFS.

### 6.2 Stills
- 2048² / 4096² default / 3840×2160 / custom. Above the WebGL max drawing-buffer size, or for watchdog safety, render in **tiles** and stitch in the Export Worker.
- PNG 8-bit via `OffscreenCanvas.convertToBlob`, or 16-bit via the WASM `png` crate. Optional transparent background (floor off). Optional 8-angle contact sheet.

### 6.3 Four-view sheet
- Same conventions as desktop: 2×2 **Front | Right / Top | Isometric**, all orthographic, one shared px/mm scale, lighting rotating with each view, white background, neutral material, layers off by default.
- **Edges come cheap in raster**: render normal, depth and object-ID targets, then run a fullscreen edge-detection pass to get crisp silhouettes and creases. Panels can use Studio (instant) or Path traced shading with the raster edge pass composited on top.
- Layout, labels and caption (file name, W×D×H mm, volume, estimated mass, layer profile) drawn with **Canvas 2D** in an OffscreenCanvas, using a bundled web font. Export PNG, plus **SVG** (raster panels embedded, vector labels and lines), which is easy in a browser.

### 6.4 GLB export (web bonus)
- Export the current model, original or **printed-look** (sliced geometry + normal maps + materials), as `.glb` via three.js `GLTFExporter`. Opens in Blender, slicer previews, `<model-viewer>`, etc.

---

## 7. Inspection (kept from desktop v2)

- **Section plane**: three.js `clippingPlanes` on X/Y/Z with a slider in mm and flip; **stencil-capped** cross-sections with the standard three.js stencil technique, plus optional hatching. Caps only on watertight meshes; otherwise uncapped with a warning.
- **See-through**: **weighted blended OIT** as a custom pass (WebGL2 MRT with float targets: accumulation + revealage, then composite). Opacity slider, edge overlay, back-face tint.
- **In exports**: Studio-renderer exports can include the section (clipping + caps work in raster). Path-traced sections need a real geometry cut: do the boolean on slice-friendly geometry in the Core Worker (clip mesh + cap polygon from the slicer's contour code), so the path tracer sees a closed, cut mesh.

---

## 8. Performance targets (estimates on the 4070 Ti in Chrome; verify in Phase 0)

| Scenario | Target |
|---|---|
| App cold load (cached PWA) | < 1.5 s to interactive |
| Import + weld + normals, 1M-triangle STL (WASM, threaded) | < 2 s |
| Slice, 60 mm part @ 0.2 mm | < 3 s |
| Studio viewport, 3M-triangle print mesh, 1440p | ~60 fps |
| Path-traced viewport, 1080p | Recognizable in < 1 s, clean in ~5–15 s |
| GIF, 960², 80 frames, **Studio** | < 20 s end to end |
| GIF, 960², 80 frames, **Path traced** | < 4 min (≈ 2–3 s/frame); exposed as "Quality" |
| 4096² still, Path traced | < 3 min |
| Four-view sheet, Studio panels | < 10 s |

Path tracing without RT cores is the web's main cost. If Phase 0 shows three-gpu-pathtracer far below these targets, Phase 10 (a WebGPU compute path tracer) moves earlier.

---

## 9. Browser support

- **Primary**: desktop Chrome and Edge (WebGL2, WebCodecs, File System Access, OPFS, File Handling for installed PWAs, cross-origin isolation, Wake Lock).
- **Supported**: Firefox and Safari. Graceful fallbacks: download links instead of save pickers, no file handling, and video encoding only where WebCodecs supports the codec.
- **Not targeted**: mobile/tablet (works for viewing; exports are best effort).
- Capability probe at startup lists what is available and shows only the features that work.

---

## 10. Phases

Rough sizes assume focused, agent-assisted work.

### Phase 0: Feasibility spikes (≈4–5 days)
- **(a) WASM core**: Rust STL import + weld + slicer compiled with `wasm-bindgen-rayon`, served with COOP/COEP. Measure threaded speed against native on 4–5 real STLs.
- **(b) Print look as PBR**: sliced mesh with wall UVs/tangents + bead normal map + top infill normal map in a three.js Studio scene. **Realism gate** against a photo of a real print.
- **(c) Path tracer**: the same scene in three-gpu-pathtracer. Measure spp/s at 960² and 4K; probe anisotropy, transmission/attenuation, normal-map support, and tiling behavior; evaluate denoiser options.
- **(d) Encoders**: 80 frames at 960² → GIF (gifski-wasm vs `gif` + NeuQuant: time, size, shimmer), WebCodecs MP4, animated WebP.
- **(e) Robustness**: forced context loss, a hidden tab during export, a 4 GB WASM heap stress test with a dense mesh.
- **Gates**: realism, slice time, path-traced s/frame, GIF quality and license decision, and whether the hidden-tab/context-loss recovery strategy works. Record results in `docs/web-performance.md` and amend this plan.

### Phase 1: App shell + Studio viewport (≈1 week)
- Vite + Svelte + three.js, PWA manifest + service worker (offline), capability probe.
- Open/drag-drop, Core Worker import with progress and generation IDs, stats panel (dims, triangles, shells, open edges, volume).
- Orbit/pan/zoom, fit/reset, views, ortho toggle, axis gizmo, turntable scrub, orientation buttons.
- Studio renderer, material presets + swatches, Light/Dark studios, AgX.

### Phase 2: Layer simulation (≈1.5 weeks)
- Sliced mode end to end, with UVs/tangents/groups and procedural normal-map tiles.
- Height profiles: uniform + first layer, adaptive, from G-code Z.
- Shaded mode (raster shader chunk), Auto mode with px/layer indicator, build vs display frame, triangle budget and fallbacks.

### Phase 3: Turntable export, Studio engine (≈1 week): **first truly useful release**
- Envelope framing, export dialog, Render Worker/OffscreenCanvas, OPFS frame store, Wake Lock.
- GIF/MP4/WebM/WebP encoders, playback + file size, save picker/download, cancel and resume.

### Phase 4: Path-traced mode (≈1–1.5 weeks)
- Progressive viewport toggle, tiled accumulation, denoiser, context-loss recovery from checkpoints.
- "Quality" engine for turntables with fixed vs varying seed stability tuning; resin defaults to Path traced.

### Phase 5: Stills (≈3–4 days)
- Tiled high-res rendering + stitching, PNG 8/16-bit, transparent background, contact sheet.

### Phase 6: Four-view sheet (≈1 week)
- Ortho cameras with shared scale, per-view lighting, raster edge pass, Canvas 2D layout/labels/caption, PNG + SVG export.

### Phase 7: Inspection (≈1 week)
- Section plane + stencil caps + hatching; WBOIT see-through with edge overlay and back-face tint.
- Sections in Studio exports; geometry-cut sections for Path-traced exports.

### Phase 8: Polish (ongoing)
- Projects in IndexedDB (with persisted File System Access handles so "reopen" works), export/import project JSON.
- PWA **File Handling** (double-click an STL to open in the installed app), user presets, GLB export, Print Bed studio, keyboard shortcuts.

### Phase 9: G-code print preview (later)
Same goals and sub-phases as desktop §10, adapted:
- **9a**: WASM G-code parser (streamed from `File.stream()` so large files never fully load) + an **instanced-segment** viewport preview (tube built in the vertex shader), colored by feature or layer.
- **9b**: tube meshes with feature culling (external perimeters, top/bottom, bridges, supports) for Studio and Path traced renders; check the triangle budget in WASM and GPU memory.
- **9c**: section/see-through with full infill, multi-material colors from tool changes, **build-up animation** GIF.
- **9d**: `.bgcode` and `.gcode.3mf` (zip in-browser via `fflate`), plus slicer-flavour coverage.

### Phase 10 (optional): WebGPU compute path tracer
- Only if Phase 0/4 measurements show three-gpu-pathtracer is too slow or too limited (anisotropy, huge G-code meshes).
- WGSL megakernel path tracer over a BVH built in the Rust core (compact wide-BVH layout), with the same scene and material data. WebGPU compute offers more control over dispatch sizing (watchdog), storage buffers for big meshes, and better occupancy.
- Watch for a standardized **WebGPU ray-tracing extension**. If browsers ship one, this phase gets dramatically faster and simpler.

---

## 11. Risks

| Risk | Mitigation |
|---|---|
| Path tracing without RT cores is too slow | Studio engine as the fast default; tune spp + denoise; Phase 10 WebGPU tracer; render-size presets |
| GPU watchdog / WebGL context loss on long renders | Tiled, bounded draws; OPFS checkpoints; automatic rebuild-and-resume |
| User switches tabs mid-export (throttling) | Render Worker with OffscreenCanvas, Wake Lock, a visible "keep this tab open" notice, resume from OPFS |
| WASM 4 GB heap limit (dense meshes, G-code) | Stream parsing, chunked GPU uploads, triangle budgets, feature culling; evaluate wasm memory64 if needed |
| COOP/COEP breaks hosting or embeds | Host on Cloudflare/Netlify with headers; `coi-serviceworker` fallback; single-threaded WASM fallback path (slower but works) |
| gifski AGPL vs a closed public site | Decide license in Phase 0; MIT encoder fallback ready |
| three-gpu-pathtracer feature gaps (anisotropy, volumes) | Phase 0 probe; per-engine fallbacks in presets; Phase 10 if blocking |
| Studio and Path traced disagree on layers | Print look is baked into geometry + normal maps, so both renderers consume identical data |
| Browser differences (Safari/Firefox) | Chromium primary; capability probe; graceful feature hiding |

---

## 12. First steps

1. Factor the Rust geometry code as a `core/` workspace from day one, so desktop and web share `mesh` and `print`. If the desktop plan hasn't started, this repo layout serves both.
2. Gather the same 4–5 real STLs, one real-print photo, and one sliced `.gcode`.
3. Run Phase 0 spikes (a)–(e) and record measurements before committing to Phase 1 UI work.
