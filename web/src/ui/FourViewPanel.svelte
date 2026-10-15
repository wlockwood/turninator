<script lang="ts">
  import { app } from '../state/app.svelte';

  const progressPct = $derived(app.exporting ? Math.round((100 * app.exporting.done) / Math.max(1, app.exporting.total)) : 0);
</script>

<section>
  <p class="muted small">Front, Right, Top and Isometric orthographic views of the print orientation, all at one shared scale, with a scale bar.</p>
  <div class="row">
    <label for="fvsize">Sheet</label>
    <select id="fvsize" bind:value={app.fourview.size}>
      <option value={2048}>2048 × 2048</option>
      <option value={4096}>4096 × 4096</option>
    </select>
  </div>
  <div class="row">
    <label for="fvss">Antialiasing</label>
    <select id="fvss" bind:value={app.fourview.supersample}>
      <option value={1}>Standard</option>
      <option value={2}>2× supersample</option>
    </select>
  </div>
  <label class="check"><input type="checkbox" bind:checked={app.fourview.edges} /> Outline edges</label>
  <label class="check"><input type="checkbox" bind:checked={app.fourview.useModelMaterial} /> Use model material (instead of neutral grey)</label>
  <label class="check"><input type="checkbox" bind:checked={app.fourview.showLayers} disabled={!app.settings.layers.enabled} /> Show layer lines</label>
  {#if app.exporting}
    <div class="progress" role="progressbar" aria-valuenow={progressPct} aria-valuemin="0" aria-valuemax="100">
      <div class="bar" style="width:{progressPct}%"></div>
    </div>
    <p class="small">Rendering panels…</p>
  {:else}
    <button class="primary" onclick={() => app.exportFourView()} disabled={!app.model}>Render four-view sheet</button>
  {/if}
</section>

<style>
  section {
    padding: 12px 14px;
    border-bottom: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 8px;
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
  }
  .check {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    font-size: 0.85rem;
  }
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 0.8rem;
    margin: 0;
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
  }
</style>
