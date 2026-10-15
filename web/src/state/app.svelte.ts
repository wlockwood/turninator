// Application state (Svelte 5 runes). The Studio renderer is attached by the
// viewport component; everything else is plain reactive data.

import { CoreClient } from '../workers/protocol';
import type { MeshStats } from '../core/mesh';
import type { Studio } from '../render/studio';
import { DEFAULT_INSPECTION, type Axis, type InspectionSettings } from '../render/inspect';
import { faceDownPose, type EulerDeg } from '../render/orient';
import { AUXILIARY_FEATURES } from '../core/gcode';
import { FEATURE_COLORS, type BeadColorMode } from '../render/beads';
import { INTERIOR_FEATURES } from '../core/beadMesh';
import type { BeadChunkGeometry } from '../render/pathtrace';
import type { BeadBake } from '../export/still';
import { pathTracingSupported } from '../render/pathtraceHost';
import { loadRemembered, makeProject, mergeInto, parseProject, remember } from './project';
import { cameraFromKey, DEFAULT_TRANSITION, keyFromCamera, pathDuration, sampleKey, type CamKey, type CamTransition } from '../render/camPath';
import * as THREE from 'three';
import { defaultSettings, MATERIAL_PRESETS, DEFAULT_LAYERS, STUDIO_PRESETS, type SceneSettings, type StudioName } from './settings';
import { exportTurntable, TURNTABLE_DEFAULTS, type OutputFile, type TurntableOptions } from '../export/turntable';
import { exportStill, STILL_DEFAULTS, type StillOptions } from '../export/still';
import { supportedVideoFormats, type VideoFormat } from '../export/video';
import { exportFourView, FOURVIEW_DEFAULTS, type FourViewOptions } from '../export/fourview';

export interface GcodeInfo {
  beads: number;
  layers: number;
  layerZ: number[];
  features: { name: string; count: number }[];
  filamentMm: number;
  volumeMm3: number;
  slicer: string | null;
  tools: number;
  parseMs: number;
}

export interface ModelInfo {
  kind: 'stl' | 'gcode';
  name: string;
  /** Mesh statistics (STL only). */
  stats: MeshStats | null;
  gcode: GcodeInfo | null;
  sizeMm: [number, number, number];
  format: string;
  warnings: string[];
  timingsMs: Record<string, number>;
}

export interface GcodeView {
  colorMode: BeadColorMode;
  /** Feature names that are not drawn. */
  hidden: string[];
  toolColors: string[];
}

export const GCODE_EXTENSIONS = /\.(gcode|gco|gc|g|bgcode|3mf)$/i;

export interface ExportResult {
  kind: OutputFile['kind'];
  url: string;
  bytes: number;
  name: string;
  label: string;
  mime: string;
}

export type ExportTab = 'turntable' | 'still' | 'fourview';

class AppState {
  settings = $state<SceneSettings>(defaultSettings());
  inspection = $state<InspectionSettings>(structuredClone(DEFAULT_INSPECTION));
  /** Build-frame extents in mm (X/Y centered on 0, Z from 0). */
  buildSize = $state<[number, number, number]>([10, 10, 10]);
  model = $state<ModelInfo | null>(null);
  status = $state('Drop an STL file to begin');
  error = $state<string | null>(null);
  loading = $state(false);
  turntableDeg = $state(0);
  playing = $state(false);
  gif = $state<TurntableOptions>({ ...TURNTABLE_DEFAULTS });
  still = $state<StillOptions>({ ...STILL_DEFAULTS });
  fourview = $state<FourViewOptions>({ ...FOURVIEW_DEFAULTS });
  exportTab = $state<ExportTab>('turntable');
  videoFormats = $state<VideoFormat[]>([]);
  ptSupported = $state(false);
  exporting = $state<{ stage: string; done: number; total: number } | null>(null);
  results = $state<ExportResult[]>([]);
  resultSummary = $state('');
  gcodeView = $state<GcodeView>({ colorMode: 'material', hidden: [...AUXILIARY_FEATURES], toolColors: ['#3d7be0', '#e0463a', '#f2c94c', '#2ecc71', '#9b59b6', '#ecf0f1', '#1d1f24', '#e67e22'] });
  ptPreview = $state(false);
  ptStatus = $state<{ samples: number; state: string; message?: string } | null>(null);
  camKeys = $state<CamKey[]>([]);
  camTransitions = $state<CamTransition[]>([]);
  camFirstHold = $state(0.5);
  pathPreview = $state(false);
  /** Build-up progress 0..1 (1 = complete part). */
  buildProgress = $state(1);
  buildPlaying = $state(false);
  /** Waiting for a face click to lay flat on the given pose. */
  layFlat = $state<'print' | 'display' | null>(null);
  slicing = $state<{ done: number; total: number } | null>(null);
  sliceInfo = $state<string | null>(null);
  studio: Studio | null = null;
  private latestSlice = 0;
  private core = new CoreClient();
  private latestImport = 0;
  private abort: AbortController | null = null;

