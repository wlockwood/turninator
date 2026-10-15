// High-resolution still export from the current viewport camera.

import type { Studio } from '../render/studio';
import type { OutputFile } from './turntable';
import { getPathTracer, type Engine } from '../render/pathtraceHost';
import type { BeadChunkGeometry } from '../render/pathtrace';

export interface StillOptions {
  width: number;
  height: number;
  supersample: number;
  transparent: boolean;
  engine: Engine;
  samples: number;
  denoise: boolean;
}

export const STILL_SIZES: { label: string; width: number; height: number }[] = [
  { label: '2048 × 2048', width: 2048, height: 2048 },
  { label: '4096 × 4096', width: 4096, height: 4096 },
  { label: '1920 × 1080', width: 1920, height: 1080 },
  { label: '3840 × 2160 (4K)', width: 3840, height: 2160 },
];

export const STILL_DEFAULTS: StillOptions = { width: 4096, height: 4096, supersample: 2, transparent: false, engine: 'studio', samples: 256, denoise: true };

/** Largest supersample factor that keeps the framebuffer within GPU limits. */
export function clampSupersample(studio: Studio, opts: StillOptions): number {
  const gl = studio.renderer.getContext();
  const maxRb = Math.min(gl.getParameter(gl.MAX_RENDERBUFFER_SIZE) as number, (gl.getParameter(gl.MAX_VIEWPORT_DIMS) as Int32Array)[0]);
  let ss = Math.max(1, Math.floor(opts.supersample));
  while (ss > 1 && (opts.width * ss > maxRb || opts.height * ss > maxRb || opts.width * opts.height * ss * ss > 80e6)) ss--;
  return ss;
}

export async function encodePng(pixels: Uint8ClampedArray, width: number, height: number): Promise<Blob> {
  const canvas = new OffscreenCanvas(width, height);
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('2D canvas unavailable');
  ctx.putImageData(new ImageData(pixels as Uint8ClampedArray<ArrayBuffer>, width, height), 0, 0);
  return canvas.convertToBlob({ type: 'image/png' });
}

export interface BeadBake {
  chunks: BeadChunkGeometry[];
  vertexColors: boolean;
  layerCount: number;
  /** Current build progress 0..1 (1 = complete). */
  progress: number;
}

export async function exportStill(studio: Studio, opts: StillOptions, signal?: AbortSignal, onProgress?: (done: number, total: number) => void, beads?: BeadBake): Promise<{ file: OutputFile; renderMs: number; supersample: number }> {
  const ss = opts.engine === 'pathtraced' ? 1 : clampSupersample(studio, opts);
  const t0 = performance.now();
  let pixels: Uint8ClampedArray;
  if (opts.engine === 'pathtraced') {
    const pt = await getPathTracer();
    await pt.setDenoise(opts.denoise);
    await pt.setScene(studio.pathTraceInput(opts.transparent, beads), studio.exportCamera());
    if (beads && beads.progress < 1) pt.setBuildLayer(beads.progress * beads.layerCount);
    const res = await pt.render(opts.width, opts.height, opts.samples, signal, (avg) => onProgress?.(Math.floor(avg), opts.samples));
    pixels = res.pixels;
  } else {
    pixels = studio.renderPixels(opts.width, opts.height, ss, studio.exportCamera(), opts.transparent);
  }
  const renderMs = performance.now() - t0;
  const t1 = performance.now();
  const blob = await encodePng(pixels, opts.width, opts.height);
  return { file: { kind: 'png', blob, ext: 'png', label: 'PNG', encodeMs: performance.now() - t1 }, renderMs, supersample: ss };
}
