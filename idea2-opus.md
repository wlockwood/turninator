# Turninator: Revised Plan (v2)

A local Windows app that takes 3D-print models (STL), makes them look like **real printed parts**, and outputs:

1. **Looping turntable GIFs** (primary)
2. **High-resolution stills**
3. **Four-view sheets** (Front / Right / Top / Isometric)

The headline feature is **printed-layer simulation**. Parts can be shown with real 0.2 mm (or any) layer lines, including stair-stepping on slopes, terraces on domes, and top-surface infill lines. The goal is that the image looks like the part you'll pull off the bed.

Target machine: RTX 4070 Ti (12 GB), Ryzen 9 9950X, 96 GB RAM (hardware details from `idea-codex.md`).

---

## 1. Decision

**A Rust app that owns all geometry, with Blender (Cycles or EEVEE) as the renderer, run as a background worker.**

- **Rust (egui + wgpu)** handles:
  - STL import, normals and framing
  - **The layer simulator**, which generates the "as printed" mesh
  - A fast native viewport
  - GIF encoding and four-view composition
- **Blender** runs as one long-lived headless worker. It is the authoritative renderer for previews, GIF frames, stills and diagram panels: **Cycles + OptiX** for path-traced quality, **EEVEE** for speed.
- **Inspection tools** (see-through, section plane with caps) live in the native viewport and work even without Blender.
- **Later: G-code print preview** (§10) renders the slicer's actual toolpaths for the most accurate "as printed" result.
- **The layer simulation lives in Rust and is independent of the renderer.** It produces geometry plus a few per-vertex attributes, so any renderer can shade it with a trivial material. The native viewport and Blender therefore show the same layers. A native path tracer could later replace Blender without touching the most distinctive part of the app.

### What changed from my v1 (native path tracer)

`idea-codex.md` is right that a native path tracer is a large graphics project. The materials, resin volumetrics, sampling and denoising would eat weeks before the print-specific value appeared. That value turns out to be **geometric**, not a rendering trick. Once layers are real geometry with attributes, Cycles renders them well with standard materials. So: spend the effort on the slicer-like layer simulator and borrow Blender's renderer.

### What changed from `idea-codex.md`

| Codex plan | v2 |
|---|---|
| Defers "printable layer texture" and "printed-layer geometry" | **Makes them the centerpiece** (§3) |
| EEVEE for GIFs/diagrams, Cycles for stills | **Both engines, user-selectable per output** (§4). Phase 0 measures which is the better *default* for GIFs; Cycles is the reference for stills and resin. One preset schema feeds both engines. |
| Deliberately plain native viewport ("don't recreate EEVEE") | Same principle, but the viewport renders the **simulated layer mesh** with the same shading formula, so layer settings are previewed live |
| FFmpeg required for GIF (palettegen/paletteuse) | **gifski in-process** (no required external tool; better temporal dithering). FFmpeg is optional, for MP4 only. Phase 0 compares the two on layer-line turntables. |
| See-through OIT and section planes | **Kept** (§5.1). They pair well with layers: a section through a sliced part shows the stepped profile, and later the G-code infill (§10). |
| Two-model comparison, ghost overlay | Deferred |
| Durable job manifests, resume, 20 GB cache budget, disk preflight | **Simplified**: frames go to a temp job dir keyed by scene fingerprint, so re-encoding never re-renders. No resume machinery in v1. |
| Kept from codex | STL robustness cases, f64 bounds and rebasing, generation IDs against stale results, the worker protocol shape, Windows Job Object cleanup, argument-array process calls, atomic output publish, rotational-envelope framing, GIF timing defaults, four-view conventions and layout, EEVEE + Cycles, see-through and section inspection, and **measure before tuning** |

---

## 2. Architecture

```
            ┌──────────────────────── Rust app (turninator.exe) ───────────────────────┐
 STL ──►  import ─► canonical mesh ─► LAYER SIMULATOR ─► print mesh + attributes        │
            │                          (slice, walls, treads,     │                     │
            │                           variable heights)         ├─► native viewport   │
            │                                                     │   (wgpu raster)     │
            │   SceneSpec (material, studio, camera, layers) ─────┤                     │
            │                                                     └─► meshbin + JSON ───┼──► Blender worker
            │                                                                           │    (Cycles/OptiX)
            │   frames (PNG/EXR) ◄──────────────────────────────────────────────────────┼───── renders
            │        └─► gifski GIF · PNG still · four-view compositor · (ffmpeg MP4)   │
            └───────────────────────────────────────────────────────────────────────────┘
```

