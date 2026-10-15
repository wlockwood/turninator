// UI-thread handle for the path tracer worker. Same API shape as the old
// in-thread PathTracer, but every GPU/BVH operation runs in the worker.

import type * as THREE from 'three';
import type { BeadChunkGeometry } from './pathtrace';
import type { CamDesc, GeomData, PtRequest, PtResponse, SceneDesc } from './pathtraceProtocol';
import type { MaterialSettings } from '../state/settings';

export interface PathTraceInput {
  geometry: THREE.BufferGeometry;
  modelMatrix: THREE.Matrix4;
  backdropGeometry: THREE.BufferGeometry;
  backdropMatrix: THREE.Matrix4;
  backdropVisible: boolean;
  background: string | null;
  material: MaterialSettings;
  envIntensity: number;
  chunks?: BeadChunkGeometry[];
  vertexColors?: boolean;
}

const attrIds = new WeakMap<object, number>();
let nextAttrId = 1;
const attrId = (a: object) => {
  let id = attrIds.get(a);
  if (!id) attrIds.set(a, (id = nextAttrId++));
  return id;
};

/** Content key: changes whenever any attribute object is replaced. */
function geomKey(g: THREE.BufferGeometry): string {
  const parts = [g.uuid];
  for (const name of Object.keys(g.attributes).sort()) parts.push(`${name}${attrId(g.attributes[name])}`);
  if (g.index) parts.push(`i${attrId(g.index)}`);
  return parts.join(':');
}

function geomData(g: THREE.BufferGeometry): GeomData {
  const attrs: GeomData['attrs'] = {};
  for (const name of ['position', 'normal', 'color', 'uv', 'tangent']) {
    const a = g.getAttribute(name) as THREE.BufferAttribute | undefined;
    if (a) attrs[name] = { array: a.array as Float32Array, itemSize: a.itemSize };
  }
  return { attrs, index: g.index ? (g.index.array as Uint32Array) : undefined };
}

export function camDesc(cam: THREE.Camera): CamDesc {
  cam.updateMatrixWorld();
  const p = cam as THREE.PerspectiveCamera, o = cam as THREE.OrthographicCamera;
  return {
    fov: p.fov ?? 50,
    aspect: p.aspect ?? 1,
    near: p.near,
    far: p.far,
    ortho: o.isOrthographicCamera ? [o.left, o.right, o.top, o.bottom] : undefined,
    matrixWorld: cam.matrixWorld.toArray(),
  };
}

