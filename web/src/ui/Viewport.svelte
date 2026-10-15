<script lang="ts">
  import { onMount } from 'svelte';
  import { Studio, type ViewName } from '../render/studio';
  import { app } from '../state/app.svelte';
  import type { LayerSettings, MaterialSettings, PoseSettings, StudioSettings } from '../state/settings';
  import type { InspectionSettings } from '../render/inspect';
  import { PathTracePreview } from '../render/ptPreview';

  let stage: HTMLDivElement;
  // Export frame overlay (CSS px within the canvas), kept in sync with resizes.
  let frame = $state<{ x: number; y: number; w: number; h: number } | null>(null);
  function updateFrame() {
    frame = studio ? studio.frameRect() : null;
  }
  $effect(() => {
    if (!studio) return;
    studio.exportAspect = app.exportAspect();
    updateFrame();
    studio.invalidate('camera');
  });
  $effect(() => {
    if (!canvas) return;
    const ro = new ResizeObserver(() => updateFrame());
    ro.observe(canvas);
    return () => ro.disconnect();
  });
  let preview: PathTracePreview | null = null;

  let canvas: HTMLCanvasElement;
  let studio = $state<Studio | null>(null);

  onMount(() => {
    const s = new Studio(canvas);
    app.attachStudio(s);
    studio = s;
    return () => s.dispose();
  });

  $effect(() => {
    const m = $state.snapshot(app.settings.material) as MaterialSettings;
    studio?.setMaterial(m);
  });
  $effect(() => {
    const l = $state.snapshot(app.settings.layers) as LayerSettings;
    studio?.setLayers(l);
  });
  $effect(() => {
    const s = $state.snapshot(app.settings.studio) as StudioSettings;
    studio?.setStudio(s);
  });
  $effect(() => {
    const p = $state.snapshot(app.settings.pose) as PoseSettings;
    if (studio && app.model) {
      studio.setPose(p);
      app.buildSize = [...studio.buildSize];
    }
  });
  $effect(() => {
    const i = $state.snapshot(app.inspection) as InspectionSettings;
    void app.settings.material.color;
    void app.model;
    studio?.setInspection(i);
  });
  $effect(() => {
    if (!app.exporting) studio?.setTurntableAngle(app.turntableDeg);
  });

  // Re-slice (debounced) when anything affecting the printed geometry changes.
  $effect(() => {
    void app.settings.layers.enabled;
    void app.settings.layers.mode;
    void app.settings.layers.heightMm;
    void app.settings.layers.firstMm;
    void app.settings.layers.profile;
    void app.gif.size;
    void app.settings.layers.adaptiveMinMm;
    void app.settings.layers.adaptiveMaxMm;
    void app.settings.layers.adaptiveCuspMm;
    void app.gcodeLayers;
    void app.settings.creaseDeg;
    void $state.snapshot(app.settings.pose.print);
    void app.model;
    if (!studio) return;
    const t = setTimeout(() => void app.updateGeometry(), 250);
    return () => clearTimeout(t);
  });

  // Path-traced preview overlay.
  $effect(() => {
    if (!app.ptPreview || !studio || !app.model) return;
    const p = new PathTracePreview(studio, stage, () => app.bakeForPathTracing(app.buildProgress < 1), (s) => { app.ptStatus = s; }, 1, 2048);
    preview = p;
    void p.start();
    return () => {
      p.stop();
      preview = null;
      app.ptStatus = null;
    };
  });
  $effect(() => {
    const busy = !!app.exporting;
    if (!preview) return;
    preview.paused = busy;
    if (!busy) preview.invalidateScene();
  });

  // Build-up progress and G-code display settings.
  $effect(() => {
    const p = app.buildProgress;
    void app.model;
    if (!app.exporting) studio?.setBuildProgress(p >= 0.9999 ? null : p);
  });
  $effect(() => {
    void $state.snapshot(app.gcodeView);
    app.applyGcodeView();
  });
  // Build-up playback: full build in ~8 s.
  $effect(() => {
    if (!app.buildPlaying) return;
    let raf = 0;
    let last = performance.now();
    const tick = (now: number) => {
      const dt = (now - last) / 1000;
      last = now;
      const next = app.buildProgress + dt / 8;
      if (next >= 1) {
        app.buildProgress = 1;
        app.buildPlaying = false;
        return;
      }
      app.buildProgress = next;
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  });

  // Turntable preview playback at the export speed.
  $effect(() => {
    if (!app.playing) return;
    let raf = 0;
    let last = performance.now();
    const tick = (now: number) => {
      const dt = (now - last) / 1000;
      last = now;
      app.turntableDeg = (app.turntableDeg + (360 / app.gif.seconds) * dt) % 360;
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  });

  // Lay-flat picking: a click (not a drag) on the model picks the face.
  let down: { x: number; y: number } | null = null;
  function onPointerDown(e: PointerEvent) {
    down = { x: e.clientX, y: e.clientY };
  }
  function onPointerUp(e: PointerEvent) {
    if (!app.layFlat || !down || !studio) return;
    if (Math.hypot(e.clientX - down.x, e.clientY - down.y) > 4) return;
    const n = studio.pickNormal(e.clientX, e.clientY);
    if (n) app.applyLayFlat(n, app.layFlat);
  }

  function view(v: ViewName) {
    studio?.setView(v);
  }

  const views: [ViewName, string][] = [['hero', 'Fit'], ['front', 'Front'], ['right', 'Right'], ['top', 'Top'], ['iso', 'Iso']];
</script>

<svelte:window onkeydown={(e) => { if (e.key === 'Escape') app.layFlat = null; }} />

<div class="viewport">
  <div class="stage" bind:this={stage}>
    <canvas bind:this={canvas} data-testid="viewport" class:picking={!!app.layFlat} onpointerdown={onPointerDown} onpointerup={onPointerUp}></canvas>
    <!-- Inside the clipped stage so the dimming shadow never leaves the viewport. -->
    {#if frame && app.model}
      <div class="frame" style="left:{frame.x}px; top:{frame.y}px; width:{frame.w}px; height:{frame.h}px">
        <span>{app.exportSizeLabel()}</span>
      </div>
    {/if}
  </div>
  {#if app.ptStatus}
    <div class="ptbadge" class:err={app.ptStatus.state === 'error'}>
      {app.ptStatus.state === 'error' ? `Raytrace error: ${app.ptStatus.message}` : app.ptStatus.state === 'tracing' ? `Raytracing · ${app.ptStatus.samples} spp` : app.ptStatus.state === 'building' ? 'Raytrace: building scene…' : 'Raytrace: starting…'}
    </div>
  {/if}
  {#if app.layFlat}
    <div class="hint">Click the face that should point down ({app.layFlat} pose) · <button onclick={() => (app.layFlat = null)}>Cancel</button></div>
  {/if}
  {#if !app.model}
    <div class="empty">
      <p class="big">Drop an STL or G-code file here</p>
      <p>or use <strong>Open</strong>. Files never leave your computer.</p>
    </div>
  {/if}
  {#if app.loading}
    <div class="loading">Loading…</div>
  {/if}
  <div class="toolbar">
    <div class="group">
      {#each views as [v, label]}
        <button onclick={() => view(v)} disabled={!app.model}>{label}</button>
      {/each}
    </div>
 <div class="group">
      <button class:active={app.ptPreview} onclick={() => (app.ptPreview = !app.ptPreview)} disabled={!app.model || !app.ptSupported} title="Progressive path-traced preview (WebGPU)">Raytrace</button>
    </div>
    <div class="group grow">
      <button class="play" onclick={() => (app.playing = !app.playing)} disabled={!app.model} aria-label={app.playing ? 'Pause turntable' : 'Play turntable'}>
        {app.playing ? '❚❚' : '▶'}
      </button>
      <input type="range" min="0" max="359" step="1" bind:value={app.turntableDeg} disabled={!app.model} aria-label="Turntable angle" />
      <span class="deg">{Math.round(app.turntableDeg)}°</span>
    </div>
  </div>
</div>

<style>
  .viewport {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background: var(--viewport-bg);
  }
  .stage {
    position: relative;
    overflow: hidden;
    flex: 1;
    min-height: 0;
    display: flex;
  }
  canvas {
    flex: 1;
    width: 100%;
    min-height: 0;
    display: block;
    touch-action: none;
  }
  .frame {
    position: absolute;
    pointer-events: none;
    border: 1px solid rgba(255, 255, 255, 0.55);
    outline: 1px solid rgba(0, 0, 0, 0.35);
    box-shadow: 0 0 0 100vmax rgba(0, 0, 0, 0.32);
  }
  .frame span {
    position: absolute;
    left: 6px;
    top: 4px;
    font-size: 0.72rem;
    color: #fff;
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.7);
    font-variant-numeric: tabular-nums;
  }
  .ptbadge {
    position: absolute;
    right: 12px;
    top: 12px;
    background: var(--panel);
    border: 1px solid var(--border);
    padding: 3px 10px;
    border-radius: 999px;
    font-size: 0.8rem;
    font-variant-numeric: tabular-nums;
  }
  .ptbadge.err {
    color: var(--warn);
    max-width: 60%;
  }
  .group .active {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  canvas.picking {
    cursor: crosshair;
  }
  .hint {
    position: absolute;
    top: 12px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--panel);
    border: 1px solid var(--accent);
    padding: 4px 6px 4px 12px;
    border-radius: 999px;
    font-size: 0.85rem;
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .hint button {
    padding: 2px 8px;
  }
  .empty {
    position: absolute;
    inset: 0 0 48px 0;
    display: grid;
    place-content: center;
    text-align: center;
    color: var(--muted);
    pointer-events: none;
  }
  .empty .big {
    font-size: 1.4rem;
    color: var(--text);
    margin: 0 0 0.25rem;
  }
  .loading {
    position: absolute;
    top: 12px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--panel);
    border: 1px solid var(--border);
    padding: 4px 12px;
    border-radius: 999px;
    font-size: 0.85rem;
  }
  .toolbar {
    display: flex;
    gap: 12px;
    align-items: center;
    padding: 8px 12px;
    border-top: 1px solid var(--border);
    background: var(--panel);
    flex-wrap: wrap;
  }
  .group {
    display: flex;
    gap: 4px;
    align-items: center;
  }
  .grow {
    flex: 1;
    min-width: 200px;
  }
  .grow input {
    flex: 1;
  }
  .deg {
    width: 3.5em;
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: var(--muted);
    font-size: 0.85rem;
  }
  .play {
    width: 34px;
  }
</style>
