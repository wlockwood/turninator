import { describe, expect, it } from 'vitest';
import { makeProject, mergeInto, parseProject } from '../src/state/project';

describe('projects', () => {
  it('merges only known keys of matching kinds', () => {
    const defaults = { a: 1, b: { c: 'x', d: [1, 2] }, e: true };
    const merged = mergeInto(defaults, { a: 5, b: { c: 7, d: [3], z: 1 }, e: 'no', extra: 9 });
    expect(merged).toEqual({ a: 5, b: { c: 'x', d: [3] }, e: true });
    expect(defaults.a).toBe(1); // not mutated
  });

  it('round-trips and rejects foreign or newer files', () => {
    const p = makeProject('egg.stl', { settings: { creaseDeg: 40 } });
    expect(parseProject(JSON.stringify(p)).state).toEqual({ settings: { creaseDeg: 40 } });
    expect(() => parseProject('{"hello":1}')).toThrow(/Not a Turninator/);
    expect(() => parseProject(JSON.stringify({ ...p, version: 99 }))).toThrow(/newer/);
  });
});