export class PathTracerProxy {
  private worker: Worker;
  private nextId = 1;
  private pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void; onProgress?: (avg: number) => void }>();
  private workerKeys = new Set<string>();
  onFrame: ((bitmap: ImageBitmap, samples: number) => void) | null = null;
  onError: ((message: string) => void) | null = null;

  private constructor(worker: Worker) {
    this.worker = worker;
    worker.onmessage = (ev: MessageEvent<PtResponse>) => {
      const m = ev.data;
      if (m.kind === 'frame') {
        if (this.onFrame) this.onFrame(m.bitmap, m.samples);
        else m.bitmap.close();
        return;
      }
      if (m.kind === 'ready') return;
      const id = 'id' in m ? m.id : undefined;
      const p = id !== undefined ? this.pending.get(id) : undefined;
      if (m.kind === 'progress') { p?.onProgress?.(m.avg); return; }
      if (m.kind === 'error') {
        if (p) {
          this.pending.delete(id!);
          p.reject(m.message === 'AbortError' ? new DOMException('Render cancelled', 'AbortError') : new Error(m.message));
        } else {
          this.onError?.(m.message);
        }
        return;
      }
      if (p && id !== undefined) {
        this.pending.delete(id);
        p.resolve(m);
      }
    };
  }

  static create(): Promise<PathTracerProxy> {
    const worker = new Worker(new URL('../workers/pathtrace.worker.ts', import.meta.url), { type: 'module' });
    const proxy = new PathTracerProxy(worker);
    return new Promise((resolve, reject) => {
      const onInit = (ev: MessageEvent<PtResponse>) => {
        if (ev.data.kind === 'ready') { worker.removeEventListener('message', onInit); resolve(proxy); }
        else if (ev.data.kind === 'error') { worker.removeEventListener('message', onInit); worker.terminate(); reject(new Error(ev.data.message)); }
      };
      worker.addEventListener('message', onInit);
      worker.onerror = (e) => reject(new Error(e.message || 'Path tracer worker failed to start'));
      const weightsUrl = new URL('oidn/rt_hdr_calb_cnrm_small.tza', document.baseURI).href;
      proxy.send({ kind: 'init', weightsUrl });
    });
  }

  private send(m: PtRequest) {
    this.worker.postMessage(m);
  }

  private call<T>(make: (id: number) => PtRequest, onProgress?: (avg: number) => void): Promise<T> {
    const id = this.nextId++;
    const p = new Promise<T>((resolve, reject) => this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject, onProgress }));
    this.send(make(id));
    return p;
  }

  /** Send the scene; geometry the worker already holds is referenced by key only. */
  setScene(input: PathTraceInput, camera: THREE.Camera): Promise<void> {
    const geoms: Record<string, GeomData> = {};
    const used = new Set<string>();
    const ref = (g: THREE.BufferGeometry) => {
      const k = geomKey(g);
      used.add(k);
      if (!this.workerKeys.has(k) && !geoms[k]) geoms[k] = geomData(g);
      return k;
    };
    const scene: SceneDesc = {
      modelKey: input.chunks ? undefined : ref(input.geometry),
      chunks: input.chunks?.map((c) => ({ key: ref(c.geometry), maxLayer: c.maxLayer })),
      modelMatrix: input.modelMatrix.toArray(),
      backdropKey: ref(input.backdropGeometry),
      backdropMatrix: input.backdropMatrix.toArray(),
      backdropVisible: input.backdropVisible,
      background: input.background,
      material: input.material,
      envIntensity: input.envIntensity,
      vertexColors: input.vertexColors,
    };
    this.workerKeys = used;
    return this.call<void>((id) => ({ kind: 'scene', id, scene, camera: camDesc(camera), geoms }));
  }

  setModelMatrix(m: THREE.Matrix4) {
    this.send({ kind: 'modelMatrix', matrix: m.toArray() });
  }

  setBuildLayer(layer: number | null) {
    this.send({ kind: 'buildLayer', layer });
  }

  setCamera(camera: THREE.Camera) {
    this.send({ kind: 'camera', camera: camDesc(camera) });
  }

  setDenoise(enabled: boolean): Promise<void> {
    return this.call<void>((id) => ({ kind: 'denoise', id, enabled }));
  }

  async render(width: number, height: number, samples: number, signal?: AbortSignal, onSample?: (avg: number) => void): Promise<{ pixels: Uint8ClampedArray; stats: { samples: number; ms: number } }> {
    const id = this.nextId;
    const onAbort = () => this.send({ kind: 'abort', id });
    signal?.addEventListener('abort', onAbort, { once: true });
    try {
      const res = await this.call<{ pixels: Uint8ClampedArray; ms: number }>((i) => ({ kind: 'render', id: i, width, height, samples }), onSample);
      return { pixels: res.pixels, stats: { samples, ms: res.ms } };
    } finally {
      signal?.removeEventListener('abort', onAbort);
    }
  }

  startPreview(width: number, height: number, maxSamples: number) {
    this.send({ kind: 'preview', on: true, width, height, maxSamples });
  }

  resizePreview(width: number, height: number) {
    this.send({ kind: 'previewSize', width, height });
  }

  pausePreview(paused: boolean) {
    this.send({ kind: 'previewPause', paused });
  }

  stopPreview() {
    this.send({ kind: 'preview', on: false, width: 1, height: 1, maxSamples: 0 });
  }
}
