// Path-traced rendering with three-gpu-pathtracer's WebGPU backend. Runs on
// its own WebGPURenderer and OffscreenCanvas — inside a dedicated worker
// (workers/pathtrace.worker.ts) so it never blocks the UI — and mirrors the Studio scene (model,
// sweep backdrop, material) lit by a procedural HDR softbox environment.
// Raster-only effects (layer shading on the original mesh, section caps,
// see-through) are not path traced; Sliced geometry is.

import * as THREE from 'three/webgpu';
import { OIDNDenoiser, WebGPUPathTracer } from 'three-gpu-pathtracer/webgpu';
import { createStudioEnvironment } from './studioEnv';
import type { MaterialSettings } from '../state/settings';

export interface PathTraceSceneInput {
  geometry: THREE.BufferGeometry;
  modelMatrix: THREE.Matrix4;
  backdropGeometry: THREE.BufferGeometry;
  backdropMatrix: THREE.Matrix4;
  backdropVisible: boolean;
  background: string | null;
  material: MaterialSettings;
  envIntensity: number;
  /** Normal map for baked layer ribs (geometry carries matching uv/tangent). */
  layerNormalMap?: THREE.Texture;
  /** Baked G-code bead chunks (replace `geometry`); revealed by build layer. */
  chunks?: BeadChunkGeometry[];
  vertexColors?: boolean;
}

export interface BeadChunkGeometry {
  geometry: THREE.BufferGeometry;
  maxLayer: number;
}

export interface PathTraceStats {
  samples: number;
  ms: number;
}

const nextTask = () => new Promise<void>((r) => setTimeout(r, 0));

export async function webgpuAvailable(): Promise<boolean> {
  try {
    const gpu = (navigator as Navigator & { gpu?: { requestAdapter(): Promise<unknown> } }).gpu;
    return !!gpu && !!(await gpu.requestAdapter());
  } catch {
    return false;
  }
}

export class PathTracer {
  readonly canvas: OffscreenCanvas;
  /** Absolute URL of the OIDN weights (relative URLs resolve differently in workers). */
  weightsUrl = '';
  readonly renderer: THREE.WebGPURenderer;
  readonly tracer: WebGPUPathTracer;
  private scene = new THREE.Scene();
  private model: THREE.Mesh;
  private backdrop: THREE.Mesh;
  private modelMaterial = new THREE.MeshPhysicalMaterial();
  private backdropMaterial = new THREE.MeshStandardMaterial({ roughness: 1, metalness: 0 });
  private camera: THREE.PerspectiveCamera | THREE.OrthographicCamera = new THREE.PerspectiveCamera(30, 1, 0.1, 10000);
  private env: THREE.DataTexture;
  private sceneSet = false;
  private chunkMeshes: { mesh: THREE.Mesh; maxLayer: number }[] = [];
  private baseMatrix = new THREE.Matrix4();
  private buildLayer: number | null = null;
  private denoiser: OIDNDenoiser | null = null;
  private denoiseEnabled = false;

  private constructor(canvas: OffscreenCanvas, renderer: THREE.WebGPURenderer) {
    this.canvas = canvas;
    this.renderer = renderer;
    renderer.toneMapping = THREE.AgXToneMapping;
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    renderer.setPixelRatio(1);
    this.tracer = new WebGPUPathTracer(renderer);
    this.tracer.synchronizeRenderSize = false;
    this.tracer.stableNoise = true;
    this.tracer.maxBounces = 8;
    this.useExportMode();

    this.env = createStudioEnvironment();
    this.scene.environment = this.env;
    this.scene.environmentRotation.set(Math.PI / 2, 0, 0);
    this.model = new THREE.Mesh(new THREE.BufferGeometry(), this.modelMaterial);
    this.backdrop = new THREE.Mesh(new THREE.BufferGeometry(), this.backdropMaterial);
    for (const m of [this.model, this.backdrop]) {
      m.matrixAutoUpdate = false;
      this.scene.add(m);
    }
  }

  /** Deterministic accumulation for exports: no low-res preview or fades. */
  private useExportMode() {
    const t = this.tracer;
    t.renderDelay = 0;
    t.dynamicLowRes = false;
    t.minSamples = 0;
    t.fadeDuration = 1;
  }

