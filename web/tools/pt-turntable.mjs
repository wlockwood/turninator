// Path-traced turntable check. Usage: node tools/pt-turntable.mjs [stl] [size] [samples] [seconds] [--sliced] [--no-denoise]
import { chromium } from 'playwright';
import { writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
const stl = resolve(process.argv[2] ?? '../test-models/spiral egg - A.stl');
const size = process.argv[3] ?? '480';
const samples = process.argv[4] ?? '32';
const seconds = process.argv[5] ?? '4';
const sliced = process.argv.includes('--sliced');
const noDenoise = process.argv.includes('--no-denoise');
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--enable-gpu', '--ignore-gpu-blocklist', '--use-angle=d3d11'] });
const page = await browser.newPage({ viewport: { width: 1600, height: 950 } });
const logs = [];
page.on('pageerror', (e) => logs.push(`[pageerror] ${e.message}`));
page.on('console', (m) => { if (m.type() !== 'debug') console.log('[console]', m.type(), m.text().slice(0, 300)); });
page.on('crash', () => console.log('[page crashed]'));
await page.goto('http://localhost:5199/');
await page.setInputFiles('input[type=file]', stl);
await page.waitForFunction(() => document.querySelector('.status')?.textContent?.includes('triangles'));
if (sliced) {
  await page.click('role=radio[name="Sliced"]');
  await page.waitForFunction(() => [...document.querySelectorAll('p')].some((p) => /layers ·/.test(p.textContent ?? '')), null, { timeout: 120000 });
}
await page.waitForFunction(() => !document.querySelector('#geng option[value=pathtraced]')?.hasAttribute('disabled'));
await page.selectOption('#gsize', size);
await page.selectOption('#gdur', seconds);
await page.selectOption('#geng', 'pathtraced');
await page.selectOption('#gsamples', samples);
if (noDenoise) await page.uncheck('#gdenoise');
if (await page.$('#gvideo option[value=mp4-h264]')) await page.selectOption('#gvideo', 'mp4-h264');
const t = Date.now();
await page.click('text=Render turntable');
const ticker = setInterval(async () => {
  const p = await page.locator('[role=progressbar] + p').textContent().catch(() => null);
  if (p) console.log(`[${((Date.now() - t) / 1000).toFixed(0)} s] ${p}`);
}, 15000);
await page.waitForFunction(() => !document.querySelector('[role=progressbar]') && (document.querySelector('a[download]') || document.querySelector('.status.error')), null, { timeout: 3600000, polling: 1000 });
clearInterval(ticker);
console.log('path-traced turntable in', Date.now() - t, 'ms |', await page.textContent('.status'));
console.log(await page.textContent('.summary'));
const links = await page.$$eval('a[download]', (as) => as.map((a) => ({ href: a.href, name: a.download })));
for (const l of links) {
  // Base64 keeps the transfer compact (a JS number array of a 15 MB GIF exhausted Node's heap).
  const b64 = await page.evaluate(async (h) => {
    const blob = await (await fetch(h)).blob();
    return await new Promise((res) => { const r = new FileReader(); r.onload = () => res(String(r.result).split(',')[1]); r.readAsDataURL(blob); });
  }, l.href);
  const ext = l.name.split('.').pop();
  writeFileSync(`test-results/pt-${sliced ? 'sliced-' : ''}${size}.${ext}`, Buffer.from(b64, 'base64'));
}
console.log(logs.filter((l) => !l.includes('404')).join('\n'));
await browser.close();
