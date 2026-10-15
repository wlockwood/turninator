// Orientation helpers: rotate a pose so a chosen face points down, and find
// the largest flat face that can rest on the bed.

import * as THREE from 'three';

export type EulerDeg = [number, number, number];

const DEG = Math.PI / 180;
const DOWN = new THREE.Vector3(0, 0, -1);

function toMatrix(e: EulerDeg): THREE.Matrix4 {
  return new THREE.Matrix4().makeRotationFromEuler(new THREE.Euler(e[0] * DEG, e[1] * DEG, e[2] * DEG, 'XYZ'));
}

function clean(v: number): number {
  const r = Math.round(v * 1e4) / 1e4;
  const n = ((r % 360) + 360) % 360;
  return Math.abs(n - 360) < 1e-6 ? 0 : n;
}

/**
 * Given a pose (XYZ Euler, degrees) and a direction expressed in the space
 * the pose rotates *from*, return a new pose that additionally rotates the
 * direction to point straight down (-Z). The added rotation is the shortest
 * arc, so the model does not spin needlessly about Z.
 */
export function faceDownPose(pose: EulerDeg, localDir: THREE.Vector3): EulerDeg {
  const R = toMatrix(pose);
  const d = localDir.clone().normalize().applyMatrix4(new THREE.Matrix4().extractRotation(R)).normalize();
  const q = new THREE.Quaternion().setFromUnitVectors(d, DOWN);
  const next = new THREE.Matrix4().makeRotationFromQuaternion(q).multiply(R);
  const e = new THREE.Euler().setFromRotationMatrix(next, 'XYZ');
  return [clean(e.x / DEG), clean(e.y / DEG), clean(e.z / DEG)];
}

/**
 * Largest planar region on the convex boundary of a mesh (a face that can sit
 * flat on the bed). Triangles are grouped by quantized normal and plane offset;
 * a group qualifies only if no vertex lies beyond its plane.
 */
export function largestFlatFace(positions: Float32Array, indices: Uint32Array, tolMm = 0.01): { normal: THREE.Vector3; areaMm2: number } | null {
  const groups = new Map<string, { n: [number, number, number]; d: number; area: number }>();
  const p = positions;
  for (let f = 0; f < indices.length; f += 3) {
    const a = indices[f] * 3, b = indices[f + 1] * 3, c = indices[f + 2] * 3;
    const ux = p[b] - p[a], uy = p[b + 1] - p[a + 1], uz = p[b + 2] - p[a + 2];
    const vx = p[c] - p[a], vy = p[c + 1] - p[a + 1], vz = p[c + 2] - p[a + 2];
    let nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
    const len = Math.hypot(nx, ny, nz);
    if (len === 0) continue;
    nx /= len; ny /= len; nz /= len;
    const d = nx * p[a] + ny * p[a + 1] + nz * p[a + 2];
    const key = `${Math.round(nx * 500)},${Math.round(ny * 500)},${Math.round(nz * 500)},${Math.round(d / Math.max(tolMm, 1e-6))}`;
    const g = groups.get(key);
    if (g) g.area += len / 2;
    else groups.set(key, { n: [nx, ny, nz], d, area: len / 2 });
  }
  const candidates = [...groups.values()].sort((x, y) => y.area - x.area).slice(0, 64);
  for (const g of candidates) {
    let beyond = false;
    for (let i = 0; i < p.length; i += 3) {
      if (g.n[0] * p[i] + g.n[1] * p[i + 1] + g.n[2] * p[i + 2] > g.d + tolMm) { beyond = true; break; }
    }
    if (!beyond) return { normal: new THREE.Vector3(...g.n), areaMm2: g.area };
  }
  return null;
}