  attachStudio(studio: Studio) {
    this.studio = studio;
  }

  constructor() {
    const saved = loadRemembered();
    if (saved) this.applyState(saved, false);
  }

  /** Everything a project (or the remembered session) stores. */
  persistentState(): Record<string, unknown> {
    return $state.snapshot({
      settings: this.settings,
      inspection: this.inspection,
      gcodeView: this.gcodeView,
      gif: this.gif,
      still: this.still,
      fourview: this.fourview,
      camKeys: this.camKeys,
      camTransitions: this.camTransitions,
      camFirstHold: this.camFirstHold,
    }) as Record<string, unknown>;
  }

  /** Apply saved state over current values (unknown/mistyped keys are ignored). */
  applyState(s: Record<string, unknown>, includePose: boolean) {
    const pose = this.settings.pose;
    this.settings = mergeInto($state.snapshot(this.settings) as SceneSettings, s.settings);
    if (!includePose) this.settings.pose = pose;
    this.inspection = mergeInto($state.snapshot(this.inspection) as InspectionSettings, s.inspection);
    this.gcodeView = mergeInto($state.snapshot(this.gcodeView) as GcodeView, s.gcodeView);
    this.gif = mergeInto($state.snapshot(this.gif) as TurntableOptions, s.gif);
    this.still = mergeInto($state.snapshot(this.still) as StillOptions, s.still);
    this.fourview = mergeInto($state.snapshot(this.fourview) as FourViewOptions, s.fourview);
    if (Array.isArray(s.camKeys) && Array.isArray(s.camTransitions) && s.camTransitions.length === Math.max(0, s.camKeys.length - 1)) {
      this.camKeys = s.camKeys as CamKey[];
      this.camTransitions = s.camTransitions as CamTransition[];
    }
    if (typeof s.camFirstHold === 'number') this.camFirstHold = s.camFirstHold;
  }

  rememberSession() {
    remember(this.persistentState());
  }

  async saveProject() {
    const s = this.studio;
    const camera = s ? { position: s.camera.position.toArray() as [number, number, number], target: s.controls.target.toArray() as [number, number, number] } : undefined;
    const data = makeProject(this.model?.name ?? null, this.persistentState(), camera);
    const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' });
    const name = `${(this.model?.name ?? 'turninator').replace(/\.[^.]+$/, '')}.turninator.json`;
    const url = URL.createObjectURL(blob);
    const saved = await this.saveResult({ kind: 'png', url, bytes: blob.size, name, label: 'Turninator project', mime: 'application/json' });
    if (!saved) {
      const a = document.createElement('a');
      a.href = url;
      a.download = name;
      a.click();
    }
    setTimeout(() => URL.revokeObjectURL(url), 10_000);
  }

  async loadProject(file: File) {
    try {
      const p = parseProject(await file.text());
      this.applyState(p.state, true);
      const sameModel = !!this.model && p.modelName === this.model.name;
      if (p.camera && sameModel) this.studio?.applyPerspective(p.camera.position, p.camera.target);
      this.status = p.modelName && !sameModel ? `Project applied. It was made for ${p.modelName}; open that file to match.` : `Project ${file.name} applied`;
    } catch (err) {
      this.error = err instanceof Error ? err.message : String(err);
    }
  }

