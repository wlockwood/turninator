// Stepped-envelope slicer: an "as printed" approximation of an STL.
//
// For each layer [b_k, b_k+1] the mesh is sliced at mid-height, contours are
// chained topologically by mesh edge and unioned (non-zero fill) into region
// R_k. Walls are vertical strips around R_k; treads are R_k \ R_k+1 (facing
// up) and R_k \ R_k-1 (facing down). Runs of identical layers share one wall
// strip. This estimates a sliced solid; it is not a toolpath (no beads,
// perimeters or infill) and does not repair broken meshes.

import ClipperLib from 'clipper-lib';
import earcut from 'earcut';

export const SCALE = 10_000; // Clipper integer units per mm (0.1 µm)

export interface SliceInput {
  /** Welded vertices in the build frame (mm, Z up, min Z = 0). */
  positions: Float32Array;
  indices: Uint32Array;
  /** Edge-connected shell id per triangle; contours are chained per shell. */
  shellOf?: Int32Array;
  /** Layer boundaries starting at 0. */
  boundaries: Float64Array;
  creaseDeg: number;
  /** Contour cleaning distance (mm); removes near-duplicate points only. */
  cleanMm: number;
  maxTriangles: number;
}

export interface SliceResult {
  /** Non-indexed triangles in the build frame, 9 floats per triangle. */
  positions: Float32Array;
  normals: Float32Array;
  triangles: number;
  layers: number;
  wallStrips: number;
  openChains: number;
  emptyLayers: number;
  warnings: string[];
  ms: number;
}

export class SliceError extends Error {}

type Paths = ClipperLib.Paths;

export class TriBuffer {
  pos: number[] = [];
  nrm: number[] = [];
  count = 0;
  constructor(private max: number) {}
  tri(ax: number, ay: number, az: number, bx: number, by: number, bz: number, cx: number, cy: number, cz: number, na: number[], nb: number[], nc: number[]) {
    if (++this.count > this.max) {
      throw new SliceError(`Sliced mesh exceeds ${this.max.toLocaleString()} triangles; use a larger layer height or Shaded mode`);
    }
    this.pos.push(ax, ay, az, bx, by, bz, cx, cy, cz);
    this.nrm.push(na[0], na[1], na[2], nb[0], nb[1], nb[2], nc[0], nc[1], nc[2]);
  }
}

