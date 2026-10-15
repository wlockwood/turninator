// Turntable export orchestration: render each angle in the Studio renderer,
// stream frames to the export worker, then encode GIF and/or video.

import type { Studio } from '../render/studio';
import { turntableAngles } from '../render/framing';
import type { ExportRequest, ExportResponse, EncodedFile } from '../workers/export.worker';
import type { Dither } from './gif';
import { VIDEO_FORMATS, type VideoFormat } from './video';
import { getPathTracer, type Engine } from '../render/pathtraceHost';
import type { BeadBake } from './still';
import type { SectionSettings } from '../render/inspect';
import type * as THREE from 'three';

/** Whole part on the left, the sectioned part on the right (same camera and angle). */
function renderSideBySide(studio: Studio, opts: TurntableOptions, cam: THREE.Camera): Uint8ClampedArray {
  const n = opts.size;
  const whole = studio.withInspection((s) => ({ ...s, section: { ...s.section, enabled: false } }), () => studio.renderPixels(n, n, opts.supersample, cam));
  const cut = studio.withInspection((s) => ({ ...s, section: { ...(opts.section ?? s.section), enabled: true } }), () => studio.renderPixels(n, n, opts.supersample, cam));
  const out = new Uint8ClampedArray(n * 2 * n * 4);
  for (let y = 0; y < n; y++) {
    out.set(whole.subarray(y * n * 4, (y + 1) * n * 4), y * n * 8);
    out.set(cut.subarray(y * n * 4, (y + 1) * n * 4), y * n * 8 + n * 4);
  }
  return out;
}
import { cameraFromKey, pathDuration, sampleKey, type CamKey, type CamTransition } from '../render/camPath';

export interface TurntableOptions {
  size: number;
  seconds: number;
  fps: number;
  supersample: number;
  elevationDeg: number;
  dither: Dither;
  ditherStrength: number;
  gif: boolean;
  video: VideoFormat | 'none';
  videoLoops: number;
  engine: Engine;
  samples: number;
  denoise: boolean;
  /** rotate: seamless spin. build: part grows (fixed view). build-rotate: grows while spinning. */
  motion: 'rotate' | 'build' | 'build-rotate' | 'path' | 'path-build';
  /** single: one view. section-side: whole part (left) beside its section (right), in sync. */
  layout: 'single' | 'section-side';
  /** Section used by the side-by-side layout. */
  section?: SectionSettings;
  /** Keyframed camera path (motion 'path' / 'path-build'); its duration replaces `seconds`. */
  path?: { keys: CamKey[]; transitions: CamTransition[]; firstHold: number };
}

export const TURNTABLE_DEFAULTS: TurntableOptions = {
  size: 960, seconds: 4, fps: 20, supersample: 2, elevationDeg: 20, dither: 'bayer', ditherStrength: 10,
  gif: true, video: 'none', videoLoops: 1, engine: 'studio', samples: 32, denoise: true, motion: 'rotate', layout: 'single',
};

export interface OutputFile {
  kind: 'gif' | 'video' | 'png';
  blob: Blob;
  ext: string;
  label: string;
  encodeMs: number;
}

export interface TurntableResult {
  files: OutputFile[];
  frames: number;
  renderMs: number;
}

export type Progress = (stage: 'render' | 'encode', done: number, total: number) => void;

let worker: Worker | null = null;
let nextJob = 1;

function exportWorker(): Worker {
  worker ??= new Worker(new URL('../workers/export.worker.ts', import.meta.url), { type: 'module' });
  return worker;
}

const nextTask = () => new Promise<void>((r) => setTimeout(r, 0));

