import type { ImportedModel, ImportOptions } from '../core/pipeline';

type DistributiveOmit<T, K extends PropertyKey> = T extends unknown ? Omit<T, K> : never;

export type CoreRequest =
  | { id: number; kind: 'import'; name: string; buffer: ArrayBuffer; options: ImportOptions }
  | { id: number; kind: 'renormal'; creaseDeg: number }
  | { id: number; kind: 'flatFace' }
  | { id: number; kind: 'gcode'; name: string; buffer: ArrayBuffer }
  | { id: number; kind: 'bakeBeads'; hiddenMask: number; colorMode: 'material' | 'feature' | 'tool' | 'layer'; featureColors: number[][]; toolColors: number[][]; chunks: number }
  | { id: number; kind: 'slice'; printMatrix: number[]; heightMm: number; firstMm: number; creaseDeg: number; maxTriangles: number; boundaries?: number[] }
  | { id: number; kind: 'adaptive'; printMatrix: number[]; minMm: number; maxMm: number; cuspMm: number; firstMm: number }
  | { id: number; kind: 'gcodeLayers'; buffer: ArrayBuffer };

export interface ToolpathMessage {
  name: string;
  count: number;
  start: Float32Array;
  end: Float32Array;
  width: Float32Array;
  height: Float32Array;
  layer: Uint32Array;
  feature: Uint8Array;
  tool: Uint8Array;
  /** Subsampled bead endpoints for bounds/framing. */
  points: Float32Array;
  layerCount: number;
  layerZ: number[];
  features: { name: string; count: number }[];
  min: [number, number, number];
  max: [number, number, number];
  stats: import('../core/gcode').Toolpath['stats'];
  warnings: string[];
  ms: number;
}

export interface SliceStats {
  triangles: number;
  layers: number;
  wallStrips: number;
  openChains: number;
  ms: number;
  warnings: string[];
}

export type CoreResponse =
  | { id: number; kind: 'imported'; model: ImportedModel }
  | { id: number; kind: 'normals'; normals: Float32Array }
  | { id: number; kind: 'flatFace'; normal: [number, number, number] | null; areaMm2: number }
  | { id: number; kind: 'toolpath'; toolpath: ToolpathMessage }
  | { id: number; kind: 'schedule'; boundaries: number[] }
  | { id: number; kind: 'baked'; chunks: import('../core/beadMesh').BakedChunk[]; triangles: number; ms: number }
  | { id: number; kind: 'sliced'; positions: Float32Array; normals: Float32Array; stats: SliceStats }
  | { id: number; kind: 'progress'; done: number; total: number }
  | { id: number; kind: 'error'; message: string };

/** Promise wrapper around the geometry worker with stale-result protection. */
export class CoreClient {
  private worker: Worker;
  private nextId = 1;
  private pending = new Map<number, { resolve: (r: CoreResponse) => void; onProgress?: (done: number, total: number) => void }>();

  constructor() {
    this.worker = new Worker(new URL('./core.worker.ts', import.meta.url), { type: 'module' });
    this.worker.onmessage = (ev: MessageEvent<CoreResponse>) => {
      const p = this.pending.get(ev.data.id);
      if (p && ev.data.kind === 'progress') {
        p.onProgress?.(ev.data.done, ev.data.total);
        return;
      }
      if (p) {
        this.pending.delete(ev.data.id);
        p.resolve(ev.data);
      }
    };
  }

  request(req: DistributiveOmit<CoreRequest, 'id'>, transfer: Transferable[] = [], onProgress?: (done: number, total: number) => void): { id: number; result: Promise<CoreResponse> } {
    const id = this.nextId++;
    const result = new Promise<CoreResponse>((resolve) => this.pending.set(id, { resolve, onProgress }));
    this.worker.postMessage({ ...req, id } as CoreRequest, transfer);
    return { id, result };
  }
}