**One Rust package with modules** (split into crates only if compile times demand it):

```
src/
  main.rs, app.rs              egui app, async task plumbing (generation IDs)
  mesh/{stl,weld,normals,stats}.rs
  print/{slice,contour,layers,heights,gcode_z}.rs   ← layer simulator
  scene/{spec,camera,presets}.rs
  viewport/{gpu,inspect,shading.wgsl,oit.wgsl}   inspect = see-through + section plane
  render/{worker,protocol}.rs  Blender process + line-delimited JSON
  export/{gif,still,diagram,video}.rs
worker/
  worker.py                    command loop; bpy only on main thread
  build_scene.py, materials.py, studios.py
assets/presets/*.json, assets/fonts/
tools/fixtures.py, tools/bench.py
```

**Render boundary**: `SceneSpec` (serde JSON) plus a `meshbin` file (versioned header, f32 positions, u32 indices, f32 corner normals, plus **named f32/u32 attribute arrays**). The Python worker loads it with NumPy `foreach_set`. This boundary is the only contract between Rust and the renderer.

---

## 3. Layer simulation (the centerpiece)

### 3.1 Modes

| Mode | What it is | Use |
|---|---|---|
| **Off** | Original mesh, smooth/crease normals | Clean renders, diagrams |
| **Shaded** | Original mesh plus a procedural layer bump (sine/bead profile on build-Z) plus anisotropic sheen | Resin (0.025–0.05 mm), tiny-on-screen layers, broken meshes |
| **Sliced** | **Real stepped geometry** generated by slicing at the actual layer heights | FDM default: true stair-stepping, terraces and silhouettes |
| **Auto** (default) | Chooses Sliced or Shaded from on-screen pixels per layer (§3.6) | Most users, most of the time |

### 3.2 Layer heights ("typically 0.2 mm, but may vary")

The layer **height profile** is a sorted list of layer boundaries `z0=0 < z1 < … < zN`. Every mode consumes this one representation:

- **Uniform**: layer height (default 0.20 mm; presets 0.08 / 0.12 / 0.16 / 0.20 / 0.24 / 0.28 / 0.32) plus a separate **first-layer height** (default 0.20).
- **Adaptive**: approximates slicer adaptive layers. It picks the thickest layer in `[min, max]` whose cusp height `h·|n_z|` (worst over triangles in that band) stays under a quality threshold. Shallow domes get thin layers; vertical walls get thick ones.
- **From G-code**: drop the `.gcode` your slicer produced and Turninator reads the actual layer Z values (`;Z:` / `;LAYER_CHANGE` / `;LAYER:` comments, falling back to Z of extruding moves). The render then matches exactly what your printer will do, including variable-layer-height edits made in the slicer.

### 3.3 Slicing algorithm (`print/`)

The input is the welded mesh in the **build frame** (STL Z-up as sliced; mm; minimum Z = 0).

1. **Contours.** For each layer `k`, slice at mid-height `(z_k + z_{k+1})/2`, which is what slicers do. Bucket triangles by Z range so each layer only tests overlapping triangles. Run layers in parallel with rayon, which suits a 9950X. Intersection points are keyed by mesh edge `(va, vb)`, so segments chain into loops topologically rather than by fuzzy distance. Nudge the plane by an epsilon if a vertex lies exactly on it.
2. **Clean regions.** Run the loops through a polygon boolean (non-zero union) to produce region `R_k` (polygons with holes). This repairs self-intersections and overlapping shells. Use `i_overlay` (pure Rust) or `clipper2`. Simplify contours with Douglas–Peucker at ~0.01 mm, which matches typical slicer resolution.
3. **Walls.** For each loop in `R_k`, emit a vertical quad strip from `z_k` to `z_{k+1}`. Each vertex carries:
   - `layer_t` (0 at the bottom, 1 at the top)
   - `layer_index`
   - `kind = wall`
   - Normals: 2D contour normals smoothed with the crease angle, so curved walls are smooth and corners stay sharp.
