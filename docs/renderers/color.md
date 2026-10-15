# Shared HDR color recipe

`turninator-agx-v1` is a portable AgX approximation for future wgpu and LuxCore
adapters. Its implementation source is Filament v1.78.0 at commit
`c1dc4e444784850d6dc891a1fb38747c30b85463`, adapted under Apache 2.0 with upstream
MIT attribution. See [the notices](../../assets/render/color/NOTICE.md) and the
[frozen numeric recipe](../../assets/render/color/recipe.json). No Blender code,
OCIO configuration, or LUT is copied.

Input is **scene-linear Rec.709 D65, straight RGBA**. Unpremultiply HDR RGB in
the producer's linear domain before calling this recipe. RGB follows this order:
negative clamp, exposure `2^exposure_ev`, Rec.709 to Rec.2020, AgX inset, bounded
log2 encoding, seventh-degree contrast polynomial, AgX outset, power 2.2,
Rec.2020 to Rec.709, display gamut clamp. `exposure_ev` defaults to zero.

The CPU and C++ entrypoints reject nonfinite values, alpha outside 0..1,
exposure outside -32..32, and exposure-scaled RGB overflow. After the overflow
check they cap exposed RGB at 65504 to bound intermediate arithmetic. A channel
this high already saturates all three log-window channels through the positive
conversion/inset matrices; the cap preserves that output. The WGSL entrypoints
require the host to enforce the same validation before dispatch. WGSL finite
math assumptions do not provide a reliable substitute for host validation.

Alpha is copied unchanged, including zero; exposure and tone mapping affect RGB
only. The final `normalize_rgba` byte boundary retains its existing zero-alpha
RGB policy. This means fully transparent float RGB survives the HDR helper but
becomes zero when normalized into the canonical CPU frame.

Choose one output path:

| Helper | Output | Next boundary |
| --- | --- | --- |
| Rust `AgxRecipe::map_linear_rgba`, C++ `map_linear`, WGSL `turninator_agx_linear` | Display-linear Rec.709 RGBA | Hardware sRGB target encoding, or quantize and declare `NativePixels.transfer = Linear` |
| Rust `AgxRecipe::map_srgb_rgba`, C++ `map_srgb`, WGSL `turninator_agx_srgb` | Display-sRGB RGBA, IEC OETF applied once | Quantize `round(channel * 255)` and declare `NativePixels.transfer = Srgb`; use a raw unorm target |

Do not combine the sRGB helper with an sRGB render target. Alpha is never sRGB
encoded. The HDR helper runs before 8-bit quantization; `normalize_rgba` remains
the existing row/channel/alpha/transfer normalizer and performs no HDR mapping.

`AgxRecipe::digest()` hashes length-prefixed version, recipe JSON, WGSL and C++
asset bytes (CRLF normalized to LF), then exposure's little-endian f32 bits
(signed zero canonicalized).
Adapters must include this value **and their own code/shader digest** in pixel
identity. The Rust implementation is covered by the independent frozen chart;
its source is not itself part of this shared asset digest.

The [30-patch chart](../../tests/fixtures/renderers/color-chart.json) was generated
by calling the locked SDK's compiled `AgxToneMapper(NONE)` with explicit
Rec.709/Rec.2020 conversions. It covers neutral/HDR/colored patches, negative
channels, exposure limits and alpha. The polynomial has a small nonzero black
residual (about 0.00000253 linear) and its saturated shoulder is about 0.961
linear. These are frozen approximation behavior, not claims of Blender parity.
Filament's adapter retains `filament-agx-v1`; Blender retains its own AgX.

To qualify the standalone C++ header offline (CMake and a C++17 compiler must
already be installed):

```powershell
cmake -S assets/render/color -B artifacts/color-build -A x64
cmake --build artifacts/color-build --config Release --target color_port
python assets/render/color/compare.py artifacts/color-build/Release/color_port.exe
& artifacts/color-build/Release/color_port.exe --invalid
```

To reproduce the chart, configure with `FILAMENT_SDK` pointing to the locked
Windows SDK and `FILAMENT_SOURCE_HEADERS` to the directory containing the pinned
`filament-ColorSpaceUtils.h`. CMake verifies both the library and header hashes.
Build `color_reference`, compare it with `compare.py`, and only for a deliberate
reference refresh use `compare.py <color_reference.exe> --freeze-reference`.
Nothing downloads a runtime or compiler.

The Rust focused test also parses and validates WGSL using the existing Naga
dependency without creating a GPU. C++ and Rust are numerically compared to the
SDK chart at absolute tolerance 0.00005; actual WGSL GPU execution remains Task 6.
Prepared Blender and rendered scene conformance remain separate Task 4 work.

`recipe.json`, the C++ header, and WGSL also freeze the existing projected layer
filter's 2-to-4-pixel smoothstep, square-root-of-two footprint factor, and
1e-12 mm footprint floor. The CPU `layer_filter_gain` behavior and first/later
layer heights and build coordinates remain as qualified.