export async function exportTurntable(studio: Studio, opts: TurntableOptions, onProgress: Progress, signal: AbortSignal, beads?: BeadBake): Promise<TurntableResult> {
  if (!opts.gif && opts.video === 'none') throw new Error('Choose at least one output format');
  const w = exportWorker();
  const jobId = nextJob++;
  const usePath = opts.motion === 'path' || opts.motion === 'path-build';
  if (usePath && (!opts.path || opts.path.keys.length < 2)) throw new Error('Add at least two camera keyframes for a camera path');
  const seconds = usePath ? pathDuration(opts.path!.keys, opts.path!.transitions, opts.path!.firstHold) : opts.seconds;
  const frames = Math.max(1, Math.round(seconds * opts.fps));
  const angles = turntableAngles(frames);
  const frameW = opts.layout === 'section-side' ? opts.size * 2 : opts.size;
  const send = (msg: ExportRequest, transfer: Transferable[] = []) => w.postMessage(msg, transfer);

  // Preserve the interactive camera; export uses its own fitted framing.
  const cam = studio.camera;
  const saved = { pos: cam.position.clone(), target: studio.controls.target.clone(), angle: studio.turntable.rotation.z };
  // Exports render exactly the viewport's export frame (no silent re-framing).
  const exportCam = () => studio.exportCamera();

  // Listen before sending anything so no response can be missed.
  let settle!: { resolve: (f: EncodedFile[]) => void; reject: (e: Error) => void };
  const done = new Promise<EncodedFile[]>((resolve, reject) => (settle = { resolve, reject }));
  done.catch(() => {}); // observed below; avoid unhandled-rejection noise on early abort
  const onMsg = (ev: MessageEvent<ExportResponse>) => {
    const m = ev.data;
    if (m.jobId !== jobId) return;
    if (m.kind === 'progress') onProgress('encode', m.done, m.total);
    else if (m.kind === 'done') settle.resolve(m.files);
    else if (m.kind === 'error') settle.reject(new Error(m.message));
  };
  w.addEventListener('message', onMsg);
  const onAbort = () => {
    send({ kind: 'abort', jobId });
    settle.reject(new DOMException('Export cancelled', 'AbortError'));
  };
  signal.addEventListener('abort', onAbort, { once: true });

  send({
    kind: 'begin',
    jobId,
    width: frameW,
    height: opts.size,
    frameCount: frames,
    formats: {
      gif: opts.gif ? { width: frameW, height: opts.size, delayMs: 1000 / opts.fps, dither: opts.dither, ditherStrength: opts.ditherStrength, maxColors: 256 } : undefined,
      video: opts.video !== 'none' ? { format: opts.video, fps: opts.fps, loops: Math.max(1, opts.videoLoops) } : undefined,
    },
  });

  const t0 = performance.now();
  const sideBySide = opts.layout === 'section-side';
  if (sideBySide && opts.engine === 'pathtraced') throw new Error('The side-by-side section layout is Studio-only for now');
  const building = opts.motion === 'build' || opts.motion === 'build-rotate' || opts.motion === 'path-build';
  // Build-up: the part completes at 85% of the clip, then holds/spins complete.
  const buildAt = (i: number) => Math.min(1, (i + 1) / Math.max(1, Math.round(frames * 0.85)));
  try {
    const pt = opts.engine === 'pathtraced' ? await getPathTracer() : null;
    if (pt) {
      await pt.setDenoise(opts.denoise);
      studio.setTurntableAngle(angles[0]);
      await pt.setScene(studio.pathTraceInput(false, beads), exportCam());
    }
    for (let i = 0; i < frames; i++) {
      if (signal.aborted) throw new DOMException('Export cancelled', 'AbortError');
      if (usePath) {
        const cam = cameraFromKey(sampleKey(opts.path!.keys, opts.path!.transitions, i / opts.fps, opts.path!.firstHold));
        studio.applyPerspective(cam.position, cam.target);
        studio.setTurntableAngle((saved.angle * 180) / Math.PI);
      } else {
        studio.setTurntableAngle(opts.motion === 'build' ? 0 : angles[i]);
      }
      if (building) studio.setBuildProgress(buildAt(i));
      let pixels: Uint8ClampedArray;
      if (pt) {
        if (beads) pt.setBuildLayer(building ? buildAt(i) * beads.layerCount : null);
        // STL build-up changes geometry each frame (cut + cap): rebuild the scene.
        if (building && studio.modelKind === 'mesh') await pt.setScene(studio.pathTraceInput(false), exportCam());
        pt.setModelMatrix(studio.modelWorldMatrix());
        if (usePath) pt.setCamera(exportCam());
        pixels = (await pt.render(opts.size, opts.size, opts.samples, signal)).pixels;
      } else {
        pixels = sideBySide ? renderSideBySide(studio, opts, exportCam()) : studio.renderPixels(opts.size, opts.size, opts.supersample, exportCam());
      }
      send({ kind: 'frame', jobId, index: i, pixels }, [pixels.buffer]);
      onProgress('render', i + 1, frames);
      await nextTask();
    }
    const renderMs = performance.now() - t0;
    send({ kind: 'finish', jobId });
    const encoded = await done;
    const files: OutputFile[] = encoded.map((f) => {
      if (f.kind === 'gif') return { kind: 'gif', blob: new Blob([f.bytes as BlobPart], { type: 'image/gif' }), ext: 'gif', label: 'GIF', encodeMs: f.ms };
      const v = VIDEO_FORMATS[f.format!];
      return { kind: 'video', blob: new Blob([f.bytes as BlobPart], { type: v.mime }), ext: v.ext, label: v.label, encodeMs: f.ms };
    });
    return { files, frames, renderMs };
  } catch (err) {
    send({ kind: 'abort', jobId });
    throw err;
  } finally {
    w.removeEventListener('message', onMsg);
    signal.removeEventListener('abort', onAbort);
    studio.turntable.rotation.z = saved.angle;
    if (building) studio.setBuildProgress(null);
    studio.applyPerspective([saved.pos.x, saved.pos.y, saved.pos.z], [saved.target.x, saved.target.y, saved.target.z]);
  }
}
