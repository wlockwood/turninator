// Camera keyframe paths: views captured from the viewport, joined by timed
// transitions in spherical coordinates around a (possibly moving) target.
// Extra turns make spirals (e.g. front view -> top view while circling).

export type Vec3 = [number, number, number];

export interface CamKey {
  /** Degrees; 0 = camera in front (-Y), +90 = camera on +X. */
  azimuth: number;
  /** Degrees above the horizon. */
  elevation: number;
  distance: number;
  target: Vec3;
}

export type Easing = 'linear' | 'ease' | 'ease-in' | 'ease-out';

export interface CamTransition {
  seconds: number;
  /** Extra full turns of azimuth (positive = counter-clockwise from above). */
  turns: number;
  easing: Easing;
  /** Seconds to hold on the destination key after arriving. */
  hold: number;
}

export const DEFAULT_TRANSITION: CamTransition = { seconds: 2, turns: 0, easing: 'ease', hold: 0.5 };

const D = Math.PI / 180;

export function keyFromCamera(position: Vec3, target: Vec3): CamKey {
  const dx = position[0] - target[0], dy = position[1] - target[1], dz = position[2] - target[2];
  const distance = Math.hypot(dx, dy, dz);
  const elevation = Math.asin(Math.max(-1, Math.min(1, dz / distance))) / D;
  const azimuth = Math.atan2(dx, -dy) / D;
  return { azimuth, elevation, distance, target: [...target] };
}

export function cameraFromKey(k: CamKey): { position: Vec3; target: Vec3 } {
  const az = k.azimuth * D, el = Math.max(-89.9, Math.min(89.9, k.elevation)) * D;
  const c = Math.cos(el);
  return {
    position: [k.target[0] + Math.sin(az) * c * k.distance, k.target[1] - Math.cos(az) * c * k.distance, k.target[2] + Math.sin(el) * k.distance],
    target: [...k.target],
  };
}

export function ease(t: number, e: Easing): number {
  switch (e) {
    case 'ease': return t * t * (3 - 2 * t);
    case 'ease-in': return t * t;
    case 'ease-out': return 1 - (1 - t) * (1 - t);
    default: return t;
  }
}

/** Total duration: an initial hold on the first key plus each transition and its hold. */
export function pathDuration(keys: CamKey[], transitions: CamTransition[], firstHold = 0): number {
  let t = firstHold;
  for (let i = 0; i < keys.length - 1; i++) t += transitions[i].seconds + transitions[i].hold;
  return t;
}

/** Shortest signed azimuth difference in degrees. */
function azDelta(a: number, b: number): number {
  return ((((b - a) % 360) + 540) % 360) - 180;
}

/** Interpolated key at time t (seconds). */
export function sampleKey(keys: CamKey[], transitions: CamTransition[], t: number, firstHold = 0): CamKey {
  if (keys.length === 0) throw new Error('Camera path has no keyframes');
  if (keys.length === 1 || t <= firstHold) return keys[0];
  let time = t - firstHold;
  for (let i = 0; i < keys.length - 1; i++) {
    const tr = transitions[i];
    if (time <= tr.seconds) {
      const u = ease(tr.seconds > 0 ? time / tr.seconds : 1, tr.easing);
      const a = keys[i], b = keys[i + 1];
      const dAz = azDelta(a.azimuth, b.azimuth) + 360 * tr.turns;
      return {
        azimuth: a.azimuth + dAz * u,
        elevation: a.elevation + (b.elevation - a.elevation) * u,
        // Distance interpolates geometrically so zooms feel even.
        distance: a.distance * Math.pow(b.distance / a.distance, u),
        target: [0, 1, 2].map((k) => a.target[k] + (b.target[k] - a.target[k]) * u) as Vec3,
      };
    }
    time -= tr.seconds;
    if (time <= tr.hold) return keys[i + 1];
    time -= tr.hold;
  }
  return keys[keys.length - 1];
}
