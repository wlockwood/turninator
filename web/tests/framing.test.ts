import { describe, expect, it } from 'vitest';
import { directionFromAngles, envelopePoints, fitPerspective, turntableAngles, type Vec3 } from '../src/render/framing';

function project(fit: ReturnType<typeof fitPerspective>, p: Vec3, aspect: number) {
  const f = [fit.target[0] - fit.position[0], fit.target[1] - fit.position[1], fit.target[2] - fit.position[2]];
  const fl = Math.hypot(f[0], f[1], f[2]);
  const fn = f.map((v) => v / fl);
  const u = fit.up;
  const r = [fn[1] * u[2] - fn[2] * u[1], fn[2] * u[0] - fn[0] * u[2], fn[0] * u[1] - fn[1] * u[0]];
  const v = [p[0] - fit.position[0], p[1] - fit.position[1], p[2] - fit.position[2]];
  const depth = v[0] * fn[0] + v[1] * fn[1] + v[2] * fn[2];
  const t = Math.tan((fit.fovDeg * Math.PI) / 360);
  return {
    x: (v[0] * r[0] + v[1] * r[1] + v[2] * r[2]) / depth / (t * aspect),
    y: (v[0] * u[0] + v[1] * u[1] + v[2] * u[2]) / depth / t,
  };
}

describe('framing', () => {
  it('turntable angles never duplicate the endpoint', () => {
    const a = turntableAngles(80);
    expect(a.length).toBe(80);
    expect(a[0]).toBe(0);
    expect(a[79]).toBeCloseTo(355.5);
  });

  for (const [label, radius, height, aspect] of [
    ['tall', 10, 120, 1],
    ['flat', 80, 5, 1],
    ['wide frame', 30, 30, 16 / 9],
  ] as const) {
    it(`keeps a ${label} rotational envelope within a 10% margin`, () => {
      const pts = envelopePoints(radius, 0, height);
      const fit = fitPerspective(pts, directionFromAngles(0, 20), 30, aspect, 0.1);
      let maxX = 0, maxY = 0;
      for (const p of pts) {
        const q = project(fit, p, aspect);
        maxX = Math.max(maxX, Math.abs(q.x));
        maxY = Math.max(maxY, Math.abs(q.y));
      }
      expect(Math.max(maxX, maxY)).toBeLessThanOrEqual(0.8 + 1e-6);
      expect(Math.max(maxX, maxY)).toBeGreaterThan(0.75); // tight, not wasteful
    });
  }
});
