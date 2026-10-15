// Instanced extrusion-bead rendering for G-code toolpaths.
//
// Every segment is one instance of a shared "pill" mesh: a stadium cross
// section (width w, height h) swept from start to end, closed by rounded caps
// revolved around the vertical axis at each end so consecutive beads join like
// real extrusions. The vertex shader builds positions and normals from
// per-instance endpoints and size; per-instance metadata drives build-up
// progress, feature visibility and colour modes.

import * as THREE from 'three';

/** Per-vertex profile: lateral = x*w + y*h, vertical = z*h, normal angle = w. */
function profile(np: number): [number, number, number, number][] {
  const pts: [number, number, number, number][] = [[0, 0, -0.5, -Math.PI / 2]]; // bottom centre
  for (let i = 0; i <= np; i++) {
    const phi = -Math.PI / 2 + (Math.PI * i) / np;
    pts.push([0.5, -0.5 + 0.5 * Math.cos(phi), 0.5 * Math.sin(phi), phi]);
  }
  pts.push([0, 0, 0.5, Math.PI / 2]); // top centre
  return pts;
}

/** Evaluate the bead formula in JS (used to fix triangle winding). */
function evalVertex(endFlag: number, theta: number, p: [number, number, number, number]): THREE.Vector3 {
  const w = 1, h = 0.5;
  const dir = new THREE.Vector3(1, 0, 0);
  const base = endFlag ? new THREE.Vector3(1, 0, 0) : new THREE.Vector3(0, 0, 0);
  const outward = endFlag ? dir.clone() : dir.clone().negate();
  const side = new THREE.Vector3(0, 0, 1).cross(outward);
  const radial = outward.multiplyScalar(Math.cos(theta)).add(side.multiplyScalar(Math.sin(theta)));
  const lateral = p[0] * w + p[1] * h;
  return base.add(radial.multiplyScalar(lateral)).add(new THREE.Vector3(0, 0, p[2] * h));
}

export function createPillGeometry(np = 4, nt = 4): THREE.InstancedBufferGeometry {
  const prof = profile(np);
  const P = prof.length;
  const aEnd: number[] = [], aTheta: number[] = [], aProf: number[] = [];
  const idx = (end: number, j: number, i: number) => end * (nt + 1) * P + j * P + i;
  for (let end = 0; end < 2; end++) {
    for (let j = 0; j <= nt; j++) {
      const theta = -Math.PI / 2 + (Math.PI * j) / nt;
      for (let i = 0; i < P; i++) {
        aEnd.push(end);
        aTheta.push(theta);
        aProf.push(...prof[i]);
      }
    }
  }
  const tris: number[] = [];
  const quad = (a: number, b: number, c: number, d: number) => tris.push(a, b, c, a, c, d);
  for (let end = 0; end < 2; end++) {
    for (let j = 0; j < nt; j++) for (let i = 0; i < P - 1; i++) quad(idx(end, j, i), idx(end, j + 1, i), idx(end, j + 1, i + 1), idx(end, j, i + 1));
  }
  // Sides: theta = +90 at the end meets theta = -90 at the start (same side).
  for (let i = 0; i < P - 1; i++) {
    quad(idx(1, nt, i), idx(0, 0, i), idx(0, 0, i + 1), idx(1, nt, i + 1));
    quad(idx(0, nt, i), idx(1, 0, i), idx(1, 0, i + 1), idx(0, nt, i + 1));
  }
  // Fix winding so every triangle faces outward (normal from the formula).
  for (let t = 0; t < tris.length; t += 3) {
    const v = [0, 1, 2].map((k) => {
      const n = tris[t + k];
      return evalVertex(aEnd[n], aTheta[n], [aProf[n * 4], aProf[n * 4 + 1], aProf[n * 4 + 2], aProf[n * 4 + 3]]);
    });
    const face = new THREE.Vector3().subVectors(v[1], v[0]).cross(new THREE.Vector3().subVectors(v[2], v[0]));
    if (face.lengthSq() < 1e-14) continue;
    const centroid = v[0].clone().add(v[1]).add(v[2]).divideScalar(3);
    const axisPoint = new THREE.Vector3(Math.min(Math.max(centroid.x, 0), 1), 0, 0);
    if (face.dot(centroid.sub(axisPoint)) < 0) [tris[t + 1], tris[t + 2]] = [tris[t + 2], tris[t + 1]];
  }
  const g = new THREE.InstancedBufferGeometry();
  // A position attribute is required by three; the shader ignores its values.
  g.setAttribute('position', new THREE.Float32BufferAttribute(new Float32Array(aEnd.length * 3), 3));
  g.setAttribute('aEnd', new THREE.Float32BufferAttribute(aEnd, 1));
  g.setAttribute('aTheta', new THREE.Float32BufferAttribute(aTheta, 1));
  g.setAttribute('aProf', new THREE.Float32BufferAttribute(aProf, 4));
  g.setIndex(tris);
  return g;
}

export interface BeadData {
  count: number;
  start: Float32Array;
  end: Float32Array;
  width: Float32Array;
  height: Float32Array;
  layer: Uint32Array;
  feature: Uint8Array;
  tool: Uint8Array;
}

