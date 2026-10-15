// STL import: binary/ASCII detection by exact size, finite checks, and
// rebasing into the physical build frame (mm, XY centered, min Z = 0).

export const MAX_TRIANGLES = 20_000_000;

export interface StlResult {
  /** Triangle soup in build-frame millimetres, 9 floats per triangle. */
  positions: Float32Array;
  triangleCount: number;
  format: 'binary' | 'ascii';
  /** Source-unit offset that was subtracted before scaling (f64). */
  sourceOrigin: [number, number, number];
  /** Bounds in millimetres after rebasing. */
  size: [number, number, number];
  degenerateDropped: number;
  warnings: string[];
}

export class StlError extends Error {}

function isAsciiCandidate(bytes: Uint8Array): boolean {
  let i = 0;
  while (i < bytes.length && i < 512 && (bytes[i] === 0x20 || bytes[i] === 0x09 || bytes[i] === 0x0a || bytes[i] === 0x0d)) i++;
  const head = String.fromCharCode(...bytes.subarray(i, i + 5)).toLowerCase();
  if (head !== 'solid') return false;
  // A binary file may also start with "solid"; require a facet keyword nearby.
  const probe = new TextDecoder('ascii').decode(bytes.subarray(0, Math.min(bytes.length, 4096))).toLowerCase();
  return probe.includes('facet') || probe.includes('endsolid');
}

function readBinary(buffer: ArrayBuffer, count: number): Float64Array {
  const view = new DataView(buffer);
  const out = new Float64Array(count * 9);
  let offset = 84;
  for (let t = 0; t < count; t++) {
    offset += 12; // facet normal is recomputed, never trusted
    for (let k = 0; k < 9; k++) {
      out[t * 9 + k] = view.getFloat32(offset, true);
      offset += 4;
    }
    offset += 2; // attribute byte count
  }
  return out;
}

function readAscii(buffer: ArrayBuffer): Float64Array {
  const text = new TextDecoder('utf-8').decode(buffer);
  const re = /vertex\s+(\S+)\s+(\S+)\s+(\S+)/gi;
  const values: number[] = [];
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    values.push(Number(m[1]), Number(m[2]), Number(m[3]));
    if (values.length / 9 > MAX_TRIANGLES) throw new StlError(`ASCII STL exceeds ${MAX_TRIANGLES} triangles`);
  }
  if (values.length % 9 !== 0) throw new StlError('ASCII STL has a vertex count that is not a multiple of three');
  return Float64Array.from(values);
}

export function parseStl(buffer: ArrayBuffer, mmPerUnit = 1): StlResult {
  if (!(mmPerUnit > 0) || !Number.isFinite(mmPerUnit)) throw new StlError('Unit scale must be a finite positive number');
  const bytes = new Uint8Array(buffer);
  const warnings: string[] = [];
  let raw: Float64Array;
  let format: 'binary' | 'ascii';

  const declared = bytes.length >= 84 ? new DataView(buffer).getUint32(80, true) : -1;
  const exactBinary = declared >= 0 && 84 + 50 * declared === bytes.length;

  if (exactBinary) {
    if (declared > MAX_TRIANGLES) throw new StlError(`STL declares ${declared} triangles; limit is ${MAX_TRIANGLES}`);
    raw = readBinary(buffer, declared);
    format = 'binary';
  } else if (isAsciiCandidate(bytes)) {
    raw = readAscii(buffer);
    format = 'ascii';
  } else if (declared >= 0 && declared <= MAX_TRIANGLES && 84 + 50 * declared < bytes.length) {
    raw = readBinary(buffer, declared);
    format = 'binary';
    warnings.push(`Binary STL has ${bytes.length - (84 + 50 * declared)} trailing bytes; ignored`);
  } else if (declared >= 0) {
    throw new StlError(`Binary STL is truncated or corrupt: declares ${declared} triangles but holds ${Math.max(0, Math.floor((bytes.length - 84) / 50))}`);
  } else {
    throw new StlError('File is too small to be an STL');
  }

  const triCount = raw.length / 9;
  if (triCount === 0) throw new StlError('STL contains no triangles');

  let minX = Infinity, minY = Infinity, minZ = Infinity;
  let maxX = -Infinity, maxY = -Infinity, maxZ = -Infinity;
  for (let i = 0; i < raw.length; i += 3) {
    const x = raw[i], y = raw[i + 1], z = raw[i + 2];
    if (!Number.isFinite(x) || !Number.isFinite(y) || !Number.isFinite(z)) {
      throw new StlError(`Non-finite coordinate in triangle ${Math.floor(i / 9)}`);
    }
    if (x < minX) minX = x; if (x > maxX) maxX = x;
    if (y < minY) minY = y; if (y > maxY) maxY = y;
    if (z < minZ) minZ = z; if (z > maxZ) maxZ = z;
  }

  const ox = (minX + maxX) / 2, oy = (minY + maxY) / 2, oz = minZ;
  const kept = new Float32Array(raw.length);
  let out = 0;
  let degenerate = 0;
  for (let t = 0; t < triCount; t++) {
    const b = t * 9;
    const ax = (raw[b] - ox) * mmPerUnit, ay = (raw[b + 1] - oy) * mmPerUnit, az = (raw[b + 2] - oz) * mmPerUnit;
    const bx = (raw[b + 3] - ox) * mmPerUnit, by = (raw[b + 4] - oy) * mmPerUnit, bz = (raw[b + 5] - oz) * mmPerUnit;
    const cx = (raw[b + 6] - ox) * mmPerUnit, cy = (raw[b + 7] - oy) * mmPerUnit, cz = (raw[b + 8] - oz) * mmPerUnit;
    // Exact zero-area check in source precision (no tolerance-based repair).
    const ux = raw[b + 3] - raw[b], uy = raw[b + 4] - raw[b + 1], uz = raw[b + 5] - raw[b + 2];
    const vx = raw[b + 6] - raw[b], vy = raw[b + 7] - raw[b + 1], vz = raw[b + 8] - raw[b + 2];
    const nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
    if (nx === 0 && ny === 0 && nz === 0) { degenerate++; continue; }
    kept[out++] = ax; kept[out++] = ay; kept[out++] = az;
    kept[out++] = bx; kept[out++] = by; kept[out++] = bz;
    kept[out++] = cx; kept[out++] = cy; kept[out++] = cz;
  }
  if (out === 0) throw new StlError('STL contains only zero-area triangles');
  if (degenerate > 0) warnings.push(`Omitted ${degenerate} zero-area triangle(s)`);

  return {
    positions: out === kept.length ? kept : kept.slice(0, out),
    triangleCount: out / 9,
    format,
    sourceOrigin: [ox, oy, oz],
    size: [(maxX - minX) * mmPerUnit, (maxY - minY) * mmPerUnit, (maxZ - minZ) * mmPerUnit],
    degenerateDropped: degenerate,
    warnings,
  };
}

/** Heuristic hint when a model's extent suggests non-millimetre units. */
export function unitHint(sizeMm: [number, number, number]): string | null {
  const maxExtent = Math.max(...sizeMm);
  if (maxExtent < 2) return 'Model is under 2 mm across. It may be in metres or inches.';
  if (maxExtent > 2000) return 'Model is over 2 m across. It may be in micrometres or a different unit.';
  return null;
}