/** Contour loops of the mesh at height z, oriented outer-CCW by face normals. */
export function sliceContours(pos: Float32Array, idx: Uint32Array, tris: Int32Array, z: number, shellOf?: Int32Array): { paths: Paths; open: number } {
  const V = pos.length / 3;
  const startKey: number[] = [], endKey: number[] = [], segShell: number[] = [];
  const sx: number[] = [], sy: number[] = [];
  const point = (a: number, b: number): [number, number, number] => {
    const lo = Math.min(a, b), hi = Math.max(a, b);
    const zl = pos[lo * 3 + 2], zh = pos[hi * 3 + 2];
    const t = (z - zl) / (zh - zl);
    return [pos[lo * 3] + t * (pos[hi * 3] - pos[lo * 3]), pos[lo * 3 + 1] + t * (pos[hi * 3 + 1] - pos[lo * 3 + 1]), lo * V + hi];
  };
  for (let i = 0; i < tris.length; i++) {
    const f = tris[i];
    const a = idx[f * 3], b = idx[f * 3 + 1], c = idx[f * 3 + 2];
    const va = [a, b, c];
    const above = va.map((v) => pos[v * 3 + 2] > z);
    if (above[0] === above[1] && above[1] === above[2]) continue;
    const pts: [number, number, number][] = [];
    for (let e = 0; e < 3; e++) {
      const p = va[e], q = va[(e + 1) % 3];
      if (above[e] !== above[(e + 1) % 3]) pts.push(point(p, q));
    }
    if (pts.length !== 2) continue;
    // Face normal (x, y components suffice for orientation).
    const ux = pos[b * 3] - pos[a * 3], uy = pos[b * 3 + 1] - pos[a * 3 + 1], uz = pos[b * 3 + 2] - pos[a * 3 + 2];
    const vx = pos[c * 3] - pos[a * 3], vy = pos[c * 3 + 1] - pos[a * 3 + 1], vz = pos[c * 3 + 2] - pos[a * 3 + 2];
    const nx = uy * vz - uz * vy, ny = uz * vx - ux * vz;
    let [p1, p2] = pts;
    const dx = p2[0] - p1[0], dy = p2[1] - p1[1];
    // Outward normal must lie to the right of travel: cross(dir, up)·n > 0.
    if (dy * nx - dx * ny < 0) [p1, p2] = [p2, p1];
    startKey.push(p1[2]); endKey.push(p2[2]); segShell.push(shellOf ? shellOf[f] : 0);
    sx.push(p1[0]); sy.push(p1[1]);
  }
  // Chain within each shell so touching shells that share welded vertices
  // never splice into each other's loops.
  const groups = new Map<number, number[]>();
  for (let s = 0; s < startKey.length; s++) {
    let g = groups.get(segShell[s]);
    if (!g) groups.set(segShell[s], (g = []));
    g.push(s);
  }
  const used = new Uint8Array(startKey.length);
  const paths: Paths = [];
  let open = 0;
  for (const group of groups.values()) {
  // Several segments can start at one key on non-manifold edges; any pairing
  // yields loops whose non-zero union is the correct region.
  const byStart = new Map<number, number[]>();
  for (const s of group) {
    const list = byStart.get(startKey[s]);
    if (list) list.push(s);
    else byStart.set(startKey[s], [s]);
  }
  const takeNext = (key: number): number | undefined => {
    const list = byStart.get(key);
    if (!list) return undefined;
    while (list.length && used[list[list.length - 1]]) list.pop();
    return list.length ? list[list.length - 1] : undefined;
  };
  for (const s of group) {
    if (used[s]) continue;
    const path: ClipperLib.Path = [];
    let cur = s;
    let closed = false;
    for (let guard = 0; guard <= startKey.length; guard++) {
      used[cur] = 1;
      path.push({ X: Math.round(sx[cur] * SCALE), Y: Math.round(sy[cur] * SCALE) });
      if (endKey[cur] === startKey[s]) { closed = true; break; }
      const next = takeNext(endKey[cur]);
      if (next === undefined) break;
      cur = next;
    }
    if (!closed) open++;
    if (path.length >= 3) paths.push(path);
  }
  }
  return { paths, open };
}

export function union(paths: Paths, cleanUnits: number): Paths {
  if (paths.length === 0) return [];
  const c = new ClipperLib.Clipper();
  c.AddPaths(paths, ClipperLib.PolyType.ptSubject, true);
  const out: Paths = [];
  c.Execute(ClipperLib.ClipType.ctUnion, out, ClipperLib.PolyFillType.pftNonZero, ClipperLib.PolyFillType.pftNonZero);
  return ClipperLib.Clipper.CleanPolygons(out, cleanUnits).filter((p) => p.length >= 3);
}

function difference(a: Paths, b: Paths): ClipperLib.PolyTree {
  const c = new ClipperLib.Clipper();
  const tree = new ClipperLib.PolyTree();
  if (a.length) c.AddPaths(a, ClipperLib.PolyType.ptSubject, true);
  if (b.length) c.AddPaths(b, ClipperLib.PolyType.ptClip, true);
  c.Execute(ClipperLib.ClipType.ctDifference, tree, ClipperLib.PolyFillType.pftNonZero, ClipperLib.PolyFillType.pftNonZero);
  return tree;
}

const MIN_AREA_UNITS = 1e-4 * SCALE * SCALE; // 1e-4 mm²
/** Layers merge into one wall strip only if they match the strip's first region this closely. */
const MERGE_AREA_UNITS = 1e-6 * SCALE * SCALE; // 1e-6 mm²
/** Treads grow under adjoining walls by this much to seal T-junction cracks (invisible). */
const SEAL_UNITS = 0.002 * SCALE; // 2 µm