4. **Treads (the step surfaces).**
   - Up-facing: `R_k \ R_{k+1}` at `z_{k+1}`. The top layer is `R_top` itself.
   - Down-facing: `R_k \ R_{k-1}` at `z_k`. Layer 0 is the bottom face.
   - Triangulate polygons with holes (`i_triangle` or `earcutr`). Attributes: `kind = top|bottom`, `layer_index`.
5. **Output**: a print mesh plus attributes, cached by `(mesh hash, height profile, settings)`. Recompute asynchronously when settings change, with a generation ID; the viewport keeps the previous result until the new one is ready.

**Size estimate**: a 60 mm-tall part at 0.2 mm is 300 layers × ~1–3k contour edges, about **1–3 M triangles** total. A 200 mm part at 0.1 mm could reach ~10 M, which is fine for Cycles (well under 12 GB) and for the raster viewport. If needed, cap the slicer by decimating contours. Expected compute time is sub-second to a few seconds; Phase 0 measures it.

**Failure behavior**: open or non-manifold meshes can produce unclosed contours. Close small gaps, and if a layer still fails, **fall back to Shaded mode with a visible warning** rather than producing garbage. Print-ready STLs are usually manifold, so this should be rare.

### 3.4 Shading the print mesh (identical in WGSL and Blender nodes)

Write the formula once in a short spec (`docs/layer-shading.md`) and implement it twice (WGSL, Blender node group), with a reference-image test.

- **Bead rounding on walls.** Perturb the normal by the layer-local position: `φ = asin(roundness · (2·layer_t − 1))` and `N' = normalize(N_wall·cos φ + Z·sin φ)`. With the default roundness of ~0.8, the top of each bead faces slightly up and the bottom slightly down. This produces the characteristic bright/dark ribbing with no extra geometry.
- **Optional bead geometry** (stills only): 4–6 vertical subdivisions per wall with a real semicircular bulge, so silhouettes also show rounded beads. This costs about 4–6× the triangles.
- **Top-surface infill lines**: on up-facing treads, a bump of lines at ±45° alternating by `layer_index`, with pitch = **line width** (default 0.42 mm). Large flat tops then look printed, not glassy.
- **Bottom face**: smooth-PEI gloss by default (option: textured PEI matte).
- **Anisotropic sheen**: tangent = `normalize(cross(Z, N))` (horizontal, along the layers). Principled BSDF anisotropy stretches highlights across the layers. This is what makes silk PLA look like silk PLA, and it is also how layers read from far away.
- **Optional Z-seam** (later): a vertical line of small blobs at the contour vertex nearest a chosen direction ("aligned rear").

**Artistic controls**: layer contrast (0–1, scales the perturbation), roundness, and line width. "Physically plausible" is the default; contrast lets you exaggerate for marketing shots.

### 3.5 Build orientation vs display orientation

Layers always follow the **build frame**. The display frame (how the model sits in the studio and spins) is a separate transform. You can show a part lying down that was printed standing up, and the layers stay where the printer would put them. Default: display = build (the part sits as it was printed).

### 3.6 Layer visibility and aliasing (important for GIFs)

From the camera framing, compute **pixels per layer** = layer height × pixels per mm. Show it in the UI ("Layers: 4.3 px").

| px/layer | Behavior in Auto mode |
|---|---|
| ≥ 6 | Sliced geometry; lines are crisp |
| 2.5–6 | Sliced, with Cycles pixel filter widened slightly to suppress moiré |
| < 2.5 | Shaded: the bump fades into anisotropic sheen. Individual lines can't be resolved and would only shimmer or moiré in a turntable and through GIF dithering. |

Example: at the default 960² GIF a 50 mm part spans ~770 px, so 0.2 mm layers are **~3 px**. That is borderline, so Auto chooses carefully, and the export dialog suggests "Large (1280)" or tighter framing when you want visible lines. A 4K still of the same part gives ~13 px per layer: true stair-stepping, clearly visible.

---

## 4. Rendering (Blender worker)

