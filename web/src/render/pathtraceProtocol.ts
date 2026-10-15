// Messages between the UI thread and the path tracer worker.

import type { MaterialSettings } from '../state/settings';

export interface GeomData {
  attrs: Record<string, { array: Float32Array; itemSize: number }>;
  index?: Uint32Array | Uint16Array;
}

export interface CamDesc {
  fov: number;
  aspect: number;
  near: number;
  far: number;
  /** left, right, top, bottom for orthographic cameras. */
  ortho?: [number, number, number, number];
  matrixWorld: number[];
}

export interface SceneDesc {
  modelKey?: string;
  chunks?: { key: string; maxLayer: number }[];
  modelMatrix: number[];
  backdropKey: string;
  backdropMatrix: number[];
  backdropVisible: boolean;
  background: string | null;
  material: MaterialSettings;
  envIntensity: number;
  vertexColors?: boolean;
  buildLayer?: number | null;
}

export type PtRequest =
  | { kind: 'init'; weightsUrl: string }
  | { kind: 'scene'; id: number; scene: SceneDesc; camera: CamDesc; geoms: Record<string, GeomData> }
  | { kind: 'modelMatrix'; matrix: number[] }
  | { kind: 'buildLayer'; layer: number | null }
  | { kind: 'camera'; camera: CamDesc }
  | { kind: 'denoise'; id: number; enabled: boolean }
  | { kind: 'render'; id: number; width: number; height: number; samples: number }
  | { kind: 'abort'; id: number }
  | { kind: 'preview'; on: boolean; width: number; height: number; maxSamples: number }
  | { kind: 'previewSize'; width: number; height: number }
  | { kind: 'previewPause'; paused: boolean };

export type PtResponse =
  | { kind: 'ready' }
  | { kind: 'done'; id: number }
  | { kind: 'progress'; id: number; avg: number }
  | { kind: 'rendered'; id: number; pixels: Uint8ClampedArray; ms: number }
  | { kind: 'frame'; bitmap: ImageBitmap; samples: number }
  | { kind: 'error'; id?: number; message: string };
