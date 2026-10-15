/// <reference lib="webworker" />
// Geometry worker: all CPU-heavy mesh work runs here, off the UI thread.
// Requests carry an id; the main thread ignores responses for stale ids.

import { importStl, renormal, transferList, type ImportedModel } from '../core/pipeline';
import type { CoreRequest, CoreResponse } from './protocol';
import { sliceMesh } from '../core/slicer';
import { adaptiveSchedule, explicitSchedule, scheduleFromLayerTops, uniformSchedule } from '../core/layers';
import { largestFlatFace } from '../render/orient';
import { AUXILIARY_FEATURES, parseGcode, type Toolpath } from '../core/gcode';
import { bakeBeads } from '../core/beadMesh';
import { decodeBgcode, isBgcode } from '../core/bgcode';
import { gcodeFrom3mf, isZip } from '../core/zip';

/** Decode any supported G-code container to text. */
async function gcodeText(buffer: ArrayBuffer): Promise<{ text: string; container: string }> {
  const head = new Uint8Array(buffer, 0, Math.min(16, buffer.byteLength));
  if (isBgcode(head)) return { text: (await decodeBgcode(buffer)).text, container: 'bgcode' };
  if (isZip(head)) {
    const r = await gcodeFrom3mf(buffer);
    return { text: r.text, container: `3mf:${r.name}${r.plates.length > 1 ? ` (1 of ${r.plates.length} plates)` : ''}` };
  }
  return { text: new TextDecoder().decode(buffer), container: 'gcode' };
}

let toolpath: Toolpath | null = null;

/** Apply a column-major 4x4 affine matrix to xyz triples (w = 1 or 0). */
function transform(src: Float32Array, m: number[], w: 0 | 1): Float32Array {
  const out = new Float32Array(src.length);
  for (let i = 0; i < src.length; i += 3) {
    const x = src[i], y = src[i + 1], z = src[i + 2];
    out[i] = m[0] * x + m[4] * y + m[8] * z + w * m[12];
    out[i + 1] = m[1] * x + m[5] * y + m[9] * z + w * m[13];
    out[i + 2] = m[2] * x + m[6] * y + m[10] * z + w * m[14];
  }
  return out;
}

/** Inverse of a rigid (rotation + translation) column-major matrix. */
function invertRigid(m: number[]): number[] {
  const r = [m[0], m[4], m[8], 0, m[1], m[5], m[9], 0, m[2], m[6], m[10], 0, 0, 0, 0, 1];
  const t = [m[12], m[13], m[14]];
  r[12] = -(r[0] * t[0] + r[4] * t[1] + r[8] * t[2]);
  r[13] = -(r[1] * t[0] + r[5] * t[1] + r[9] * t[2]);
  r[14] = -(r[2] * t[0] + r[6] * t[1] + r[10] * t[2]);
  return r;
}

let current: ImportedModel | null = null;

function reply(msg: CoreResponse, transfer: Transferable[] = []) {
  (self as unknown as DedicatedWorkerGlobalScope).postMessage(msg, transfer);
}