  async openFile(file: File) {
    this.error = null;
    this.loading = true;
    this.status = `Loading ${file.name}…`;
    const buffer = await file.arrayBuffer();
    if (GCODE_EXTENSIONS.test(file.name)) return this.openGcode(file.name, buffer);
    const { id, result } = this.core.request(
      { kind: 'import', name: file.name, buffer, options: { mmPerUnit: this.settings.mmPerUnit, creaseDeg: this.settings.creaseDeg } },
      [buffer],
    );
    this.latestImport = id;
    const res = await result;
    if (id !== this.latestImport) return; // a newer import superseded this one
    this.loading = false;
    if (res.kind === 'error') {
      this.error = res.message;
      this.status = `Could not load ${file.name}`;
      return;
    }
    if (res.kind !== 'imported') return;
    const m = res.model;
    const s = m.stats;
    this.buildProgress = 1;
    this.model = { kind: 'stl', name: m.name, stats: s, gcode: null, sizeMm: [s.max[0] - s.min[0], s.max[1] - s.min[1], s.max[2] - s.min[2]], format: m.format, warnings: m.warnings, timingsMs: m.timingsMs };
    this.status = `${m.name} — ${m.stats.triangles.toLocaleString()} triangles, loaded in ${m.timingsMs.total.toFixed(0)} ms`;
    if (this.studio) {
      this.studio.setModel(m.positions, m.normals, m.weldedPositions);
      this.studio.setPose($state.snapshot(this.settings.pose) as SceneSettings['pose']);
      this.studio.fitHero();
    }
  }

  private async openGcode(name: string, buffer: ArrayBuffer) {
    const { id, result } = this.core.request({ kind: 'gcode', name, buffer }, [buffer]);
    this.latestImport = id;
    const res = await result;
    if (id !== this.latestImport) return;
    this.loading = false;
    if (res.kind === 'error') {
      this.error = res.message;
      this.status = `Could not load ${name}`;
      return;
    }
    if (res.kind !== 'toolpath') return;
    const tp = res.toolpath;
    this.settings.pose.print = [0, 0, 0]; // G-code is already in the printer's frame
    this.buildProgress = 1;
    this.model = {
      kind: 'gcode',
      name,
      stats: null,
      gcode: { beads: tp.count, layers: tp.layerCount, layerZ: tp.layerZ, features: tp.features, filamentMm: tp.stats.filamentMm, volumeMm3: tp.stats.volumeMm3, slicer: tp.stats.slicer, tools: tp.stats.tools, parseMs: tp.ms },
      sizeMm: [tp.max[0] - tp.min[0], tp.max[1] - tp.min[1], tp.max[2] - tp.min[2]],
      format: 'gcode',
      warnings: tp.warnings,
      timingsMs: { total: tp.ms },
    };
    this.status = `${name} — ${tp.count.toLocaleString()} beads, ${tp.layerCount} layers, parsed in ${tp.ms.toFixed(0)} ms`;
    if (this.studio) {
      this.studio.setBeads(tp, tp.layerCount, tp.points);
      this.applyGcodeView();
      this.studio.setPose($state.snapshot(this.settings.pose) as SceneSettings['pose']);
      this.studio.fitHero();
    }
  }

  private bakeCache: { key: string; chunks: BeadChunkGeometry[]; vertexColors: boolean } | null = null;