- **Version**: pin the current Blender LTS (codex suggests 5.2.x; verify). Detect it from settings or PATH, or point at a portable Blender zip extracted next to the app. Record the exact build.
- **Process**:
  - One long-lived `blender -b --factory-startup --disable-autoexec -P worker/worker.py` launched with no console window and tracked by a **Windows Job Object** so it dies with the app.
  - Requests are line-delimited JSON on stdin (`Probe`, `LoadMesh`, `SetScene`, `Render`, `Shutdown`), each with a request ID and scene revision. Events come back on stdout with a `TURNINATOR_EVENT ` prefix; all other output goes to a log.
  - Cancellation kills and restarts the worker, which is honest and simple.
- **Engines**: the user picks **Cycles** or **EEVEE** per output, with sensible defaults.

  | | Cycles (OptiX) | EEVEE (Next) |
  |---|---|---|
  | Character | Path traced: true contact shadows, AO in layer grooves, correct glass/resin | Rasterized with screen-space/ray-traced approximations; very fast and temporally stable |
  | Default for | Stills, resin/translucent anything, final-quality GIFs | Rendered Preview, fast GIFs, four-view panels (if Phase 0 confirms quality) |
  | Settings | Adaptive sampling, GPU denoiser (OptiX or OIDN; Phase 0 picks), `use_persistent_data` on, fixed seed unless Phase 0 shows otherwise | Fixed sample count (start at 16 preview / 64 final), ray-traced reflections on, shadows at high resolution, fixed exposure |

  - **One material definition for both.** Presets are Principled BSDF + the `PrintLayer` node group, which both engines evaluate, including mesh attributes. Some features differ by engine, for example anisotropy and volume/refraction thickness in EEVEE. Probe these in Phase 0 and record a per-engine fallback (e.g. EEVEE uses isotropic sheen if anisotropy is unsupported).
  - **Agreement standard** (from codex): geometry, camera, orientation, layers and base color must match across engines. Highlights may differ.
  - **Resin** routes to Cycles by default. EEVEE is allowed with an "approximate refraction" label, because EEVEE's refraction can't show internal structure correctly.
  - The export dialog shows the chosen engine and its last measured s/frame, so the speed/quality trade is visible.
- **Color**: AgX view transform, fixed exposure, sRGB output. No per-frame adaptation.
- **Materials** (versioned JSON presets, built by `materials.py` on a Principled BSDF plus the `PrintLayer` node group):

  | Preset | Notes |
  |---|---|
  | **Matte PLA** (default) | Diffuse-heavy, roughness ~0.55, mild sheen |
  | Silk PLA | Strong anisotropy, metallic tint; optional gradient over Z (rainbow silk) |
  | Glossy PETG | Lower roughness, slight transmission |
  | Translucent PETG | Transmission + volume absorption, colored |
  | Grey resin | Satin, Shaded layers at 0.05 mm |
  | Clear / tinted resin | Transmission + absorption; deeper bounce limits; slowest preset |
  | Clay | Inspection look, layers off |
  | Anodized metal | For machined/cast what-ifs |

  Each preset exposes Color plus 2–3 sliders. Resin adds Tint, Clarity and Cloudiness (from codex).
- **Studios**: *Light Studio* and *Dark Studio*: large soft key/fill/rim area lights, subdued world, seamless sweep backdrop. Diagram mode overrides with white background, neutral grey material and no floor. Later: a *Print Bed* studio with a PEI-sheet backdrop.

---

## 5. Native viewport (wgpu, deliberately modest)

- Renders the **print mesh** (Sliced, or the original mesh with the Shaded formula) with the same layer shading, a simple GGX + diffuse model, 3 fixed lights, a gradient environment and an optional shadow. It doesn't chase Blender fidelity, but layer settings, framing and orientation are all live.
- Orbit/pan/zoom, fit, reset, perspective/ortho toggle, Front/Right/Top/Iso buttons, axis gizmo, turntable scrub slider, and an optional edge overlay.
- **Rendered Preview** toggle: asks the worker for a render of the current camera and overlays it. Engine is selectable: EEVEE by default for speed, Cycles 16–32 spp denoised for accuracy. It is marked stale when the camera moves, and the last preview stays visible while updating. Manual refresh by default; auto-refresh with 300 ms debounce is optional.