function offsetPaths(paths: Paths, delta: number): Paths {
  if (paths.length === 0) return [];
  const co = new ClipperLib.ClipperOffset(2, 0.25);
  co.AddPaths(paths, ClipperLib.JoinType.jtMiter, ClipperLib.EndType.etClosedPolygon);
  const out: Paths = [];
  co.Execute(out, delta);
  return out;
}

function pathsArea(p: Paths): number {
  let a = 0;
  for (const path of p) a += ClipperLib.Clipper.Area(path);
  return Math.abs(a);
}

function sameRegion(a: Paths, b: Paths, areaA = pathsArea(a), areaB = pathsArea(b)): boolean {
  // |area(a) - area(b)| bounds the symmetric difference from below: cheap reject.
  if (Math.abs(areaA - areaB) >= MERGE_AREA_UNITS) return false;
  return treeArea(difference(a, b)) < MERGE_AREA_UNITS && treeArea(difference(b, a)) < MERGE_AREA_UNITS;
}

function treeArea(node: ClipperLib.PolyNode): number {
  let a = 0;
  for (const ch of node.Childs()) {
    a += Math.abs(ClipperLib.Clipper.Area(ch.Contour())) * (ch.IsHole() ? -1 : 1);
    a += treeArea(ch);
  }
  return a;
}

export function triangulateTree(node: ClipperLib.PolyNode, z: number, up: boolean, out: TriBuffer) {
  const n = up ? [0, 0, 1] : [0, 0, -1];
  for (const outer of node.Childs()) {
    if (outer.IsHole()) continue;
    const contour = outer.Contour();
    if (Math.abs(ClipperLib.Clipper.Area(contour)) < MIN_AREA_UNITS) {
      for (const hole of outer.Childs()) triangulateTree(hole, z, up, out);
      continue;
    }
    const flat: number[] = [];
    const holes: number[] = [];
    for (const p of contour) flat.push(p.X / SCALE, p.Y / SCALE);
    for (const hole of outer.Childs()) {
      holes.push(flat.length / 2);
      for (const p of hole.Contour()) flat.push(p.X / SCALE, p.Y / SCALE);
    }
    const t = earcut(flat, holes, 2);
    for (let i = 0; i < t.length; i += 3) {
      let a = t[i], b = t[i + 1], c = t[i + 2];
      const area = (flat[b * 2] - flat[a * 2]) * (flat[c * 2 + 1] - flat[a * 2 + 1]) - (flat[c * 2] - flat[a * 2]) * (flat[b * 2 + 1] - flat[a * 2 + 1]);
      if ((area > 0) !== up) [b, c] = [c, b];
      out.tri(flat[a * 2], flat[a * 2 + 1], z, flat[b * 2], flat[b * 2 + 1], z, flat[c * 2], flat[c * 2 + 1], z, n, n, n);
    }
    for (const hole of outer.Childs()) triangulateTree(hole, z, up, out);
  }
}

function emitWalls(paths: Paths, z0: number, z1: number, cosCrease: number, out: TriBuffer) {
  for (const path of paths) {
    const m = path.length;
    const px = path.map((p) => p.X / SCALE), py = path.map((p) => p.Y / SCALE);
    // Edge normals: right of travel (outward for CCW outers and CW holes).
    const enx: number[] = [], eny: number[] = [];
    for (let i = 0; i < m; i++) {
      const j = (i + 1) % m;
      const dx = px[j] - px[i], dy = py[j] - py[i];
      const l = Math.hypot(dx, dy) || 1;
      enx.push(dy / l); eny.push(-dx / l);
    }
    // Vertex normals with crease splitting.
    const vertexNormal = (edge: number, vertex: number): number[] => {
      const other = vertex === edge ? (edge - 1 + m) % m : (edge + 1) % m;
      const d = enx[edge] * enx[other] + eny[edge] * eny[other];
      if (d < cosCrease) return [enx[edge], eny[edge], 0];
      const x = enx[edge] + enx[other], y = eny[edge] + eny[other];
      const l = Math.hypot(x, y) || 1;
      return [x / l, y / l, 0];
    };
    for (let i = 0; i < m; i++) {
      const j = (i + 1) % m;
      const ni = vertexNormal(i, i), nj = vertexNormal(i, j);
      out.tri(px[i], py[i], z0, px[j], py[j], z0, px[j], py[j], z1, ni, nj, nj);
      out.tri(px[i], py[i], z0, px[j], py[j], z1, px[i], py[i], z1, ni, nj, ni);
    }
  }
}