  /**
   * Bake G-code beads into meshes for the path tracer (cached). Interior
   * features are culled when they cannot be seen: opaque material, no section
   * and no build-up (a growing print exposes its infill).
   */
  async bakeForPathTracing(building: boolean): Promise<BeadBake | undefined> {
    const g = this.model?.gcode;
    if (!g) return undefined;
    const cullInterior = this.settings.material.transmission === 0 && !this.inspection.section.enabled && !building;
    let mask = 0;
    g.features.forEach((f, i) => {
      if (i < 31 && (this.gcodeView.hidden.includes(f.name) || (cullInterior && INTERIOR_FEATURES.has(f.name)))) mask |= 1 << i;
    });
    const colorMode = this.gcodeView.colorMode;
    const lin = (hex: string) => {
      const c = new THREE.Color(hex);
      return [c.r, c.g, c.b];
    };
    const featureColors = g.features.map((f) => lin(FEATURE_COLORS[f.name] ?? '#cccccc'));
    const toolColors = this.gcodeView.toolColors.map(lin);
    const key = JSON.stringify([this.model!.name, g.beads, mask, colorMode, colorMode === 'tool' ? toolColors : 0]);
    if (this.bakeCache?.key !== key) {
      this.status = 'Baking toolpath for path tracing…';
      const res = await this.core.request({ kind: 'bakeBeads', hiddenMask: mask, colorMode, featureColors, toolColors, chunks: 96 }).result;
      if (res.kind === 'error') throw new Error(res.message);
      if (res.kind !== 'baked') throw new Error('Unexpected bake response');
      for (const c of this.bakeCache?.chunks ?? []) c.geometry.dispose();
      const chunks = res.chunks.map((c) => {
        const geo = new THREE.BufferGeometry();
        geo.setIndex(new THREE.BufferAttribute(c.index, 1));
        geo.setAttribute('position', new THREE.BufferAttribute(c.positions, 3));
        geo.setAttribute('normal', new THREE.BufferAttribute(c.normals, 3));
        if (c.colors) geo.setAttribute('color', new THREE.BufferAttribute(c.colors, 3));
        geo.computeBoundingSphere();
        return { geometry: geo, maxLayer: c.maxLayer };
      });
      this.bakeCache = { key, chunks, vertexColors: colorMode !== 'material' };
      this.status = `Baked ${res.triangles.toLocaleString()} triangles in ${(res.ms / 1000).toFixed(1)} s${cullInterior ? ' (hidden interior culled)' : ''}`;
    }
    return { chunks: this.bakeCache!.chunks, vertexColors: this.bakeCache!.vertexColors, layerCount: g.layers, progress: this.buildProgress };
  }

  /** Push G-code colour/visibility settings to the renderer. */
  applyGcodeView() {
    const g = this.model?.gcode;
    if (!this.studio || !g) return;
    let mask = 0;
    g.features.forEach((f, i) => {
      if (i < 31 && this.gcodeView.hidden.includes(f.name)) mask |= 1 << i;
    });
    this.studio.setBeadDisplay({
      colorMode: this.gcodeView.colorMode,
      hiddenMask: mask,
      featureColors: g.features.map((f) => FEATURE_COLORS[f.name] ?? '#cccccc'),
      toolColors: [...this.gcodeView.toolColors],
    });
  }

  addCamKey() {
    const s = this.studio;
    if (!s) return;
    const p = s.camera.position, t = s.controls.target;
    this.camKeys.push(keyFromCamera([p.x, p.y, p.z], [t.x, t.y, t.z]));
    if (this.camKeys.length > 1) this.camTransitions.push({ ...DEFAULT_TRANSITION });
  }

  removeCamKey(i: number) {
    this.camKeys.splice(i, 1);
    // Drop the transition leading into this key (or out of the first key).
    if (this.camTransitions.length) this.camTransitions.splice(Math.max(0, i - 1), 1);
  }

  gotoCamKey(i: number) {
    const k = this.camKeys[i];
    if (!k || !this.studio) return;
    const c = cameraFromKey($state.snapshot(k) as CamKey);
    this.studio.applyPerspective(c.position, c.target);
  }

  camPathSeconds(): number {
    return this.camKeys.length > 1 ? pathDuration(this.camKeys, this.camTransitions, this.camFirstHold) : 0;
  }

