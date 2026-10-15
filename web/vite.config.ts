/// <reference types="vitest/config" />
import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Cross-origin isolation enables SharedArrayBuffer for future threaded WASM.
const isolation = {
  'Cross-Origin-Opener-Policy': 'same-origin',
  'Cross-Origin-Embedder-Policy': 'require-corp',
};

export default defineConfig({
  // Relative base so the static build works from any path (e.g. GitHub Pages).
  base: './',
  plugins: [svelte()],
  server: { headers: isolation },
  preview: { headers: isolation },
  worker: { format: 'es' },
  // Pre-bundle lazily imported deps so dev doesn't full-reload on first use.
  optimizeDeps: { include: ['three', 'three/webgpu', 'three-gpu-pathtracer/webgpu', 'mediabunny', 'gifenc', 'clipper-lib', 'earcut', 'oidn-web'] },
  build: { target: 'es2022', chunkSizeWarningLimit: 2000 },
  test: { environment: 'node', include: ['tests/**/*.test.ts'] },
});
