import { describe, expect, it } from 'vitest';
import { readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { parseStl } from '../src/core/stl';
import { weldExact, analyzeTopology, computeStats } from '../src/core/mesh';
import { uniformSchedule } from '../src/core/layers';
import { sliceMesh } from '../src/core/slicer';
import { binaryStl, box, type Tri } from './helpers';

function slice(tris: Tri[], h = 0.2, first = h) {
  const parsed = parseStl(binaryStl(tris));
  const mesh = weldExact(parsed.positions);
  const schedule = uniformSchedule(h, first, parsed.size[2]);
  const shellOf = analyzeTopology(mesh).shellOf;
  return sliceMesh({ positions: mesh.positions, indices: mesh.indices, shellOf, boundaries: schedule.boundaries, creaseDeg: 30, cleanMm: 0.001, maxTriangles: 5_000_000 });
}

function soupVolume(p: Float32Array): number {
  let v = 0;
  for (let i = 0; i < p.length; i += 9) {
    const [ax, ay, az, bx, by, bz, cx, cy, cz] = p.subarray(i, i + 9);
    v += (ax * (by * cz - bz * cy) - ay * (bx * cz - bz * cx) + az * (bx * cy - by * cx)) / 6;
  }
  return v;
}

function wedge(): Tri[] {
  // Right-triangle prism: x 0..10, y 0..4, z rises from 0 at x=0 to 10 at x=10.
  const A = [0, 0, 0], B = [10, 0, 0], C = [10, 0, 10], D = [0, 4, 0], E = [10, 4, 0], F = [10, 4, 10];
  const t = (p: number[], q: number[], r: number[]) => [...p, ...q, ...r] as Tri;
  return [
    t(A, C, B), t(D, E, F), // sides
    t(A, B, E), t(A, E, D), // bottom
    t(B, C, F), t(B, F, E), // back wall x=10
    t(A, D, F), t(A, F, C), // slope
  ];
}

describe('stepped slicer', () => {
  it('collapses a prism into one wall strip with exact volume', () => {
    const r = slice(box(0, 0, 0, 10, 20, 5));
    expect(r.layers).toBe(25);
    expect(r.wallStrips).toBe(1);
    expect(r.triangles).toBe(12);
    expect(Math.abs(soupVolume(r.positions)) / 1000).toBeCloseTo(1, 3); // 2 µm seal lips
    expect(r.openChains).toBe(0);
  });

  it('turns a slope into stairs with treads facing up', () => {
    const r = slice(wedge(), 0.5);
    expect(r.layers).toBe(20);
    expect(r.wallStrips).toBe(20);
    let upTreads = 0;
    for (let i = 0; i < r.normals.length; i += 9) if (r.normals[i + 2] > 0.99) upTreads++;
    expect(upTreads).toBeGreaterThanOrEqual(20 * 2);
    // Mid-layer slicing of a 45° slope preserves volume (200 mm³) exactly.
    // Tread seals (2 µm lips) add a hair of volume.
    expect(Math.abs(soupVolume(r.positions)) / 200).toBeCloseTo(1, 2);
  });

  it('keeps holes as holes', () => {
    const outer = box(0, 0, 0, 20, 20, 2);
    // Frame: subtract a central square by building four bars.
    const frame = [...box(0, 0, 0, 20, 5, 2), ...box(0, 15, 0, 20, 20, 2), ...box(0, 5, 0, 5, 15, 2), ...box(15, 5, 0, 20, 15, 2)];
    const r = slice(frame, 0.2);
    expect(Math.abs(soupVolume(r.positions)) / 600).toBeCloseTo(1, 2);
    expect(outer.length).toBe(12);
  });
});

const egg = join(__dirname, '..', '..', 'test-models', 'spiral egg - A.stl');
describe.skipIf(!existsSync(egg))('real model slicing', () => {
  it('slices the spiral egg at 0.2 mm and stays close to the source volume', () => {
    const f = readFileSync(egg);
    const parsed = parseStl(f.buffer.slice(f.byteOffset, f.byteOffset + f.byteLength) as ArrayBuffer);
    const mesh = weldExact(parsed.positions);
    const stats = computeStats(mesh, analyzeTopology(mesh));
    const s = uniformSchedule(0.2, 0.2, parsed.size[2]);
    const r = sliceMesh({ positions: mesh.positions, indices: mesh.indices, boundaries: s.boundaries, creaseDeg: 30, cleanMm: 0.001, maxTriangles: 10_000_000 });
    const v = Math.abs(soupVolume(r.positions));
    console.log(`egg: ${r.layers} layers, ${r.triangles} tris, ${r.wallStrips} strips, ${r.ms.toFixed(0)} ms, volume ${v.toFixed(0)} vs ${stats.volumeMm3.toFixed(0)}`);
    expect(Math.abs(v - stats.volumeMm3) / stats.volumeMm3).toBeLessThan(0.02);
  });
});

import { cutBelow } from '../src/core/cut';
import { expand, cornerNormals } from '../src/core/mesh';

describe('build-up cut', () => {
  it('cuts a box at half height into a closed half box', () => {
    const mesh = weldExact(parseStl(binaryStl(box(0, 0, 0, 10, 20, 8))).positions);
    const soup = expand(mesh);
    const normals = cornerNormals(mesh, analyzeTopology(mesh), 30);
    const r = cutBelow(soup, normals, 4);
    expect(Math.abs(soupVolume(r.positions))).toBeCloseTo(800, 3);
    let maxZ = 0;
    for (let i = 2; i < r.positions.length; i += 3) maxZ = Math.max(maxZ, r.positions[i]);
    expect(maxZ).toBeCloseTo(4, 6);
  });

  it('cuts the spiral egg and keeps the cut volume closed', () => {
    if (!existsSync(egg)) return;
    const f = readFileSync(egg);
    const mesh = weldExact(parseStl(f.buffer.slice(f.byteOffset, f.byteOffset + f.byteLength) as ArrayBuffer).positions);
    const r = cutBelow(expand(mesh), cornerNormals(mesh, analyzeTopology(mesh), 30), 20);
    const full = computeStats(mesh, analyzeTopology(mesh)).volumeMm3;
    const v = Math.abs(soupVolume(r.positions));
    expect(v).toBeGreaterThan(full * 0.2);
    expect(v).toBeLessThan(full * 0.8);
    // A closed result has the same volume however it is measured: compare with a shifted origin.
    const shifted = r.positions.map((x, i) => (i % 3 === 0 ? x + 50 : x));
    expect(Math.abs(soupVolume(shifted))).toBeCloseTo(v, 0);
  });
});
