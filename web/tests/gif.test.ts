import { describe, expect, it } from 'vitest';
import { encodeGif } from '../src/export/gif';

interface GifInfo {
  width: number;
  height: number;
  frames: number;
  delaysCs: number[];
  loop: number | null;
  localTables: number;
  globalTable: boolean;
}

/** Minimal GIF block walker for verifying timing/loop metadata. */
function inspectGif(b: Uint8Array): GifInfo {
  const ascii = String.fromCharCode(...b.subarray(0, 6));
  if (ascii !== 'GIF89a') throw new Error('not GIF89a');
  const width = b[6] | (b[7] << 8), height = b[8] | (b[9] << 8);
  const packed = b[10];
  const globalTable = (packed & 0x80) !== 0;
  let p = 13;
  if (globalTable) p += 3 * (1 << ((packed & 7) + 1));
  const info: GifInfo = { width, height, frames: 0, delaysCs: [], loop: null, localTables: 0, globalTable };
  const skipSub = () => { while (b[p] !== 0) p += b[p] + 1; p++; };
  for (;;) {
    const t = b[p++];
    if (t === 0x3b) break;
    if (t === 0x21) {
      const label = b[p++];
      if (label === 0xf9) {
        info.delaysCs.push(b[p + 2] | (b[p + 3] << 8));
        p += b[p] + 1;
        skipSub();
      } else if (label === 0xff) {
        const id = String.fromCharCode(...b.subarray(p + 1, p + 12));
        p += b[p] + 1;
        if (id === 'NETSCAPE2.0') info.loop = b[p + 2] | (b[p + 3] << 8);
        skipSub();
      } else {
        p += b[p] + 1;
        skipSub();
      }
    } else if (t === 0x2c) {
      const lp = b[p + 8];
      p += 9;
      if (lp & 0x80) { info.localTables++; p += 3 * (1 << ((lp & 7) + 1)); }
      p++; // LZW min code size
      skipSub();
      info.frames++;
    } else {
      throw new Error(`unexpected block 0x${t.toString(16)} at ${p - 1}`);
    }
  }
  return info;
}

function frame(w: number, h: number, shade: number): Uint8ClampedArray {
  const f = new Uint8ClampedArray(w * h * 4);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const o = (y * w + x) * 4;
      f[o] = (x * 255) / w; f[o + 1] = (y * 255) / h; f[o + 2] = shade; f[o + 3] = 255;
    }
  }
  return f;
}

describe('GIF encoding', () => {
  it('writes 80 uniformly timed frames, infinite loop and one global palette', () => {
    const frames = Array.from({ length: 80 }, (_, i) => frame(32, 24, i * 3));
    const bytes = encodeGif(frames, { width: 32, height: 24, delayMs: 50, dither: 'bayer', ditherStrength: 10, maxColors: 256 });
    const info = inspectGif(bytes);
    expect(info.width).toBe(32);
    expect(info.height).toBe(24);
    expect(info.frames).toBe(80);
    expect(info.delaysCs.length).toBe(80);
    expect(new Set(info.delaysCs)).toEqual(new Set([5]));
    expect(info.delaysCs.reduce((a, b) => a + b, 0)).toBe(400); // 4.00 s
    expect(info.loop).toBe(0);
    expect(info.globalTable).toBe(true);
    expect(info.localTables).toBe(0);
  });
});
