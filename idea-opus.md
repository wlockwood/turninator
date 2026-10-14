# Turninator: Implementation Plan

A local desktop app that loads 3D-print models (STL first), applies good-looking print-material presets, and produces:

1. **Looping turntable animations** (GIF first, plus WebP/MP4)
2. **High-resolution stills**
3. **Four-view diagrams** (front / side / top / 3/4 view) with dimensions

Target hardware is an **NVIDIA RTX 4070 Ti** (Ada, hardware RT cores, 12 GB VRAM).

---

## 1. Recommendation summary

**Build one native Rust app around a single GPU path tracer.** Use **wgpu hardware ray queries** for tracing and **Intel Open Image Denoise (OIDN, CUDA device)** for denoising. The same renderer drives both the interactive viewport and the final output, so what you see while editing is what gets exported.

Why this instead of driving Blender headless:

| | Native wgpu path tracer | Blender (Cycles/EEVEE) subprocess |
|---|---|---|
| Interactive preview | Yes, progressive and denoised, same as final | No, render and wait (or a second preview renderer that never quite matches) |
| Install | Single folder: exe + OIDN DLLs | Requires Blender (~1 GB) and version pinning |
| Effort to first nice image | ~1–2 weeks | ~2–3 days |
| Control over print-specific looks | Full: layer lines, real-mm scale, diagram AOVs | Possible via node graphs, but awkward |
| Quality ceiling | Very high for one object in a studio | Highest |

The scene is always **one mesh, a backdrop, and a studio light rig**, which is the easiest case for a path tracer. There's no texture streaming, no instancing, and no thousand-material scenes. On a 4070 Ti, a compact megakernel path tracer will be well within the performance budget (see §8). Blender's advantage, years of shading features, mostly goes unused here.

**Fallbacks are built into the plan** (§11). If wgpu ray queries turn out to be immature, swap in a software BVH traversal in a compute shader. For one mesh that's still fast enough. Blender stays available as an optional "Export to Blender" escape hatch, not as the core dependency.

---

## 2. Key insights that shape the design

1. **Print models are in millimetres and usually watertight.** This makes possible:
   - **Real-scale layer lines.** A procedural bump on the *build axis* with a real layer height (0.2 mm FDM, 0.05 mm resin) looks correct at any zoom.
   - **Correct translucent resin.** Closed meshes allow proper volumetric absorption (Beer–Lambert) and scattering inside the part.
   - **Real numbers for the diagram**: bounding-box dimensions, volume, and estimated mass.
2. **STLs are already oriented for printing.** Keep the **build orientation** (STL Z-up, used for layer lines and the "on the bed" pose) separate from the **display rotation**. Layer lines stay physically correct even if the user flips the model for a nicer shot.
3. **GIFs are small.** A typical sharing GIF is 480–800 px. At that resolution even 512 spp per frame is cheap, so turntables can be very clean and temporally stable.
4. **Turntables and diagrams need opposite lighting rules.**
   - Turntable: **the object rotates; camera and lights stay fixed**, so reflections slide realistically across the surface.
   - Four-view: **the light rig rotates with each camera**, so every view is well lit and none of them is shot into the back light.
5. **STL has no smooth normals or UVs.** The mesh pipeline must weld vertices and compute crease-aware normals, and every material must work without UVs (procedural, object-space, or triplanar).

---

## 3. Architecture

```
turninator/                      (cargo workspace)
├── crates/
│   ├── tn-mesh/      STL/3MF/OBJ import, welding, crease normals, stats, validation
│   ├── tn-scene/     Scene description (serde): mesh ref, material, lights, backdrop, camera, output
│   ├── tn-render/    wgpu path tracer: accel structures, integrator WGSL, accumulation, AOVs
│   ├── tn-denoise/   OIDN wrapper (CUDA device, CPU fallback)
│   ├── tn-output/    Tone mapping finalize, GIF/WebP/MP4/PNG/EXR writers, diagram compositor
│   ├── tn-app/       eframe/egui GUI (binary: turninator.exe)
│   └── tn-cli/       Headless batch renderer (binary: turninator-cli.exe)
├── assets/
│   ├── hdri/         A few CC0 studio HDRIs (e.g. from Poly Haven), 2K
│   ├── presets/      materials.ron, rigs.ron, output.ron
│   └── fonts/        Font for diagram labels (OFL-licensed, e.g. Inter / JetBrains Mono)
└── idea-opus.md
```