### 5.1 Inspection: see-through and section plane

Separate from studio materials (from codex): these are **inspection tools in the native viewport** and don't change the export material.

- **See-through**
  - An opacity slider plus an optional edge overlay. Implemented with **weighted blended order-independent transparency** (two render targets + a composite pass), not triangle sorting, so it stays stable while orbiting. It is an approximation with many overlapping layers, which is fine for inspection.
  - Shows internal shells, embedded parts, captive nuts and cavities. Works on both the original and the sliced mesh.
  - Optional **X-ray tint by facing**: back faces in a second color, so inner and outer surfaces are distinguishable.
- **Section plane**
  - Clip along X, Y or Z (and later an arbitrary plane from a gizmo), with a slider position in mm and a flip side. Implemented with fragment clipping against a plane uniform.
  - **Section caps.** Uncapped clipping makes a solid look hollow. Capping uses the stencil trick: count front/back faces behind the clip plane and fill a flat cap where the parity says "inside". Watertight print meshes make this reliable. Cap color and an optional hatch pattern.
  - On a **sliced** mesh the cut shows the stepped layer profile in cross-section. In the G-code phase (§10) it shows the **actual infill, perimeters and wall count**. This is the main reason to build capping properly.
- **Inspection in exports.** v1: viewport only, and the export dialog says so when inspection is active. Later option: render a **section still/turntable** in Blender by applying a Boolean cut with a box (Cycles/EEVEE both handle the capped result natively). Section turntables of a print are a great way to show internals.
- Inspection works without Blender installed.

---

## 6. Outputs

### 6.1 Looping GIF (primary)
- Defaults (from codex): **960 × 960, 4 s, 20 fps, 80 frames, 5 cs delay, infinite loop**. Sizes: Compact 640 / Default 960 / Large 1280. Durations 3/4/6 s; rates 10/20/25 fps (integral cs delays).
- Fixed camera and lights; the object rotates about its display-frame XY center. Angle `i = start + 360·i/N` for `i in 0..N`; no duplicate endpoint, no end pause. Motion blur off.
- **Rotational-envelope framing**: fit the swept cylinder with 10% margins so nothing clips at any angle.
- Frames are rendered to a job dir as 16-bit PNG, then encoded with **gifski** (quality ~90; note gifski is AGPL, which is fine for a personal tool; `gif` + `color_quant` is the MIT fallback). Re-encoding at another size or quality never re-renders. After encoding, show the real GIF playing and its file size; if it's huge, *offer* (never silently apply) a smaller preset.
- Also: **animated WebP** (full color, much smaller) and **MP4** via FFmpeg if present, from the same frames.

### 6.2 High-res still
- Default 4096² PNG (also 2048², 3840×2160, custom up to 64 MP). Cycles at Final quality (e.g. 256 max spp, 0.01 noise threshold, denoise with albedo/normal). Optional transparent background (floor removed).
- This is where **Sliced + bead geometry** shines, so Auto picks it.
- Optional contact sheet: 8 angles in one image.

### 6.3 Four-view sheet
- Layout from codex: a 2×2 grid **Front | Right / Top | Isometric**, all orthographic.
  - Canonical frame X-right, Y-back, Z-up. Front looks from −Y, Right from +X, Top from +Z (+Y up), Iso from `(1, −1, 1)`.
  - 4096² sheet, 64 px margins and gutters, a label strip per panel, and a 2048² variant.
- **One shared pixels-per-mm scale** across all four panels; no independent zooming.
- Lighting rotates with each view so every panel is well lit (the opposite of the turntable rule). White background, neutral material, layers **Off** by default. "Show layers" is a toggle for print-preview sheets.
- **Edges**: Cycles outputs Normal, Depth and Object-index passes. Rust detects silhouette and crease edges (depth/normal discontinuity) and composites crisp lines. This is renderer-independent and avoids Freestyle. With layers on, layer steps are suppressed from the edge pass by thresholding on normal angle only.
- Caption: filename, overall W × D × H in mm (marked "assumed mm"), triangle count, volume and estimated solid mass for the chosen material, and the layer profile if shown. No dimension arrows in v1 (maybe later).
- Composited in Rust with `tiny-skia` and a bundled font (`ab_glyph` or `cosmic-text`).

