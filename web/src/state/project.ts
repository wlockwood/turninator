// Project files and remembered settings. A project stores how a model is
// presented (not the model itself): scene settings, inspection, G-code view,
// export options, camera keyframes and the current camera.

export const PROJECT_VERSION = 1;
const STORAGE_KEY = 'turninator.settings.v1';

export interface ProjectData {
  version: number;
  app: 'turninator';
  modelName: string | null;
  savedAt: string;
  state: Record<string, unknown>;
  camera?: { position: [number, number, number]; target: [number, number, number] };
}

export function makeProject(modelName: string | null, state: Record<string, unknown>, camera?: ProjectData['camera']): ProjectData {
  return { version: PROJECT_VERSION, app: 'turninator', modelName, savedAt: new Date().toISOString(), state, camera };
}

export function parseProject(text: string): ProjectData {
  const data = JSON.parse(text) as ProjectData;
  if (data?.app !== 'turninator' || typeof data.state !== 'object') throw new Error('Not a Turninator project file');
  if (data.version > PROJECT_VERSION) throw new Error(`Project version ${data.version} is newer than this app supports`);
  return data;
}

/**
 * Deep-merge saved values onto current defaults: only keys that exist in the
 * target with the same primitive/array/object kind are applied, so older or
 * newer files cannot inject unexpected shapes.
 */
export function mergeInto<T>(target: T, saved: unknown): T {
  if (saved === null || typeof saved !== 'object' || Array.isArray(saved) || target === null || typeof target !== 'object') return target;
  const out = (Array.isArray(target) ? [...target] : { ...target }) as Record<string, unknown>;
  for (const [k, v] of Object.entries(saved as Record<string, unknown>)) {
    if (!(k in out)) continue;
    const cur = out[k];
    if (Array.isArray(cur)) {
      if (Array.isArray(v) && (cur.length === 0 || v.every((x) => typeof x === typeof cur[0]))) out[k] = v;
    } else if (cur !== null && typeof cur === 'object') {
      out[k] = mergeInto(cur, v);
    } else if (typeof v === typeof cur) {
      out[k] = v;
    }
  }
  return out as T;
}

export function loadRemembered(): Record<string, unknown> | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? (JSON.parse(raw) as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

export function remember(state: Record<string, unknown>) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    // storage unavailable (private mode, quota); settings simply aren't remembered
  }
}