export function createBeadGeometry(d: BeadData, np = 4, nt = 4): THREE.InstancedBufferGeometry {
  const g = createPillGeometry(np, nt);
  const size = new Float32Array(d.count * 2);
  const meta = new Float32Array(d.count * 4);
  for (let i = 0; i < d.count; i++) {
    size[i * 2] = d.width[i];
    size[i * 2 + 1] = d.height[i];
    meta[i * 4] = i;
    meta[i * 4 + 1] = d.layer[i];
    meta[i * 4 + 2] = d.feature[i];
    meta[i * 4 + 3] = d.tool[i];
  }
  g.setAttribute('iStart', new THREE.InstancedBufferAttribute(d.start, 3));
  g.setAttribute('iEnd', new THREE.InstancedBufferAttribute(d.end, 3));
  g.setAttribute('iSize', new THREE.InstancedBufferAttribute(size, 2));
  g.setAttribute('iMeta', new THREE.InstancedBufferAttribute(meta, 4));
  g.instanceCount = d.count;
  return g;
}

export type BeadColorMode = 'material' | 'feature' | 'tool' | 'layer';

export interface BeadUniforms {
  uProgress: { value: number };
  uHiddenMask: { value: number };
  uColorMode: { value: number };
  uLayerCount: { value: number };
  uFeatureColors: { value: THREE.Color[] };
  uToolColors: { value: THREE.Color[] };
}

export const FEATURE_COLORS: Record<string, string> = {
  'Outer wall': '#f2a33a', 'Inner wall': '#e8d44d', 'Overhang wall': '#3a7bf2', 'Top surface': '#e0463a',
  'Bottom surface': '#9b59b6', Bridge: '#4a90d9', 'Solid infill': '#c0392b', 'Sparse infill': '#a5452c',
  'Gap fill': '#ffffff', Support: '#2ecc71', 'Support interface': '#27ae60', 'Skirt/Brim': '#16a085',
  'Wipe tower': '#7f8c8d', Custom: '#95a5a6',
};

export function createBeadUniforms(): BeadUniforms {
  return {
    uProgress: { value: 1e12 },
    uHiddenMask: { value: 0 },
    uColorMode: { value: 0 },
    uLayerCount: { value: 1 },
    uFeatureColors: { value: Array.from({ length: 32 }, () => new THREE.Color(0.7, 0.7, 0.7)) },
    uToolColors: { value: Array.from({ length: 8 }, () => new THREE.Color(0.7, 0.7, 0.7)) },
  };
}

const VERTEX_DECL = /* glsl */ `
attribute float aEnd;
attribute float aTheta;
attribute vec4 aProf;
attribute vec3 iStart;
attribute vec3 iEnd;
attribute vec2 iSize;
attribute vec4 iMeta;
uniform float uProgress;
uniform int uHiddenMask;
uniform float uColorMode;
uniform float uLayerCount;
uniform vec3 uFeatureColors[32];
uniform vec3 uToolColors[8];
varying vec3 vBeadColor;
vec3 tnBeadPos;
vec3 tnBeadNormal;
bool tnBeadVisible() {
  if (iMeta.x >= uProgress) return false;
  uint bit = 1u << uint(min(iMeta.z, 31.0));
  return (uint(uHiddenMask) & bit) == 0u;
}
void tnBeadBuild() {
  vec3 up = vec3(0.0, 0.0, 1.0);
  float shown = clamp(uProgress - iMeta.x, 0.0, 1.0);
  vec3 p1 = mix(iStart, iEnd, shown);
  vec2 d2 = p1.xy - iStart.xy;
  float L = length(d2);
  vec3 dir = L > 1e-6 ? vec3(d2 / L, 0.0) : vec3(normalize(iEnd.xy - iStart.xy + vec2(1e-6, 0.0)), 0.0);
  vec3 base = aEnd > 0.5 ? p1 : iStart;
  vec3 outward = aEnd > 0.5 ? dir : -dir;
  vec3 radial = outward * cos(aTheta) + cross(up, outward) * sin(aTheta);
  float lateral = aProf.x * iSize.x + aProf.y * iSize.y;
  tnBeadPos = base + radial * lateral + up * (aProf.z * iSize.y);
  tnBeadNormal = normalize(radial * cos(aProf.w) + up * sin(aProf.w));
  if (!tnBeadVisible()) tnBeadPos = vec3(0.0, 0.0, -1e6); // collapse hidden instances
  int f = int(iMeta.z);
  int t = int(iMeta.w);
  if (uColorMode < 0.5) vBeadColor = vec3(-1.0); // keep material colour
  else if (uColorMode < 1.5) vBeadColor = uFeatureColors[f];
  else if (uColorMode < 2.5) vBeadColor = uToolColors[t];
  else {
    float k = iMeta.y / max(uLayerCount - 1.0, 1.0);
    vBeadColor = 0.5 + 0.5 * cos(6.28318 * (k + vec3(0.0, 0.33, 0.67)));
  }
}
`;

export function applyBeadShader(material: THREE.Material, uniforms: BeadUniforms, withColor = true) {
  material.onBeforeCompile = (shader) => {
    Object.assign(shader.uniforms, uniforms);
    shader.vertexShader = shader.vertexShader
      .replace('#include <common>', `#include <common>\n${VERTEX_DECL}`)
      .replace('#include <beginnormal_vertex>', 'tnBeadBuild();\nvec3 objectNormal = tnBeadNormal;')
      // begin_vertex always rebuilds: depth materials guard normals behind #ifdefs.
      .replace('#include <begin_vertex>', 'tnBeadBuild();\nvec3 transformed = tnBeadPos;');
    if (withColor) {
      shader.fragmentShader = shader.fragmentShader
        .replace('#include <common>', '#include <common>\nvarying vec3 vBeadColor;')
        .replace('#include <color_fragment>', '#include <color_fragment>\nif (vBeadColor.r >= 0.0) diffuseColor.rgb = vBeadColor;');
    }
  };
  material.customProgramCacheKey = () => `turninator-beads-${withColor ? 'c' : 'd'}`;
  material.needsUpdate = true;
}