---

## 7. Import and robustness (keep codex's rigor where it's cheap)

- STL binary/ASCII detection by **size check** (`84 + 50·n`), not by a `solid` prefix. Reject truncated data, absurd counts, non-finite values and empty meshes with clear errors.
- f64 bounds; rebase near the origin before converting to f32. Units default to mm (labeled), with inch/m/custom options. Prompt if the bounding box looks like it's in metres or inches.
- Exact-coordinate welding only (never tolerance-weld separate shells). Drop zero-area triangles (and show a count). Crease-aware corner normals (default 30–35°); never smooth across disconnected shells.
- Stats: bounds, triangles, shells, open and non-manifold edge counts, volume, area.
- Up-axis buttons (±90° about X/Y), "lay flat on largest face". These change the **build** frame and trigger re-slicing.
- Generation IDs on import, slicing and previews, so a stale result never overwrites a newer one.
- Process hygiene: argument arrays only, a Job Object, atomic output publish (write temp → verify decode → rename), and never touching source STLs or unrelated Blender processes.

---

## 8. Performance targets (to be measured in Phase 0 and recorded in `docs/performance.md`)

| Scenario | Target |
|---|---|
| Import + weld + normals, 1M-triangle STL | < 1 s |
| Slice, 60 mm part @ 0.2 mm (uniform) | < 2 s |
| Native viewport orbit, 3M-triangle print mesh, 1440p | ~60 fps |
| Native viewport with see-through (OIT) or capped section, 3M triangles | ≥ 30 fps |
| Rendered preview, 768², warm worker | EEVEE < 1 s, Cycles < 2 s |
| Default GIF (960², 80 frames, Sliced, Matte PLA), EEVEE | < 1 min end to end |
| Default GIF, Cycles | < 2 min end to end (≈ 1 s/frame budget) |
| 4096² still, Final, opaque | < 2 min |
| Four-view sheet, 4096² | < 1 min |
| Clear resin anything | Stress case; reported, not targeted |

If Cycles GIF frames come in far over budget, first lower max spp (relying on denoise), then make EEVEE the GIF default for opaque materials. The geometry and attributes are the same, so layers still work. Don't optimize before measuring.

---

## 9. Phases

Each phase ends with something runnable. Rough sizes assume focused, agent-assisted work.

**Phase 0: Prove the risky parts (≈3–4 days, no GUI)**
- Rust CLI: STL → weld → **slice at 0.2 mm** → print mesh + attributes → meshbin. Use 4–5 real print STLs from you, plus procedural fixtures (bracket with holes and thin ribs, dome, tall asymmetric part, dense 1M organic mesh, open/broken mesh).
- Python worker: load meshbin, build the `PrintLayer` node group + Matte PLA + Light Studio, render with Cycles/OptiX.
- Render an 80-frame turntable in **both Cycles and EEVEE** and encode each with gifski **and** ffmpeg-palette for comparison; render a 4K still.
- **Gates**:
  - (a) Does Sliced look like a real print? Compare side by side with a photo of a real print.
  - (b) Slicing time and triangle counts.
  - (c) Cycles and EEVEE s/frame at 960².
  - (d) Layer moiré/shimmer at 960² vs 1280² in each engine; tune the Auto thresholds.
  - (e) Fixed vs animated seed stability (Cycles).
  - (f) EEVEE feature probe: mesh attributes, anisotropy, headless background-mode rendering on your desktop. Pick the default GIF engine.
- Amend this plan with the results.

**Phase 1: App shell + viewport (≈1 week)**
- egui window, open/drop, async import with generation IDs, viewport with orbit/fit/views/scrub, orientation buttons, stats panel. Shaded layer mode in WGSL.

**Phase 2: Worker integration + materials/studios (≈1–1.5 weeks)**
- Worker lifecycle (probe, Job Object, cancel = restart), Rendered Preview with an engine toggle, presets and swatches, color/sliders, Light/Dark studios.
- Material builder for both engines, with recorded per-engine fallbacks; resin routed to Cycles by default.