**Data flow**

```
STL ──► tn-mesh (weld, normals, stats) ──► GPU buffers + BLAS
                                                   │
Scene (material, rig, camera, backdrop) ──► tn-render ──► accum (RGB32F) + AOVs (albedo, normal, depth, objID)
                                                   │
                                     tn-denoise (OIDN) ──► tone map (AgX) ──► viewport texture / tn-output
```

**One GPU device shared by GUI and renderer.** eframe is created with its wgpu backend, and the device is requested with the ray-query features enabled (eframe exposes the device descriptor through its wgpu configuration). The path tracer writes into a texture that egui shows directly, so the viewport has no CPU round-trip. Only the denoiser input is read back.

**Scene description is plain serde data** (RON or TOML). The GUI and CLI both produce a `Scene`. Presets are just partial scenes. Saving a "project" means saving the scene file plus the mesh path, which gives reproducible renders and easy batch jobs.

---

## 4. Mesh pipeline (`tn-mesh`)

1. **Import**
   - STL binary and ASCII: hand-rolled parser, memory-mapped, with rayon for the weld. It's tiny and faster than generic crates.
   - 3MF (phase 6): the modern slicer format, which may carry multiple objects and colors. Use the `threemf` crate, or zip + quick-xml.
   - OBJ via `tobj` (phase 6).
2. **Weld** vertices by quantized position hash, with epsilon relative to the bounding-box diagonal (~1e-6 × diag).
3. **Crease-aware normals.** Compute per-corner normals by averaging angle-weighted face normals across edges whose dihedral angle is below the threshold (default 35°, adjustable). Split vertices at creases. Without this, mechanical parts look either faceted or melted.
4. **Normalize for display.** Center on X/Y and rest the minimum Z on the floor (Z=0). Keep units in mm; never rescale the geometry, because layer lines and dimensions depend on real units.
   - Unit heuristic: if the bounding box is < 2 units, prompt "Looks like metres/inches?" and let the user choose mm / inch / m.
5. **Stats and validation** (shown in the UI and in the diagram title block):
   - Bounding box (mm), triangle count, surface area
   - **Volume** via the signed tetrahedron sum (valid when watertight)
   - **Watertight / manifold check** via edge-use counting (each edge used exactly twice); report boundary and non-manifold edge counts
   - Estimated **solid mass** = volume × material density (PLA 1.24, PETG 1.27, ABS 1.04, standard resin ~1.15 g/cm³). Labelled "solid, 100% infill"
6. **Up-axis fixups.** Buttons to rotate ±90° about X/Y, plus "lay flat on largest face" (pick the convex-hull face with the largest area).

Large scans (several million triangles) are fine. A 5M-triangle BLAS is a few hundred MB, which is comfortable in 12 GB.

---

## 5. Renderer (`tn-render`)

### 5.1 Acceleration structures
- **BLAS**: one for the model, one for the backdrop geometry. Built once per mesh load.
- **TLAS**: two instances. **Per turntable frame only the model instance transform changes**, so a TLAS rebuild costs microseconds. No BLAS rebuild between frames.

### 5.2 Integrator
- WGSL **compute megakernel**: one thread per pixel per sample batch, using `rayQuery` for closest-hit and shadow rays. A wavefront design is unnecessary for one material per scene.
- Unidirectional path tracing with:
  - **Next-event estimation** to area lights (rect softboxes) and the environment
  - **MIS** (power heuristic) between BSDF and light sampling
  - **Importance-sampled HDRI** (marginal/conditional CDF built on load)
  - Russian roulette after bounce 3, max depth ~8 (12+ for translucent resin)
- **Sampling**: Owen-scrambled Sobol or blue-noise-dithered PCG. Start with PCG, then upgrade; the better sampler visibly helps at low spp in the viewport.
- **Shadow-terminator fix** for smooth normals on low-poly geometry: offset shadow-ray origins using the Hanika 2021 technique. Prevents faceted black blotches on low-res STLs.
- Accumulate into an **RGBA32F buffer** plus the **AOVs**: first-hit albedo, first-hit world normal, depth, object ID, and alpha/coverage.