self.onmessage = async (ev: MessageEvent<CoreRequest>) => {
  const req = ev.data;
  try {
    if (req.kind === 'import') {
      const model = importStl(req.name, req.buffer, req.options);
      // Keep a private copy of welded data for later re-normalling/slicing.
      toolpath = null;
      current = {
        ...model,
        weldedPositions: model.weldedPositions.slice(),
        weldedIndices: model.weldedIndices.slice(),
        shellOf: model.shellOf.slice(),
      };
      reply({ id: req.id, kind: 'imported', model }, transferList(model));
    } else if (req.kind === 'slice') {
      if (!current) throw new Error('No model loaded');
      const build = transform(current.weldedPositions, req.printMatrix, 1);
      let maxZ = 0;
      for (let i = 2; i < build.length; i += 3) maxZ = Math.max(maxZ, build[i]);
      const schedule = req.boundaries ? explicitSchedule(req.boundaries) : uniformSchedule(req.heightMm, req.firstMm, maxZ);
      const r = sliceMesh(
        { positions: build, indices: current.weldedIndices, shellOf: current.shellOf, boundaries: schedule.boundaries, creaseDeg: req.creaseDeg, cleanMm: 0.001, maxTriangles: req.maxTriangles },
        (done, total) => reply({ id: req.id, kind: 'progress', done, total }),
      );
      const inv = invertRigid(req.printMatrix);
      const positions = transform(r.positions, inv, 1);
      const normals = transform(r.normals, inv, 0);
      reply(
        { id: req.id, kind: 'sliced', positions, normals, stats: { triangles: r.triangles, layers: r.layers, wallStrips: r.wallStrips, openChains: r.openChains, ms: r.ms, warnings: r.warnings } },
        [positions.buffer, normals.buffer],
      );
    } else if (req.kind === 'gcode') {
      const t0 = performance.now();
      const { text, container } = await gcodeText(req.buffer);
      const tp = parseGcode(text);
      if (container !== 'gcode') tp.warnings.unshift(`Decoded from ${container}`);
      // Keep a private copy for baking; the originals are transferred.
      toolpath = { ...tp, start: tp.start.slice(), end: tp.end.slice(), width: tp.width.slice(), height: tp.height.slice(), layer: tp.layer.slice(), feature: tp.feature.slice(), tool: tp.tool.slice() };
      const counts = new Array(tp.features.length).fill(0);
      for (let i = 0; i < tp.count; i++) counts[tp.feature[i]]++;
      // Endpoints (subsampled) give framing and bounds the same inputs as STL vertices.
      const stride = Math.max(1, Math.floor(tp.count / 200_000));
      const pts: number[] = [];
      const auxIds = new Set(tp.features.map((f, i) => (AUXILIARY_FEATURES.has(f) ? i : -1)).filter((i) => i >= 0));
      for (let i = 0; i < tp.count; i += stride) if (!auxIds.has(tp.feature[i])) pts.push(tp.start[i * 3], tp.start[i * 3 + 1], tp.start[i * 3 + 2], tp.end[i * 3], tp.end[i * 3 + 1], tp.end[i * 3 + 2]);
      pts.push(tp.min[0], tp.min[1], tp.min[2], tp.max[0], tp.max[1], tp.max[2]);
      const points = Float32Array.from(pts);
      current = null;
      reply(
        {
          id: req.id,
          kind: 'toolpath',
          toolpath: {
            name: req.name, count: tp.count, start: tp.start, end: tp.end, width: tp.width, height: tp.height,
            layer: tp.layer, feature: tp.feature, tool: tp.tool, points, layerCount: tp.layerZ.length, layerZ: tp.layerZ,
            features: tp.features.map((name, i) => ({ name, count: counts[i] })), min: tp.min, max: tp.max,
            stats: tp.stats, warnings: tp.warnings, ms: performance.now() - t0,
          },
        },
        [tp.start.buffer, tp.end.buffer, tp.width.buffer, tp.height.buffer, tp.layer.buffer, tp.feature.buffer, tp.tool.buffer, points.buffer],
      );
    } else if (req.kind === 'adaptive') {
      if (!current) throw new Error('No model loaded');
      const build = transform(current.weldedPositions, req.printMatrix, 1);
      const s = adaptiveSchedule(build, current.weldedIndices, { minMm: req.minMm, maxMm: req.maxMm, cuspMm: req.cuspMm, firstMm: req.firstMm });
      reply({ id: req.id, kind: 'schedule', boundaries: Array.from(s.boundaries) });
    } else if (req.kind === 'gcodeLayers') {
      const { text } = await gcodeText(req.buffer);
      const tp = parseGcode(text);
      reply({ id: req.id, kind: 'schedule', boundaries: Array.from(scheduleFromLayerTops(tp.layerZ).boundaries) });
    } else if (req.kind === 'bakeBeads') {
      if (!toolpath) throw new Error('No G-code loaded');
      const t0 = performance.now();
      const tp = toolpath;
      const layers = Math.max(1, tp.layerZ.length);
      const colorOf =
        req.colorMode === 'material' ? null
        : req.colorMode === 'feature' ? (i: number) => req.featureColors[tp.feature[i]] as [number, number, number]
        : req.colorMode === 'tool' ? (i: number) => req.toolColors[tp.tool[i]] as [number, number, number]
        : (i: number) => {
            const k = tp.layer[i] / Math.max(layers - 1, 1);
            return [0, 0.33, 0.67].map((o) => Math.pow(0.5 + 0.5 * Math.cos(6.28318 * (k + o)), 2.2)) as [number, number, number];
          };
      const chunks = bakeBeads(tp, { hiddenMask: req.hiddenMask, colorOf, chunks: req.chunks, layerCount: layers });
      const triangles = chunks.reduce((a, c) => a + c.index.length / 3, 0);
      const transfer: Transferable[] = [];
      for (const c of chunks) transfer.push(c.positions.buffer, c.normals.buffer, c.index.buffer, ...(c.colors ? [c.colors.buffer] : []));
      reply({ id: req.id, kind: 'baked', chunks, triangles, ms: performance.now() - t0 }, transfer);
    } else if (req.kind === 'flatFace') {
      if (!current) throw new Error('No model loaded');
      const f = largestFlatFace(current.weldedPositions, current.weldedIndices);
      reply({ id: req.id, kind: 'flatFace', normal: f ? [f.normal.x, f.normal.y, f.normal.z] : null, areaMm2: f?.areaMm2 ?? 0 });
    } else if (req.kind === 'renormal') {
      if (!current) throw new Error('No model loaded');
      const normals = renormal(current, req.creaseDeg);
      reply({ id: req.id, kind: 'normals', normals }, [normals.buffer]);
    }
  } catch (err) {
    reply({ id: req.id, kind: 'error', message: err instanceof Error ? err.message : String(err) });
  }
};
