// Frame storage for exports: memory for small jobs, OPFS (origin private file
// system, synchronous access handles in workers) for large ones so long or
// high-resolution animations do not exhaust worker memory.

import type { FrameSource } from './gif';

export interface FrameStore extends FrameSource {
  put(i: number, px: Uint8ClampedArray): void;
  has(i: number): boolean;
  dispose(): void;
  readonly kind: 'memory' | 'opfs';
}

export class MemoryFrameStore implements FrameStore {
  readonly kind = 'memory';
  private frames: Uint8ClampedArray[] = [];
  constructor(public count: number) {}
  put(i: number, px: Uint8ClampedArray) { this.frames[i] = px; }
  has(i: number) { return !!this.frames[i]; }
  get(i: number) { return this.frames[i]; }
  dispose() { this.frames = []; }
}

interface SyncHandle {
  write(buf: ArrayBufferView, opts: { at: number }): number;
  read(buf: ArrayBufferView, opts: { at: number }): number;
  truncate(size: number): void;
  flush(): void;
  close(): void;
}

export class OpfsFrameStore implements FrameStore {
  readonly kind = 'opfs';
  private written = new Set<number>();
  private constructor(public count: number, private frameBytes: number, private handle: SyncHandle, private name: string) {}

  static async create(count: number, frameBytes: number, name: string): Promise<OpfsFrameStore> {
    const root = await navigator.storage.getDirectory();
    const file = await root.getFileHandle(name, { create: true });
    const handle = (await (file as unknown as { createSyncAccessHandle(): Promise<SyncHandle> }).createSyncAccessHandle());
    handle.truncate(0);
    return new OpfsFrameStore(count, frameBytes, handle, name);
  }

  put(i: number, px: Uint8ClampedArray) {
    const n = this.handle.write(px, { at: i * this.frameBytes });
    if (n !== this.frameBytes) throw new Error(`OPFS short write (${n}/${this.frameBytes} bytes); disk may be full`);
    this.written.add(i);
  }
  has(i: number) { return this.written.has(i); }
  get(i: number) {
    const buf = new Uint8ClampedArray(this.frameBytes);
    this.handle.read(buf, { at: i * this.frameBytes });
    return buf;
  }
  dispose() {
    try { this.handle.truncate(0); this.handle.close(); } catch { /* already closed */ }
    void navigator.storage.getDirectory().then((root) => root.removeEntry(this.name)).catch(() => {});
  }
}

/** Above this many bytes of frames, spill to OPFS when available. */
export const OPFS_THRESHOLD = 384 * 1024 * 1024;

export async function createFrameStore(count: number, frameBytes: number): Promise<FrameStore> {
  const total = count * frameBytes;
  const canOpfs = typeof navigator !== 'undefined' && !!navigator.storage?.getDirectory;
  if (total > OPFS_THRESHOLD && canOpfs) {
    try {
      return await OpfsFrameStore.create(count, frameBytes, `turninator-frames-${Date.now()}.bin`);
    } catch {
      // fall back to memory
    }
  }
  return new MemoryFrameStore(count);
}
