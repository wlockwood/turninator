// Physical layer schedule shared by layer appearance, slicing and G-code.
// Matches final-plan.md: boundaries 0, h0, h0+h, ...; heights 0.02-2.00 mm;
// at most 50,000 layers.

export const MIN_LAYER_MM = 0.02;
export const MAX_LAYER_MM = 2.0;
export const MAX_LAYERS = 50_000;

export class LayerError extends Error {}

export interface LayerSchedule {
  /** Strictly increasing boundaries starting at 0; layer i spans [b[i], b[i+1]]. */
  boundaries: Float64Array;
  uniformHeight: number | null;
  firstHeight: number | null;
}

function checkHeight(h: number, label: string) {
  if (!Number.isFinite(h) || h < MIN_LAYER_MM || h > MAX_LAYER_MM) {
    throw new LayerError(`${label} must be between ${MIN_LAYER_MM} and ${MAX_LAYER_MM} mm (got ${h})`);
  }
}

export function uniformSchedule(heightMm: number, firstMm: number, buildHeightMm: number): LayerSchedule {
  checkHeight(heightMm, 'Layer height');
  checkHeight(firstMm, 'First layer height');
  if (!Number.isFinite(buildHeightMm) || buildHeightMm <= 0) throw new LayerError('Build height must be positive');
  const count = buildHeightMm <= firstMm ? 1 : 1 + Math.ceil((buildHeightMm - firstMm) / heightMm - 1e-9);
  if (count > MAX_LAYERS) throw new LayerError(`${count} layers exceeds the ${MAX_LAYERS} layer limit`);
  const b = new Float64Array(count + 1);
  b[0] = 0;
  b[1] = firstMm;
  for (let i = 2; i <= count; i++) b[i] = firstMm + (i - 1) * heightMm;
  return { boundaries: b, uniformHeight: heightMm, firstHeight: firstMm };
}

export function explicitSchedule(boundaries: number[]): LayerSchedule {
  if (boundaries.length < 2 || boundaries[0] !== 0) throw new LayerError('Boundaries must start at 0 and contain at least one layer');
  if (boundaries.length - 1 > MAX_LAYERS) throw new LayerError(`Too many layers (${boundaries.length - 1})`);
  for (let i = 1; i < boundaries.length; i++) {
    const h = boundaries[i] - boundaries[i - 1];
    if (!(h > 0)) throw new LayerError('Boundaries must be strictly increasing');
    checkHeight(Number(h.toFixed(6)), `Layer ${i} height`);
  }
  return { boundaries: Float64Array.from(boundaries), uniformHeight: null, firstHeight: null };
}

/** Index of the layer containing z (clamped to the schedule). */
export function layerIndexAt(s: LayerSchedule, z: number): number {
  const b = s.boundaries;
  if (z <= b[0]) return 0;
  if (z >= b[b.length - 1]) return b.length - 2;
  let lo = 0, hi = b.length - 1;
  while (hi - lo > 1) {
    const mid = (lo + hi) >> 1;
    if (b[mid] <= z) lo = mid; else hi = mid;
  }
  return lo;
}

export function layerCount(s: LayerSchedule): number {
  return s.boundaries.length - 1;
}

export interface AdaptiveOptions {
  minMm: number;
  maxMm: number;
  /** Maximum stair-step cusp height in mm (smaller = finer on shallow slopes). */
  cuspMm: number;
  firstMm: number;
}

/**
 * Slicer-style adaptive layers: each layer is as thick as possible (up to
 * max) while the stair-step cusp h * |n_z| on any sloped facet crossing it
 * stays under `cuspMm`. Near-horizontal facets (|n_z| > 0.98) are ignored;
 * they are flat tops, not slopes.
 */
export function adaptiveSchedule(positions: Float32Array, indices: Uint32Array, o: AdaptiveOptions): LayerSchedule {
  checkHeight(o.minMm, 'Minimum layer height');
  checkHeight(o.maxMm, 'Maximum layer height');
  checkHeight(o.firstMm, 'First layer height');
  if (!(o.cuspMm > 0)) throw new LayerError('Cusp height must be positive');
  const triCount = indices.length / 3;
  const zmin = new Float64Array(triCount), zmax = new Float64Array(triCount), nz = new Float64Array(triCount);
  let top = 0;
  for (let f = 0; f < triCount; f++) {
    const a = indices[f * 3] * 3, b = indices[f * 3 + 1] * 3, c = indices[f * 3 + 2] * 3;
    const ux = positions[b] - positions[a], uy = positions[b + 1] - positions[a + 1], uz = positions[b + 2] - positions[a + 2];
    const vx = positions[c] - positions[a], vy = positions[c + 1] - positions[a + 1], vz = positions[c + 2] - positions[a + 2];
    const x = uy * vz - uz * vy, y = uz * vx - ux * vz, z = ux * vy - uy * vx;
    const len = Math.hypot(x, y, z) || 1;
    nz[f] = Math.abs(z / len);
    zmin[f] = Math.min(positions[a + 2], positions[b + 2], positions[c + 2]);
    zmax[f] = Math.max(positions[a + 2], positions[b + 2], positions[c + 2]);
    top = Math.max(top, zmax[f]);
  }
  // Facets sorted by zmin; sweep an active window like the slicer does.
  const order = Array.from({ length: triCount }, (_, i) => i).filter((f) => nz[f] <= 0.98 && nz[f] > 1e-3).sort((p, q) => zmin[p] - zmin[q]);
  const b: number[] = [0, Math.min(o.firstMm, top)];
  let z = b[1];
  let next = 0;
  let active: number[] = [];
  while (z < top - 1e-6) {
    let h = o.maxMm;
    for (let iter = 0; iter < 4; iter++) {
      while (next < order.length && zmin[order[next]] < z + h) active.push(order[next++]);
      active = active.filter((f) => zmax[f] > z);
      let limit = o.maxMm;
      for (const f of active) if (zmin[f] < z + h) limit = Math.min(limit, o.cuspMm / nz[f]);
      const nh = Math.max(o.minMm, Math.min(o.maxMm, limit));
      if (nh >= h - 1e-9) break;
      h = nh;
    }
    z = Math.min(top, z + h);
    // Avoid a sliver final layer below the minimum.
    if (top - z > 0 && top - z < o.minMm) z = top;
    b.push(z);
    if (b.length - 1 > MAX_LAYERS) throw new LayerError(`Adaptive schedule exceeds ${MAX_LAYERS} layers`);
  }
  return { boundaries: Float64Array.from(b), uniformHeight: null, firstHeight: o.firstMm };
}

/** Boundaries from G-code layer tops (mm, build frame with min Z = 0). */
export function scheduleFromLayerTops(layerTops: number[]): LayerSchedule {
  const tops = [...layerTops].filter((z) => z > 0).sort((a, b) => a - b);
  const b = [0];
  for (const z of tops) if (z - b[b.length - 1] >= MIN_LAYER_MM * 0.5) b.push(z);
  if (b.length < 2) throw new LayerError('G-code has no layers');
  if (b.length - 1 > MAX_LAYERS) throw new LayerError(`Too many layers (${b.length - 1})`);
  return { boundaries: Float64Array.from(b), uniformHeight: null, firstHeight: b[1] };
}
