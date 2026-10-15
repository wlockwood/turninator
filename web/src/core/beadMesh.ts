// Bake G-code beads into ordinary indexed triangle meshes (for the path
// tracer and GLB export). Uses the same bead formula as the instanced shader
// in render/beads.ts at a lower resolution, and splits the result into
// layer-range chunks so build-up animation can reveal chunks without
// rebuilding acceleration structures.

export interface BeadSource {
  count: number;
  start: Float32Array;
  end: Float32Array;
  width: Float32Array;
  height: Float32Array;
  layer: Uint32Array;
  feature: Uint8Array;
  tool: Uint8Array;
}

export interface BakeOptions {
  /** Bit i set = feature id i is skipped. */
  hiddenMask: number;
  /** Linear RGB per vertex, or null for material colour. */
  colorOf: ((i: number) => [number, number, number]) | null;
  chunks: number;
  layerCount: number;
  /** Cross-section and cap resolution. */
  np?: number;
  nt?: number;
  maxTriangles?: number;
}

export interface BakedChunk {
  /** Highest layer index contained in this chunk. */
  maxLayer: number;
  positions: Float32Array;
  normals: Float32Array;
  colors: Float32Array | null;
  index: Uint32Array;
}

interface Template {
  /** Per template vertex: end flag, theta, lateral w coeff, lateral h coeff, vertical h coeff, normal angle. */
  verts: [number, number, number, number, number, number][];
  tris: number[];
}

function buildTemplate(np: number, nt: number): Template {
  const prof: [number, number, number, number][] = [[0, 0, -0.5, -Math.PI / 2]];
  for (let i = 0; i <= np; i++) {
    const phi = -Math.PI / 2 + (Math.PI * i) / np;
    prof.push([0.5, -0.5 + 0.5 * Math.cos(phi), 0.5 * Math.sin(phi), phi]);
  }
  prof.push([0, 0, 0.5, Math.PI / 2]);
  const P = prof.length;
  const verts: Template['verts'] = [];
  for (let end = 0; end < 2; end++) {
    for (let j = 0; j <= nt; j++) {
      const theta = -Math.PI / 2 + (Math.PI * j) / nt;
      for (const p of prof) verts.push([end, theta, p[0], p[1], p[2], p[3]]);
    }
  }
  const idx = (end: number, j: number, i: number) => end * (nt + 1) * P + j * P + i;
  const tris: number[] = [];
  const quad = (a: number, b: number, c: number, d: number) => tris.push(a, b, c, a, c, d);
  for (let end = 0; end < 2; end++) for (let j = 0; j < nt; j++) for (let i = 0; i < P - 1; i++) quad(idx(end, j, i), idx(end, j + 1, i), idx(end, j + 1, i + 1), idx(end, j, i + 1));
  for (let i = 0; i < P - 1; i++) {
    quad(idx(1, nt, i), idx(0, 0, i), idx(0, 0, i + 1), idx(1, nt, i + 1));
    quad(idx(0, nt, i), idx(1, 0, i), idx(1, 0, i + 1), idx(0, nt, i + 1));
  }
  // Fix winding on a canonical bead (start origin, end +X, w = 1, h = 0.5).
  const pos = verts.map((v) => evalVertex(v, 0, 0, 0, 1, 0, 0, 1, 0.5).p);
  for (let t = 0; t < tris.length; t += 3) {
    const [a, b, c] = [pos[tris[t]], pos[tris[t + 1]], pos[tris[t + 2]]];
    const ux = b[0] - a[0], uy = b[1] - a[1], uz = b[2] - a[2];
    const vx = c[0] - a[0], vy = c[1] - a[1], vz = c[2] - a[2];
    const nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
    const cx = (a[0] + b[0] + c[0]) / 3, cy = (a[1] + b[1] + c[1]) / 3, cz = (a[2] + b[2] + c[2]) / 3;
    const ax = Math.min(Math.max(cx, 0), 1);
    if (nx * (cx - ax) + ny * cy + nz * cz < 0) [tris[t + 1], tris[t + 2]] = [tris[t + 2], tris[t + 1]];
  }
  return { verts, tris };
}

