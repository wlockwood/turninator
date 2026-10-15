// Indexed mesh construction (exact-coordinate welding), topology statistics
// and crease-aware corner normals that never smooth across separate shells.

export interface IndexedMesh {
  positions: Float32Array; // unique vertices, xyz
  indices: Uint32Array; // triangles
}

export interface MeshStats {
  triangles: number;
  vertices: number;
  shells: number;
  openEdges: number;
  nonManifoldEdges: number;
  watertight: boolean;
  areaMm2: number;
  /** Signed-tetrahedron volume; meaningful only when watertight. */
  volumeMm3: number;
  min: [number, number, number];
  max: [number, number, number];
}

/** Weld vertices whose float32 coordinates are bit-identical. */
export function weldExact(soup: Float32Array): IndexedMesh {
  const cornerCount = soup.length / 3;
  const bits = new Uint32Array(soup.buffer, soup.byteOffset, soup.length);
  let cap = 1;
  while (cap < cornerCount * 2) cap <<= 1;
  const table = new Int32Array(cap).fill(-1);
  const mask = cap - 1;
  const unique = new Float32Array(soup.length);
  const uniqueBits = new Uint32Array(unique.buffer);
  const indices = new Uint32Array(cornerCount);
  let count = 0;
  for (let c = 0; c < cornerCount; c++) {
    const x = bits[c * 3], y = bits[c * 3 + 1], z = bits[c * 3 + 2];
    let h = (Math.imul(x, 0x9e3779b1) ^ Math.imul(y, 0x85ebca77) ^ Math.imul(z, 0xc2b2ae3d)) >>> 0;
    h ^= h >>> 15;
    let slot = h & mask;
    for (;;) {
      const idx = table[slot];
      if (idx === -1) {
        table[slot] = count;
        uniqueBits[count * 3] = x; uniqueBits[count * 3 + 1] = y; uniqueBits[count * 3 + 2] = z;
        indices[c] = count++;
        break;
      }
      if (uniqueBits[idx * 3] === x && uniqueBits[idx * 3 + 1] === y && uniqueBits[idx * 3 + 2] === z) {
        indices[c] = idx;
        break;
      }
      slot = (slot + 1) & mask;
    }
  }
  return { positions: unique.slice(0, count * 3), indices };
}

class UnionFind {
  parent: Int32Array;
  constructor(n: number) {
    this.parent = new Int32Array(n);
    for (let i = 0; i < n; i++) this.parent[i] = i;
  }
  find(a: number): number {
    const p = this.parent;
    while (p[a] !== a) { p[a] = p[p[a]]; a = p[a]; }
    return a;
  }
  union(a: number, b: number) {
    const ra = this.find(a), rb = this.find(b);
    if (ra !== rb) this.parent[ra] = rb;
  }
}

export interface Topology {
  /** Connected component id per triangle (edge-connected). */
  shellOf: Int32Array;
  shells: number;
  openEdges: number;
  nonManifoldEdges: number;
}

export function analyzeTopology(mesh: IndexedMesh): Topology {
  const triCount = mesh.indices.length / 3;
  const vertCount = mesh.positions.length / 3;
  const edgeFaces = new Map<number, number>(); // edge key -> first face, or -2 after 2nd use
  const edgeUses = new Map<number, number>();
  const uf = new UnionFind(triCount);
  const idx = mesh.indices;
  for (let f = 0; f < triCount; f++) {
    for (let e = 0; e < 3; e++) {
      const a = idx[f * 3 + e], b = idx[f * 3 + ((e + 1) % 3)];
      const key = a < b ? a * vertCount + b : b * vertCount + a;
      const first = edgeFaces.get(key);
      if (first === undefined) edgeFaces.set(key, f);
      else uf.union(first, f);
      edgeUses.set(key, (edgeUses.get(key) ?? 0) + 1);
    }
  }
  let openEdges = 0, nonManifold = 0;
  for (const uses of edgeUses.values()) {
    if (uses === 1) openEdges++;
    else if (uses > 2) nonManifold++;
  }
  const shellOf = new Int32Array(triCount);
  const ids = new Map<number, number>();
  for (let f = 0; f < triCount; f++) {
    const r = uf.find(f);
    let id = ids.get(r);
    if (id === undefined) { id = ids.size; ids.set(r, id); }
    shellOf[f] = id;
  }
  return { shellOf, shells: ids.size, openEdges, nonManifoldEdges: nonManifold };
}

export function computeStats(mesh: IndexedMesh, topo: Topology): MeshStats {
  const p = mesh.positions, idx = mesh.indices;
  let area = 0, volume = 0;
  for (let f = 0; f < idx.length; f += 3) {
    const a = idx[f] * 3, b = idx[f + 1] * 3, c = idx[f + 2] * 3;
    const ax = p[a], ay = p[a + 1], az = p[a + 2];
    const bx = p[b], by = p[b + 1], bz = p[b + 2];
    const cx = p[c], cy = p[c + 1], cz = p[c + 2];
    const ux = bx - ax, uy = by - ay, uz = bz - az;
    const vx = cx - ax, vy = cy - ay, vz = cz - az;
    const nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
    area += 0.5 * Math.hypot(nx, ny, nz);
    volume += (ax * (by * cz - bz * cy) - ay * (bx * cz - bz * cx) + az * (bx * cy - by * cx)) / 6;
  }
  const min: [number, number, number] = [Infinity, Infinity, Infinity];
  const max: [number, number, number] = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < p.length; i += 3) {
    for (let k = 0; k < 3; k++) {
      if (p[i + k] < min[k]) min[k] = p[i + k];
      if (p[i + k] > max[k]) max[k] = p[i + k];
    }
  }
  return {
    triangles: idx.length / 3,
    vertices: p.length / 3,
    shells: topo.shells,
    openEdges: topo.openEdges,
    nonManifoldEdges: topo.nonManifoldEdges,
    watertight: topo.openEdges === 0 && topo.nonManifoldEdges === 0,
    areaMm2: area,
    volumeMm3: Math.abs(volume),
    min,
    max,
  };
}

