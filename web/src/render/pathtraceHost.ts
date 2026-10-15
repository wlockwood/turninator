// Shared path tracer handle. The tracer itself lives in a worker
// (workers/pathtrace.worker.ts); this returns its UI-thread proxy.

import { PathTracerProxy } from './pathtraceProxy';

export type Engine = 'studio' | 'pathtraced';

let instance: Promise<PathTracerProxy> | null = null;

export function getPathTracer(): Promise<PathTracerProxy> {
  instance ??= PathTracerProxy.create().catch((err) => {
    instance = null;
    throw err;
  });
  return instance;
}

export async function pathTracingSupported(): Promise<boolean> {
  try {
    const gpu = (navigator as Navigator & { gpu?: { requestAdapter(): Promise<unknown> } }).gpu;
    return !!gpu && !!(await gpu.requestAdapter());
  } catch {
    return false;
  }
}