**Phase 3: GIF export (≈4–5 days)**: the first truly useful release
- Envelope framing, frame job dir, gifski, playback/size display, WebP; MP4 if ffmpeg is present.

**Phase 4: Full layer simulator in the app (≈1–1.5 weeks)**
- Sliced mode in the viewport and renders. Height profiles: uniform + first layer, adaptive, from G-code.
- Top infill lines, anisotropic sheen, px/layer indicator, Auto mode, build-vs-display frames, fallback warnings, bead-geometry option.

**Phase 5: Stills (≈3 days)**
- Presets, transparent background, contact sheet.

**Phase 6: Four-view sheet (≈1 week)**
- Ortho cameras with shared scale, per-view lighting, pass-based edges, layout, labels, caption.

**Phase 7: Inspection (≈1 week)**
- See-through with weighted blended OIT, edge overlay, and back-face tint.
- Section plane on X/Y/Z with slider and flip, plus **stencil-capped** cross-sections with optional hatching.
- Export-dialog notice when inspection is active.
- Stretch: Blender section stills and turntables via a Boolean box cut.

**Phase 8: Polish (ongoing)**
- Project files (`.tnproj` JSON), file watching (re-render when the STL is re-exported from CAD), CLI batch mode, user presets, Print Bed studio, Z-seam, Blender version check UI.

**Phase 9: G-code print preview (later; see §10)**

---

## 10. Phase 9 (later): G-code print preview

**Goal**: drop in the `.gcode` your slicer produced and render **the extrusions the printer will actually lay down**. That means real perimeters, seams, top-surface patterns, bridges, supports, infill (visible in section or through translucent filament) and multi-material colors. It is the most accurate "how will this print look" the app can give. Sliced mode (§3) is the fast approximation; this is ground truth from your own slicer profile.

Deferred because it is a large, separable feature. **Earlier phases are designed so it slots in cleanly:**
- Phase 4's G-code layer-Z reader is the start of the parser.
- meshbin already carries named attributes (feature type, tool, layer).
- Phase 7's capped section plane is what makes infill visible.
- Engine selection and turntable/still/four-view outputs all work unchanged on a toolpath mesh.

### 10.1 Parsing
- **Plain `.gcode` first.** Handle:
  - Motion: `G0/G1`, `G2/G3` arcs (tessellated), `G90/G91`, `G92` resets
  - Extrusion modes: `M82/M83` absolute/relative E, firmware retract `G10/G11`
  - Units: `G20/G21`
  - Flavours: Marlin and Klipper
- **Feature types from slicer comments**:
  - PrusaSlicer/Orca/Bambu: `;TYPE:External perimeter`, `;TYPE:Solid infill`, `;TYPE:Support material`, …
  - Cura: `;TYPE:WALL-OUTER`, …
  - Layer markers: `;LAYER_CHANGE`, `;Z:`, `;LAYER:`
- **Extrusion width and height**: use `;WIDTH:` / `;HEIGHT:` comments when present (Prusa/Orca emit them). Otherwise derive from E: cross-section area `= E · π(d_fil/2)² / segment_length`, solved for width with a stadium profile `(w − h)·h + π(h/2)²`.
- **Later containers**: Prusa `.bgcode` (binary blocks; heatshrink/meatpack compression) and Bambu/Orca `.gcode.3mf` (zip containing `Metadata/plate_N.gcode`).
- Parse in parallel by chunks; a multi-million-line file should parse in seconds.

### 10.2 Geometry
Each extrusion segment becomes a tube with a **stadium (squished) cross-section** of w × h, joined along each continuous path.

- **Scale is the challenge.** A typical print has 10⁵–10⁶ segments; large prints have more. Mitigations:
  - **Visibility culling by feature**: for opaque turntables, keep only external perimeters, top/bottom solid surfaces, overhangs, bridges and supports. Drop sparse infill and inner perimeters, which are invisible from outside. Include everything when a section plane, see-through, or translucent material needs it. This typically cuts geometry 5–10×.
  - **Path simplification**: merge near-collinear segments within a tolerance.
  - **Representation (decided by a sub-phase experiment)**:
    - (a) Mesh tubes with a 6–8-sided stadium profile. Accurate, heavy.
    - (b) Blender **Curves** objects. Cycles renders curve primitives memory-efficiently, but the cross-section is round, so the squish has to be approximated.
    - (c) In the native viewport: **instanced segments**, with the tube built in the vertex shader (how slicer previews do it), for interactive display of everything.
