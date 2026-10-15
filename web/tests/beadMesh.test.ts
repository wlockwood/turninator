import { describe, expect, it } from 'vitest';
import { bakeBeads, type BeadSource } from '../src/core/beadMesh';

function source(n: number, layers: number[]): BeadSource {
  const start = new Float32Array(n * 3), end = new Float32Array(n * 3);
  for (let i = 0; i < n; i++) {
    start.set([0, i, layers[i] * 0.2 + 0.1], i * 3);
    end.set([10, i, layers[i] * 0.2 + 0.1], i * 3);
  }
  return {
    count: n, start, end,
    width: new Float32Array(n).fill(0.45), height: new Float32Array(n).fill(0.2),
    layer: Uint32Array.from(layers), feature: new Uint8Array(n), tool: new Uint8Array(n),
  };
}

function signedVolume(p: Float32Array, idx: Uint32Array) {
  let v = 0;
  for (let t = 0; t < idx.length; t += 3) {
    const a = idx[t] * 3, b = idx[t + 1] * 3, c = idx[t + 2] * 3;
    v += (p[a] * (p[b + 1] * p[c + 2] - p[b + 2] * p[c + 1]) - p[a + 1] * (p[b] * p[c + 2] - p[b + 2] * p[c]) + p[a + 2] * (p[b] * p[c + 1] - p[b + 1] * p[c])) / 6;
  }
  return v;
}

describe('bead baking', () => {
  it('produces an outward-wound closed bead close to the stadium volume', () => {
    const [chunk] = bakeBeads(source(1, [0]), { hiddenMask: 0, colorOf: null, chunks: 1, layerCount: 1, np: 6, nt: 6 });
    const v = signedVolume(chunk.positions, chunk.index);
    const stadium = 0.2 * (0.45 - 0.2) + (Math.PI * 0.2 * 0.2) / 4;
    expect(v).toBeGreaterThan(0);
    expect(v).toBeGreaterThan(stadium * 10 * 0.9);
    expect(v).toBeLessThan(stadium * 10 * 1.15); // includes rounded end caps
  });

  it('splits by layer into chunks and skips hidden features', () => {
    const src = source(4, [0, 1, 2, 3]);
    src.feature[3] = 2;
    const chunks = bakeBeads(src, { hiddenMask: 1 << 2, colorOf: () => [1, 0, 0], chunks: 2, layerCount: 4 });
    expect(chunks.length).toBe(2);
    expect(chunks[0].maxLayer).toBe(1);
    expect(chunks[1].maxLayer).toBe(3);
    const t0 = chunks[0].index.length, t1 = chunks[1].index.length;
    expect(t0).toBe(t1 * 2); // second chunk lost its hidden bead
    expect(chunks[0].colors![0]).toBe(1);
  });
});
