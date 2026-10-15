<script lang="ts">
  import { onMount } from 'svelte';
  import { app, type ExportTab } from '../state/app.svelte';
  import { STILL_SIZES } from '../export/still';
  import { VIDEO_FORMATS } from '../export/video';
  import FourViewPanel from './FourViewPanel.svelte';

  const sizes = [480, 640, 960, 1280];
  const fpsOptions = [10, 20, 25, 30];
  const durations = [3, 4, 6, 8];
  const tabs: [ExportTab, string][] = [['turntable', 'Turntable'], ['still', 'Still'], ['fourview', 'Four-view']];

  onMount(() => void app.detectVideo());
  const canPick = typeof window !== 'undefined' && 'showSaveFilePicker' in window;

  const frames = $derived(Math.round((app.gif.motion.startsWith('path') ? app.camPathSeconds() : app.gif.seconds) * app.gif.fps));
  const pxPerLayer = $derived.by(() => {
    void app.gif.size;
    void app.settings.layers.heightMm;
    void app.model;
    void app.settings.pose.display;
    return app.exportPxPerLayer();
  });
  const layerNote = $derived.by(() => {
    if (pxPerLayer === null || !app.settings.layers.enabled || app.model?.kind === 'gcode') return null;
    if (pxPerLayer >= 4) return { level: 'ok', text: `Layers ≈ ${pxPerLayer.toFixed(1)} px: visible lines` };
    if (pxPerLayer >= 2) return { level: 'mid', text: `Layers ≈ ${pxPerLayer.toFixed(1)} px: partially faded to avoid shimmer` };
    return { level: 'low', text: `Layers ≈ ${pxPerLayer.toFixed(1)} px: too small to show; try a larger size` };
  });
  const progressPct = $derived(app.exporting ? Math.round((100 * app.exporting.done) / Math.max(1, app.exporting.total)) : 0);
  const stillKey = $derived(`${app.still.width}x${app.still.height}`);
  const gifDelayOk = $derived(Number.isInteger(100 / app.gif.fps));

  function setStillSize(key: string) {
    const [w, h] = key.split('x').map(Number);
    app.still.width = w;
    app.still.height = h;
  }
</script>

