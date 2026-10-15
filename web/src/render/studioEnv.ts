// Procedural HDR studio environment for path tracing: a soft gradient dome
// plus rectangular softboxes (key, fill, rim, top strip). Generated in the
// standard Y-up equirect convention; rotate it into the Z-up world with
// scene.environmentRotation.x = PI / 2 (env +Y -> world +Z, env +Z -> world -Y,
// i.e. the camera/front side).

import * as THREE from 'three';

export interface Softbox {
  /** Azimuth in degrees around up; 0 = front (camera side), +90 = right. */
  azimuth: number;
  elevation: number;
  widthDeg: number;
  heightDeg: number;
  intensity: number;
  color?: [number, number, number];
}

export interface StudioEnvOptions {
  width: number;
  height: number;
  /** Dome radiance at horizon/zenith and below the horizon (linear). */
  horizon: number;
  zenith: number;
  ground: number;
  softboxes: Softbox[];
}

export const DEFAULT_SOFTBOXES: Softbox[] = [
  // Key: a compact, bright softbox so surface relief (layer lines) reads clearly.
  { azimuth: -45, elevation: 38, widthDeg: 16, heightDeg: 12, intensity: 40 }, // key, front-left
  { azimuth: 70, elevation: 12, widthDeg: 40, heightDeg: 22, intensity: 2.2 }, // fill, right
  { azimuth: 165, elevation: 30, widthDeg: 26, heightDeg: 34, intensity: 7 }, // rim, behind
  { azimuth: 0, elevation: 80, widthDeg: 70, heightDeg: 14, intensity: 2.5 }, // top strip
];

export function createStudioEnvironment(opts: Partial<StudioEnvOptions> = {}): THREE.DataTexture {
  const o: StudioEnvOptions = { width: 1024, height: 512, horizon: 0.35, zenith: 0.55, ground: 0.12, softboxes: DEFAULT_SOFTBOXES, ...opts };
  const data = new Float32Array(o.width * o.height * 4);
  const boxes = o.softboxes.map((b) => ({
    ...b,
    az: (b.azimuth * Math.PI) / 180,
    el: (b.elevation * Math.PI) / 180,
    hw: (b.widthDeg * Math.PI) / 360,
    hh: (b.heightDeg * Math.PI) / 360,
    c: b.color ?? [1, 1, 1],
  }));
  for (let y = 0; y < o.height; y++) {
    // three's equirect: v = asin(dir.y) / PI + 0.5 (texture row 0 = v 0 when flipY is false)
    const v = (y + 0.5) / o.height;
    const lat = (v - 0.5) * Math.PI; // elevation
    for (let x = 0; x < o.width; x++) {
      const u = (x + 0.5) / o.width;
      const lon = (u - 0.5) * 2 * Math.PI; // atan2(dir.z, dir.x)
      // Env-space direction (Y up); front (+Z env) = azimuth 0.
      const dx = Math.cos(lat) * Math.cos(lon), dy = Math.sin(lat), dz = Math.cos(lat) * Math.sin(lon);
      const az = Math.atan2(dx, dz); // 0 at +Z, +90° at +X
      let r: number, g: number, b: number;
      if (lat >= 0) {
        const k = Math.pow(lat / (Math.PI / 2), 0.7);
        r = g = b = o.horizon + (o.zenith - o.horizon) * k;
      } else {
        r = g = b = o.ground;
      }
      for (const box of boxes) {
        let da = az - box.az;
        da = Math.atan2(Math.sin(da), Math.cos(da));
        const de = lat - box.el;
        const ex = Math.abs(da * Math.cos(box.el)) / box.hw, ey = Math.abs(de) / box.hh;
        if (ex < 1.15 && ey < 1.15) {
          // Soft-edged rectangle.
          const fx = 1 - THREE.MathUtils.smoothstep(ex, 0.85, 1.15);
          const fy = 1 - THREE.MathUtils.smoothstep(ey, 0.85, 1.15);
          const w = fx * fy * box.intensity;
          r += w * box.c[0]; g += w * box.c[1]; b += w * box.c[2];
        }
      }
      void dy;
      const i = (y * o.width + x) * 4;
      data[i] = r; data[i + 1] = g; data[i + 2] = b; data[i + 3] = 1;
    }
  }
  const tex = new THREE.DataTexture(data, o.width, o.height, THREE.RGBAFormat, THREE.FloatType);
  tex.mapping = THREE.EquirectangularReflectionMapping;
  tex.colorSpace = THREE.LinearSRGBColorSpace;
  tex.magFilter = THREE.LinearFilter;
  tex.minFilter = THREE.LinearFilter;
  tex.generateMipmaps = false;
  tex.needsUpdate = true;
  return tex;
}
