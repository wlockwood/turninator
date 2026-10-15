/// <reference lib="webworker" />
// Export worker: receives rendered frames, streams them into a video encoder
// (if requested) and keeps them for the two-pass GIF encoder (if requested).

import { encodeGif, type GifOptions } from '../export/gif';
import { VideoWriter, type VideoFormat } from '../export/video';
import { createFrameStore, type FrameStore } from '../export/frameStore';

export interface ExportFormats {
  gif?: GifOptions;
  video?: { format: VideoFormat; fps: number; loops: number };
}

export type ExportRequest =
  | { kind: 'begin'; jobId: number; width: number; height: number; frameCount: number; formats: ExportFormats }
  | { kind: 'frame'; jobId: number; index: number; pixels: Uint8ClampedArray }
  | { kind: 'finish'; jobId: number }
  | { kind: 'abort'; jobId: number };

export interface EncodedFile {
  kind: 'gif' | 'video';
  bytes: Uint8Array;
  ms: number;
  format?: VideoFormat;
}

export type ExportResponse =
  | { kind: 'progress'; jobId: number; stage: 'encode'; done: number; total: number }
  | { kind: 'done'; jobId: number; files: EncodedFile[] }
  | { kind: 'error'; jobId: number; message: string };

interface Job {
  id: number;
  width: number;
  height: number;
  frameCount: number;
  formats: ExportFormats;
  frames: FrameStore | null;
  video: VideoWriter | null;
  videoMs: number;
  queue: Promise<void>;
  failed: Error | null;
}

let job: Job | null = null;

const post = (msg: ExportResponse, transfer: Transferable[] = []) =>
  (self as unknown as DedicatedWorkerGlobalScope).postMessage(msg, transfer);

function enqueue(j: Job, fn: () => Promise<void>) {
  j.queue = j.queue.then(async () => {
    if (j.failed) return;
    try {
      await fn();
    } catch (err) {
      j.failed = err instanceof Error ? err : new Error(String(err));
    }
  });
}

self.onmessage = (ev: MessageEvent<ExportRequest>) => {
  const req = ev.data;
  if (req.kind === 'begin') {
    if (job?.video) void job.video.cancel().catch(() => {});
    job?.frames?.dispose();
    const j: Job = { id: req.jobId, width: req.width, height: req.height, frameCount: req.frameCount, formats: req.formats, frames: null, video: null, videoMs: 0, queue: Promise.resolve(), failed: null };
    job = j;
    const keep = !!req.formats.gif || (req.formats.video?.loops ?? 1) > 1;
    if (keep) enqueue(j, async () => { j.frames = await createFrameStore(j.frameCount, j.width * j.height * 4); });
    const v = req.formats.video;
    if (v) enqueue(j, async () => { j.video = await VideoWriter.create(v.format, j.width, j.height, v.fps); });
    return;
  }
  const j = job;
  if (!j || req.jobId !== j.id) return; // stale
  if (req.kind === 'abort') {
    if (j.video) void j.video.cancel().catch(() => {});
    j.queue.then(() => j.frames?.dispose());
    job = null;
    return;
  }
  if (req.kind === 'frame') {
    const keep = !!j.formats.gif || (j.formats.video?.loops ?? 1) > 1;
    if (keep) {
      const px = req.pixels;
      enqueue(j, async () => { j.frames!.put(req.index, px); });
    }
    if (j.formats.video) {
      const pixels = req.pixels;
      enqueue(j, async () => {
        const t = performance.now();
        await j.video!.addFrame(pixels, req.index);
        j.videoMs += performance.now() - t;
      });
    }
    return;
  }
  if (req.kind === 'finish') {
    enqueue(j, async () => {
      const files: EncodedFile[] = [];
      if (j.video && j.formats.video) {
        const t = performance.now();
        for (let loop = 1; loop < j.formats.video.loops; loop++) {
          for (let i = 0; i < j.frameCount; i++) await j.video.addFrame(j.frames!.get(i), loop * j.frameCount + i);
        }
        const bytes = await j.video.finish();
        files.push({ kind: 'video', bytes, ms: j.videoMs + performance.now() - t, format: j.formats.video.format });
      }
      if (j.formats.gif) {
        const t = performance.now();
        const missing = Array.from({ length: j.frameCount }, (_, i) => i).find((i) => !j.frames!.has(i));
        if (missing !== undefined) throw new Error(`Frame ${missing} missing`);
        const bytes = encodeGif(j.frames!, j.formats.gif, (done) => post({ kind: 'progress', jobId: j.id, stage: 'encode', done, total: j.frameCount }));
        files.push({ kind: 'gif', bytes, ms: performance.now() - t });
      }
      j.frames?.dispose();
      j.frames = null;
      post({ kind: 'done', jobId: j.id, files }, files.map((f) => f.bytes.buffer));
    });
    j.queue = j.queue.then(() => {
      if (j.failed) post({ kind: 'error', jobId: j.id, message: j.failed.message });
    });
  }
};
