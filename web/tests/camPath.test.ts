import { describe, expect, it } from 'vitest';
import { cameraFromKey, keyFromCamera, pathDuration, sampleKey, type CamKey, type CamTransition } from '../src/render/camPath';

const front: CamKey = { azimuth: 0, elevation: 10, distance: 100, target: [0, 0, 10] };
const top: CamKey = { azimuth: 0, elevation: 89, distance: 80, target: [0, 0, 10] };

describe('camera paths', () => {
  it('round-trips camera position and key', () => {
    const cam = cameraFromKey({ azimuth: 30, elevation: 20, distance: 50, target: [1, 2, 3] });
    const k = keyFromCamera(cam.position, cam.target);
    expect(k.azimuth).toBeCloseTo(30, 6);
    expect(k.elevation).toBeCloseTo(20, 6);
    expect(k.distance).toBeCloseTo(50, 6);
  });

  it('places azimuth 0 in front (-Y) and +90 on +X', () => {
    expect(cameraFromKey({ ...front, elevation: 0 }).position[1]).toBeCloseTo(-100);
    expect(cameraFromKey({ ...front, azimuth: 90, elevation: 0 }).position[0]).toBeCloseTo(100);
  });

  it('spirals: extra turns add full revolutions while elevation rises', () => {
    const tr: CamTransition[] = [{ seconds: 4, turns: 1, easing: 'linear', hold: 1 }];
    expect(pathDuration([front, top], tr, 0.5)).toBe(5.5);
    const mid = sampleKey([front, top], tr, 0.5 + 2, 0.5);
    expect(mid.azimuth).toBeCloseTo(180, 6);
    expect(mid.elevation).toBeCloseTo((10 + 89) / 2, 6);
    expect(sampleKey([front, top], tr, 5.2, 0.5)).toEqual(top);
  });

  it('takes the shortest way round between keys without turns', () => {
    const a = { ...front, azimuth: 170 }, b = { ...front, azimuth: -170 };
    const mid = sampleKey([a, b], [{ seconds: 2, turns: 0, easing: 'linear', hold: 0 }], 1);
    expect(Math.abs(((mid.azimuth % 360) + 360) % 360 - 180)).toBeLessThan(1e-6);
  });
});
