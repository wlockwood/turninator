// Path-traced still check. Usage: node tools/pathtrace.mjs [stl] [size] [samples] [--sliced]
import { chromium } from 'playwright';
import { writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
const stl = resolve(process.argv[2] ?? '../test-models/spiral egg - A.stl');
const size = process.argv[3] ?? '2048x2048';
const samples = process.argv[4] ?? '128';
const sliced = process.argv.includes('--sliced');
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--enable-gpu', '--ignore-gpu-blocklist', '--use-angle=d3d11', '--enable-unsafe-webgpu'] });
const page = await browser.newPage({ viewport: { width: 1600, height: 950 } });
const logs = [];
page.on('console', (m) => { if (m.type() === 'error' || m.type() === 'warning') logs.push(`[${m.type()}] ${m.text().slice(0, 300)}`); });
page.on('pageerror', (e) => logs.push(`[pageerror] ${e.message}`));
await page.goto('http://localhost:5199/');
console.log('webgpu:', await page.evaluate(async () => !!(navigator.gpu && (await navigator.gpu.requestAdapter()))));
await page.setInputFiles('input[type=file]', stl);
await page.waitForFunction(() => document.querySelector('.status')?.textContent?.includes('triangles'));
if (sliced) {
  await page.click('role=radio[name="Sliced"]');
  await page.waitForFunction(() => [...document.querySelectorAll('p')].some((p) => /layers ·/.test(p.textContent ?? '')), null, { timeout: 120000 });
}
await page.click('role=tab[name="Still"]');
await page.selectOption('#ssize', { label: '2048 × 2048' }).catch(() => {});
if (size !== '2048x2048') await page.selectOption('#ssize', size);
await page.waitForFunction(() => !document.querySelector('#seng option[value=pathtraced]')?.hasAttribute('disabled'), null, { timeout: 30000 });
await page.selectOption('#seng', 'pathtraced');
await page.selectOption('#ssamples', samples);
const t = Date.now();
await page.click('text=Render still');
await page.waitForFunction(() => !document.querySelector('[role=progressbar]') && (document.querySelector('a[download]') || document.querySelector('.status.error')), null, { timeout: 600000 });
console.log('path traced in', Date.now() - t, 'ms', '|', await page.textContent('.status'));
const href = await page.getAttribute('a[download]', 'href').catch(() => null);
if (href) {
  const bytes = await page.evaluate(async (h) => { const b = await (await fetch(h)).blob(); return await new Promise((r) => { const fr = new FileReader(); fr.onload = () => r(String(fr.result).split(',')[1]); fr.readAsDataURL(b); }); }, href);
  writeFileSync(`test-results/pathtraced${sliced ? '-sliced' : ''}.png`, Buffer.from(bytes, 'base64'));
  console.log(await page.textContent('.summary'));
}
console.log(logs.filter((l) => !l.includes('X4122') && !l.includes('404')).slice(0, 20).join('\n'));
await browser.close();