### 5.3 Material model (one compact "print principled" BSDF)
Layered, evaluated in the shader from a small uniform struct:
- **Base**: Lambert, or Burley diffuse for chalky matte PLA
- **Specular**: GGX with VNDF sampling, optional **anisotropy** (tangent = horizontal around the build axis, i.e. along the layer lines)
- **Metal**: conductor Fresnel with artist-friendly reflectivity/edge-tint (Gulbrandsen) or F82-tint
- **Clearcoat**: second GGX lobe
- **Transmission**: rough dielectric
- **Interior medium**: homogeneous absorption + scattering coefficients for closed meshes (resin, PETG, "jade"), using delta tracking or a simple random walk because the medium is homogeneous
- **Sheen**: optional, for TPU/fabric-like prints

### 5.4 Procedural print surface details (the signature feature)
All in **object space along the build axis**, in real millimetres:
- **Layer lines**: `bump = profile(fract(z_build / layer_height))` with a rounded profile (the extruded bead is roughly elliptical). Perturb the shading normal analytically. Parameters: layer height, depth, rounding.
  - Fade the amplitude when the pixel footprint (estimated from hit distance and camera) is much larger than the layer pitch. This keeps the denoiser's normal AOV stable and avoids moiré at GIF resolutions.
- **Slight per-layer variation** via hashed noise per layer index, so layers aren't perfectly uniform.
- **Resin**: very fine 0.05 mm lines that are barely visible; mostly glossy.
- Optional extras: subtle surface noise / "sanded" roughness variation using triplanar noise.

### 5.5 Material presets (ship these)
| Preset | Look |
|---|---|
| Matte PLA | Burley diffuse, roughness ~0.6, 0.2 mm layer lines |
| Silk PLA | High anisotropic spec along layers, metallic-ish tint (silver/gold/copper/rainbow-gradient over Z) |
| PETG | Glossy, slight translucency via a weak interior medium |
| Grey resin | Smooth, satin, very faint lines; the classic "miniature" look |
| Clear/tinted resin | Dielectric + absorption medium; striking in turntables |
| Clay | Neutral grey matte, no lines; for sculpt and form review |
| Metal (polished/brushed) | For "what if I cast/machined this" |
| Painted miniature primer | Flat grey/black, high roughness |
| Carbon-fiber nylon | Dark matte with fine sparkle noise |

Every preset exposes a color picker plus 3–4 sliders. Presets are RON data, so adding more needs no code.

### 5.6 Lighting rigs and backdrops
- **Rigs** (procedural, no files needed): *Studio Soft* (large key softbox + fill + rim + gradient env), *Product Hard* (smaller key, strong rim), *Dramatic* (single key + rim, dark env), *Even* (for diagrams: big overhead + ring fill). Plus **HDRI** mode with bundled CC0 studio HDRIs and a rotation slider.
- **Backdrops**:
  - **Cyclorama sweep**: generated mesh, curved floor-to-wall, any color. Default for GIFs.
  - **Infinite floor + environment**
  - **Shadow catcher / transparent**: for PNG/WebP. Render the floor only for shadow and occlusion and output alpha. (GIF only has 1-bit alpha, so disable this for GIF or flatten onto a color.)
  - **Print bed** (stretch goal): textured PEI sheet with a subtle gold speckle, model resting on it. Very on-theme.

### 5.7 Camera
- Perspective, default ~70 mm-equivalent focal length (product-shot compression), elevation 20°. Orthographic for the diagram views.
- **Turntable auto-framing**: frame the **swept volume**, a cylinder of radius max(distance from rotation axis) and height of the bounding box, so the model never leaves frame or clips during rotation.
- Optional thin-lens DOF (off by default).

