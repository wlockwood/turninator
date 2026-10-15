// Serializable scene settings (the web analogue of SceneSpec).

export interface MaterialSettings {
  preset: string;
  color: string; // sRGB hex
  roughness: number;
  metalness: number;
  clearcoat: number;
  clearcoatRoughness: number;
  sheen: number;
  transmission: number;
  thickness: number; // mm
  attenuationDistance: number; // mm
  ior: number;
}

export type LayerMode = 'shaded' | 'sliced' | 'auto';
export type LayerProfile = 'uniform' | 'adaptive' | 'gcode';

export interface LayerSettings {
  enabled: boolean;
  /** Shaded: normal perturbation on the original mesh. Sliced: stepped geometry. */
  mode: LayerMode;
  /** How layer heights vary over Z. */
  profile: LayerProfile;
  adaptiveMinMm: number;
  adaptiveMaxMm: number;
  /** Stair-step cusp tolerance for adaptive layers (mm). */
  adaptiveCuspMm: number;
  /** Extrusion line width for top-surface infill ridges (mm). */
  lineWidthMm: number;
  topLines: boolean;
  heightMm: number;
  firstMm: number;
  /** Bead bulge depth in mm (visual strength). */
  depthMm: number;
  /** Profile exponent: lower = rounder beads with sharper seams. */
  roundness: number;
}

export type StudioName = 'light' | 'dark';

export interface StudioSettings {
  name: StudioName;
  background: string;
  envIntensity: number;
  keyIntensity: number;
  shadows: boolean;
  /** Screen-space ambient occlusion (Studio engine). */
  ao: boolean;
}

export interface PoseSettings {
  /** Print pose: rotation (deg, XYZ) applied before layers; defines the build frame. */
  print: [number, number, number];
  /** Display pose: presentation-only rotation (deg, XYZ) after layers. */
  display: [number, number, number];
}

export interface SceneSettings {
  material: MaterialSettings;
  layers: LayerSettings;
  studio: StudioSettings;
  pose: PoseSettings;
  creaseDeg: number;
  mmPerUnit: number;
}

const base: Omit<MaterialSettings, 'preset' | 'color'> = {
  roughness: 0.55, metalness: 0, clearcoat: 0, clearcoatRoughness: 0.1, sheen: 0,
  transmission: 0, thickness: 4, attenuationDistance: 30, ior: 1.5,
};

export const MATERIAL_PRESETS: Record<string, { label: string; material: MaterialSettings; layers?: Partial<LayerSettings> }> = {
  'matte-pla': { label: 'Matte PLA', material: { ...base, preset: 'matte-pla', color: '#3d7be0', roughness: 0.62, sheen: 0.25 } },
  'silk-pla': { label: 'Silk PLA', material: { ...base, preset: 'silk-pla', color: '#d4a64a', roughness: 0.28, metalness: 0.55, clearcoat: 0.3 } },
  'glossy-petg': { label: 'Glossy PETG', material: { ...base, preset: 'glossy-petg', color: '#d8433b', roughness: 0.22, clearcoat: 0.7, clearcoatRoughness: 0.08 } },
  'translucent-petg': { label: 'Translucent PETG', material: { ...base, preset: 'translucent-petg', color: '#3fae8a', roughness: 0.2, transmission: 0.75, thickness: 3, attenuationDistance: 12 } },
  'grey-resin': { label: 'Grey resin', material: { ...base, preset: 'grey-resin', color: '#8d9198', roughness: 0.38, clearcoat: 0.2 }, layers: { heightMm: 0.05, firstMm: 0.05, depthMm: 0.004 } },
  'clear-resin': { label: 'Clear resin', material: { ...base, preset: 'clear-resin', color: '#f2c46b', roughness: 0.05, transmission: 1, thickness: 6, attenuationDistance: 25 }, layers: { heightMm: 0.05, firstMm: 0.05, depthMm: 0.003 } },
  clay: { label: 'Clay', material: { ...base, preset: 'clay', color: '#b9b2a6', roughness: 0.92 }, layers: { enabled: false } },
  'anodized-metal': { label: 'Anodized metal', material: { ...base, preset: 'anodized-metal', color: '#6f8fb8', roughness: 0.32, metalness: 1 }, layers: { enabled: false } },
};

export const STUDIO_PRESETS: Record<StudioName, StudioSettings> = {
  light: { name: 'light', background: '#e9e7e3', envIntensity: 0.55, keyIntensity: 2.4, shadows: true, ao: true },
  dark: { name: 'dark', background: '#1d1f24', envIntensity: 0.25, keyIntensity: 3.0, shadows: true, ao: true },
};

export const DEFAULT_LAYERS: LayerSettings = { enabled: true, mode: 'shaded', profile: 'uniform', adaptiveMinMm: 0.08, adaptiveMaxMm: 0.3, adaptiveCuspMm: 0.03, lineWidthMm: 0.42, topLines: true, heightMm: 0.2, firstMm: 0.2, depthMm: 0.03, roundness: 0.5 };

export function defaultSettings(): SceneSettings {
  return {
    material: { ...MATERIAL_PRESETS['matte-pla'].material },
    layers: { ...DEFAULT_LAYERS },
    studio: { ...STUDIO_PRESETS.light },
    pose: { print: [0, 0, 0], display: [0, 0, 0] },
    creaseDeg: 30,
    mmPerUnit: 1,
  };
}