  /** Play the camera path in the viewport (aspect differs from square exports). */
  previewCamPath() {
    if (this.camKeys.length < 2 || !this.studio || this.pathPreview) return;
    const keys = $state.snapshot(this.camKeys) as CamKey[];
    const trs = $state.snapshot(this.camTransitions) as CamTransition[];
    const total = pathDuration(keys, trs, this.camFirstHold);
    const t0 = performance.now();
    this.pathPreview = true;
    const tick = () => {
      if (!this.pathPreview || !this.studio) return;
      const t = (performance.now() - t0) / 1000;
      const c = cameraFromKey(sampleKey(keys, trs, Math.min(t, total), this.camFirstHold));
      this.studio.applyPerspective(c.position, c.target);
      if (t < total) requestAnimationFrame(tick);
      else this.pathPreview = false;
    };
    requestAnimationFrame(tick);
  }

  toggleBuildPlay() {
    if (!this.buildPlaying && this.buildProgress >= 0.999) this.buildProgress = 0;
    this.buildPlaying = !this.buildPlaying;
  }

  /** Which layer the current build progress reaches (G-code), 1-based. */
  buildLayerLabel(): string {
    const m = this.model;
    if (!m) return '';
    if (m.gcode) {
      const z = this.buildProgress * m.sizeMm[2];
      const idx = m.gcode.layerZ.findIndex((lz) => lz >= z - 1e-6);
      return `layer ${idx < 0 ? m.gcode.layers : idx + 1} / ${m.gcode.layers}`;
    }
    return `${(this.buildProgress * m.sizeMm[2]).toFixed(1)} / ${m.sizeMm[2].toFixed(1)} mm`;
  }

  /** Rebuild the rendered geometry for the current layer mode (debounced by caller). */
  /** Variable layer boundaries in use (null = uniform). */
  layerSchedule = $state<{ boundaries: number[]; info: string } | null>(null);
  /** Layer boundaries loaded from a G-code file for the 'gcode' profile. */
  gcodeLayers = $state<{ name: string; boundaries: number[] } | null>(null);
  private latestSchedule = 0;

  async loadLayerHeights(file: File) {
    const buffer = await file.arrayBuffer();
    const res = await this.core.request({ kind: 'gcodeLayers', buffer }, [buffer]).result;
    if (res.kind === 'error') {
      this.error = `Could not read layer heights: ${res.message}`;
      return;
    }
    if (res.kind !== 'schedule') return;
    this.gcodeLayers = { name: file.name, boundaries: res.boundaries };
    this.settings.layers.profile = 'gcode';
  }

  /** Compute the active layer schedule (variable profiles) and push it to the shader. */
  private async updateSchedule(): Promise<number[] | null> {
    const studio = this.studio;
    const l = this.settings.layers;
    const id = ++this.latestSchedule;
    let b: number[] | null = null;
    if (studio && this.model?.kind === 'stl' && l.enabled) {
      if (l.profile === 'adaptive') {
        const res = await this.core.request({ kind: 'adaptive', printMatrix: studio.printMatrix, minMm: l.adaptiveMinMm, maxMm: l.adaptiveMaxMm, cuspMm: l.adaptiveCuspMm, firstMm: l.firstMm }).result;
        if (res.kind === 'schedule') b = res.boundaries;
        else if (res.kind === 'error') this.error = res.message;
      } else if (l.profile === 'gcode' && this.gcodeLayers) {
        b = this.gcodeLayers.boundaries;
      }
    }
    if (id !== this.latestSchedule) return null;
    studio?.setLayerSchedule(b);
    if (b) {
      let lo = Infinity, hi = 0;
      for (let i = 1; i < b.length; i++) { lo = Math.min(lo, b[i] - b[i - 1]); hi = Math.max(hi, b[i] - b[i - 1]); }
      const top = b[b.length - 1];
      const mismatch = this.model && Math.abs(top - this.model.sizeMm[2]) > 0.5 ? ` · G-code top ${top.toFixed(1)} mm vs model ${this.model.sizeMm[2].toFixed(1)} mm` : '';
      this.layerSchedule = { boundaries: b, info: `${b.length - 1} layers, ${lo.toFixed(2)}–${hi.toFixed(2)} mm${mismatch}` };
    } else {
      this.layerSchedule = null;
    }
    return b;
  }