function evalVertex(v: Template['verts'][number], sx: number, sy: number, sz: number, ex: number, ey: number, ez: number, w: number, h: number) {
  let dx = ex - sx, dy = ey - sy;
  const L = Math.hypot(dx, dy);
  if (L > 1e-9) { dx /= L; dy /= L; } else { dx = 1; dy = 0; }
  const [endFlag, theta, lw, lh, vh, nAng] = v;
  const ox = endFlag ? dx : -dx, oy = endFlag ? dy : -dy;
  // radial = outward*cos(theta) + (up x outward)*sin(theta); up x (ox,oy,0) = (-oy, ox, 0)
  const rx = ox * Math.cos(theta) - oy * Math.sin(theta);
  const ry = oy * Math.cos(theta) + ox * Math.sin(theta);
  const lateral = lw * w + lh * h;
  const bx = endFlag ? ex : sx, by = endFlag ? ey : sy, bz = endFlag ? ez : sz;
  return {
    p: [bx + rx * lateral, by + ry * lateral, bz + vh * h] as [number, number, number],
    n: [rx * Math.cos(nAng), ry * Math.cos(nAng), Math.sin(nAng)] as [number, number, number],
  };
}

export class BakeError extends Error {}

export function bakeBeads(src: BeadSource, o: BakeOptions): BakedChunk[] {
  const tpl = buildTemplate(o.np ?? 2, o.nt ?? 2);
  const V = tpl.verts.length;
  const T = tpl.tris.length / 3;
  const chunks = Math.max(1, Math.min(o.chunks, o.layerCount));
  const chunkOfLayer = (l: number) => Math.min(chunks - 1, Math.floor((l * chunks) / Math.max(1, o.layerCount)));
  const counts = new Array(chunks).fill(0);
  for (let i = 0; i < src.count; i++) {
    if (o.hiddenMask & (1 << Math.min(src.feature[i], 31))) continue;
    counts[chunkOfLayer(src.layer[i])]++;
  }
  const total = counts.reduce((a, b) => a + b, 0) * T;
  if (total > (o.maxTriangles ?? 20_000_000)) {
    throw new BakeError(`Baked toolpath would have ${total.toLocaleString()} triangles; hide more features (e.g. infill) to path trace it`);
  }
  const out: BakedChunk[] = counts.map((c, k) => ({
    maxLayer: Math.max(0, Math.ceil(((k + 1) * o.layerCount) / chunks) - 1),
    positions: new Float32Array(c * V * 3),
    normals: new Float32Array(c * V * 3),
    colors: o.colorOf ? new Float32Array(c * V * 3) : null,
    index: new Uint32Array(c * T * 3),
  }));
  const fill = new Array(chunks).fill(0);
  for (let i = 0; i < src.count; i++) {
    if (o.hiddenMask & (1 << Math.min(src.feature[i], 31))) continue;
    const k = chunkOfLayer(src.layer[i]);
    const ch = out[k];
    const seg = fill[k]++;
    const vb = seg * V;
    const sx = src.start[i * 3], sy = src.start[i * 3 + 1], sz = src.start[i * 3 + 2];
    const ex = src.end[i * 3], ey = src.end[i * 3 + 1], ez = src.end[i * 3 + 2];
    const col = o.colorOf ? o.colorOf(i) : null;
    for (let v = 0; v < V; v++) {
      const r = evalVertex(tpl.verts[v], sx, sy, sz, ex, ey, ez, src.width[i], src.height[i]);
      const o3 = (vb + v) * 3;
      ch.positions[o3] = r.p[0]; ch.positions[o3 + 1] = r.p[1]; ch.positions[o3 + 2] = r.p[2];
      ch.normals[o3] = r.n[0]; ch.normals[o3 + 1] = r.n[1]; ch.normals[o3 + 2] = r.n[2];
      if (col && ch.colors) { ch.colors[o3] = col[0]; ch.colors[o3 + 1] = col[1]; ch.colors[o3 + 2] = col[2]; }
    }
    const ib = seg * T * 3;
    for (let t = 0; t < T * 3; t++) ch.index[ib + t] = vb + tpl.tris[t];
  }
  return out.filter((c) => c.index.length > 0);
}

/** Features that are normally hidden inside an opaque part. */
export const INTERIOR_FEATURES = new Set(['Inner wall', 'Sparse infill', 'Solid infill', 'Gap fill']);
