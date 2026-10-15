// Browser smoke test: loads a model, captures the viewport, renders a GIF.
// Usage: node tools/smoke.mjs [url] [stl-path] [gif-size]
// Uses the locally installed Microsoft Edge with hardware GPU.

import { chromium } from 'playwright';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const url = process.argv[2] ?? 'http://localhost:5199/';
const stl = resolve(process.argv[3] ?? '../test-models/spiral egg - A.stl');
const gifSize = process.argv[4] ?? '480';
const outDir = resolve('test-results');
mkdirSync(outDir, { recursive: true });

const browser = await chromium.launch({
  channel: 'msedge',
  headless: true,
  args: ['--enable-gpu', '--ignore-gpu-blocklist', '--use-angle=d3d11', '--enable-unsafe-webgpu'],
});
const page = await browser.newPage({ viewport: { width: 1600, height: 950 } });
const logs = [];
page.on('console', (m) => logs.push(`[${m.type()}] ${m.text()}`));
page.on('pageerror', (e) => logs.push(`[pageerror] ${e.message}`));

await page.goto(url);
const gpu = await page.evaluate(() => {
  const c = document.createElement('canvas');
  const gl = c.getContext('webgl2');
  const ext = gl?.getExtension('WEBGL_debug_renderer_info');
  return ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : 'unknown';
});
console.log('GPU:', gpu);

const t0 = Date.now();
await page.setInputFiles('input[type=file]', stl);
await page.waitForFunction(() => document.querySelector('.status')?.textContent?.includes('triangles'), null, { timeout: 60000 });
console.log('Loaded in', Date.now() - t0, 'ms:', await page.textContent('.status'));
await page.waitForTimeout(500);
await page.screenshot({ path: `${outDir}/app.png` });
await page.locator('canvas[data-testid=viewport]').screenshot({ path: `${outDir}/viewport.png` });

if (gifSize !== '0') {
  await page.selectOption('#gsize', gifSize);
  const t1 = Date.now();
  await page.click('text=Render turntable');
  await page.waitForSelector('a[download]', { timeout: 600000 });
  console.log('GIF done in', Date.now() - t1, 'ms');
  const href = await page.getAttribute('a[download]', 'href');
  const bytes = await page.evaluate(async (h) => { const b = await (await fetch(h)).blob(); return await new Promise((r) => { const fr = new FileReader(); fr.onload = () => r(String(fr.result).split(',')[1]); fr.readAsDataURL(b); }); }, href);
  writeFileSync(`${outDir}/turntable.gif`, Buffer.from(bytes, 'base64'));
  console.log('GIF bytes', Buffer.from(bytes, 'base64').length, '|', await page.textContent('.summary'));
  await page.screenshot({ path: `${outDir}/app-after-gif.png` });
}

console.log(logs.filter((l) => !l.includes('[vite]')).join('\n'));
await browser.close();