### 5.8 Tone mapping and color
- Linear Rec.709 working space, **AgX** view transform (Blender's default since 4.0, so the output has the familiar "Blender look") plus exposure and optional "punchy" look. Filmic/ACES-fit available as alternatives.
- The tone map happens after denoising.

---

## 6. Denoising (`tn-denoise`)

- **OIDN 2.x** through the `oidn` Rust crate, using the **CUDA device** on the 4070 Ti. Inputs: beauty + albedo + normal (prefiltered auxiliaries, "clean aux" mode), HDR.
- Viewport: read back each accumulation step (~1080p, 3 × RGB32F ≈ 75 MB/s at 3 Hz, trivial), denoise, upload. Denoise only after the first N samples, then periodically; skip denoising while the camera is moving (show raw low-spp output, or ~1 spp + denoise).
- Final renders: denoise once per frame at the target spp.
- **Temporal stability for turntables**: because GIFs are low-res, the default turntable quality uses enough spp (256–512) that per-frame OIDN does not flicker. Use a **fixed sampler seed per pixel** (static noise pattern); the object moves under it, and with high spp plus denoise the residual is invisible. Measure both fixed and varying seeds in Phase 3 and pick empirically.
- Packaging: OIDN's DLLs (with the CUDA device) are large but redistributable (Apache-2.0). Ship them next to the exe. CPU device fallback if CUDA is unavailable.

---

## 7. Output modes (`tn-output`)

### 7.1 Turntable
- Settings: frames (default 72), duration (default 4 s), resolution (default 720×720), quality preset, start angle, direction, **seamless loop** (render angles `360·i/N` for `i in 0..N`, so the last frame never duplicates the first).
- Optional **easing / hold**: pause on the hero angle for 0.5 s (repeat frame-delay on the first frame; no extra renders).
- Optional slow **light sweep** variant (object fixed, rig rotates). Looks great on silk PLA and resin.
- Writers:
  - **GIF via `gifski`**: by far the best GIF quality in the ecosystem (per-frame palettes, temporal dithering). *Note: gifski is AGPL-3.0, fine for a personal tool; swap for `gif` + `color_quant` (MIT) if you ever distribute closed-source.* Keep frame delays ≥ 2 centiseconds; browsers clamp smaller delays to 10 cs, which makes "fast" GIFs play slowly.
  - **Animated WebP** (`webp-animation` crate): full color, alpha, far smaller than GIF. Offer it as the "better GIF".
  - **MP4/H.264 and WebM**: pipe frames to `ffmpeg` if it's on PATH (optional dependency).
  - PNG sequence (16-bit optional) and EXR for compositing.
- GIF banding: smooth gradient backdrops band in 256 colors. Mitigations are gifski's dithering, a "GIF-friendly" flatter backdrop preset, and a warning in the UI.

### 7.2 High-res still
- Up to 8K: an 8K accumulation buffer plus AOVs is ~1 GB, so no tiling is needed on 12 GB. Add tiling later only if it's ever needed.
- Quality presets by spp (e.g. 256 / 1024 / 4096) **or** a time budget ("render for 30 s").
- Output PNG (8/16-bit), EXR, optional transparent background. Optional **contact sheet**: 8 angles in a grid in one image.

### 7.3 Four-view diagram
- Views: **Front, Right, Top** (orthographic) plus **3/4 view** (perspective, the hero shot).
- **Projection layout**: third-angle (US/ASME; the default) or first-angle (ISO/EU), selectable.
- **One shared orthographic scale** across the three ortho views, so relative sizes are truthful. The 3/4 view is framed independently.
- **Lighting rotates with each view** (per §2.4); default rig *Even*.
- **Line overlay** (optional, on by default): silhouette and crease edges detected from the **depth + normal + object ID AOVs** (Sobel or Roberts on the GPU), drawn in a crisp dark line. This gives the "technical illustration" look over the path-traced shading.
- **Annotations** drawn with `tiny-skia` + `cosmic-text`:
  - Overall dimensions in mm with dimension lines and arrows on the ortho views (width/depth/height)
  - Scale bar
  - Title block: file name, date, dimensions, triangle count, volume (cm³), estimated mass for the chosen material, watertight ✓/✗
  - View labels (FRONT / RIGHT / TOP / ISO)
- Export PNG (and later **SVG** with the raster views embedded and vector annotations on top).

---

## 8. Performance expectations (estimates, to be validated in Phase 0)

On a 4070 Ti, a single-material megakernel path tracer with hardware RT should reach very roughly **0.5–1.5 G samples/s** for opaque materials at ~5 bounces in a simple studio scene. Translucent resin will be several times slower.

| Job | Settings | Estimated time |
|---|---|---|
| Viewport | 1080p, progressive | Usable denoised image in < 0.3 s, clean in 1–3 s |
| GIF turntable | 720², 72 frames, 256 spp | ~10–30 s total (OIDN ~10–20 ms/frame) |
| WebP/MP4 turntable | 1080², 120 frames, 512 spp | ~1–2 min |
| 4K still | 3840×2160, 1024 spp | ~10–30 s |
| 8K still | 7680×4320, 2048 spp | ~1–2 min |
| Four-view sheet | 4 × 1500², 1024 spp | ~20–40 s |

These fit comfortably inside the "a minute or two" budget. If real numbers come in 3× worse, everything still fits except 8K, which then needs a lower spp setting.

---

## 9. GUI (`tn-app`, eframe/egui)

```
┌───────────────────────────────────────────────────────────────────────────┐
│ File  View  Presets  Help                               [Render ▶]         │
├────────────────┬──────────────────────────────────────┬───────────────────┤
│ MODEL          │                                      │ OUTPUT            │
│ part.stl       │                                      │ ○ Turntable       │
│ 84×52×31 mm    │          Viewport (progressive,      │ ○ Still           │
│ 212k tris  ✓   │          denoised, orbit/zoom)       │ ○ Four-view       │
│ Units [mm ▾]   │                                      │                   │
│ Up [Z ▾] ⟲     │                                      │ 720×720  72 fr 4s │
│ Smooth ∠ 35°   │                                      │ Quality [Good ▾]  │
│                │                                      │ Format [GIF ▾]    │
│ MATERIAL       │                                      │ Backdrop [Sweep]  │
│ [swatch grid]  │                                      │                   │
│ Color ■        ├──────────────────────────────────────┤ RENDER QUEUE      │
│ Layer lines ☑  │ Turntable scrub ◀━━━━━●━━━━━━━━━▶    │ ▓▓▓▓▓░░ 42/72    │
│  0.20 mm       │ spp 128 · 0.9 s · denoised           │ [frame strip ...] │
│ LIGHTING       │                                      │                   │
│ Rig [Studio ▾] │                                      │                   │
│ Rotate  ◯      │                                      │                   │
└────────────────┴──────────────────────────────────────┴───────────────────┘
```

- **Drag and drop** an STL anywhere. **Recent files**.
- **Live file watching** (`notify` crate): when the STL changes on disk (re-exported from CAD), reload and re-render automatically. Very handy in a design-iterate-print loop.
- **Material swatches rendered by the path tracer itself** (a small sphere or the model, cached thumbnails).
- Turntable scrub slider previews any frame angle in the viewport.
- Four-view mode shows the composed sheet layout in the viewport.
- During a final render the viewport pauses (one GPU), a **frame strip** fills in as frames finish, and the result opens in a preview with "Open folder" and "Copy to clipboard".
- Save/load **project** (`.tnproj`, RON) and **user presets**.

`tn-cli` takes the same scene/preset options for batch jobs, e.g.:
```
turninator-cli render parts/*.stl --material silk-pla --color "#c0a060" --mode turntable --format gif --out renders/
```

---

## 10. Phased implementation

### Phase 0: Feasibility spike (1–2 days)
- eframe + wgpu with ray-query features on the 4070 Ti (try both Vulkan and DX12).
- Load an STL, build BLAS/TLAS, trace primary rays in a compute shader, and display normals in an egui panel.
- Measure primary-ray throughput.
- **Go/no-go**: if ray queries are broken or unbearably unstable in the current wgpu, switch to a software BVH (§11) before writing anything else.

### Phase 1: Mesh pipeline + basic path tracer (~1 week)
- `tn-mesh`: STL import, weld, crease normals, stats, watertight check, unit/up-axis handling.
- Path tracer: diffuse + GGX, area lights, NEE + MIS, accumulation, AgX tone map.
- Viewport: orbit camera, progressive accumulation, reset on change.
- OIDN integration (CUDA device), denoised viewport.

### Phase 2: Materials, lighting, backdrops (~1–1.5 weeks)
- Full print-principled BSDF (metal, clearcoat, anisotropy, transmission, interior medium).
- Layer-line bump with footprint fade; build-axis vs display-rotation separation.
- Material presets (§5.5), procedural rigs + HDRI importance sampling, cyclorama/floor/shadow-catcher backdrops.
- Shadow-terminator fix. Better sampler (Sobol/blue noise).

### Phase 3: Turntable + export (~1 week)
- Swept-volume auto-framing, seamless frame angles, TLAS-only per-frame updates.
- Render queue on a worker thread with progress + frame strip.
- gifski GIF, animated WebP, ffmpeg MP4, PNG sequence.
- Measure temporal stability (fixed vs varying seed) and tune default spp per quality preset.

### Phase 4: High-res stills (~3–4 days)
- Large-resolution accumulation, spp or time-budget presets, PNG16/EXR, transparent output, contact sheet.

### Phase 5: Four-view diagram (~1 week)
- Ortho cameras with a shared scale, per-view rig rotation, first/third-angle layout.
- Edge-detection overlay from AOVs.
- Dimensions, scale bar, title block (tiny-skia + cosmic-text); PNG export (SVG later).

### Phase 6: Polish (ongoing)
- 3MF and OBJ import (3MF multi-object/multi-color would be a strong differentiator).
- File watching, CLI batch, project files, user presets, clipboard copy.
- Print-bed backdrop, light-sweep turntable variant, rainbow silk gradient.
- Optional "Export to Blender" (write a `.py` that rebuilds the scene with equivalent Principled BSDF settings) for hand-tuning.

Rough total to a very usable tool: **~5–6 weeks of focused part-time work**, with nice images arriving at the end of Phase 1.

---

## 11. Risks and fallbacks

| Risk | Mitigation |
|---|---|
| wgpu ray queries are experimental; API churn or driver bugs | Pin the wgpu version. Keep traversal behind a `trace()` WGSL function so the backend can be swapped. **Fallback A**: a software BVH (SAH-built on the CPU, compact 8-wide or 2-wide node layout) traversed in WGSL. Slower (~3–5×), but still well within budget for one mesh, and uses no experimental features. **Fallback B**: `ash` (raw Vulkan) with `VK_KHR_ray_query`; more boilerplate, rock solid. |
| eframe doesn't expose a device with the right features | Use `egui-wgpu` + `egui-winit` directly with your own device, at the cost of a bit more boilerplate. |
| OIDN CUDA DLL size / setup | CPU device fallback. OIDN CPU at 720² is still ~50–100 ms/frame, fine for turntables. |
| Denoiser flicker in turntables | Higher spp defaults (cheap at GIF res), fixed seeds, consistent aux buffers. As a last resort, a temporal blend using known object rotation (reprojection is exact here). |
| Translucent resin is slow and noisy | Cap the depth, use a dedicated "resin" quality preset with more spp, and treat it as the premium look rather than the default. |
| Low-poly STLs look faceted or blotchy | Crease-normal splitting + shadow-terminator offset + an adjustable smoothing angle. |
| GIF banding | gifski, GIF-friendly flat backdrops, steer users to WebP. |

---

## 12. Crate shortlist

| Purpose | Crate |
|---|---|
| GUI | `eframe`, `egui`, `egui-wgpu`, `rfd` (file dialogs), `arboard` (clipboard) |
| GPU | `wgpu` (ray-query features), `bytemuck`, `glam` |
| Mesh | hand-rolled STL parser, `memmap2`, `rayon`; `threemf` / `tobj` later; optional `bvh` for the software fallback |
| Denoise | `oidn` |
| Images | `image`, `exr`, `png` |
| Animation out | `gifski`, `webp-animation`, ffmpeg subprocess |
| Diagram | `tiny-skia`, `cosmic-text` |
| Data/config | `serde`, `ron` |
| Misc | `notify` (file watch), `anyhow`/`thiserror`, `tracing`, `clap` (CLI) |

---

## 13. First concrete steps

1. `cargo new` the workspace with `tn-app`, `tn-mesh`, `tn-render`.
2. Phase 0 spike: eframe window → wgpu device with ray query → STL → BLAS/TLAS → normals visualization → throughput numbers.
3. Write the `Scene` struct and a `Renderer` trait (`set_scene`, `render_progressive`, `render_final(frame) -> Image`) before writing more shader code, so the GUI, CLI, and possible fallback backends share one contract.
