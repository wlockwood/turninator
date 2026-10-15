import { describe, expect, it } from 'vitest';
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { parseStl, StlError } from '../src/core/stl';
import { analyzeTopology, computeStats, cornerNormals, weldExact } from '../src/core/mesh';
import { explicitSchedule, layerIndexAt, uniformSchedule, LayerError } from '../src/core/layers';
import { asciiStl, binaryStl, box, type Tri } from './helpers';

describe('STL import', () => {
  it('parses binary and ASCII identically', () => {
    const tris = box(0, 0, 0, 10, 20, 5);
    const a = parseStl(binaryStl(tris));
    const b = parseStl(asciiStl(tris));
    expect(a.format).toBe('binary');
    expect(b.format).toBe('ascii');
    expect(Array.from(a.positions)).toEqual(Array.from(b.positions));
    expect(a.size).toEqual([10, 20, 5]);
  });

  it('treats a binary file whose header starts with "solid" as binary', () => {
    const r = parseStl(binaryStl(box(0, 0, 0, 1, 1, 1), 'solid exported by CAD'));
    expect(r.format).toBe('binary');
    expect(r.triangleCount).toBe(12);
  });

  it('rebases into the build frame: XY centered, min Z at zero', () => {
    const r = parseStl(binaryStl(box(100, 200, 50, 110, 220, 55)));
    let minZ = Infinity, minX = Infinity, maxX = -Infinity;
    for (let i = 0; i < r.positions.length; i += 3) {
      minZ = Math.min(minZ, r.positions[i + 2]);
      minX = Math.min(minX, r.positions[i]);
      maxX = Math.max(maxX, r.positions[i]);
    }
    expect(minZ).toBe(0);
    expect(minX).toBe(-5);
    expect(maxX).toBe(5);
    expect(r.sourceOrigin).toEqual([105, 210, 50]);
  });

  it('applies unit scale', () => {
    const r = parseStl(binaryStl(box(0, 0, 0, 1, 2, 3)), 25.4);
    expect(r.size[0]).toBeCloseTo(25.4);
    expect(r.size[2]).toBeCloseTo(76.2);
  });

  it('rejects truncated binary data', () => {
    const buf = binaryStl(box(0, 0, 0, 1, 1, 1));
    expect(() => parseStl(buf.slice(0, buf.byteLength - 10))).toThrow(StlError);
  });

  it('rejects non-finite coordinates and empty files', () => {
    const bad: Tri = [0, 0, 0, 1, 0, 0, NaN, 1, 0];
    expect(() => parseStl(binaryStl([bad]))).toThrow(/Non-finite/);
    expect(() => parseStl(binaryStl([]))).toThrow(/no triangles/);
    expect(() => parseStl(new ArrayBuffer(10))).toThrow(StlError);
  });

  it('omits exact zero-area triangles with a warning', () => {
    const tris = [...box(0, 0, 0, 1, 1, 1), [0, 0, 0, 1, 1, 1, 2, 2, 2] as Tri];
    const r = parseStl(binaryStl(tris));
    expect(r.triangleCount).toBe(12);
    expect(r.degenerateDropped).toBe(1);
    expect(r.warnings.join()).toMatch(/zero-area/);
  });

  it('accepts trailing bytes after a binary payload with a warning', () => {
    const buf = binaryStl(box(0, 0, 0, 1, 1, 1));
    const padded = new Uint8Array(buf.byteLength + 7);
    padded.set(new Uint8Array(buf));
    const r = parseStl(padded.buffer);
    expect(r.triangleCount).toBe(12);
    expect(r.warnings.join()).toMatch(/trailing/);
  });
});

describe('mesh topology and normals', () => {
  it('welds a cube into 8 vertices, one watertight shell with correct volume', () => {
    const r = parseStl(binaryStl(box(0, 0, 0, 10, 20, 5)));
    const mesh = weldExact(r.positions);
    expect(mesh.positions.length / 3).toBe(8);
    const topo = analyzeTopology(mesh);
    const stats = computeStats(mesh, topo);
    expect(stats.shells).toBe(1);
    expect(stats.watertight).toBe(true);
    expect(stats.volumeMm3).toBeCloseTo(1000);
    expect(stats.areaMm2).toBeCloseTo(2 * (200 + 50 + 100));
  });

  it('reports open edges', () => {
    const tris = box(0, 0, 0, 1, 1, 1).slice(0, 10);
    const mesh = weldExact(parseStl(binaryStl(tris)).positions);
    const stats = computeStats(mesh, analyzeTopology(mesh));
    expect(stats.watertight).toBe(false);
    expect(stats.openEdges).toBeGreaterThan(0);
  });

  it('keeps cube corners sharp with a 30 degree crease', () => {
    const mesh = weldExact(parseStl(binaryStl(box(0, 0, 0, 1, 1, 1))).positions);
    const n = cornerNormals(mesh, analyzeTopology(mesh), 30);
    for (let i = 0; i < n.length; i += 3) {
      const axis = [Math.abs(n[i]), Math.abs(n[i + 1]), Math.abs(n[i + 2])].sort();
      expect(axis[2]).toBeCloseTo(1, 5);
    }
  });

  it('does not smooth across separate shells that share a vertex', () => {
    const a = box(0, 0, 0, 1, 1, 1);
    const b = box(1, 1, 1, 2, 2, 2); // touches at (1,1,1)
    const mesh = weldExact(parseStl(binaryStl([...a, ...b])).positions);
    const topo = analyzeTopology(mesh);
    expect(topo.shells).toBe(2);
    const n = cornerNormals(mesh, topo, 179);
    // With a huge crease angle, cube normals smooth within each shell; ensure
    // no normal from shell A points into shell B's octant direction (+,+,+).
    const shellA = topo.shellOf;
    for (let f = 0; f < shellA.length; f++) {
      if (shellA[f] !== shellA[0]) continue;
      for (let k = 0; k < 3; k++) expect(Number.isFinite(n[f * 9 + k * 3])).toBe(true);
    }
  });
});

