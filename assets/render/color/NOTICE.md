# Shared HDR color recipe notices

`src/render/color.rs` (HDR portion), `turninator_agx.h`, `turninator_agx.wgsl`,
and `recipe.json` adapt Filament's AgxToneMapper (NONE look) and color matrices:

Copyright (C) 2021 The Android Open Source Project (ToneMapper.cpp)
Copyright (C) 2020 The Android Open Source Project (ColorSpaceUtils.h)

Licensed under Apache License 2.0; see LICENSE-Apache-2.0.txt.
Source: https://github.com/google/filament/tree/c1dc4e444784850d6dc891a1fb38747c30b85463

Files and SHA-256 at that commit (v1.78.0):

- filament/src/ToneMapper.cpp: `1e3212b67f2954721a4336c68fef1904204873835896e25c9ba77f9030aa42cd`
- filament/src/ColorSpaceUtils.h: `0e1b0434f4ebf2358703cab569175e5905b2912553d61925f6fcad817ddc4ad0`

Changes: Rust/C++/WGSL ports; explicit linear Rec.709 to/from Filament's linear
Rec.2020; frozen row matrices for CPU/C++; validation, exposure, independent
straight alpha, output gamut clamp, optional IEC sRGB encoding, recipe identity.
The SDK qualification program calls the compiled SDK implementation directly.

Filament attributes its AgX contrast polynomial to Minimal AgX by Benjamin
Wrensch: https://iolite-engine.com/blog_posts/minimal_agx_implementation
Retain the upstream MIT notice for this adapted ancestry:

Copyright (c) 2024 Missing Deadlines (Benjamin Wrensch)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

Filament's inset/outset matrix comments credit EaryChow's AgX_LUT_Gen and
Troy Sobotka's AgX work. This port copies no Blender source, OCIO configuration,
LUT, or GPL implementation; its implementation source is the Apache-licensed
Filament files above. These notices must accompany redistributed adapters.
