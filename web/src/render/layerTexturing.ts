// Bake the Layer Appearance shading (raster shader in layerShader.ts) into
// standard PBR data for renderers without custom shaders (the path tracer):
// per-corner UVs + tangents and a 2-row normal-map strip.
//   Row 0 (v = 0.25): wall bead profile; u = layer index + position in layer,
//                     tangent = surface direction of increasing build Z.
//   Row 1 (v = 0.75): top-surface infill ridges; u = distance across lines /
//                     line width (±45° alternating per layer), tangent = across lines.
// Flat bottoms and downward faces sample u = 0.5 (bead crest, zero slope).

import * as THREE from 'three';

export interface LayerTexParams {
  heightMm: number;
  firstMm: number;
  depthMm: number;
  roundness: number;
  lineWidthMm: number;
  topLines: boolean;
  /** Variable boundaries (build mm) or null for uniform heights. */
  boundaries: ArrayLike<number> | null;
}

const RES = 256;

function beadSlope(t: number, k: number): number {
  const s = Math.max(Math.sin(Math.PI * t), 0.03);
  return k * Math.PI * Math.cos(Math.PI * t) * Math.pow(s, k - 1);
}

/**
 * Normal-map strip (tangent space, +X = profile direction). `strength`
 * exaggerates tilt: soft area lighting shows small tilts much more weakly than
 * the raster Studio's directional key, so the path tracer uses ~3x.
 */
export function createLayerNormalMap(p: LayerTexParams, strength = 3): THREE.DataTexture {
  const data = new Uint8Array(RES * 2 * 4);
  const h = Math.min(p.heightMm, p.firstMm);
  for (let row = 0; row < 2; row++) {
    for (let i = 0; i < RES; i++) {
      const t = (i + 0.5) / RES;
      const slope = row === 0
        ? Math.max(-2.5, Math.min(2.5, (strength * p.depthMm * beadSlope(t, p.roundness)) / h))
        : p.topLines ? Math.max(-1.5, Math.min(1.5, (strength * p.depthMm * 1.5 * beadSlope(t, 0.6)) / p.lineWidthMm)) : 0;
      // Displacement grows along +T, so the normal tilts toward -T.
      const len = Math.hypot(slope, 1);
      const o = (row * RES + i) * 4;
      data[o] = Math.round(((-slope / len) * 0.5 + 0.5) * 255);
      data[o + 1] = 128;
      data[o + 2] = Math.round(((1 / len) * 0.5 + 0.5) * 255);
      data[o + 3] = 255;
    }
  }
  const tex = new THREE.DataTexture(data, RES, 2, THREE.RGBAFormat, THREE.UnsignedByteType);
  tex.wrapS = THREE.RepeatWrapping;
  tex.wrapT = THREE.ClampToEdgeWrapping;
  tex.magFilter = THREE.LinearFilter;
  tex.minFilter = THREE.LinearFilter;
  tex.colorSpace = THREE.NoColorSpace;
  tex.needsUpdate = true;
  return tex;
}

function layerCoord(z: number, p: LayerTexParams): number {
  const b = p.boundaries;
  if (b && b.length >= 2) {
    if (z <= b[0]) return 0;
    let lo = 0, hi = b.length - 1;
    while (hi - lo > 1) {
      const mid = (lo + hi) >> 1;
      if (b[mid] <= z) lo = mid; else hi = mid;
    }
    if (lo >= b.length - 1) return b.length - 1;
    return lo + (z - b[lo]) / Math.max(b[lo + 1] - b[lo], 1e-6);
  }
  if (z < p.firstMm) return z / p.firstMm;
  return 1 + (z - p.firstMm) / p.heightMm;
}

/**
 * Return a copy of a non-indexed geometry (model space) with `uv` and
 * `tangent` attributes for the layer normal map. `printMatrix` maps model
 * space to the build frame.
 */
export function addLayerTexturing(geometry: THREE.BufferGeometry, printMatrix: THREE.Matrix4, p: LayerTexParams): THREE.BufferGeometry {
  const pos = geometry.getAttribute('position') as THREE.BufferAttribute;
  const nrm = geometry.getAttribute('normal') as THREE.BufferAttribute;
  const count = pos.count;
  const uv = new Float32Array(count * 2);
  const tangent = new Float32Array(count * 4);
  const rot = new THREE.Matrix3().setFromMatrix4(printMatrix);
  const invRot = rot.clone().invert();
  const P = [new THREE.Vector3(), new THREE.Vector3(), new THREE.Vector3()];
  const N = new THREE.Vector3(), T = new THREE.Vector3(), up = new THREE.Vector3(0, 0, 1);
  const e1 = new THREE.Vector3(), e2 = new THREE.Vector3(), fn = new THREE.Vector3();
  for (let f = 0; f < count; f += 3) {
    for (let k = 0; k < 3; k++) P[k].fromBufferAttribute(pos, f + k).applyMatrix4(printMatrix);
    fn.crossVectors(e1.subVectors(P[1], P[0]), e2.subVectors(P[2], P[0])).normalize();
    const flat = Math.abs(fn.z) > 0.985;
    if (flat && fn.z > 0) {
      // Up-facing horizontal: infill ridges at ±45°, alternating per layer.
      const layer = Math.floor(layerCoord((P[0].z + P[1].z + P[2].z) / 3 - 1e-3, p));
      const ang = layer % 2 === 0 ? Math.PI / 4 : -Math.PI / 4;
      const dx = Math.cos(ang), dy = Math.sin(ang);
      T.set(dx, dy, 0).applyMatrix3(invRot).normalize();
      for (let k = 0; k < 3; k++) {
        const c = f + k;
        uv[c * 2] = (P[k].x * dx + P[k].y * dy) / p.lineWidthMm;
        uv[c * 2 + 1] = 0.75;
        tangent.set([T.x, T.y, T.z, 1], c * 4);
      }
      continue;
    }
    for (let k = 0; k < 3; k++) {
      const c = f + k;
      if (flat) {
        uv[c * 2] = 0.5; // crest: no slope on bottoms
        uv[c * 2 + 1] = 0.25;
        T.set(1, 0, 0);
      } else {
        N.fromBufferAttribute(nrm, c).applyMatrix3(rot).normalize();
        T.copy(up).addScaledVector(N, -up.dot(N));
        if (T.lengthSq() < 1e-8) T.set(1, 0, 0);
        T.normalize().applyMatrix3(invRot).normalize();
        uv[c * 2] = layerCoord(P[k].z, p);
        uv[c * 2 + 1] = 0.25;
      }
      tangent.set([T.x, T.y, T.z, 1], c * 4);
    }
  }
  const g = new THREE.BufferGeometry();
  g.setAttribute('position', pos);
  g.setAttribute('normal', nrm);
  g.setAttribute('uv', new THREE.BufferAttribute(uv, 2));
  g.setAttribute('tangent', new THREE.BufferAttribute(tangent, 4));
  g.boundingSphere = geometry.boundingSphere?.clone() ?? null;
  return g;
}