  /** Interactive progressive preview at a given size (call `previewSample` per frame). */
  startPreview(width: number, height: number, maxSamples: number) {
    const t = this.tracer;
    t.renderDelay = 80;
    t.dynamicLowRes = true;
    t.lowResScale = 0.25;
    t.minSamples = 1;
    t.fadeDuration = 250;
    t.maxSamples = maxSamples;
    this.resizePreview(width, height);
  }

  resizePreview(width: number, height: number) {
    const persp = this.camera as THREE.PerspectiveCamera;
    if (persp.isPerspectiveCamera) {
      persp.aspect = width / height;
      persp.updateProjectionMatrix();
    }
    this.renderer.setSize(width, height, false);
    this.tracer.setSize(width, height);
    this.tracer.frameBudget = Math.max(250_000, Math.floor((width * height) / 2));
    this.tracer.updateCamera();
  }

  get ready() {
    return this.sceneSet;
  }

  /** Resolves when all submitted GPU work has finished (keeps the queue short). */
  async gpuIdle() {
    await (this.renderer.backend as unknown as { device?: GPUDevice }).device?.queue.onSubmittedWorkDone();
  }

  /** Snapshot of the presented preview image. */
  frameBitmap(): Promise<ImageBitmap> {
    return createImageBitmap(this.canvas);
  }

  previewSample() {
    if (!this.sceneSet) return;
    if (this.gpuError) throw new Error(`Path tracer GPU error: ${this.gpuError.split('\n')[0]}`);
    this.tracer.renderSample();
  }

  async previewSamples(): Promise<number> {
    return (await this.tracer.getSampleCountsAsync()).avg;
  }

  /** Last uncaptured WebGPU error (validation, out of memory, device lost). */
  private gpuError: string | null = null;

  static async create(): Promise<PathTracer> {
    const gpu = (navigator as Navigator & { gpu?: GPU }).gpu;
    const adapter = gpu ? await gpu.requestAdapter() : null;
    if (!adapter) throw new Error('WebGPU is not available in this browser');
    const canvas = new OffscreenCanvas(64, 64);
    // Large toolpaths need BVH/geometry buffers beyond the 128 MB default binding size.
    const requiredLimits = {
      maxStorageBufferBindingSize: adapter.limits.maxStorageBufferBindingSize,
      maxBufferSize: adapter.limits.maxBufferSize,
    };
    const renderer = new THREE.WebGPURenderer({ canvas: canvas as unknown as HTMLCanvasElement, antialias: false, alpha: true, requiredLimits } as ConstructorParameters<typeof THREE.WebGPURenderer>[0]);
    await renderer.init();
    const pt = new PathTracer(canvas, renderer);
    const device = (renderer.backend as unknown as { device?: GPUDevice }).device;
    device?.addEventListener('uncapturederror', (e) => {
      pt.gpuError ??= (e as GPUUncapturedErrorEvent).error.message;
    });
    device?.lost.then((info) => {
      pt.gpuError ??= `GPU device lost: ${info.message}`;
    });
    return pt;
  }

