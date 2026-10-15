// Progressive path-traced overlay for the viewport. All tracing runs in the
// path tracer worker; this only forwards scene/camera changes and draws the
// frames the worker streams back, so the UI thread stays responsive.

import type { Studio } from './studio';
import type { BeadBake } from '../export/still';
import { getPathTracer } from './pathtraceHost';
import type { PathTracerProxy } from './pathtraceProxy';

export interface PreviewStatus {
  samples: number;
  state: 'starting' | 'building' | 'tracing' | 'error';
  message?: string;
}

export class PathTracePreview {
  private pt: PathTracerProxy | null = null;
  private running = false;
  private raf = 0;
  private sceneDirty = true;
  private sceneChangedAt = 0;
  private cameraDirty = false;
  private transformDirty = false;
  private building = false;
  private size = [0, 0];
  private prevOnChange: Studio['onChange'] = null;
  private display: HTMLCanvasElement;
  private ctx: ImageBitmapRenderingContext | null;

  constructor(
    private studio: Studio,
    private host: HTMLElement,
    private getBeads: () => Promise<BeadBake | undefined>,
    private onStatus: (s: PreviewStatus) => void,
    private scale = 1,
    private maxSamples = 1024,
  ) {
    this.display = document.createElement('canvas');
    Object.assign(this.display.style, { position: 'absolute', left: '0', top: '0', width: '100%', height: '100%', pointerEvents: 'none' });
    this.ctx = this.display.getContext('bitmaprenderer');
  }

  async start() {
    if (this.running) return;
    this.running = true;
    this.onStatus({ samples: 0, state: 'starting' });
    try {
      this.pt = await getPathTracer();
    } catch (err) {
      this.running = false;
      this.onStatus({ samples: 0, state: 'error', message: err instanceof Error ? err.message : String(err) });
      return;
    }
    if (!this.running) return;
    // The frame overlay is a later sibling, so it stays on top of this canvas.
    this.host.insertBefore(this.display, this.host.firstChild?.nextSibling ?? null);
    this.pt.onFrame = (bitmap, samples) => {
      if (!this.running) { bitmap.close(); return; }
      this.ctx?.transferFromImageBitmap(bitmap);
      if (!this.building) this.onStatus({ samples, state: 'tracing' });
    };
    this.pt.onError = (message) => this.onStatus({ samples: 0, state: 'error', message });
    this.prevOnChange = this.studio.onChange;
    this.studio.onChange = (reason) => {
      this.prevOnChange?.(reason);
      if (reason === 'camera') this.cameraDirty = true;
      else if (reason === 'transform') this.transformDirty = true;
      else { this.sceneDirty = true; this.sceneChangedAt = performance.now(); }
    };
    this.sceneDirty = true;
    this.sceneChangedAt = 0;
    this.loop();
  }

  stop() {
    this.running = false;
    cancelAnimationFrame(this.raf);
    this.pt?.stopPreview();
    if (this.pt) { this.pt.onFrame = null; this.pt.onError = null; }
    this.display.remove();
    this.studio.onChange = this.prevOnChange;
  }

  /** Pause tracing (e.g. while an export uses the same path tracer). */
  set paused(p: boolean) {
    this.pt?.pausePreview(p);
  }

  /** Force a scene rebuild (after an export reused the path tracer). */
  invalidateScene() {
    this.sceneDirty = true;
    this.sceneChangedAt = 0;
  }

  /** Cheap per-frame bookkeeping only; the worker does the tracing. */
  private loop = () => {
    if (!this.running) return;
    this.raf = requestAnimationFrame(this.loop);
    const pt = this.pt;
    if (!pt || this.building) return;
    const w = Math.max(1, Math.floor(this.host.clientWidth * this.scale));
    const h = Math.max(1, Math.floor(this.host.clientHeight * this.scale));
    if (this.sceneDirty) {
      if (performance.now() - this.sceneChangedAt < 350) return; // let edits settle
      this.sceneDirty = false;
      void this.rebuild(w, h);
      return;
    }
    if (w !== this.size[0] || h !== this.size[1]) {
      this.size = [w, h];
      pt.resizePreview(w, h);
    }
    if (this.transformDirty) {
      this.transformDirty = false;
      pt.setModelMatrix(this.studio.modelWorldMatrix());
      if (this.studio.modelKind === 'beads') pt.setBuildLayer(this.studio.beadBuildLayer());
    }
    if (this.cameraDirty) {
      this.cameraDirty = false;
      pt.setCamera(this.studio.activeCamera());
      pt.resizePreview(w, h);
    }
  };

  private async rebuild(w: number, h: number) {
    const pt = this.pt!;
    this.building = true;
    this.onStatus({ samples: 0, state: 'building' });
    try {
      const beads = this.studio.modelKind === 'beads' ? await this.getBeads() : undefined;
      if (!this.running) return;
      await pt.setScene(this.studio.pathTraceInput(false, beads), this.studio.activeCamera());
      if (beads) pt.setBuildLayer(this.studio.beadBuildLayer());
      this.size = [w, h];
      this.display.width = w;
      this.display.height = h;
      pt.startPreview(w, h, this.maxSamples);
      this.onStatus({ samples: 0, state: 'tracing' });
    } catch (err) {
      this.onStatus({ samples: 0, state: 'error', message: err instanceof Error ? err.message : String(err) });
    } finally {
      this.building = false;
    }
  }
}
