// Looping GIF encoding with one animation-wide palette and ordered (Bayer)
// dithering. Ordered dithering is position-stable, so static regions do not
// shimmer between frames the way error diffusion can.

import { GIFEncoder, quantize, type Palette } from 'gifenc';

export type Dither = 'none' | 'bayer';

export interface GifOptions {
  width: number;
  height: number;
  /** Per-frame delay in milliseconds; GIF stores centiseconds. */
  delayMs: number;
  dither: Dither;
  /** Dither amplitude in 8-bit units. */
  ditherStrength: number;
  maxColors: number;
}

const BAYER8 = (() => {
  const m = [
    [0, 32, 8, 40, 2, 34, 10, 42], [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38], [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41], [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37], [63, 31, 55, 23, 61, 29, 53, 21],
  ];
  const out = new Float32Array(64);
  for (let y = 0; y < 8; y++) for (let x = 0; x < 8; x++) out[y * 8 + x] = (m[y][x] + 0.5) / 64 - 0.5;
  return out;
})();

/** Random access to frames that may live in memory or on disk (OPFS). */
export interface FrameSource {
  count: number;
  get(i: number): Uint8ClampedArray;
}

export const arraySource = (frames: Uint8ClampedArray[]): FrameSource => ({ count: frames.length, get: (i) => frames[i] });

/** Build one palette from a strided sample of every frame. */
export function buildPalette(source: FrameSource | Uint8ClampedArray[], maxColors = 256, samplePixels = 1_500_000): Palette {
  const src = Array.isArray(source) ? arraySource(source) : source;
  // All frames share one size; avoid touching every frame just to count.
  const total = src.count > 0 ? (src.count * src.get(0).length) / 4 : 0;
  const stride = Math.max(1, Math.floor(total / samplePixels));
  const sample = new Uint8Array(Math.ceil(total / stride) * 4 + 4);
  let n = 0;
  let k = 0;
  for (let fi = 0; fi < src.count; fi++) {
    const f = src.get(fi);
    for (let p = 0; p < f.length; p += 4, k++) {
      if (k % stride !== 0) continue;
      sample[n++] = f[p]; sample[n++] = f[p + 1]; sample[n++] = f[p + 2]; sample[n++] = 255;
    }
  }
  return quantize(sample.subarray(0, n), maxColors, { format: 'rgb444' });
}

/** 64^3 nearest-colour lookup table (6 bits per channel). */
export function buildLut(palette: Palette): Uint8Array {
  const lut = new Uint8Array(64 * 64 * 64);
  const pr = palette.map((c) => c[0]), pg = palette.map((c) => c[1]), pb = palette.map((c) => c[2]);
  for (let r = 0; r < 64; r++) {
    const rv = r * 4 + 2;
    for (let g = 0; g < 64; g++) {
      const gv = g * 4 + 2;
      for (let b = 0; b < 64; b++) {
        const bv = b * 4 + 2;
        let best = 0, bestD = Infinity;
        for (let i = 0; i < palette.length; i++) {
          const dr = rv - pr[i], dg = gv - pg[i], db = bv - pb[i];
          const d = 2 * dr * dr + 4 * dg * dg + 3 * db * db;
          if (d < bestD) { bestD = d; best = i; }
        }
        lut[(r << 12) | (g << 6) | b] = best;
      }
    }
  }
  return lut;
}

export function indexFrame(rgba: Uint8ClampedArray, width: number, height: number, lut: Uint8Array, dither: Dither, strength: number): Uint8Array {
  const out = new Uint8Array(width * height);
  const amp = dither === 'bayer' ? strength : 0;
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const i = y * width + x;
      const o = i * 4;
      const t = amp ? BAYER8[(y & 7) * 8 + (x & 7)] * amp : 0;
      const r = Math.min(255, Math.max(0, rgba[o] + t)) >> 2;
      const g = Math.min(255, Math.max(0, rgba[o + 1] + t)) >> 2;
      const b = Math.min(255, Math.max(0, rgba[o + 2] + t)) >> 2;
      out[i] = lut[(r << 12) | (g << 6) | b];
    }
  }
  return out;
}

export function encodeGif(source: FrameSource | Uint8ClampedArray[], opts: GifOptions, onProgress?: (done: number) => void): Uint8Array {
  const src = Array.isArray(source) ? arraySource(source) : source;
  const palette = buildPalette(src, opts.maxColors);
  const lut = buildLut(palette);
  const gif = GIFEncoder();
  for (let i = 0; i < src.count; i++) {
    const index = indexFrame(src.get(i), opts.width, opts.height, lut, opts.dither, opts.ditherStrength);
    // Palette on the first frame becomes the global colour table; later
    // frames reuse it (no local tables). repeat 0 = loop forever.
    gif.writeFrame(index, opts.width, opts.height, i === 0 ? { palette, delay: opts.delayMs, repeat: 0 } : { delay: opts.delayMs });
    onProgress?.(i + 1);
  }
  gif.finish();
  return gif.bytes();
}