  setScene(input: PathTraceSceneInput, camera: THREE.Camera) {
    for (const c of this.chunkMeshes) this.scene.remove(c.mesh);
    this.chunkMeshes = [];
    this.baseMatrix.copy(input.modelMatrix);
    this.buildLayer = null;
    if (input.chunks) {
      this.scene.remove(this.model);
      for (const c of input.chunks) {
        const mesh = new THREE.Mesh(c.geometry, this.modelMaterial);
        mesh.matrixAutoUpdate = false;
        this.scene.add(mesh);
        this.chunkMeshes.push({ mesh, maxLayer: c.maxLayer });
      }
    } else {
      if (!this.model.parent) this.scene.add(this.model);
      this.model.geometry = input.geometry;
    }
    this.applyModelMatrices();
    this.modelMaterial.vertexColors = !!input.vertexColors;
    this.backdrop.geometry = input.backdropGeometry;
    this.backdrop.matrix.copy(input.backdropMatrix);
    this.backdrop.matrixWorld.copy(input.backdropMatrix);
    this.backdrop.visible = input.backdropVisible;
    this.applyMaterial(input.material);
    if (input.vertexColors) this.modelMaterial.color.set(1, 1, 1);
    this.modelMaterial.normalMap = input.layerNormalMap ?? null;
    this.modelMaterial.normalScale.set(1, 1); // tilt boost is baked into the map
    this.modelMaterial.needsUpdate = true;
    const transmissive = input.material.transmission > 0;
    // Beads are thousands of overlapping volumes; refracting through each one
    // exhausts the bounce budget and goes black. Translucent G-code instead uses
    // stochastic alpha (pass-through events do not consume bounces), giving
    // stacked translucency that darkens with depth like a real translucent print.
    const beadAlpha = !!input.chunks && transmissive;
    this.modelMaterial.transparent = beadAlpha;
    this.modelMaterial.depthWrite = !beadAlpha;
    if (beadAlpha) {
      const clarity = input.material.transmission; // 0..1
      this.modelMaterial.transmission = 0;
      this.modelMaterial.opacity = 0.45 - 0.33 * clarity; // tuned: frosted "translucent FDM" look
    } else {
      this.modelMaterial.opacity = 1;
    }
    this.tracer.maxTransparentBounces = beadAlpha ? 160 : 5;
    this.tracer.maxBounces = transmissive && !beadAlpha ? 24 : 8;
    this.backdropMaterial.color.set(input.background ?? '#ffffff');
    this.scene.background = input.background ? new THREE.Color(input.background) : null;
    this.renderer.setClearColor(0x000000, input.background ? 1 : 0);
    this.scene.environmentIntensity = input.envIntensity;
    this.copyCamera(camera);
    this.tracer.setScene(this.scene, this.camera);
    this.sceneSet = true;
  }

  private applyMaterial(m: MaterialSettings) {
    const mat = this.modelMaterial;
    mat.color.set(m.color);
    mat.roughness = m.roughness;
    mat.metalness = m.metalness;
    mat.clearcoat = m.clearcoat;
    mat.clearcoatRoughness = m.clearcoatRoughness;
    mat.sheen = m.sheen;
    mat.sheenColor.set(m.color).lerp(new THREE.Color(1, 1, 1), 0.5);
    mat.sheenRoughness = 0.6;
    mat.transmission = m.transmission;
    mat.thickness = m.thickness;
    mat.attenuationDistance = m.attenuationDistance;
    mat.attenuationColor.set(m.color);
    mat.ior = m.ior;
    mat.side = THREE.DoubleSide;
  }

  private copyCamera(src: THREE.Camera) {
    if ((src as THREE.OrthographicCamera).isOrthographicCamera) {
      const o = src as THREE.OrthographicCamera;
      if (!(this.camera as THREE.OrthographicCamera).isOrthographicCamera) this.camera = new THREE.OrthographicCamera();
      const c = this.camera as THREE.OrthographicCamera;
      c.left = o.left; c.right = o.right; c.top = o.top; c.bottom = o.bottom;
    } else {
      const p = src as THREE.PerspectiveCamera;
      if (!(this.camera as THREE.PerspectiveCamera).isPerspectiveCamera) this.camera = new THREE.PerspectiveCamera();
      const c = this.camera as THREE.PerspectiveCamera;
      c.fov = p.fov; c.aspect = p.aspect;
    }
    this.camera.near = (src as THREE.PerspectiveCamera).near;
    this.camera.far = (src as THREE.PerspectiveCamera).far;
    src.updateMatrixWorld();
    this.camera.matrixAutoUpdate = false;
    this.camera.matrix.copy(src.matrixWorld);
    this.camera.matrixWorld.copy(src.matrixWorld);
    this.camera.matrixWorldInverse.copy(src.matrixWorldInverse);
    this.camera.updateProjectionMatrix();
  }

  /** Enable OIDN denoising (weights load lazily on first use). */
  async setDenoise(enabled: boolean) {
    if (enabled && !this.denoiser) {
      const { initUNetFromURL } = await import('oidn-web');
      this.denoiser = new OIDNDenoiser({ initUNetFromURL, auxWeightsUrl: this.weightsUrl || `${import.meta.env.BASE_URL}oidn/rt_hdr_calb_cnrm_small.tza` });
    }
    this.denoiseEnabled = enabled;
    this.tracer.setDenoiser(enabled ? this.denoiser : null);
  }

