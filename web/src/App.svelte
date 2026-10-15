<script lang="ts">
  import { app } from './state/app.svelte';
  import Sidebar from './ui/Sidebar.svelte';
  import ExportPanel from './ui/ExportPanel.svelte';
  import Viewport from './ui/Viewport.svelte';

  let fileInput: HTMLInputElement;
  let projectInput: HTMLInputElement;

  // Remember settings between sessions (debounced).
  $effect(() => {
    const snapshot = JSON.stringify(app.persistentState());
    const t = setTimeout(() => app.rememberSession(), 500);
    return () => {
      clearTimeout(t);
      void snapshot;
    };
  });
  let dragging = $state(false);

  function pick(files: FileList | null | undefined) {
    const f = files?.[0];
    if (f) void app.openFile(f);
  }
</script>

<svelte:window
  ondragover={(e) => {
    e.preventDefault();
    dragging = true;
  }}
  ondragleave={(e) => {
    if (e.relatedTarget === null) dragging = false;
  }}
  ondrop={(e) => {
    e.preventDefault();
    dragging = false;
    pick(e.dataTransfer?.files);
  }}
/>

<div class="app">
  <header>
    <h1>Turninator</h1>
    <button class="primary" onclick={() => fileInput.click()}>Open STL / G-code…</button>
    <input bind:this={fileInput} type="file" accept=".stl,model/stl,.gcode,.gco,.gc,.g,.bgcode,.3mf" hidden onchange={(e) => pick(e.currentTarget.files)} />
    <span class="status" class:error={!!app.error}>{app.error ?? app.status}</span>
    <span class="spacer"></span>
    <button onclick={() => app.saveProject()} title="Save settings, camera and keyframes for this model">Save project</button>
    <button onclick={() => projectInput.click()} title="Apply a saved .turninator.json project">Load project</button>
    <input bind:this={projectInput} type="file" accept=".json,application/json" hidden onchange={(e) => { const f = e.currentTarget.files?.[0]; if (f) void app.loadProject(f); e.currentTarget.value = ''; }} />
  </header>
  <main>
    <Sidebar />
    <Viewport />
    <ExportPanel />
  </main>
  {#if dragging}
    <div class="drop">Drop to open</div>
  {/if}
</div>

<style>
  .app {
    height: 100vh;
    display: flex;
    flex-direction: column;
  }
  header {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 8px 14px;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
  }
  h1 {
    font-size: 1rem;
    margin: 0;
    letter-spacing: 0.02em;
  }
  .status {
    color: var(--muted);
    font-size: 0.85rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .spacer {
    flex: 1;
  }
  .status.error {
    color: var(--warn);
  }
  main {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .drop {
    position: fixed;
    inset: 0;
    display: grid;
    place-content: center;
    font-size: 1.5rem;
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    border: 3px dashed var(--accent);
    pointer-events: none;
  }
</style>
