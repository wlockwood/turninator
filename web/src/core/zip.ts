// Minimal ZIP reader (stored + deflate entries, no zip64) for .gcode.3mf.

export class ZipError extends Error {}

export interface ZipEntry {
  name: string;
  method: number;
  compressedSize: number;
  size: number;
  localOffset: number;
}

export function isZip(bytes: Uint8Array): boolean {
  return bytes.length >= 4 && bytes[0] === 0x50 && bytes[1] === 0x4b && bytes[2] === 0x03 && bytes[3] === 0x04;
}

export function listZip(buffer: ArrayBuffer): ZipEntry[] {
  const view = new DataView(buffer);
  const bytes = new Uint8Array(buffer);
  // End of central directory: scan backwards over a possible comment.
  let eocd = -1;
  for (let i = bytes.length - 22; i >= Math.max(0, bytes.length - 22 - 65535); i--) {
    if (view.getUint32(i, true) === 0x06054b50) { eocd = i; break; }
  }
  if (eocd < 0) throw new ZipError('Not a zip archive (no end of central directory)');
  const count = view.getUint16(eocd + 10, true);
  let p = view.getUint32(eocd + 16, true);
  if (p === 0xffffffff) throw new ZipError('zip64 archives are not supported');
  const entries: ZipEntry[] = [];
  const dec = new TextDecoder();
  for (let n = 0; n < count; n++) {
    if (view.getUint32(p, true) !== 0x02014b50) throw new ZipError('Corrupt central directory');
    const method = view.getUint16(p + 10, true);
    const compressedSize = view.getUint32(p + 20, true);
    const size = view.getUint32(p + 24, true);
    const nameLen = view.getUint16(p + 28, true), extraLen = view.getUint16(p + 30, true), commentLen = view.getUint16(p + 32, true);
    const localOffset = view.getUint32(p + 42, true);
    const name = dec.decode(bytes.subarray(p + 46, p + 46 + nameLen));
    entries.push({ name, method, compressedSize, size, localOffset });
    p += 46 + nameLen + extraLen + commentLen;
  }
  return entries;
}

export async function readZipEntry(buffer: ArrayBuffer, e: ZipEntry): Promise<Uint8Array> {
  const view = new DataView(buffer);
  if (view.getUint32(e.localOffset, true) !== 0x04034b50) throw new ZipError(`Bad local header for ${e.name}`);
  const nameLen = view.getUint16(e.localOffset + 26, true), extraLen = view.getUint16(e.localOffset + 28, true);
  const start = e.localOffset + 30 + nameLen + extraLen;
  const data = new Uint8Array(buffer, start, e.compressedSize);
  if (e.method === 0) return data.slice();
  if (e.method !== 8) throw new ZipError(`Unsupported zip compression ${e.method} for ${e.name}`);
  const stream = new Blob([data as BlobPart]).stream().pipeThrough(new DecompressionStream('deflate-raw'));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

/** First plate G-code inside a Bambu Studio / OrcaSlicer .gcode.3mf. */
export async function gcodeFrom3mf(buffer: ArrayBuffer): Promise<{ name: string; text: string; plates: string[] }> {
  const entries = listZip(buffer);
  const plates = entries.filter((e) => /\.gcode$/i.test(e.name)).sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true }));
  if (plates.length === 0) throw new ZipError('This 3MF has no sliced G-code (export a sliced .gcode.3mf from your slicer)');
  const bytes = await readZipEntry(buffer, plates[0]);
  return { name: plates[0].name, text: new TextDecoder().decode(bytes), plates: plates.map((p) => p.name) };
}
