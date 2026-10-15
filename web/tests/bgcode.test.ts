import { describe, expect, it } from 'vitest';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { decodeBgcode, heatshrinkDecode, meatpackDecode, crc32 } from '../src/core/bgcode';
import { parseGcode } from '../src/core/gcode';
import { gcodeFrom3mf, listZip } from '../src/core/zip';
import { deflateRawSync } from 'node:zlib';

describe('bgcode primitives', () => {
  it('computes standard CRC32', () => {
    expect(crc32(new TextEncoder().encode('123456789'))).toBe(0xcbf43926);
  });

  it('decodes heatshrink literals and back-references', () => {
    // Bits: 1 'a'(0x61) 1 'b'(0x62) then backref offset 2 (stored 1), count 4 (stored 3) with w=4, l=3.
    const bits = '1' + '01100001' + '1' + '01100010' + '0' + '0001' + '011';
    const padded = bits.padEnd(Math.ceil(bits.length / 8) * 8, '0');
    const bytes = new Uint8Array(padded.length / 8);
    for (let i = 0; i < bytes.length; i++) bytes[i] = parseInt(padded.slice(i * 8, i * 8 + 8), 2);
    expect(new TextDecoder().decode(heatshrinkDecode(bytes, 4, 3, 6))).toBe('ababab');
  });

  it('decodes MeatPack packed pairs, full-width characters and no-space mode', () => {
    // Enable packing, then "G1" -> 'G'(13) | '1'(1)<<4, then 'M' full-width + '\n'.
    const bytes = new Uint8Array([0xff, 0xff, 0xfb, 13 | (1 << 4), 0x0f | (12 << 4), 'M'.charCodeAt(0)]);
    expect(meatpackDecode(bytes)).toBe('G1M\n');
    const ns = new Uint8Array([0xff, 0xff, 0xfb, 0xff, 0xff, 0xf7, 11 | (5 << 4)]);
    expect(meatpackDecode(ns)).toBe('E5');
  });
});

const dir = join(__dirname, '..', 'test-results', 'gcode');
const bin = join(dir, 'WandRing.bgcode'), txt = join(dir, 'WandRing.gcode');
describe.skipIf(!existsSync(bin) || !existsSync(txt))('PrusaSlicer bgcode vs ASCII export', () => {
  it('decodes to the same toolpath as the ASCII G-code', async () => {
    const f = readFileSync(bin);
    const res = await decodeBgcode(f.buffer.slice(f.byteOffset, f.byteOffset + f.byteLength) as ArrayBuffer);
    const a = parseGcode(res.text);
    const b = parseGcode(readFileSync(txt, 'utf8'));
    console.log(`bgcode: ${res.blocks} blocks, ${res.gcodeBlocks} gcode; beads ${a.count} vs ${b.count}; features ${a.features.join(', ')}`);
    expect(a.count).toBe(b.count);
    expect(a.layerZ.length).toBe(b.layerZ.length);
    expect(a.stats.filamentMm).toBeCloseTo(b.stats.filamentMm, 3);
    for (const i of [0, Math.floor(a.count / 2), a.count - 1]) {
      expect(a.start[i * 3]).toBeCloseTo(b.start[i * 3], 4);
      expect(a.width[i]).toBeCloseTo(b.width[i], 4);
    }
    expect(a.features).toEqual(b.features);
  });
});


function makeZip(files: { name: string; data: Uint8Array; deflate: boolean }[]): ArrayBuffer {
  const enc = new TextEncoder();
  const chunks: Uint8Array[] = [];
  const central: Uint8Array[] = [];
  let offset = 0;
  for (const f of files) {
    const name = enc.encode(f.name);
    const body = f.deflate ? new Uint8Array(deflateRawSync(f.data)) : f.data;
    const local = new DataView(new ArrayBuffer(30));
    local.setUint32(0, 0x04034b50, true);
    local.setUint16(8, f.deflate ? 8 : 0, true);
    local.setUint32(18, body.length, true);
    local.setUint32(22, f.data.length, true);
    local.setUint16(26, name.length, true);
    chunks.push(new Uint8Array(local.buffer), name, body);
    const cd = new DataView(new ArrayBuffer(46));
    cd.setUint32(0, 0x02014b50, true);
    cd.setUint16(10, f.deflate ? 8 : 0, true);
    cd.setUint32(20, body.length, true);
    cd.setUint32(24, f.data.length, true);
    cd.setUint16(28, name.length, true);
    cd.setUint32(42, offset, true);
    central.push(new Uint8Array(cd.buffer), name);
    offset += 30 + name.length + body.length;
  }
  const cdSize = central.reduce((a, c) => a + c.length, 0);
  const eocd = new DataView(new ArrayBuffer(22));
  eocd.setUint32(0, 0x06054b50, true);
  eocd.setUint16(8, files.length, true);
  eocd.setUint16(10, files.length, true);
  eocd.setUint32(12, cdSize, true);
  eocd.setUint32(16, offset, true);
  const all = [...chunks, ...central, new Uint8Array(eocd.buffer)];
  const out = new Uint8Array(all.reduce((a, c) => a + c.length, 0));
  let p = 0;
  for (const c of all) { out.set(c, p); p += c.length; }
  return out.buffer;
}

describe('gcode.3mf', () => {
  it('extracts the first plate G-code from a zip', async () => {
    const g = new TextEncoder().encode('G90\nM83\n;WIDTH:0.4\n;HEIGHT:0.2\nG1 Z0.2\nG1 X0 Y0\nG1 X10 E0.5\n');
    const zip = makeZip([
      { name: '3D/3dmodel.model', data: new TextEncoder().encode('<model/>'), deflate: true },
      { name: 'Metadata/plate_2.gcode', data: g, deflate: true },
      { name: 'Metadata/plate_1.gcode', data: g, deflate: false },
    ]);
    expect(listZip(zip).length).toBe(3);
    const res = await gcodeFrom3mf(zip);
    expect(res.name).toBe('Metadata/plate_1.gcode');
    expect(res.plates.length).toBe(2);
    expect(parseGcode(res.text).count).toBe(1);
  });
});
