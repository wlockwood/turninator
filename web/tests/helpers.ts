export type Tri = [number, number, number, number, number, number, number, number, number];

export function binaryStl(tris: Tri[], header = 'binary test'): ArrayBuffer {
  const buf = new ArrayBuffer(84 + 50 * tris.length);
  const view = new DataView(buf);
  const enc = new TextEncoder().encode(header.padEnd(80, ' ').slice(0, 80));
  new Uint8Array(buf).set(enc, 0);
  view.setUint32(80, tris.length, true);
  tris.forEach((t, i) => {
    const o = 84 + i * 50 + 12;
    t.forEach((v, k) => view.setFloat32(o + k * 4, v, true));
  });
  return buf;
}

export function asciiStl(tris: Tri[]): ArrayBuffer {
  const lines = ['solid test'];
  for (const t of tris) {
    lines.push(' facet normal 0 0 0', '  outer loop');
    for (let k = 0; k < 9; k += 3) lines.push(`   vertex ${t[k]} ${t[k + 1]} ${t[k + 2]}`);
    lines.push('  endloop', ' endfacet');
  }
  lines.push('endsolid test');
  return new TextEncoder().encode(lines.join('\n')).buffer as ArrayBuffer;
}

/** Axis-aligned box with outward winding, 12 triangles. */
export function box(x0: number, y0: number, z0: number, x1: number, y1: number, z1: number): Tri[] {
  const v = [
    [x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0],
    [x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1],
  ];
  const quads = [
    [0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7],
  ];
  const tris: Tri[] = [];
  for (const [a, b, c, d] of quads) {
    tris.push([...v[a], ...v[b], ...v[c]] as Tri, [...v[a], ...v[c], ...v[d]] as Tri);
  }
  return tris;
}