/**
 * Corner normals for a non-indexed render buffer. A corner averages the
 * angle-weighted normals of faces sharing its vertex, restricted to faces in
 * the same shell whose normal is within `creaseDeg` of the corner's face.
 * creaseDeg <= 0 produces faceted normals.
 */
export function cornerNormals(mesh: IndexedMesh, topo: Topology, creaseDeg: number): Float32Array {
  const p = mesh.positions, idx = mesh.indices;
  const triCount = idx.length / 3;
  const vertCount = p.length / 3;
  const fn = new Float32Array(triCount * 3); // unit face normals
  const angles = new Float32Array(triCount * 3); // corner angles
  for (let f = 0; f < triCount; f++) {
    const ia = idx[f * 3] * 3, ib = idx[f * 3 + 1] * 3, ic = idx[f * 3 + 2] * 3;
    const ux = p[ib] - p[ia], uy = p[ib + 1] - p[ia + 1], uz = p[ib + 2] - p[ia + 2];
    const vx = p[ic] - p[ia], vy = p[ic + 1] - p[ia + 1], vz = p[ic + 2] - p[ia + 2];
    let nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
    const len = Math.hypot(nx, ny, nz) || 1;
    nx /= len; ny /= len; nz /= len;
    fn[f * 3] = nx; fn[f * 3 + 1] = ny; fn[f * 3 + 2] = nz;
    const corners = [ia, ib, ic];
    for (let k = 0; k < 3; k++) {
      const o = corners[k], n1 = corners[(k + 1) % 3], n2 = corners[(k + 2) % 3];
      const ax = p[n1] - p[o], ay = p[n1 + 1] - p[o + 1], az = p[n1 + 2] - p[o + 2];
      const bx = p[n2] - p[o], by = p[n2 + 1] - p[o + 1], bz = p[n2 + 2] - p[o + 2];
      const d = (ax * bx + ay * by + az * bz) / ((Math.hypot(ax, ay, az) * Math.hypot(bx, by, bz)) || 1);
      angles[f * 3 + k] = Math.acos(Math.min(1, Math.max(-1, d)));
    }
  }

  const out = new Float32Array(triCount * 9);
  if (creaseDeg <= 0) {
    for (let f = 0; f < triCount; f++) {
      for (let k = 0; k < 3; k++) {
        out[f * 9 + k * 3] = fn[f * 3]; out[f * 9 + k * 3 + 1] = fn[f * 3 + 1]; out[f * 9 + k * 3 + 2] = fn[f * 3 + 2];
      }
    }
    return out;
  }

  // Vertex -> incident corners (CSR).
  const start = new Uint32Array(vertCount + 1);
  for (let c = 0; c < idx.length; c++) start[idx[c] + 1]++;
  for (let v = 0; v < vertCount; v++) start[v + 1] += start[v];
  const fill = start.slice(0, vertCount);
  const corners = new Uint32Array(idx.length);
  for (let c = 0; c < idx.length; c++) corners[fill[idx[c]]++] = c;

  const cosLimit = Math.cos((creaseDeg * Math.PI) / 180);
  const shell = topo.shellOf;
  for (let c = 0; c < idx.length; c++) {
    const f = (c / 3) | 0;
    const v = idx[c];
    const fx = fn[f * 3], fy = fn[f * 3 + 1], fz = fn[f * 3 + 2];
    let sx = 0, sy = 0, sz = 0;
    for (let j = start[v]; j < start[v + 1]; j++) {
      const oc = corners[j];
      const g = (oc / 3) | 0;
      if (shell[g] !== shell[f]) continue;
      const gx = fn[g * 3], gy = fn[g * 3 + 1], gz = fn[g * 3 + 2];
      if (g !== f && fx * gx + fy * gy + fz * gz < cosLimit) continue;
      const w = angles[oc];
      sx += gx * w; sy += gy * w; sz += gz * w;
    }
    const len = Math.hypot(sx, sy, sz);
    if (len > 1e-12) { sx /= len; sy /= len; sz /= len; } else { sx = fx; sy = fy; sz = fz; }
    out[c * 3] = sx; out[c * 3 + 1] = sy; out[c * 3 + 2] = sz;
  }
  return out;
}

/** Expand an indexed mesh into a triangle soup for rendering. */
export function expand(mesh: IndexedMesh): Float32Array {
  const out = new Float32Array(mesh.indices.length * 3);
  for (let c = 0; c < mesh.indices.length; c++) {
    const v = mesh.indices[c] * 3;
    out[c * 3] = mesh.positions[v]; out[c * 3 + 1] = mesh.positions[v + 1]; out[c * 3 + 2] = mesh.positions[v + 2];
  }
  return out;
}
