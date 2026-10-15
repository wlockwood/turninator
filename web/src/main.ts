import { mount } from 'svelte';
import App from './App.svelte';
import './app.css';
import { app as appState } from './state/app.svelte';

const app = mount(App, { target: document.getElementById('app')! });

// Offline support in production builds (dev uses Vite's server).
if (import.meta.env.PROD && 'serviceWorker' in navigator) {
  navigator.serviceWorker.register(`${import.meta.env.BASE_URL}sw.js`).catch((err) => console.warn('Service worker registration failed', err));
}

// Installed PWA: open files passed by the OS (File Handling API).
interface LaunchParams { files: FileSystemFileHandle[] }
const launchQueue = (window as unknown as { launchQueue?: { setConsumer(cb: (p: LaunchParams) => void): void } }).launchQueue;
launchQueue?.setConsumer(async (params) => {
  const handle = params.files[0];
  if (handle) void appState.openFile(await handle.getFile());
});

// Dev-only handle for debugging and browser tests.
if (import.meta.env.DEV) (window as unknown as { __turninator: typeof appState }).__turninator = appState;

export default app;
