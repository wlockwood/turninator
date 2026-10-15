import { describe, expect, it } from 'vitest';
import { detectEdges, scaleBarMm, sheetLayout } from '../src/export/fourview';

describe('four-view layout', () => {
  it('matches the 4096 sheet spec: 1952 x 1856 panel images', () => {
    const l = sheetLayout(4096);
    expect(l.imageW).toBe(1952);
    expect(l.imageH).toBe(1856);
    expect(l.origins).toEqual([[64, 64], [2080, 64], [64, 2080], [2080, 2080]]);
  });

  it('scales proportionally for 2048', () => {
    const l = sheetLayout(2048);
    expect(l.margin).toBe(32);
    expect(l.imageW).toBe(976);
    expect(l.imageH).toBe(928);
  });

  it('picks 1/2/5 scale bars', () => {
    expect(scaleBarMm(0.05, 1952)).toBe(10);
    expect(scaleBarMm(0.2, 1952)).toBe(50);
    expect(scaleBarMm(0.01, 976)).toBe(1);
  });
});

describe('edge detection', () => {
  function buffer(w: number, h: number, fill: (x: number, y: number) => [number, number, number, number]) {
    const b = new Float32Array(w * h * 4);
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) b.set(fill(x, y), (y * w + x) * 4);
    return b;
  }
  const opts = { normalThreshold: 0.1, depthThresholdMm: 0.5, radius: 0 };

  it('finds silhouettes but not the interior of a flat plane', () => {
    const nd = buffer(20, 20, (x) => (x < 10 ? [0, 0, 1, 5] : [0, 0, 0, -1]));
    const e = detectEdges(nd, 20, 20, opts);
    expect(e[10 * 20 + 9]).toBe(1);
    expect(e[10 * 20 + 4]).toBe(0);
  });

  it('ignores a smoothly sloped plane but finds a crease', () => {
    const slope = buffer(20, 20, (x) => [0, 0, 1, x * 0.8]);
    expect(Array.from(detectEdges(slope, 20, 20, opts)).some((v) => v > 0)).toBe(false);
    const crease = buffer(20, 20, (x) => (x < 10 ? [0, 0, 1, 5] : [0.7071, 0, 0.7071, 5]));
    expect(detectEdges(crease, 20, 20, opts)[10 * 20 + 9]).toBe(1);
  });
});