  /**
   * Mode actually used: Auto picks Sliced when layers span at least 6 px in the
   * turntable export (steps read as geometry), else Shaded (cheaper, moiré-safe).
   */
  effectiveLayerMode(): 'shaded' | 'sliced' {
    const m = this.settings.layers.mode;
    if (m !== 'auto') return m;
    const px = this.exportPxPerLayer();
    return px !== null && px >= 6 ? 'sliced' : 'shaded';
  }

  async updateGeometry() {
    const studio = this.studio;
    if (!studio || !this.model || this.model.kind !== 'stl') return;
    const l = this.settings.layers;
    const boundaries = await this.updateSchedule();
    if (!l.enabled || this.effectiveLayerMode() !== 'sliced') {
      this.latestSlice++;
      this.slicing = null;
      this.sliceInfo = null;
      studio.restoreOriginalGeometry();
      return;
    }
    const { id, result } = this.core.request(
      { kind: 'slice', printMatrix: studio.printMatrix, heightMm: l.heightMm, firstMm: l.firstMm, creaseDeg: this.settings.creaseDeg, maxTriangles: 12_000_000, boundaries: boundaries ?? undefined },
      [],
      (done, total) => {
        if (id === this.latestSlice) this.slicing = { done, total };
      },
    );
    this.latestSlice = id;
    this.slicing = { done: 0, total: 1 };
    const res = await result;
    if (id !== this.latestSlice) return;
    this.slicing = null;
    if (res.kind === 'error') {
      this.sliceInfo = `Slicing failed: ${res.message}`;
      studio.restoreOriginalGeometry();
      return;
    }
    if (res.kind !== 'sliced') return;
    studio.setRenderGeometry(res.positions, res.normals);
    const st = res.stats;
    this.sliceInfo = `${st.layers} layers · ${(st.triangles / 1000).toFixed(0)}k triangles · ${(st.ms / 1000).toFixed(1)} s${st.warnings.length ? ` · ${st.warnings.join('; ')}` : ''}`;
  }

  async setCrease(deg: number) {
    this.settings.creaseDeg = deg;
    if (!this.model) return;
    const { result } = this.core.request({ kind: 'renormal', creaseDeg: deg });
    const res = await result;
    if (res.kind === 'normals') this.studio?.setNormals(res.normals);
  }

  applyMaterialPreset(key: string) {
    const p = MATERIAL_PRESETS[key];
    if (!p) return;
    this.settings.material = { ...p.material };
    const keep = this.settings.layers;
    this.settings.layers = { ...DEFAULT_LAYERS, enabled: keep.enabled, mode: keep.mode, topLines: keep.topLines, lineWidthMm: keep.lineWidthMm, ...p.layers };
  }

  applyStudio(name: StudioName) {
    this.settings.studio = { ...STUDIO_PRESETS[name] };
  }

  /** Pixels per layer at the export framing (center of the model). */
  exportPxPerLayer(): number | null {
    if (!this.studio || !this.model) return null;
    const { radius, height } = this.studio.envelope;
    const extent = Math.max(radius * 2, height) * 1.25; // ~10% margins and perspective slack
    let h = this.settings.layers.heightMm;
    const b = this.layerSchedule?.boundaries;
    if (b) for (let i = 1; i < b.length; i++) h = Math.min(h, b[i] - b[i - 1]);
    return (h / extent) * this.gif.size;
  }

  async detectVideo() {
    this.videoFormats = await supportedVideoFormats(1280, 1280);
    this.ptSupported = await pathTracingSupported();
  }

  private baseName() {
    return (this.model?.name ?? 'model').replace(/\.[^.]+$/, '');
  }

