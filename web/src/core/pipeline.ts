// Pure import pipeline shared by the worker and tests: STL bytes -> render
// buffers + stats. Kept free of DOM/worker APIs.

import { parseStl, unitHint } from './stl';
import { analyzeTopology, computeStats, cornerNormals, expand, weldExact, type MeshStats } from './mesh';

export interface ImportOptions {
  mmPerUnit: number;
  creaseDeg: number;
}

export interface ImportedModel {
  name: string;
  /** Non-indexed build-frame positions (mm), 9 floats per triangle. */
  positions: Float32Array;
  normals: Float32Array;
  /** Welded mesh retained for slicing and re-normalling. */
  weldedPositions: Float32Array;
  weldedIndices: Uint32Array;
  shellOf: Int32Array;
  stats: MeshStats;
  format: 'binary' | 'ascii';
  warnings: string[];
  timingsMs: Record<string, number>;
}

export function importStl(name: string, buffer: ArrayBuffer, opts: ImportOptions): ImportedModel {
  const t0 = performance.now();
  const parsed = parseStl(buffer, opts.mmPerUnit);
  const t1 = performance.now();
  const mesh = weldExact(parsed.positions);
  const t2 = performance.now();
  const topo = analyzeTopology(mesh);
  const stats = computeStats(mesh, topo);
  const t3 = performance.now();
  const normals = cornerNormals(mesh, topo, opts.creaseDeg);
  const positions = expand(mesh);
  const t4 = performance.now();

  const warnings = [...parsed.warnings];
  const hint = unitHint(parsed.size);
  if (hint && opts.mmPerUnit === 1) warnings.push(hint);
  if (!stats.watertight) {
    warnings.push(`Mesh is not watertight (${stats.openEdges} open, ${stats.nonManifoldEdges} non-manifold edges); volume and section caps are unreliable`);
  }
  return {
    name,
    positions,
    normals,
    weldedPositions: mesh.positions,
    weldedIndices: mesh.indices,
    shellOf: topo.shellOf,
    stats,
    format: parsed.format,
    warnings,
    timingsMs: { parse: t1 - t0, weld: t2 - t1, topology: t3 - t2, normals: t4 - t3, total: t4 - t0 },
  };
}

export function renormal(model: ImportedModel, creaseDeg: number): Float32Array {
  const mesh = { positions: model.weldedPositions, indices: model.weldedIndices };
  const topo = { shellOf: model.shellOf, shells: model.stats.shells, openEdges: model.stats.openEdges, nonManifoldEdges: model.stats.nonManifoldEdges };
  return cornerNormals(mesh, topo, creaseDeg);
}

export function transferList(m: ImportedModel): Transferable[] {
  return [m.positions.buffer, m.normals.buffer, m.weldedPositions.buffer, m.weldedIndices.buffer, m.shellOf.buffer];
}
