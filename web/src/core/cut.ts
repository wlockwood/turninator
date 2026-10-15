// Cut a closed triangle soup at a horizontal plane and cap the opening, for
// path-traced build-up of STL models (the raster engine uses a clip plane).
// Works on any closed render geometry (original or Sliced).

import ClipperLib from 'clipper-lib';
import { SCALE, TriBuffer, triangulateTree, union } from './slicer';

export interface CutResult {
  positions: Float32Array;
  normals: Float32Array;
  triangles: number;
}

/**
 * Keep the part of `positions` (build frame, 9 floats per triangle) at or
 * below `z`, with per-corner `normals`, and add an upward-facing cap.
 */
export function cutBelow(positions: Float32Array, normals: Float32Array, z: number): CutResult {
  const out = new TriBuffer(50_000_000);
  const segments: [number, number, number, number][] = [];
  const P = (i: number, k: number) => positions[i * 3 + k];
  const N = (i: number, k: number) => normals[i * 3 + k];
  // Edge-plane intersection computed in a canonical endpoint order, so the two
  // triangles sharing an edge produce bit-identical points.
  const cross = (a: number, b: number): { p: number[]; n: number[] } => {
    const swap = P(a, 0) > P(b, 0) || (P(a, 0) === P(b, 0) && (P(a, 1) > P(b, 1) || (P(a, 1) === P(b, 1) && P(a, 2) > P(b, 2))));
    const [u, v] = swap ? [b, a] : [a, b];
    const t = (z - P(u, 2)) / (P(v, 2) - P(u, 2));
    return {
      p: [0, 1, 2].map((k) => P(u, k) + t * (P(v, k) - P(u, k))),
      n: [0, 1, 2].map((k) => N(u, k) + t * (N(v, k) - N(u, k))),
    };
  };
  const tri = (a: { p: number[]; n: number[] }, b: { p: number[]; n: number[] }, c: { p: number[]; n: number[] }) =>
    out.tri(a.p[0], a.p[1], a.p[2], b.p[0], b.p[1], b.p[2], c.p[0], c.p[1], c.p[2], a.n, b.n, c.n);
  const vert = (i: number) => ({ p: [P(i, 0), P(i, 1), P(i, 2)], n: [N(i, 0), N(i, 1), N(i, 2)] });

  const triCount = positions.length / 9;
  for (let f = 0; f < triCount; f++) {
    const ids = [f * 3, f * 3 + 1, f * 3 + 2];
    const below = ids.map((i) => P(i, 2) <= z);
    const nBelow = below.filter(Boolean).length;
    if (nBelow === 3) { tri(vert(ids[0]), vert(ids[1]), vert(ids[2])); continue; }
    if (nBelow === 0) continue;
    // Rotate so the lone vertex (below if one, above if two) comes first.
    const lone = nBelow === 1 ? below.indexOf(true) : below.indexOf(false);
    const [a, b, c] = [ids[lone], ids[(lone + 1) % 3], ids[(lone + 2) % 3]];
    const ab = cross(a, b), ac = cross(a, c);
    if (nBelow === 1) {
      tri(vert(a), ab, ac);
    } else {
      tri(ab, vert(b), vert(c));
      tri(ab, vert(c), ac);
    }
    // Cap boundary segment; direction is irrelevant for a non-zero union of loops
    // closed by chaining, so record endpoints only.
    segments.push([ab.p[0], ab.p[1], ac.p[0], ac.p[1]]);
  }

  // Chain segments into loops by exact endpoint keys.
  const key = (x: number, y: number) => `${Math.round(x * SCALE)},${Math.round(y * SCALE)}`;
  const adj = new Map<string, number[]>();
  segments.forEach((s, i) => {
    for (const k of [key(s[0], s[1]), key(s[2], s[3])]) {
      const l = adj.get(k);
      if (l) l.push(i); else adj.set(k, [i]);
    }
  });
  const used = new Uint8Array(segments.length);
  const paths: ClipperLib.Paths = [];
  for (let s0 = 0; s0 < segments.length; s0++) {
    if (used[s0]) continue;
    used[s0] = 1;
    const path: ClipperLib.Path = [];
    const startKey = key(segments[s0][0], segments[s0][1]);
    let endX = segments[s0][2], endY = segments[s0][3];
    path.push({ X: Math.round(segments[s0][0] * SCALE), Y: Math.round(segments[s0][1] * SCALE) });
    for (let guard = 0; guard < segments.length; guard++) {
      const k = key(endX, endY);
      path.push({ X: Math.round(endX * SCALE), Y: Math.round(endY * SCALE) });
      if (k === startKey) break;
      const nextIdx = adj.get(k)?.find((i) => !used[i]);
      if (nextIdx === undefined) break;
      used[nextIdx] = 1;
      const s = segments[nextIdx];
      const forward = key(s[0], s[1]) === k;
      endX = forward ? s[2] : s[0];
      endY = forward ? s[3] : s[1];
    }
    if (path.length >= 3) paths.push(path);
  }
  // Even-odd: loop orientation from chaining is arbitrary, nesting is not.
  const c = new ClipperLib.Clipper();
  c.AddPaths(paths, ClipperLib.PolyType.ptSubject, true);
  const tree = new ClipperLib.PolyTree();
  c.Execute(ClipperLib.ClipType.ctUnion, tree, ClipperLib.PolyFillType.pftEvenOdd, ClipperLib.PolyFillType.pftEvenOdd);
  triangulateTree(tree, z, true, out);
  void union;

  return { positions: Float32Array.from(out.pos), normals: Float32Array.from(out.nrm), triangles: out.count };
}
