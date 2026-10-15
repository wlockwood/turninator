// Camera framing math (Z-up, millimetres). Pure functions so they can be
// tested without WebGL.

export type Vec3 = [number, number, number];

export interface PerspectiveFit {
  position: Vec3;
  target: Vec3;
  up: Vec3;
  fovDeg: number;
}

export interface OrthoFit {
  position: Vec3;
  target: Vec3;
  up: Vec3;
  /** Half-height of the orthographic view volume in mm. */
  halfHeight: number;
  halfWidth: number;
}

const sub = (a: Vec3, b: Vec3): Vec3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const dot = (a: Vec3, b: Vec3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a: Vec3, b: Vec3): Vec3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const scale = (a: Vec3, s: number): Vec3 => [a[0] * s, a[1] * s, a[2] * s];
const add = (a: Vec3, b: Vec3): Vec3 => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const norm = (a: Vec3): Vec3 => scale(a, 1 / Math.hypot(a[0], a[1], a[2]));

/** Points on the swept cylinder of a model rotating about the Z axis. */
export function envelopePoints(radius: number, zMin: number, zMax: number, samples = 96): Vec3[] {
  const pts: Vec3[] = [];
  for (let i = 0; i < samples; i++) {
    const a = (i / samples) * Math.PI * 2;
    const x = Math.cos(a) * radius, y = Math.sin(a) * radius;
    pts.push([x, y, zMin], [x, y, zMax]);
  }
  return pts;
}

function basis(viewDir: Vec3, upHint: Vec3) {
  const f = norm(viewDir); // camera looks along f
  let r = cross(f, upHint);
  if (Math.hypot(...r) < 1e-9) r = cross(f, [0, 1, 0]);
  r = norm(r);
  const u = cross(r, f);
  return { f, r, u };
}

/**
 * Fit a perspective camera so every point stays inside the frame with the
 * given margin fraction (0.1 = 10% each side) for the given view direction.
 */
export function fitPerspective(points: Vec3[], viewDir: Vec3, fovDeg: number, aspect: number, margin = 0.1, upHint: Vec3 = [0, 0, 1]): PerspectiveFit {
  const { f, r, u } = basis(viewDir, upHint);
  const tanY = Math.tan((fovDeg * Math.PI) / 360) * (1 - 2 * margin);
  const tanX = tanY * aspect;
  // Start from the bounding-box center; iterate to center the projection.
  const c: Vec3 = [0, 0, 0];
  for (const p of points) for (let k = 0; k < 3; k++) c[k] += p[k] / points.length;
  let target = c;
  let distance = 1;
  for (let iter = 0; iter < 6; iter++) {
    // Required distance: for each point with lateral offsets (x, y) and depth
    // offset d along f (relative to the target), need |x| <= tanX*(D + d).
    let need = 0;
    for (const p of points) {
      const v = sub(p, target);
      const x = dot(v, r), y = dot(v, u), d = dot(v, f);
      need = Math.max(need, Math.abs(x) / tanX - d, Math.abs(y) / tanY - d);
    }
    distance = Math.max(need, 1e-3);
    // Re-center: shift target so the projected extents are symmetric.
    let xmin = Infinity, xmax = -Infinity, ymin = Infinity, ymax = -Infinity;
    for (const p of points) {
      const v = sub(p, target);
      const depth = distance + dot(v, f);
      const x = dot(v, r) / depth, y = dot(v, u) / depth;
      xmin = Math.min(xmin, x); xmax = Math.max(xmax, x);
      ymin = Math.min(ymin, y); ymax = Math.max(ymax, y);
    }
    const shiftX = ((xmin + xmax) / 2) * distance;
    const shiftY = ((ymin + ymax) / 2) * distance;
    if (Math.abs(shiftX) < 1e-6 && Math.abs(shiftY) < 1e-6) break;
    target = add(add(target, scale(r, shiftX)), scale(u, shiftY));
  }
  return { position: sub(target, scale(f, distance)), target, up: u, fovDeg };
}

/** Orthographic fit with an explicit shared scale (mm per half-height). */
export function orthoView(center: Vec3, viewDir: Vec3, upHint: Vec3, halfHeight: number, aspect: number, distance: number): OrthoFit {
  const { f, u } = basis(viewDir, upHint);
  return { position: sub(center, scale(f, distance)), target: center, up: u, halfHeight, halfWidth: halfHeight * aspect };
}

/** Projected half-extents of points for an orthographic view direction. */
export function orthoExtents(points: Vec3[], center: Vec3, viewDir: Vec3, upHint: Vec3): { halfW: number; halfH: number } {
  const { r, u } = basis(viewDir, upHint);
  let halfW = 0, halfH = 0;
  for (const p of points) {
    const v = sub(p, center);
    halfW = Math.max(halfW, Math.abs(dot(v, r)));
    halfH = Math.max(halfH, Math.abs(dot(v, u)));
  }
  return { halfW, halfH };
}

export function directionFromAngles(azimuthDeg: number, elevationDeg: number): Vec3 {
  // View direction for a camera placed at the given azimuth (0 = in front, on -Y)
  // and elevation, looking at the origin.
  const az = (azimuthDeg * Math.PI) / 180, el = (elevationDeg * Math.PI) / 180;
  const pos: Vec3 = [Math.sin(az) * Math.cos(el), -Math.cos(az) * Math.cos(el), Math.sin(el)];
  return scale(pos, -1);
}

/** Canonical turntable angles: no duplicated 360-degree endpoint. */
export function turntableAngles(frames: number, startDeg = 0): number[] {
  return Array.from({ length: frames }, (_, i) => startDeg + (360 * i) / frames);
}
