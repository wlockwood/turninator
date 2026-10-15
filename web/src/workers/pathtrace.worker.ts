/// <reference lib="webworker" />
// Path tracer worker: owns the WebGPU device, BVHs and progressive rendering
// so path tracing never blocks the UI thread. Geometry arrives once per
// geometry key (cached here); later scenes reference cached keys.

import * as THREE from 'three/webgpu';
import { PathTracer } from '../render/pathtrace';
import type { CamDesc, GeomData, PtRequest, PtResponse, SceneDesc } from '../render/pathtraceProtocol';

const post = (m: PtResponse, transfer: Transferable[] = []) => (self as unknown as DedicatedWorkerGlobalScope).postMessage(m, transfer);
const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

let pt: PathTracer | null = null;
const geoms = new Map<string, THREE.BufferGeometry>();
const aborted = new Set<number>();
let exporting = false;
const preview = { on: false, paused: false, width: 1, height: 1, maxSamples: 1024 };
let previewLoopRunning = false;

function buildGeometry(d: GeomData): THREE.BufferGeometry {
  const g = new THREE.BufferGeometry();
  for (const [name, a] of Object.entries(d.attrs)) g.setAttribute(name, new THREE.BufferAttribute(a.array, a.itemSize));
  if (d.index) g.setIndex(new THREE.BufferAttribute(d.index, 1));
  g.computeBoundingSphere();
  return g;
}

function buildCamera(c: CamDesc): THREE.Camera {
  const cam = c.ortho ? new THREE.OrthographicCamera(c.ortho[0], c.ortho[1], c.ortho[2], c.ortho[3], c.near, c.far) : new THREE.PerspectiveCamera(c.fov, c.aspect, c.near, c.far);
  cam.matrixAutoUpdate = false;
  cam.matrix.fromArray(c.matrixWorld);
  cam.matrixWorld.fromArray(c.matrixWorld);
  cam.matrixWorldInverse.copy(cam.matrixWorld).invert();
  cam.updateProjectionMatrix();
  return cam;
}

function setScene(scene: SceneDesc, camera: CamDesc, incoming: Record<string, GeomData>) {
  for (const [key, data] of Object.entries(incoming)) geoms.set(key, buildGeometry(data));
  const used = new Set([scene.modelKey, scene.backdropKey, ...(scene.chunks?.map((c) => c.key) ?? [])].filter(Boolean) as string[]);
  for (const [key, g] of geoms) if (!used.has(key)) { g.dispose(); geoms.delete(key); }
  const need = (k: string) => {
    const g = geoms.get(k);
    if (!g) throw new Error(`Path tracer worker is missing geometry ${k}`);
    return g;
  };
  pt!.setScene(
    {
      geometry: scene.modelKey ? need(scene.modelKey) : new THREE.BufferGeometry(),
      modelMatrix: new THREE.Matrix4().fromArray(scene.modelMatrix),
      backdropGeometry: need(scene.backdropKey),
      backdropMatrix: new THREE.Matrix4().fromArray(scene.backdropMatrix),
      backdropVisible: scene.backdropVisible,
      background: scene.background,
      material: scene.material,
      envIntensity: scene.envIntensity,
      chunks: scene.chunks?.map((c) => ({ geometry: need(c.key), maxLayer: c.maxLayer })),
      vertexColors: scene.vertexColors,
    },
    buildCamera(camera),
  );
  if (scene.buildLayer !== undefined) pt!.setBuildLayer(scene.buildLayer);
}

/** Progressive preview: one sample batch at a time, waiting for the GPU between batches. */
async function previewLoop() {
  if (previewLoopRunning) return;
  previewLoopRunning = true;
  let lastFrame = 0, batches = 0, samples = 0;
  try {
    while (preview.on) {
      if (!pt || preview.paused || exporting || !pt.ready) {
        await sleep(40);
        continue;
      }
      pt.previewSample();
      await pt.gpuIdle();
      batches++;
      if (batches % 20 === 0) samples = Math.floor(await pt.previewSamples());
      const now = performance.now();
      if (now - lastFrame > 50) {
        lastFrame = now;
        const bitmap = await pt.frameBitmap();
        post({ kind: 'frame', bitmap, samples }, [bitmap]);
      }
      await sleep(0);
    }
  } catch (err) {
    post({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
  } finally {
    previewLoopRunning = false;
  }
}

self.onmessage = async (ev: MessageEvent<PtRequest>) => {
  const req = ev.data;
  try {
    switch (req.kind) {
      case 'init': {
        pt = await PathTracer.create();
        pt.weightsUrl = req.weightsUrl;
        post({ kind: 'ready' });
        break;
      }
      case 'scene':
        setScene(req.scene, req.camera, req.geoms);
        post({ kind: 'done', id: req.id });
        break;
      case 'modelMatrix':
        pt?.setModelMatrix(new THREE.Matrix4().fromArray(req.matrix));
        break;
      case 'buildLayer':
        pt?.setBuildLayer(req.layer);
        break;
      case 'camera':
        pt?.setCamera(buildCamera(req.camera));
        break;
      case 'denoise':
        await pt!.setDenoise(req.enabled);
        post({ kind: 'done', id: req.id });
        break;
      case 'render': {
        exporting = true;
        try {
          const signal = { get aborted() { return aborted.has(req.id); } } as AbortSignal;
          const res = await pt!.render(req.width, req.height, req.samples, signal, (avg) => post({ kind: 'progress', id: req.id, avg }));
          post({ kind: 'rendered', id: req.id, pixels: res.pixels, ms: res.stats.ms }, [res.pixels.buffer]);
        } finally {
          exporting = false;
          aborted.delete(req.id);
        }
        break;
      }
      case 'abort':
        aborted.add(req.id);
        break;
      case 'preview':
        preview.on = req.on;
        if (req.on && pt) {
          Object.assign(preview, { width: req.width, height: req.height, maxSamples: req.maxSamples, paused: false });
          pt.startPreview(req.width, req.height, req.maxSamples);
          void previewLoop();
        }
        break;
      case 'previewSize':
        preview.width = req.width;
        preview.height = req.height;
        pt?.resizePreview(req.width, req.height);
        break;
      case 'previewPause':
        preview.paused = req.paused;
        break;
    }
  } catch (err) {
    const message = err instanceof Error ? (err.name === 'AbortError' ? 'AbortError' : err.message) : String(err);
    post({ kind: 'error', id: 'id' in req ? (req as { id?: number }).id : undefined, message });
  }
};