<aside class="panel">
  <div class="tabs" role="tablist">
    {#each tabs as [id, label]}
      <button role="tab" aria-selected={app.exportTab === id} class:active={app.exportTab === id} onclick={() => (app.exportTab = id)}>{label}</button>
    {/each}
  </div>

  {#if app.exportTab === 'turntable'}
    <section>
      <div class="row">
        <label for="gsize">Size</label>
        <select id="gsize" bind:value={app.gif.size}>
          {#each sizes as s}<option value={s}>{s} × {s}</option>{/each}
        </select>
      </div>
      <div class="row" class:hidden={app.gif.motion.startsWith('path')}>
        <label for="gdur">Duration</label>
        <select id="gdur" bind:value={app.gif.seconds}>
          {#each durations as d}<option value={d}>{d} s</option>{/each}
        </select>
      </div>
      <div class="row">
        <label for="gfps">Frame rate</label>
        <select id="gfps" bind:value={app.gif.fps}>
          {#each fpsOptions as f}<option value={f}>{f} fps</option>{/each}
        </select>
      </div>
      <div class="row">
        <label for="gelev">Camera height</label>
        <select id="gelev" bind:value={app.gif.elevationDeg} onchange={() => app.frameTurntable()}>
          {#each [0, 10, 20, 30, 45] as e}<option value={e}>{e}°</option>{/each}
        </select>
        <button onclick={() => app.frameTurntable()} disabled={!app.model} title="Fit the whole rotation inside the frame">Frame</button>
      </div>
      <p class="muted small">Renders exactly what's inside the viewport frame. “Frame” fits the full rotation; orbit/zoom to compose your own shot.</p>
      <div class="row">
        <label for="gmotion">Motion</label>
        <select id="gmotion" bind:value={app.gif.motion}>
          <option value="rotate">Rotate (seamless loop)</option>
          <option value="build">Build-up (fixed view)</option>
          <option value="build-rotate">Build-up while rotating</option>
          <option value="path">Camera path (keyframes)</option>
          <option value="path-build">Camera path + build-up</option>
        </select>
      </div>
      {#if app.gif.motion === 'path' || app.gif.motion === 'path-build'}
        <div class="keys">
          {#each app.camKeys as k, i}
            {#if i > 0}
              {@const tr = app.camTransitions[i - 1]}
              <div class="transition">
                <span class="muted">↓</span>
                <input type="number" min="0.2" max="30" step="0.1" bind:value={tr.seconds} aria-label="Transition seconds" title="Transition (s)" />s
                <input type="number" min="-5" max="5" step="1" bind:value={tr.turns} aria-label="Extra turns" title="Extra orbit turns (spiral)" />turns
                <select bind:value={tr.easing} aria-label="Easing">
                  <option value="ease">ease</option>
                  <option value="linear">linear</option>
                  <option value="ease-in">ease in</option>
                  <option value="ease-out">ease out</option>
                </select>
                hold <input type="number" min="0" max="10" step="0.1" bind:value={tr.hold} aria-label="Hold seconds" />s
              </div>
            {/if}
            <div class="key">
              <span>#{i + 1}</span>
              <span class="muted small">az {k.azimuth.toFixed(0)}° · el {k.elevation.toFixed(0)}°</span>
              <button onclick={() => app.gotoCamKey(i)} title="Move the viewport camera here">Go</button>
              <button onclick={() => app.removeCamKey(i)} aria-label="Remove keyframe {i + 1}">×</button>
            </div>
          {/each}
          <div class="row">
            <button onclick={() => app.addCamKey()} disabled={!app.model}>Add current view</button>
            <button onclick={() => app.previewCamPath()} disabled={app.camKeys.length < 2 || app.pathPreview}>Preview</button>
          </div>
          <p class="muted small">{app.camKeys.length < 2 ? 'Orbit the viewport to a view and add it; add at least two.' : `${app.camPathSeconds().toFixed(1)} s path. Use “turns” for spirals, e.g. front → top with 1 turn.`}</p>
        </div>
      {/if}
      <div class="row">
        <label for="glayout">Layout</label>
        <select id="glayout" bind:value={app.gif.layout}>
          <option value="single">Single view</option>
          <option value="section-side">Side by side: whole | section</option>
        </select>
      </div>
      {#if app.gif.layout === 'section-side'}
        <p class="muted small indent">{app.inspection.section.enabled ? 'Uses your section plane from Inspect.' : 'Uses a centred front-to-back cut (set one in Inspect to customise).'} Output is {app.gif.size * 2} × {app.gif.size}.</p>
      {/if}
      <div class="row">
        <label for="geng">Engine</label>
        <select id="geng" bind:value={app.gif.engine}>
          <option value="studio">Studio (seconds)</option>
          <option value="pathtraced" disabled={!app.ptSupported}>Path traced (minutes)</option>
        </select>
      </div>
      {#if app.gif.engine === 'pathtraced'}
        <div class="row indent">
          <label for="gsamples">Samples</label>
          <select id="gsamples" bind:value={app.gif.samples}>
            {#each [16, 32, 64, 128, 256] as n}<option value={n}>{n} per pixel</option>{/each}
          </select>
        </div>
        <label class="check indent"><input id="gdenoise" type="checkbox" bind:checked={app.gif.denoise} /> Denoise (OIDN)</label>
      {/if}
      <div class="row">
        <label for="gss">Antialiasing</label>
        <select id="gss" bind:value={app.gif.supersample} disabled={app.gif.engine === 'pathtraced'}>
          <option value={1}>Standard</option>
          <option value={2}>2× supersample</option>
          <option value={3}>3× supersample</option>
        </select>
      </div>
      <p class="muted small">{frames} frames · {({ rotate: 'one full rotation · loops seamlessly', build: 'the part prints itself, then holds', 'build-rotate': 'prints while turning one full rotation', path: 'follows your camera keyframes', 'path-build': 'prints while following your camera keyframes' } as Record<string, string>)[app.gif.motion]}</p>
      {#if layerNote}
        <p class="note {layerNote.level}">{layerNote.text}</p>
      {/if}
    </section>
    <section>
      <h2>Formats</h2>
      <label class="check"><input type="checkbox" bind:checked={app.gif.gif} /> GIF (256 colours)</label>
      {#if app.gif.gif}
        <div class="row indent">
          <label for="gdither">Dithering</label>
          <select id="gdither" bind:value={app.gif.dither}>
            <option value="bayer">Ordered (stable)</option>
            <option value="none">None</option>
          </select>
        </div>
        {#if !gifDelayOk}
          <p class="note mid">GIF stores delays in 1/100 s; {app.gif.fps} fps will play at {(100 / Math.round(100 / app.gif.fps)).toFixed(1)} fps.</p>
        {/if}
      {/if}
      <div class="row">
        <label for="gvideo">Video</label>
        <select id="gvideo" bind:value={app.gif.video}>
          <option value="none">None</option>
          {#each app.videoFormats as f}<option value={f}>{VIDEO_FORMATS[f].label}</option>{/each}
        </select>
      </div>
      {#if app.gif.video !== 'none'}
        <div class="row indent">
          <label for="gloops">Repeat</label>
          <select id="gloops" bind:value={app.gif.videoLoops}>
            {#each [1, 2, 3, 5] as n}<option value={n}>{n} rotation{n > 1 ? 's' : ''}</option>{/each}
          </select>
        </div>
      {/if}
      {@render action('Render turntable', () => app.exportTurntable(), !app.gif.gif && app.gif.video === 'none')}
    </section>
  {:else if app.exportTab === 'still'}
    <section>
      <div class="row">
        <label for="ssize">Size</label>
        <select id="ssize" value={stillKey} onchange={(e) => setStillSize(e.currentTarget.value)}>
          {#each STILL_SIZES as s}<option value="{s.width}x{s.height}">{s.label}</option>{/each}
        </select>
      </div>
      <div class="row">
        <label for="sss">Antialiasing</label>
        <select id="sss" bind:value={app.still.supersample}>
          <option value={1}>Standard</option>
          <option value={2}>2× supersample</option>
          <option value={3}>3× supersample</option>
        </select>
      </div>
      <div class="row">
        <label for="seng">Engine</label>
        <select id="seng" bind:value={app.still.engine}>
          <option value="studio">Studio (instant)</option>
          <option value="pathtraced" disabled={!app.ptSupported}>Path traced (quality)</option>
        </select>
      </div>
      {#if app.still.engine === 'pathtraced'}
        <div class="row indent">
          <label for="ssamples">Samples</label>
          <select id="ssamples" bind:value={app.still.samples}>
            {#each [32, 64, 128, 256, 512, 1024, 2048] as n}<option value={n}>{n} per pixel</option>{/each}
          </select>
        </div>
        <label class="check indent"><input type="checkbox" bind:checked={app.still.denoise} /> Denoise (OIDN)</label>
        <p class="muted small">Raytraced with WebGPU: soft shadows, reflections and real refraction. Sliced layers are traced as geometry; Shaded layer ribs, sections and see-through are Studio-only.</p>
      {/if}
      <label class="check"><input type="checkbox" bind:checked={app.still.transparent} /> Transparent background (no floor)</label>
      <p class="muted small">Uses the current viewport camera. Frame the shot in the viewport first.</p>
      {@render action('Render still', () => app.exportStill(), false)}
    </section>
  {:else}
    <FourViewPanel />
  {/if}

  {#if app.results.length}
    <section>
      <h2>Result</h2>
      {#each app.results as r}
        {#if r.kind === 'video'}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video src={r.url} class="preview" autoplay loop muted playsinline></video>
        {:else}
          <img src={r.url} alt="Rendered output" class="preview" />
        {/if}
        <div class="saverow">
          <a class="button primary" href={r.url} download={r.name}>Download {r.name.split('.').pop()?.toUpperCase()} · {(r.bytes / 1e6).toFixed(2)} MB</a>
          {#if canPick}<button onclick={() => app.saveResult(r)} title="Choose where to save">Save as…</button>{/if}
        </div>
      {/each}
      {#if app.results.some((r) => r.kind === 'gif' && r.bytes > 15e6)}
        <p class="note mid">Large GIF. For sharing, try 640 × 640, a shorter duration, 20 fps, or MP4 / WebM (AV1) which are ~5–10× smaller.</p>
      {/if}
      <p class="small muted summary">{app.resultSummary}</p>
    </section>
  {/if}
</aside>

{#snippet action(label: string, run: () => void, disabled: boolean)}
  {#if app.exporting}
    <div class="progress" role="progressbar" aria-valuenow={progressPct} aria-valuemin="0" aria-valuemax="100">
      <div class="bar" style="width:{progressPct}%"></div>
    </div>
    <p class="small">{app.exporting.stage === 'render' ? 'Rendering' : 'Encoding'} {app.exporting.done}/{app.exporting.total}{app.exportTab === 'still' && app.still.engine === 'pathtraced' ? ' samples' : ''}</p>
    <button onclick={() => app.cancelExport()}>Cancel</button>
  {:else}
    <button class="primary" onclick={run} disabled={!app.model || disabled}>{label}</button>
  {/if}
{/snippet}

<style>
  .panel {
    width: 300px;
    flex-shrink: 0;
    overflow-y: auto;
    background: var(--panel);
    border-left: 1px solid var(--border);
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 8px 10px 0;
    border-bottom: 1px solid var(--border);
  }
  .tabs button {
    flex: 1;
    border: none;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    background: none;
    padding: 6px 4px;
    color: var(--muted);
  }
  .tabs button.active {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  section {
    padding: 12px 14px;
    border-bottom: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  h2 {
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--muted);
    margin: 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 0.85rem;
  }
  .row label {
    width: 7.5em;
    color: var(--muted);
  }
  .row select {
    flex: 1;
    min-width: 0;
  }
  .indent {
    padding-left: 1.2em;
  }
  .hidden {
    display: none !important;
  }
  .keys {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 0.8rem;
  }
  .key {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .key span:nth-child(2) {
    flex: 1;
  }
  .key button {
    padding: 2px 8px;
  }
  .transition {
    display: flex;
    align-items: center;
    gap: 3px;
    flex-wrap: wrap;
    padding-left: 0.6em;
    color: var(--muted);
  }
  .transition input {
    width: 3.6em;
    font: inherit;
    color: var(--text);
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 1px 3px;
  }
  .transition select {
    padding: 1px 3px;
  }
  .indent label {
    width: 6.3em;
  }
  .check {
    display: flex;
    gap: 6px;
    align-items: center;
    font-size: 0.85rem;
  }
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 0.8rem;
    margin: 0;
  }
  .note {
    font-size: 0.8rem;
    margin: 0;
    padding: 6px 8px;
    border-radius: 6px;
    background: var(--accent-soft);
  }
  .note.mid,
  .note.low {
    background: var(--warn-soft);
  }
  .note.low {
    color: var(--warn);
  }
  .progress {
    height: 6px;
    background: var(--border);
    border-radius: 3px;
    overflow: hidden;
  }
  .bar {
    height: 100%;
    background: var(--accent);
    transition: width 0.1s;
  }
  .preview {
    width: 100%;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: repeating-conic-gradient(var(--border) 0 25%, transparent 0 50%) 50% / 16px 16px;
  }
  .saverow {
    display: flex;
    gap: 6px;
  }
  .saverow .button {
    flex: 1;
  }
  .summary {
    line-height: 1.35;
  }
</style>