export function sliceMesh(input: SliceInput, onProgress?: (done: number, total: number) => void): SliceResult {
  const t0 = performance.now();
  const { positions: pos, indices: idx, boundaries: b } = input;
  const layers = b.length - 1;
  const triCount = idx.length / 3;
  const warnings: string[] = [];

  // Sort triangles by min Z for a sweeping active set.
  const zmin = new Float64Array(triCount), zmax = new Float64Array(triCount);
  for (let f = 0; f < triCount; f++) {
    const za = pos[idx[f * 3] * 3 + 2], zb = pos[idx[f * 3 + 1] * 3 + 2], zc = pos[idx[f * 3 + 2] * 3 + 2];
    zmin[f] = Math.min(za, zb, zc); zmax[f] = Math.max(za, zb, zc);
  }
  const order = Array.from({ length: triCount }, (_, i) => i).sort((p, q) => zmin[p] - zmin[q]);
  let next = 0;
  let active: number[] = [];

  const cleanUnits = Math.max(1, input.cleanMm * SCALE);
  const regions: Paths[] = [];
  let openChains = 0, emptyLayers = 0;
  for (let k = 0; k < layers; k++) {
    const z = (b[k] + b[k + 1]) / 2;
    while (next < triCount && zmin[order[next]] <= z) active.push(order[next++]);
    active = active.filter((f) => zmax[f] > z);
    const { paths, open } = sliceContours(pos, idx, Int32Array.from(active), z, input.shellOf);
    openChains += open;
    const r = union(paths, cleanUnits);
    if (r.length === 0) emptyLayers++;
    regions.push(r);
    if (onProgress && k % 8 === 0) onProgress(k, layers * 2);
  }
  if (openChains > 0) warnings.push(`${openChains} contour(s) did not close (mesh is not watertight); they were closed straight across`);

  const out = new TriBuffer(input.maxTriangles);
  const cosCrease = Math.cos((input.creaseDeg * Math.PI) / 180);
  let wallStrips = 0;
  let groupStart = 0;
  const areas = regions.map(pathsArea);
  let prevSameAbove = false;
  for (let k = 0; k < layers; k++) {
    const r = regions[k];
    const above = k + 1 < layers ? regions[k + 1] : [];
    const below = k > 0 ? regions[k - 1] : [];
    // Treads exist only where neighbouring layers differ. They are grown
    // slightly over this layer's wall and under the neighbour's so wall/tread
    // seams (T-junctions) cannot open hairline cracks.
    const sameAbove = k + 1 < layers && sameRegion(r, above, areas[k], areas[k + 1]);
    const sameBelow = k > 0 && prevSameAbove;
    prevSameAbove = sameAbove;
    const grown = sameAbove && sameBelow ? [] : offsetPaths(r, SEAL_UNITS);
    if (!sameAbove) triangulateTree(difference(grown, offsetPaths(above, -SEAL_UNITS)), b[k + 1], true, out);
    if (!sameBelow) triangulateTree(difference(grown, offsetPaths(below, -SEAL_UNITS)), b[k], false, out);
    // Merge walls while the next layer has an identical region.
    // Compare with the strip's first layer (not just the previous one) so small
    // per-layer drift cannot accumulate into a wall that misses its treads.
    const nextSame = sameAbove && (groupStart === k || sameRegion(regions[groupStart], above, areas[groupStart], areas[k + 1]));
    if (!nextSame) {
      emitWalls(regions[groupStart], b[groupStart], b[k + 1], cosCrease, out);
      wallStrips++;
      groupStart = k + 1;
    }
    if (onProgress && k % 8 === 0) onProgress(layers + k, layers * 2);
  }

  return {
    positions: Float32Array.from(out.pos),
    normals: Float32Array.from(out.nrm),
    triangles: out.count,
    layers,
    wallStrips,
    openChains,
    emptyLayers,
    warnings,
    ms: performance.now() - t0,
  };
}
