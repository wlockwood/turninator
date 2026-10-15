import { describe, expect, it } from 'vitest';
import * as THREE from 'three';
import { faceDownPose, largestFlatFace } from '../src/render/orient';
import { parseStl } from '../src/core/stl';
import { weldExact } from '../src/core/mesh';
import { binaryStl, box, type Tri } from './helpers';

function rotate(e: [number, number, number], v: THREE.Vector3) {
  const m = new THREE.Matrix4().makeRotationFromEuler(new THREE.Euler(...e.map((d) => (d * Math.PI) / 180) as [number, number, number], 'XYZ'));
  return v.clone().applyMatrix4(m);
}

describe('orientation', () => {
  it('turns +X to face down from the identity pose', () => {
    const pose = faceDownPose([0, 0, 0], new THREE.Vector3(1, 0, 0));
    const d = rotate(pose, new THREE.Vector3(1, 0, 0));
    expect(d.z).toBeCloseTo(-1, 6);
  });

  it('composes with an existing pose', () => {
    const start: [number, number, number] = [30, 45, 10];
    const local = new THREE.Vector3(0.2, -0.5, 0.8).normalize();
    const pose = faceDownPose(start, local);
    expect(rotate(pose, local).z).toBeCloseTo(-1, 6);
  });

  it('leaves an already-down face unchanged', () => {
    expect(faceDownPose([0, 0, 0], new THREE.Vector3(0, 0, -1))).toEqual([0, 0, 0]);
  });

  it('finds the largest face of a box that can rest flat', () => {
    const mesh = weldExact(parseStl(binaryStl(box(0, 0, 0, 30, 20, 5))).positions);
    const f = largestFlatFace(mesh.positions, mesh.indices)!;
    expect(Math.abs(f.normal.z)).toBeCloseTo(1, 6);
    expect(f.areaMm2).toBeCloseTo(600, 3);
  });

  it('ignores a large face that is not on the convex boundary', () => {
    // A thin tall plate plus a wide low step whose top is inside the hull.
    const tris: Tri[] = [...box(0, 0, 0, 2, 40, 40), ...box(2, 0, 0, 40, 40, 1)];
    const mesh = weldExact(parseStl(binaryStl(tris)).positions);
    const f = largestFlatFace(mesh.positions, mesh.indices)!;
    // The bottom (z = 0, normal -Z) spans both parts and is the largest valid face.
    expect(f.normal.z).toBeCloseTo(-1, 6);
  });
});