  private applyModelMatrices() {
    const hidden = new THREE.Matrix4().makeTranslation(0, 0, -1e5).multiply(this.baseMatrix);
    this.model.matrix.copy(this.baseMatrix);
    this.model.matrixWorld.copy(this.baseMatrix);
    for (const c of this.chunkMeshes) {
      const show = this.buildLayer === null || c.maxLayer < this.buildLayer;
      c.mesh.matrix.copy(show ? this.baseMatrix : hidden);
      c.mesh.matrixWorld.copy(c.mesh.matrix);
    }
  }

  setModelMatrix(m: THREE.Matrix4) {
    this.baseMatrix.copy(m);
    this.applyModelMatrices();
    this.tracer.updateTransforms();
  }

  /** Reveal bead chunks whose layers are below `layer` (null = everything). */
  setBuildLayer(layer: number | null) {
    this.buildLayer = layer;
    this.applyModelMatrices();
    if (this.sceneSet) this.tracer.updateTransforms();
  }

  setCamera(camera: THREE.Camera) {
    this.copyCamera(camera);
    this.tracer.setCamera(this.camera);
    this.tracer.updateCamera();
  }

  /**
   * Accumulate `samples` per pixel at the given size and return top-down,
   * tone-mapped sRGB RGBA (straight alpha).
   */
  async render(width: number, height: number, samples: number, signal?: AbortSignal, onSample?: (avg: number) => void): Promise<{ pixels: Uint8ClampedArray; stats: PathTraceStats }> {
    if (!this.sceneSet) throw new Error('Path tracer scene not set');
    this.useExportMode();
    const t0 = performance.now();
    const persp = this.camera as THREE.PerspectiveCamera;
    if (persp.isPerspectiveCamera) {
      persp.aspect = width / height;
      persp.updateProjectionMatrix();
    }
    this.renderer.setSize(width, height, false);
    this.tracer.setSize(width, height);
    this.tracer.frameBudget = Math.max(250_000, width * height);
    this.tracer.maxSamples = samples;
    this.tracer.updateCamera();
    this.tracer.reset();
    this.gpuError = null;
    let calls = 0;
    let lastAvg = -1, lastProgress = performance.now();
    for (;;) {
      if (signal?.aborted) throw new DOMException('Render cancelled', 'AbortError');
      const gpuError = this.gpuError as string | null; // set asynchronously by the device
      if (gpuError) throw new Error(`Path tracer GPU error: ${gpuError.split('\n')[0]}`);
      this.tracer.renderSample();
      calls++;
      if (calls % 4 === 0) {
        const counts = await this.tracer.getSampleCountsAsync();
        onSample?.(counts.avg);
        if (counts.min >= samples) break;
        if (counts.avg > lastAvg) { lastAvg = counts.avg; lastProgress = performance.now(); }
        else if (performance.now() - lastProgress > 60_000) throw new Error('Path tracer made no progress for 60 s');
      } else {
        await nextTask();
      }
    }
    // Present once more so the canvas holds the converged (and denoised) image.
    this.tracer.renderSample();
    if (this.denoiseEnabled && this.denoiser) {
      const deadline = performance.now() + 60_000;
      while (!this.denoiser.complete) {
        if (signal?.aborted) throw new DOMException('Render cancelled', 'AbortError');
        if (performance.now() > deadline) throw new Error('Denoiser timed out');
        await nextTask();
        this.tracer.renderSample();
      }
      this.tracer.renderSample();
    }
    await (this.renderer.backend as unknown as { device?: { queue: { onSubmittedWorkDone(): Promise<void> } } }).device?.queue.onSubmittedWorkDone();
    const bitmap = await createImageBitmap(this.canvas);
    const off = new OffscreenCanvas(width, height);
    const ctx = off.getContext('2d')!;
    ctx.drawImage(bitmap, 0, 0);
    bitmap.close();
    const pixels = ctx.getImageData(0, 0, width, height).data;
    return { pixels, stats: { samples, ms: performance.now() - t0 } };
  }

  dispose() {
    this.tracer.dispose();
    this.env.dispose();
    this.renderer.dispose();
  }
}