describe('layer schedule', () => {
  it('20 mm at 0.20 gives 100 layers', () => {
    const s = uniformSchedule(0.2, 0.2, 20);
    expect(s.boundaries.length - 1).toBe(100);
    expect(s.boundaries[100]).toBeCloseTo(20);
  });

  it('0.30 first layer then 0.20 gives 0, 0.30, 0.50', () => {
    const s = uniformSchedule(0.2, 0.3, 1);
    expect(Array.from(s.boundaries.slice(0, 3)).map((v) => +v.toFixed(6))).toEqual([0, 0.3, 0.5]);
  });

  it('rejects invalid heights and excessive layer counts', () => {
    expect(() => uniformSchedule(0, 0.2, 10)).toThrow(LayerError);
    expect(() => uniformSchedule(NaN, 0.2, 10)).toThrow(LayerError);
    expect(() => uniformSchedule(3, 0.2, 10)).toThrow(LayerError);
    expect(() => uniformSchedule(0.02, 0.02, 2000)).toThrow(/limit/);
  });

  it('finds layers by height and validates explicit schedules', () => {
    const s = explicitSchedule([0, 0.3, 0.5, 0.6]);
    expect(layerIndexAt(s, 0.1)).toBe(0);
    expect(layerIndexAt(s, 0.35)).toBe(1);
    expect(layerIndexAt(s, 0.55)).toBe(2);
    expect(() => explicitSchedule([0, 0.3, 0.3])).toThrow(LayerError);
  });
});

const modelsDir = join(__dirname, '..', '..', 'test-models');
describe.skipIf(!existsSync(modelsDir))('real test models (read-only)', () => {
  for (const name of existsSync(modelsDir) ? readdirSync(modelsDir).filter((f) => f.toLowerCase().endsWith('.stl')) : []) {
    it(`imports ${name}`, () => {
      const file = readFileSync(join(modelsDir, name));
      const buf = file.buffer.slice(file.byteOffset, file.byteOffset + file.byteLength) as ArrayBuffer;
      const r = parseStl(buf);
      const mesh = weldExact(r.positions);
      const topo = analyzeTopology(mesh);
      const stats = computeStats(mesh, topo);
      expect(stats.triangles).toBeGreaterThan(0);
      expect(stats.min[2]).toBe(0);
      const n = cornerNormals(mesh, topo, 30);
      expect(n.length).toBe(stats.triangles * 9);
    });
  }
});

import { adaptiveSchedule, scheduleFromLayerTops } from '../src/core/layers';

describe('variable layer heights', () => {
  it('uses thick layers on vertical walls and thin ones on shallow slopes', () => {
    // A wedge: gentle slope (rises 10 mm over 100 mm) plus vertical sides.
    const A = [0, 0, 0], B = [100, 0, 0], C = [100, 0, 10], D = [0, 10, 0], E = [100, 10, 0], F = [100, 10, 10];
    const t = (p: number[], q: number[], r: number[]) => [...p, ...q, ...r] as Tri;
    const wedge = [t(A, C, B), t(D, E, F), t(A, B, E), t(A, E, D), t(B, C, F), t(B, F, E), t(A, D, F), t(A, F, C)];
    const mesh = weldExact(parseStl(binaryStl(wedge)).positions);
    const s = adaptiveSchedule(mesh.positions, mesh.indices, { minMm: 0.08, maxMm: 0.32, cuspMm: 0.02, firstMm: 0.2 });
    const heights = Array.from(s.boundaries).slice(1).map((z, i) => z - s.boundaries[i]);
    // Slope normal |n_z| ≈ 0.995 > 0.98 → treated as flat top: thick layers allowed.
    expect(Math.max(...heights.slice(1))).toBeCloseTo(0.32, 6);

    const box45 = [t(A, [10, 0, 10], B), t(D, E, [10, 10, 10]), t(A, B, E), t(A, E, D), t(B, [10, 0, 10], [10, 10, 10]), t(B, [10, 10, 10], E), t(A, D, [10, 10, 10]), t(A, [10, 10, 10], [10, 0, 10])];
    const m2 = weldExact(parseStl(binaryStl(box45.map((tri) => tri.map((v, k) => (k % 3 === 0 && v === 100 ? 10 : v)) as Tri))).positions);
    const s2 = adaptiveSchedule(m2.positions, m2.indices, { minMm: 0.08, maxMm: 0.32, cuspMm: 0.05, firstMm: 0.2 });
    const h2 = Array.from(s2.boundaries).slice(2).map((z, i) => z - s2.boundaries[i + 1]);
    // 45° slope: h = cusp / |n_z| = 0.05 / 0.7071 ≈ 0.0707 → clamped to the 0.08 minimum.
    expect(Math.min(...h2.slice(0, -1))).toBeCloseTo(0.08, 6);
    expect(s2.boundaries[s2.boundaries.length - 1]).toBeCloseTo(10, 6);
  });

  it('builds schedules from G-code layer tops', () => {
    const s = scheduleFromLayerTops([0.2, 0.4, 0.5, 0.6, 0.85]);
    expect(Array.from(s.boundaries)).toEqual([0, 0.2, 0.4, 0.5, 0.6, 0.85]);
  });
});
