// Layer Appearance: perturbs the shading normal in the build frame so printed
// layers appear without extra geometry.
//
// Walls: bead displacement d(z) = depth * sin(pi t)^k along the surface normal,
// t = position within the current layer. Perturbed normal
// N' = normalize(N - d'(z) * (Up - (Up.N) N)), which vanishes on horizontal caps.
// Tops: up-facing horizontal faces get top-infill ridges at +/-45 degrees
// (alternating per layer) with pitch = line width.
// Both fade out below 2-4 pixels per feature to avoid moire.

import * as THREE from 'three';

export interface LayerUniforms {
  uLayerOn: { value: number };
  uLayerH: { value: number };
  uFirstH: { value: number };
  uLayerDepth: { value: number };
  uLayerRound: { value: number };
  uLineW: { value: number };
  uTopOn: { value: number };
  uPrint: { value: THREE.Matrix4 };
  uBuildToView: { value: THREE.Matrix3 };
  uFadeLo: { value: number };
  uFadeHi: { value: number };
  /** Variable layer heights: boundaries in a float texture (row-major). */
  uVarOn: { value: number };
  uBounds: { value: THREE.Texture | null };
  uBoundCount: { value: number };
  uBoundW: { value: number };
}

export function createLayerUniforms(): LayerUniforms {
  return {
    uLayerOn: { value: 1 },
    uLayerH: { value: 0.2 },
    uFirstH: { value: 0.2 },
    uLayerDepth: { value: 0.03 },
    uLayerRound: { value: 0.5 },
    uLineW: { value: 0.42 },
    uTopOn: { value: 1 },
    uPrint: { value: new THREE.Matrix4() },
    uBuildToView: { value: new THREE.Matrix3() },
    uFadeLo: { value: 2 },
    uFadeHi: { value: 4 },
    uVarOn: { value: 0 },
    uBounds: { value: null },
    uBoundCount: { value: 0 },
    uBoundW: { value: 1 },
  };
}

const VERTEX_DECL = /* glsl */ `
uniform mat4 uPrint;
varying vec3 vBuild;
`;

const FRAGMENT_DECL = /* glsl */ `
uniform float uLayerOn;
uniform float uLayerH;
uniform float uFirstH;
uniform float uLayerDepth;
uniform float uLayerRound;
uniform float uLineW;
uniform float uTopOn;
uniform mat3 uBuildToView;
uniform float uFadeLo;
uniform float uFadeHi;
uniform float uVarOn;
uniform sampler2D uBounds;
uniform int uBoundCount;
uniform int uBoundW;
varying vec3 vBuild;

float tnBound(int i) {
  return texelFetch(uBounds, ivec2(i % uBoundW, i / uBoundW), 0).r;
}

// Layer index, height and position within the layer at build height z.
vec3 tnLayerAt(float z) {
  if (uVarOn > 0.5) {
    int lo = 0, hi = uBoundCount - 1;
    for (int k = 0; k < 17; k++) {
      if (hi - lo > 1) {
        int mid = (lo + hi) / 2;
        if (tnBound(mid) <= z) lo = mid; else hi = mid;
      }
    }
    float b0 = tnBound(lo), b1 = tnBound(lo + 1);
    float h = max(b1 - b0, 1e-4);
    return vec3(float(lo), h, clamp((z - b0) / h, 0.0, 1.0));
  }
  bool first = z < uFirstH;
  float h = first ? uFirstH : uLayerH;
  float idx = first ? 0.0 : floor((z - uFirstH) / uLayerH) + 1.0;
  float t = first ? z / uFirstH : fract((z - uFirstH) / uLayerH);
  return vec3(idx, h, t);
}

// Slope of the bead profile sin(pi t)^k with respect to t.
float tnBeadSlope(float t, float k) {
  float s = max(sin(PI * t), 0.03);
  return k * PI * cos(PI * t) * pow(s, k - 1.0);
}

vec3 tnLayerNormal(vec3 n) {
  if (uLayerOn < 0.5) return n;
  vec3 nb = transpose(uBuildToView) * n; // normal in the build frame
  float z = vBuild.z;

  // Top-surface infill ridges on up-facing horizontal faces.
  if (uTopOn > 0.5 && nb.z > 0.985) {
    // The tread sits on a layer boundary; sample just below it.
    float idx = tnLayerAt(z - 0.25 * uLayerH).x;
    float ang = mod(idx, 2.0) < 0.5 ? 0.78539816 : -0.78539816;
    vec2 dir = vec2(cos(ang), sin(ang));
    float s = dot(vBuild.xy, dir) / uLineW;
    float fadeTop = smoothstep(uFadeLo, uFadeHi, 1.0 / max(fwidth(s), 1e-6));
    float slope = clamp(uLayerDepth * 1.5 * tnBeadSlope(fract(s), 0.6) / uLineW, -1.5, 1.5) * fadeTop;
    vec3 g = uBuildToView * vec3(dir, 0.0);
    vec3 r = n - slope * g;
    return dot(r, r) > 1e-10 ? normalize(r) : n;
  }

  vec3 L = tnLayerAt(z);
  float h = L.y;
  float t = L.z;
  float fade = smoothstep(uFadeLo, uFadeHi, h / max(fwidth(z), 1e-7));
  if (fade <= 0.0) return n;
  float slope = clamp(uLayerDepth * tnBeadSlope(t, uLayerRound) / h, -2.5, 2.5) * fade;
  vec3 up = uBuildToView * vec3(0.0, 0.0, 1.0);
  vec3 g = up - dot(up, n) * n;
  vec3 r = n - slope * g;
  return dot(r, r) > 1e-10 ? normalize(r) : n; // never emit NaN normals
}
`;

export function applyLayerShader(material: THREE.Material, uniforms: LayerUniforms) {
  material.onBeforeCompile = (shader) => {
    Object.assign(shader.uniforms, uniforms);
    shader.vertexShader = shader.vertexShader
      .replace('#include <common>', `#include <common>\n${VERTEX_DECL}`)
      .replace('#include <begin_vertex>', `#include <begin_vertex>\nvBuild = (uPrint * vec4(position, 1.0)).xyz;`);
    shader.fragmentShader = shader.fragmentShader
      .replace('#include <common>', `#include <common>\n${FRAGMENT_DECL}`)
      .replace('#include <normal_fragment_maps>', `#include <normal_fragment_maps>\nnormal = tnLayerNormal(normal);`);
  };
  material.customProgramCacheKey = () => 'turninator-layers-v3';
  material.needsUpdate = true;
}
