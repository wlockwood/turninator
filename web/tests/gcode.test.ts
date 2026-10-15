import { describe, expect, it } from 'vitest';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { parseGcode, normaliseFeature, GcodeError } from '../src/core/gcode';

const g = (...lines: string[]) => lines.join('\n');

describe('G-code interpreter', () => {
  it('repays retraction debt before depositing and ignores E-only moves', () => {
    const tp = parseGcode(g(
      'G90', 'M82', 'G92 E0', ';HEIGHT:0.2', ';WIDTH:0.45',
      'G1 Z0.2', 'G1 X0 Y0',
      'G1 X10 E1', // bead 1
      'G1 E0', 'G92 E0', // retract 1 mm (absolute E), reset counter
      'G1 X20', // travel
      'G1 E1', // unretract: repays debt, no bead
      'G1 X30 E2', // bead 2 (net 1)
    ));
    expect(tp.count).toBe(2);
    expect(tp.stats.filamentMm).toBeCloseTo(2, 6);
  });

  it('handles relative extrusion and derives width from volume', () => {
    const area = 0.2 * (0.45 - 0.2) + (Math.PI * 0.2 * 0.2) / 4;
    const dE = (area * 10) / (Math.PI * 0.875 ** 2);
    const tp = parseGcode(g('G90', 'M83', ';HEIGHT:0.2', 'G1 Z0.2', 'G1 X0 Y0', `G1 X10 E${dE}`));
    expect(tp.count).toBe(1);
    expect(tp.width[0]).toBeCloseTo(0.45, 4);
    expect(tp.stats.widthFromVolume).toBe(1);
  });

  it('puts the bead top at nozzle Z and the print bottom at zero', () => {
    const tp = parseGcode(g('G90', 'M83', ';WIDTH:0.5', ';HEIGHT:0.3', 'G1 Z0.3', 'G1 X0 Y0', 'G1 X10 E0.5'));
    expect(tp.start[2]).toBeCloseTo(0.15, 6); // centre of a 0.3 bead after shifting
    expect(tp.min[2]).toBe(0);
    expect(tp.max[2]).toBeCloseTo(0.3, 6);
  });

  it('counts layers from extruding moves only (Z-hop is not a layer)', () => {
    const tp = parseGcode(g(
      'G90', 'M83', ';WIDTH:0.45',
      'G1 Z0.2', 'G1 X0 Y0', 'G1 X10 E0.5',
      'G1 Z0.6', 'G1 X0 Y5', 'G1 Z0.2', // hop + travel
      'G1 X10 E0.5',
      'G1 Z0.4', 'G1 X0 E0.5',
    ));
    expect(tp.layerZ.length).toBe(2);
    expect(Array.from(tp.layer)).toEqual([0, 0, 1]);
    expect(tp.height[2]).toBeCloseTo(0.2, 6);
  });

  it('tessellates arcs (G2/G3 with I/J) and accepts compact words', () => {
    const tp = parseGcode(g('G90', 'M83', ';WIDTH:0.45', ';HEIGHT:0.2', 'G1Z0.2', 'G1X10Y0', 'G3X0Y10I-10J0E1'));
    expect(tp.stats.arcs).toBe(1);
    expect(tp.count).toBeGreaterThan(8);
    let len = 0;
    for (let i = 0; i < tp.count; i++) {
      len += Math.hypot(tp.end[i * 3] - tp.start[i * 3], tp.end[i * 3 + 1] - tp.start[i * 3 + 1]);
    }
    expect(len).toBeCloseTo((Math.PI * 10) / 2, 1);
  });

  it('normalises feature names across slicers', () => {
    expect(normaliseFeature('External perimeter')).toBe('Outer wall');
    expect(normaliseFeature(' Outer wall')).toBe('Outer wall');
    expect(normaliseFeature('WALL-OUTER')).toBe('Outer wall');
    expect(normaliseFeature('Top solid infill')).toBe('Top surface');
    expect(normaliseFeature('Internal infill')).toBe('Sparse infill');
    expect(normaliseFeature('Skirt/Brim')).toBe('Skirt/Brim');
  });

  it('rejects files without extrusion', () => {
    expect(() => parseGcode(g('G90', 'G1 X10 Y10'))).toThrow(GcodeError);
  });
});

const dir = join(__dirname, '..', 'test-results', 'gcode');
const real = existsSync(dir) ? readdirSync(dir).filter((f) => f.endsWith('.gcode')) : [];
describe.skipIf(real.length === 0)('real slicer output (PrusaSlicer fixtures in test-results/gcode)', () => {
  for (const f of real) {
    it(`parses ${f}`, () => {
      const t0 = performance.now();
      const tp = parseGcode(readFileSync(join(dir, f), 'utf8'));
      const ms = performance.now() - t0;
      console.log(`${f}: ${tp.count} beads, ${tp.layerZ.length} layers, features [${tp.features.join(', ')}], ${ms.toFixed(0)} ms`);
      expect(tp.count).toBeGreaterThan(100);
      expect(tp.features).toContain('Outer wall');
      expect(tp.min[2]).toBe(0);
      // Layer heights should be the sliced 0.2 mm.
      const h = tp.layerZ[tp.layerZ.length - 1] - tp.layerZ[tp.layerZ.length - 2];
      expect(h).toBeCloseTo(0.2, 3);
    });
  }
});
