// Prusa binary G-code (.bgcode) decoder -> plain G-code text.
//
// Container (libbgcode spec): file header "GCDE", u32 version, u16 checksum
// type (0 none, 1 CRC32). Blocks: u16 type, u16 compression, u32 uncompressed
// size, [u32 compressed size if compressed], parameters (u16 encoding; 6 bytes
// for thumbnails), data, [u32 CRC32]. Compression: 0 none, 1 deflate (zlib),
// 2 heatshrink 11/4, 3 heatshrink 12/4. G-code blocks may be MeatPack encoded
// (1) or MeatPack with comments (2).

export class BgcodeError extends Error {}

const BLOCK_GCODE = 1;
const BLOCK_THUMBNAIL = 5;

export function isBgcode(bytes: Uint8Array): boolean {
  return bytes.length >= 10 && bytes[0] === 0x47 && bytes[1] === 0x43 && bytes[2] === 0x44 && bytes[3] === 0x45;
}

/** heatshrink LZSS decoder (window 2^w, lookahead 2^l). */
export function heatshrinkDecode(input: Uint8Array, w: number, l: number, outSize: number): Uint8Array {
  const out = new Uint8Array(outSize);
  let o = 0;
  let bit = 0;
  const total = input.length * 8;
  const read = (n: number) => {
    let v = 0;
    for (let i = 0; i < n; i++, bit++) v = (v << 1) | ((input[bit >> 3] >> (7 - (bit & 7))) & 1);
    return v;
  };
  while (o < outSize) {
    if (bit + 1 > total) break;
    if (read(1) === 1) {
      if (bit + 8 > total) break;
      out[o++] = read(8);
    } else {
      if (bit + w + l > total) break;
      const offset = read(w) + 1;
      const count = read(l) + 1;
      if (offset > o) throw new BgcodeError('heatshrink back-reference before start of output');
      for (let i = 0; i < count && o < outSize; i++, o++) out[o] = out[o - offset];
    }
  }
  if (o !== outSize) throw new BgcodeError(`heatshrink produced ${o} of ${outSize} bytes`);
  return out;
}

const MP_TABLE = '0123456789. \nGX';

/** MeatPack decoder (Prusa variant used by bgcode). */
export function meatpackDecode(input: Uint8Array): string {
  const out: string[] = [];
  let packing = false;
  let noSpaces = false;
  let signal = 0;
  let fullChars = 0;
  let pendingSecond = '';
  const emit = (ch: string) => out.push(ch);
  const unpack = (code: number) => (code === 11 && noSpaces ? 'E' : MP_TABLE[code]);
  const handleChar = (c: number) => {
    if (!packing) {
      emit(String.fromCharCode(c));
      return;
    }
    if (fullChars > 0) {
      emit(String.fromCharCode(c));
      if (pendingSecond) {
        emit(pendingSecond);
        pendingSecond = '';
      }
      fullChars--;
      return;
    }
    const lo = c & 0x0f, hi = (c >> 4) & 0x0f;
    if (lo === 0x0f) {
      fullChars++;
      if (hi === 0x0f) fullChars++;
      else pendingSecond = unpack(hi);
      return;
    }
    const first = unpack(lo);
    emit(first);
    if (first !== '\n') {
      if (hi === 0x0f) fullChars++;
      else emit(unpack(hi));
    }
  };
  for (let i = 0; i < input.length; i++) {
    const c = input[i];
    if (c === 0xff) {
      if (signal === 1) {
        // Two signal bytes: the next byte is a command.
        const cmd = input[++i];
        signal = 0;
        if (cmd === 0xfb) packing = true;
        else if (cmd === 0xfa) packing = false;
        else if (cmd === 0xf9) { packing = false; noSpaces = false; }
        else if (cmd === 0xf7) noSpaces = true;
        else if (cmd === 0xf6) noSpaces = false;
        continue;
      }
      signal = 1;
      continue;
    }
    if (signal === 1) {
      handleChar(0xff);
      signal = 0;
    }
    handleChar(c);
  }
  return out.join('');
}

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

export function crc32(data: Uint8Array, start = 0, end = data.length): number {
  let c = 0xffffffff;
  for (let i = start; i < end; i++) c = CRC_TABLE[(c ^ data[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

async function inflate(data: Uint8Array): Promise<Uint8Array> {
  const stream = new Blob([data as BlobPart]).stream().pipeThrough(new DecompressionStream('deflate'));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

export interface BgcodeResult {
  text: string;
  blocks: number;
  gcodeBlocks: number;
  metadata: Record<string, string>;
}

export async function decodeBgcode(buffer: ArrayBuffer): Promise<BgcodeResult> {
  const bytes = new Uint8Array(buffer);
  if (!isBgcode(bytes)) throw new BgcodeError('Not a binary G-code file');
  const view = new DataView(buffer);
  const version = view.getUint32(4, true);
  if (version !== 1) throw new BgcodeError(`Unsupported bgcode version ${version}`);
  const checksumType = view.getUint16(8, true);
  let p = 10;
  const parts: string[] = [];
  const metadata: Record<string, string> = {};
  let blocks = 0, gcodeBlocks = 0;
  while (p < bytes.length) {
    if (p + 8 > bytes.length) throw new BgcodeError('Truncated block header');
    const blockStart = p;
    const type = view.getUint16(p, true);
    const compression = view.getUint16(p + 2, true);
    const usize = view.getUint32(p + 4, true);
    p += 8;
    let csize = usize;
    if (compression !== 0) {
      csize = view.getUint32(p, true);
      p += 4;
    }
    const paramSize = type === BLOCK_THUMBNAIL ? 6 : 2;
    const encoding = view.getUint16(p, true);
    p += paramSize;
    if (p + csize > bytes.length) throw new BgcodeError('Truncated block data');
    const raw = bytes.subarray(p, p + csize);
    p += csize;
    if (checksumType === 1) {
      const stored = view.getUint32(p, true);
      if (crc32(bytes, blockStart, p) !== stored) throw new BgcodeError(`Checksum mismatch in block ${blocks}`);
      p += 4;
    }
    blocks++;
    if (type === BLOCK_THUMBNAIL) continue;
    let data: Uint8Array;
    if (compression === 0) data = raw;
    else if (compression === 1) data = await inflate(raw);
    else if (compression === 2) data = heatshrinkDecode(raw, 11, 4, usize);
    else if (compression === 3) data = heatshrinkDecode(raw, 12, 4, usize);
    else throw new BgcodeError(`Unknown compression ${compression}`);
    if (type === BLOCK_GCODE) {
      gcodeBlocks++;
      parts.push(encoding === 0 ? new TextDecoder().decode(data) : meatpackDecode(data));
    } else {
      // Metadata blocks are INI-style key=value lines.
      for (const line of new TextDecoder().decode(data).split('\n')) {
        const eq = line.indexOf('=');
        if (eq > 0) metadata[line.slice(0, eq).trim()] = line.slice(eq + 1).trim();
      }
    }
  }
  // Slicer metadata carries settings like filament_diameter; expose as comments.
  const header = Object.entries(metadata)
    .filter(([k]) => /filament_diameter|layer_height|nozzle_diameter|generated|Producer/i.test(k))
    .map(([k, v]) => `; ${k} = ${v}`)
    .join('\n');
  return { text: `${header}\n${parts.join('')}`, blocks, gcodeBlocks, metadata };
}