- **Options**:
  - Show/hide supports (a rough "after support removal" view)
  - Hide skirt/brim/purge/wipe tower
  - Show/hide travel moves (debug)
  - Color by feature type or by layer (debug views)

### 10.3 What it enables
- **True seams, zits, bridges and top patterns**, exactly as configured in your slicer (monotonic, concentric, ironing).
- **Infill and wall count in section views**, and visible through translucent PETG/PLA. The latter is a real-world look that is gorgeous in Cycles.
- **Multi-material/AMS prints**: tool changes (`T0/T1…`) map to per-tool filament colors and materials. This renders actual multicolor prints.
- **Build-up animation**: a turntable or fixed-camera GIF of the part printing layer by layer (or path by path). A uniquely fun output.
- **Accuracy check**: STL Sliced mode, G-code mode and a real photo side by side to tune Sliced mode's look.

### 10.4 Sub-phases
- **9a**: parser + feature/layer-colored line preview in the native viewport (instanced segments)
- **9b**: tube geometry, feature culling, Blender render (mesh vs Curves experiment), still and turntable
- **9c**: section/see-through integration with full infill, multi-material colors, build-up animation
- **9d**: `.bgcode` and `.gcode.3mf` support; slicer-flavour coverage (Prusa, Orca, Bambu Studio, Cura)

---

## 10b. Other deferred ideas

- 3MF import (Bambu/Prusa project files; multi-object, colors) and OBJ.
- Native path tracer as a progressive preview renderer behind the same `SceneSpec` + meshbin boundary. Only worth building if the Rendered Preview loop feels too slow in practice.
- Two-model comparison and ghost overlay (from codex).
- Elephant-foot and overhang-droop approximations in Sliced mode; multi-color painting on STL/3MF.
- Job resume across app restarts, dimension arrows on diagrams.

---

## 11. Risks

| Risk | Mitigation |
|---|---|
| Sliced geometry looks "CG" rather than printed | Phase 0 gate (a) against a real photo. Tune roundness, contrast, top-line bump and sheen before building UI. |
| Messy STLs break slicing | Boolean union cleans most issues; per-layer fallback to Shaded with a warning; never block rendering |
| Layer lines moiré or shimmer in GIFs | px/layer indicator, Auto mode thresholds, pixel filter width, gifski temporal dithering; suggest Large size |
| Print mesh too heavy (tall parts, thin layers) | Contour simplification, triangle cap with Shaded fallback for the excess; measure in Phase 0 |
| Blender API differences across versions (custom normals, attributes, OIDN GPU) | Pin one LTS, probe capabilities at startup, run a render regression set before accepting a new version |
| Viewport, EEVEE and Cycles disagree on layers | One shading spec, implemented in WGSL and a node group, plus a reference-image comparison test across all three |
| EEVEE lacks a feature a preset relies on (anisotropy, refraction depth) | Phase 0 feature probe; per-engine fallbacks recorded in the preset schema; resin defaults to Cycles |
| OIT smears with many overlapping layers; section caps wrong on open meshes | OIT is an inspection aid, not a render. Caps only when the mesh is watertight; otherwise uncapped with a warning. |
| G-code geometry too heavy to render (Phase 9) | Feature culling, path simplification, instanced viewport segments, mesh-vs-Curves experiment before committing |
| Blender dependency friction | Settings picker; support a portable Blender folder next to the app; the viewport and inspection work without Blender |

---

## 12. First steps

1. Collect 4–5 representative STLs you actually print, plus one photo of a real printed part for the realism gate.
2. Scaffold the Cargo package and `worker/`; write `fixtures.py`.
3. Build the Phase 0 CLI slicer → meshbin → Cycles/EEVEE turntable → gifski pipeline and record the gate results in `docs/performance.md` and `docs/visual-validation.md`.
4. Optional, to keep Phase 9 in view: save the `.gcode` for one of those STLs from your usual slicer. Phase 4's layer-Z reader gets tested against real output, and the file becomes the first Phase 9 fixture.