  private setResults(files: OutputFile[], suffix: string, summary: string) {
    for (const r of this.results) URL.revokeObjectURL(r.url);
    const base = this.baseName();
    this.results = files.map((f) => ({
      kind: f.kind,
      url: URL.createObjectURL(f.blob),
      bytes: f.blob.size,
      name: `${base}-${suffix}.${f.ext}`,
      label: f.label,
      mime: f.blob.type,
    }));
    this.resultSummary = summary;
  }

  private async runExport(fn: (signal: AbortSignal) => Promise<void>) {
    if (!this.studio || !this.model || this.exporting) return;
    this.abort = new AbortController();
    this.error = null;
    this.playing = false;
    this.exporting = { stage: 'render', done: 0, total: 1 };
    // Keep the screen awake: a sleeping machine stalls GPU work.
    type WakeLock = { release(): Promise<void> };
    const wl = (navigator as Navigator & { wakeLock?: { request(type: 'screen'): Promise<WakeLock> } }).wakeLock;
    const lock = await wl?.request('screen').catch(() => null);
    try {
      await fn(this.abort.signal);
    } catch (err) {
      if ((err as DOMException).name === 'AbortError') this.status = 'Export cancelled';
      else this.error = err instanceof Error ? err.message : String(err);
    } finally {
      this.exporting = null;
      this.abort = null;
      void lock?.release().catch(() => {});
    }
  }

  /** Save a result with the native save dialog where supported (Chromium). */
  async saveResult(r: ExportResult) {
    const picker = (window as unknown as { showSaveFilePicker?: (o: unknown) => Promise<FileSystemFileHandle> }).showSaveFilePicker;
    if (!picker) return false;
    const ext = r.name.split('.').pop() ?? 'bin';
    try {
      const handle = await picker({ suggestedName: r.name, types: [{ description: r.label, accept: { [r.mime || 'application/octet-stream']: [`.${ext}`] } }] });
      const blob = await (await fetch(r.url)).blob();
      const w = await handle.createWritable();
      await w.write(blob);
      await w.close();
      this.status = `Saved ${handle.name}`;
    } catch (err) {
      if ((err as DOMException).name !== 'AbortError') this.error = err instanceof Error ? err.message : String(err);
    }
    return true;
  }

  exportTurntable() {
    return this.runExport(async (signal) => {
      const opts = $state.snapshot(this.gif) as TurntableOptions;
      // Side-by-side uses the user's section if enabled, else a centred front-to-back cut.
      opts.section = this.inspection.section.enabled
        ? ($state.snapshot(this.inspection.section) as InspectionSettings['section'])
        : { ...DEFAULT_INSPECTION.section, enabled: true, axis: 'y', offsetMm: 0 };
      opts.path = { keys: $state.snapshot(this.camKeys) as CamKey[], transitions: $state.snapshot(this.camTransitions) as CamTransition[], firstHold: this.camFirstHold };
      const beads = opts.engine === 'pathtraced' ? await this.bakeForPathTracing(opts.motion.includes('build')) : undefined;
      const res = await exportTurntable(this.studio!, opts, (stage, done, total) => { this.exporting = { stage, done, total }; }, signal, beads);
      const enc = res.files.map((f) => `${f.label.split(' ')[0]} ${(f.encodeMs / 1000).toFixed(1)} s`).join(', ');
      this.setResults(res.files, 'turntable', `${res.frames} frames at ${opts.size}², rendered in ${(res.renderMs / 1000).toFixed(1)} s; encoded ${enc}`);
      this.status = `Turntable ready (${res.files.map((f) => `${(f.blob.size / 1e6).toFixed(1)} MB ${f.ext.toUpperCase()}`).join(', ')})`;
    });
  }

