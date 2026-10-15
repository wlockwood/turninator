<script lang="ts">
  import { app } from '../state/app.svelte';
  import { MATERIAL_PRESETS } from '../state/settings';
  import { FEATURE_COLORS } from '../render/beads';
  import Slider from './Slider.svelte';

  const fmt = (n: number, d = 1) => n.toLocaleString(undefined, { maximumFractionDigits: d, minimumFractionDigits: d });
  const mat = $derived(app.settings.material);
  const layers = $derived(app.settings.layers);
  const isGcode = $derived(app.model?.kind === 'gcode');
  let gcodeInput = $state<HTMLInputElement>();

  function toggleFeature(name: string, visible: boolean) {
    const hidden = app.gcodeView.hidden.filter((n) => n !== name);
    if (!visible) hidden.push(name);
    app.gcodeView.hidden = hidden;
    app.applyGcodeView();
  }

  function rotate(which: 'print' | 'display', axis: 0 | 1 | 2, deg: number) {
    const pose = app.settings.pose[which];
    pose[axis] = (((pose[axis] + deg) % 360) + 360) % 360;
  }

  const units: [string, number][] = [['mm', 1], ['inch', 25.4], ['cm', 10], ['m', 1000]];
</script>

<aside class="panel">
  <section>
    <h2>Model</h2>
    {#if app.model?.gcode}
      {@const g = app.model.gcode}
      {@const z = app.model.sizeMm}
      <p class="name" title={app.model.name}>{app.model.name}</p>
      <dl>
        <dt>Size</dt>
        <dd>{fmt(z[0])} × {fmt(z[1])} × {fmt(z[2])} mm</dd>
        <dt>Beads</dt>
        <dd>{g.beads.toLocaleString()}</dd>
        <dt>Layers</dt>
        <dd>{g.layers}</dd>
        <dt>Filament</dt>
        <dd>{fmt(g.filamentMm / 1000, 2)} m · {fmt(g.volumeMm3 / 1000, 1)} cm³</dd>
        {#if g.tools > 1}
          <dt>Tools</dt>
          <dd>{g.tools}</dd>
        {/if}
        {#if g.slicer}
          <dt>Slicer</dt>
          <dd>{g.slicer}</dd>
        {/if}
      </dl>
      {#each app.model.warnings as w}
        <p class="warn">{w}</p>
      {/each}
    {:else if app.model?.stats}
      {@const s = app.model.stats}
      <p class="name" title={app.model.name}>{app.model.name}</p>
      <dl>
        <dt>Size</dt>
        <dd>{fmt(s.max[0] - s.min[0])} × {fmt(s.max[1] - s.min[1])} × {fmt(s.max[2] - s.min[2])} mm</dd>
        <dt>Triangles</dt>
        <dd>{s.triangles.toLocaleString()}</dd>
        <dt>Shells</dt>
        <dd>{s.shells}</dd>
        <dt>Watertight</dt>
        <dd class:bad={!s.watertight}>{s.watertight ? 'Yes' : `No (${s.openEdges} open edges)`}</dd>
        {#if s.watertight}
          <dt>Volume</dt>
          <dd>{fmt(s.volumeMm3 / 1000, 2)} cm³</dd>
        {/if}
      </dl>
      {#each app.model.warnings as w}
        <p class="warn">{w}</p>
      {/each}
    {:else}
      <p class="muted">No model loaded.</p>
    {/if}
    <div class="row" class:hidden={isGcode}>
      <label for="units">Units</label>
      <select id="units" bind:value={app.settings.mmPerUnit} title="Applies to the next file you open">
        {#each units as [label, v]}
          <option value={v}>{label}</option>
        {/each}
      </select>
      <span class="muted small">(STL assumed mm)</span>
    </div>
    <Slider label="Smoothing" bind:value={app.settings.creaseDeg} min={0} max={90} step={1} digits={0} unit="°" onchange={(v) => app.setCrease(v)} disabled={isGcode} />
  </section>

  {#if app.model?.gcode}
    {@const g = app.model.gcode}
    <section>
      <h2>Print preview</h2>
      <p class="muted small">Real extrusion beads from the slicer's toolpaths. Width and height come from the G-code.</p>
      <div class="row">
        <label for="gcolor">Colour by</label>
        <select id="gcolor" bind:value={app.gcodeView.colorMode} onchange={() => app.applyGcodeView()}>
          <option value="material">Material</option>
          <option value="feature">Feature type</option>
          <option value="tool">Tool / filament</option>
          <option value="layer">Layer</option>
        </select>
      </div>
      <div class="features">
        {#each g.features as f}
          <label class="check small">
            <input type="checkbox" checked={!app.gcodeView.hidden.includes(f.name)} onchange={(e) => toggleFeature(f.name, e.currentTarget.checked)} />
            {#if app.gcodeView.colorMode === 'feature'}<span class="chip" style="background:{FEATURE_COLORS[f.name] ?? '#ccc'}"></span>{/if}
            {f.name} <span class="muted">({f.count.toLocaleString()})</span>
          </label>
        {/each}
      </div>
      {#if g.tools > 1 || app.gcodeView.colorMode === 'tool'}
        <div class="row tools">
          <span class="muted small">Filaments</span>
          {#each app.gcodeView.toolColors.slice(0, Math.max(g.tools, 1)) as _, i}
            <input type="color" title="T{i}" bind:value={app.gcodeView.toolColors[i]} onchange={() => app.applyGcodeView()} />
          {/each}
        </div>
      {/if}
    </section>
  {/if}

  {#if app.model}
    <section>
      <h2>Build-up</h2>
      <div class="row">
        <button class="play" onclick={() => app.toggleBuildPlay()} aria-label={app.buildPlaying ? 'Pause build-up' : 'Play build-up'}>{app.buildPlaying ? '❚❚' : '▶'}</button>
        <input type="range" min="0" max="1" step="0.0005" bind:value={app.buildProgress} aria-label="Build progress" class="grow" />
      </div>
      <p class="muted small">{app.buildLayerLabel()} · {isGcode ? 'plays back the toolpath bead by bead' : 'cuts the solid with a rising plane'}. Use “Build-up” motion in the Turntable export to animate it.</p>
    </section>
  {/if}

  <section>
    <h2>Orientation</h2>
    <p class="muted small">{isGcode ? 'G-code is already oriented as printed; display pose changes only how it is shown.' : "Print pose sets the build axis that layers follow. Display pose only changes how it is shown."}</p>
    {#each (isGcode ? [['display', 'Display']] : [['print', 'Print'], ['display', 'Display']]) as [key, label]}
      <div class="rot">
        <span>{label}</span>
        {#each ['X', 'Y', 'Z'] as axisName, axis}
          <button onclick={() => rotate(key as 'print' | 'display', axis as 0 | 1 | 2, 90)} disabled={!app.model} title="Rotate 90° about {axisName}">
            {axisName}+90
          </button>
        {/each}
        <button onclick={() => (app.settings.pose[key as 'print' | 'display'] = [0, 0, 0])} disabled={!app.model}>Reset</button>
      </div>
    {/each}
    <div class="rot">
      <span>Lay flat</span>
      {#if !isGcode}
        <button class:active={app.layFlat === 'print'} onclick={() => (app.layFlat = app.layFlat === 'print' ? null : 'print')} disabled={!app.model} title="Click a face to print it face-down (re-orients layers)">Pick (print)</button>
      {/if}
      <button class:active={app.layFlat === 'display'} onclick={() => (app.layFlat = app.layFlat === 'display' ? null : 'display')} disabled={!app.model} title="Click a face to rest it on the floor for presentation only">Pick (display)</button>
      {#if !isGcode}
        <button onclick={() => app.autoLayFlat()} disabled={!app.model} title="Print on the largest flat face">Auto</button>
      {/if}
    </div>
  </section>

  <section>
    <h2>Material</h2>
    <div class="swatches">
      {#each Object.entries(MATERIAL_PRESETS) as [key, p]}
        <button class="swatch" class:active={mat.preset === key} onclick={() => app.applyMaterialPreset(key)} title={p.label}>
          <span class="chip" style="background:{p.material.color}"></span>
          {p.label}
        </button>
      {/each}
    </div>
    <div class="row">
      <label for="color">Color</label>
      <input id="color" type="color" bind:value={app.settings.material.color} />
    </div>
    <Slider label="Roughness" bind:value={app.settings.material.roughness} min={0} max={1} />
    {#if mat.transmission > 0}
      <Slider label="Clarity" bind:value={app.settings.material.transmission} min={0} max={1} />
      <Slider label="Tint depth" bind:value={app.settings.material.attenuationDistance} min={1} max={100} step={1} digits={0} unit=" mm" />
    {/if}
  </section>

  {#if !isGcode}
  <section>
    <h2>Layer lines</h2>
    <label class="check">
      <input type="checkbox" bind:checked={app.settings.layers.enabled} />
      Show printed layers
    </label>
    <div class="seg" role="radiogroup" aria-label="Layer mode">
      <button role="radio" aria-checked={layers.mode === 'shaded'} class:active={layers.mode === 'shaded'} disabled={!layers.enabled} onclick={() => (app.settings.layers.mode = 'shaded')} title="Fast: layer ribs shaded on the original surface">Shaded</button>
      <button role="radio" aria-checked={layers.mode === 'sliced'} class:active={layers.mode === 'sliced'} disabled={!layers.enabled} onclick={() => (app.settings.layers.mode = 'sliced')} title="Real stepped geometry: stair-stepping on slopes and silhouettes">Sliced</button>
      <button role="radio" aria-checked={layers.mode === 'auto'} class:active={layers.mode === 'auto'} disabled={!layers.enabled} onclick={() => (app.settings.layers.mode = 'auto')} title="Sliced when layers are at least 6 px in the turntable export, else Shaded">Auto</button>
    </div>
    {#if layers.enabled && layers.mode === 'auto'}
      <p class="muted small">Auto → {app.effectiveLayerMode() === 'sliced' ? 'Sliced' : 'Shaded'} at the current export size.</p>
    {/if}
    {#if layers.enabled && app.effectiveLayerMode() === 'sliced'}
      {#if app.slicing}
        <p class="muted small">Slicing… {Math.round((100 * app.slicing.done) / Math.max(1, app.slicing.total))}%</p>
      {:else if app.sliceInfo}
        <p class="muted small">{app.sliceInfo}</p>
      {/if}
    {/if}
    <div class="row">
      <label for="lprofile">Heights</label>
      <select id="lprofile" bind:value={app.settings.layers.profile} disabled={!layers.enabled}>
        <option value="uniform">Uniform</option>
        <option value="adaptive">Adaptive (by slope)</option>
        <option value="gcode">From a G-code file…</option>
      </select>
    </div>
    {#if layers.profile === 'uniform'}
      <Slider label="Layer height" bind:value={app.settings.layers.heightMm} min={0.04} max={0.4} step={0.01} unit=" mm" disabled={!layers.enabled} />
    {:else if layers.profile === 'adaptive'}
      <Slider label="Min height" bind:value={app.settings.layers.adaptiveMinMm} min={0.04} max={0.3} step={0.01} unit=" mm" disabled={!layers.enabled} />
      <Slider label="Max height" bind:value={app.settings.layers.adaptiveMaxMm} min={0.08} max={0.4} step={0.01} unit=" mm" disabled={!layers.enabled} />
      <Slider label="Quality" bind:value={app.settings.layers.adaptiveCuspMm} min={0.005} max={0.1} step={0.005} digits={3} unit=" mm" disabled={!layers.enabled} />
    {:else}
      <div class="row">
        <button onclick={() => gcodeInput?.click()} disabled={!layers.enabled}>Choose G-code…</button>
        <span class="muted small ellipsis">{app.gcodeLayers?.name ?? 'No file chosen'}</span>
        <input bind:this={gcodeInput} type="file" accept=".gcode,.gco,.g,.bgcode,.3mf" hidden onchange={(e) => { const f = e.currentTarget.files?.[0]; if (f) void app.loadLayerHeights(f); }} />
      </div>
    {/if}
    {#if app.layerSchedule && layers.enabled}
      <p class="muted small">{app.layerSchedule.info}</p>
    {/if}
    <Slider label="First layer" bind:value={app.settings.layers.firstMm} min={0.04} max={0.5} step={0.01} unit=" mm" disabled={!layers.enabled} />
    <Slider label="Depth" bind:value={app.settings.layers.depthMm} min={0.002} max={0.08} step={0.001} digits={3} unit=" mm" disabled={!layers.enabled} />
    <Slider label="Roundness" bind:value={app.settings.layers.roundness} min={0.25} max={1} disabled={!layers.enabled} />
    <label class="check">
      <input type="checkbox" bind:checked={app.settings.layers.topLines} disabled={!layers.enabled} />
      Top-surface infill lines
    </label>
    <Slider label="Line width" bind:value={app.settings.layers.lineWidthMm} min={0.2} max={1} step={0.01} unit=" mm" disabled={!layers.enabled || !layers.topLines} />
    <p class="muted small">Layers follow the print pose's Z axis. Lines fade out when smaller than ~3 px to avoid shimmer. Sliced mode estimates the sliced solid; it is not a toolpath.</p>
  </section>

  {/if}

  <section>
    <h2>Inspect</h2>
    <label class="check">
      <input type="checkbox" bind:checked={app.inspection.seeThrough.enabled} disabled={!app.model} />
      See-through (X-ray)
    </label>
    {#if app.inspection.seeThrough.enabled}
      <Slider label="Opacity" bind:value={app.inspection.seeThrough.opacity} min={0.05} max={0.9} />
      <label class="check indent"><input type="checkbox" bind:checked={app.inspection.seeThrough.edges} /> Feature edges</label>
    {/if}
    <label class="check">
      <input type="checkbox" bind:checked={app.inspection.section.enabled} disabled={!app.model} />
      Section plane
    </label>
    {#if app.inspection.section.enabled}
      {@const range = app.sectionRange(app.inspection.section.axis)}
      <div class="seg">
        {#each ['x', 'y', 'z'] as const as axis}
          <button class:active={app.inspection.section.axis === axis} onclick={() => app.setSectionAxis(axis)}>{axis.toUpperCase()}</button>
        {/each}
        <button class:active={app.inspection.section.flip} onclick={() => (app.inspection.section.flip = !app.inspection.section.flip)} title="Keep the other side">Flip</button>
      </div>
      <Slider label="Position" bind:value={app.inspection.section.offsetMm} min={range[0]} max={range[1]} step={Math.max(0.01, (range[1] - range[0]) / 500)} unit=" mm" />
      <div class="row">
        <label for="capc">Cut face</label>
        <input id="capc" type="color" bind:value={app.inspection.section.capColor} />
      </div>
      <p class="muted small">Solid caps need a watertight mesh. Sections also appear in exports.</p>
    {/if}
  </section>

  <section>
    <h2>Studio</h2>
    <div class="seg">
      <button class:active={app.settings.studio.name === 'light'} onclick={() => app.applyStudio('light')}>Light</button>
      <button class:active={app.settings.studio.name === 'dark'} onclick={() => app.applyStudio('dark')}>Dark</button>
    </div>
    <div class="row">
      <label for="bg">Background</label>
      <input id="bg" type="color" bind:value={app.settings.studio.background} />
    </div>
    <Slider label="Key light" bind:value={app.settings.studio.keyIntensity} min={0} max={6} step={0.1} digits={1} />
    <Slider label="Ambient" bind:value={app.settings.studio.envIntensity} min={0} max={1.5} />
    <label class="check">
      <input type="checkbox" bind:checked={app.settings.studio.shadows} />
      Shadows
    </label>
    <label class="check">
      <input type="checkbox" bind:checked={app.settings.studio.ao} />
      Ambient occlusion
    </label>
  </section>
</aside>

<style>
  .panel {
    width: 300px;
    flex-shrink: 0;
    overflow-y: auto;
    background: var(--panel);
    border-right: 1px solid var(--border);
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
  .name {
    margin: 0;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  dl {
    display: grid;
    grid-template-columns: 6.5em 1fr;
    gap: 2px 8px;
    margin: 0;
    font-size: 0.85rem;
  }
  dt {
    color: var(--muted);
  }
  dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
  }
  .bad {
    color: var(--warn);
  }
  .warn {
    margin: 0;
    font-size: 0.8rem;
    color: var(--warn);
  }
  .muted {
    color: var(--muted);
    margin: 0;
  }
  .small {
    font-size: 0.78rem;
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
  .rot {
    display: flex;
    gap: 4px;
    align-items: center;
    font-size: 0.8rem;
  }
  .rot span {
    width: 4.2em;
    color: var(--muted);
  }
  .rot button {
    padding: 3px 6px;
    font-size: 0.75rem;
  }
  .swatches {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px;
  }
  .swatch {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 0.78rem;
    text-align: left;
    padding: 4px 6px;
  }
  .swatch.active,
  .rot .active,
  .seg .active {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .chip {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    flex-shrink: 0;
    border: 1px solid var(--border);
  }
  .hidden {
    display: none !important;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 1;
  }
  .features {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 180px;
    overflow-y: auto;
  }
  .tools {
    flex-wrap: wrap;
  }
  .row .grow {
    flex: 1;
  }
  .play {
    width: 34px;
  }
  .indent {
    padding-left: 1.4em;
  }
  .check {
    display: flex;
    gap: 6px;
    align-items: center;
    font-size: 0.85rem;
  }
  .seg {
    display: flex;
    gap: 4px;
  }
  .seg button {
    flex: 1;
  }
</style>