  exportStill() {
    return this.runExport(async (signal) => {
      const opts = $state.snapshot(this.still) as StillOptions;
      await new Promise((r) => setTimeout(r, 30)); // let the progress UI paint
      const beads = opts.engine === 'pathtraced' ? await this.bakeForPathTracing(this.buildProgress < 1) : undefined;
      const res = await exportStill(this.studio!, opts, signal, (done, total) => { this.exporting = { stage: 'render', done, total }; }, beads);
      const how = opts.engine === 'pathtraced' ? `path traced, ${opts.samples} samples` : `${res.supersample}× supersampled`;
      this.setResults([res.file], `still-${opts.width}x${opts.height}`, `${opts.width}×${opts.height}, ${how}, rendered in ${(res.renderMs / 1000).toFixed(1)} s`);
      this.status = `Still ready (${(res.file.blob.size / 1e6).toFixed(1)} MB PNG)`;
    });
  }

  exportFourView() {
    return this.runExport(async () => {
      const opts = $state.snapshot(this.fourview) as FourViewOptions;
      const current = {
        material: $state.snapshot(this.settings.material) as SceneSettings['material'],
        layers: $state.snapshot(this.settings.layers) as SceneSettings['layers'],
        name: this.model!.name,
      };
      const res = await exportFourView(this.studio!, opts, current, (done, total) => { this.exporting = { stage: 'render', done, total }; });
      this.setResults([{ kind: 'png', blob: res.blob, ext: 'png', label: 'PNG', encodeMs: 0 }], `four-view-${opts.size}`, `${opts.size}² sheet, shared scale ${(1 / res.mmPerPx).toFixed(1)} px/mm, ${(res.ms / 1000).toFixed(1)} s`);
      this.status = `Four-view sheet ready (${(res.blob.size / 1e6).toFixed(1)} MB PNG)`;
    });
  }

  /** Aspect of what the active export tab renders (null: four-view has its own cameras). */
  exportAspect(): number | null {
    if (this.exportTab === 'turntable') return 1;
    if (this.exportTab === 'still') return this.still.width / this.still.height;
    return null;
  }

  /** Output size label for the viewport frame. */
  exportSizeLabel(): string {
    if (this.exportTab === 'turntable') return `${this.gif.size} × ${this.gif.size}${this.gif.layout === 'section-side' ? ' (each half)' : ''}`;
    if (this.exportTab === 'still') return `${this.still.width} × ${this.still.height}`;
    return '';
  }

  /** Fit the whole rotation into the turntable frame at the chosen camera height. */
  frameTurntable() {
    this.studio?.fitHero(0, this.gif.elevationDeg, 0.1);
  }

  /** Rotate the chosen pose so a geometry-space face normal points down. */
  applyLayFlat(localNormal: THREE.Vector3, target: 'print' | 'display') {
    const pose = this.settings.pose;
    if (target === 'print') {
      pose.print = faceDownPose([...pose.print] as EulerDeg, localNormal);
    } else {
      const R = new THREE.Matrix4().makeRotationFromEuler(new THREE.Euler(...(pose.print.map((d) => (d * Math.PI) / 180) as EulerDeg), 'XYZ'));
      pose.display = faceDownPose([...pose.display] as EulerDeg, localNormal.clone().applyMatrix4(R));
    }
    this.layFlat = null;
  }

  async autoLayFlat() {
    if (!this.model) return;
    this.status = 'Finding the largest flat face…';
    const res = await this.core.request({ kind: 'flatFace' }).result;
    if (res.kind === 'flatFace' && res.normal) {
      this.applyLayFlat(new THREE.Vector3(...res.normal), 'print');
      this.status = `Laid flat on a ${res.areaMm2.toFixed(0)} mm² face`;
    } else {
      this.status = 'No flat face found that can rest on the bed';
    }
  }

  /** Valid section offset range (mm) along an axis of the build frame. */
  sectionRange(axis: Axis): [number, number] {
    const [sx, sy, sz] = this.buildSize;
    if (axis === 'x') return [-sx / 2, sx / 2];
    if (axis === 'y') return [-sy / 2, sy / 2];
    return [0, sz];
  }

  setSectionAxis(axis: Axis) {
    const [lo, hi] = this.sectionRange(axis);
    this.inspection.section.axis = axis;
    this.inspection.section.offsetMm = (lo + hi) / 2;
  }

  cancelExport() {
    this.abort?.abort();
  }
}

export const app = new AppState();
