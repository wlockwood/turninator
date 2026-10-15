// End-to-end browser check of every output mode on the real GPU.
// Usage: node tools/e2e.mjs [stl] [--url http://localhost:5199/] [--only turntable,still,fourview]
import { chromium } from 'playwright';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve, basename } from 'node:path';

const args = process.argv.slice(2);
const flag = (name, def) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : def;
};
const stl = resolve(args.find((a) => !a.startsWith('--') && !args[args.indexOf(a) - 1]?.startsWith('--')) ?? '../test-models/spiral egg - A.stl');
const url = flag('--url', 'http://localhost:5199/');
const only = flag('--only', 'turntable,still,fourview').split(',');
const out = resolve('test-results', basename(stl, '.stl').replace(/\W+/g, '_'));
mkdirSync(out, { recursive: true });

const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--enable-gpu', '--ignore-gpu-blocklist', '--use-angle=d3d11'] });
const page = await browser.newPage({ viewport: { width: 1600, height: 950 }, acceptDownloads: true });
const logs = [];
page.on('console', (m) => { if (m.type() === 'error' || m.type() === 'warning') logs.push(`[${m.type()}] ${m.text().slice(0, 300)}`); });
page.on('pageerror', (e) => logs.push(`[pageerror] ${e.message}`));
await page.goto(url);
await page.setInputFiles('input[type=file]', stl);
await page.waitForFunction(() => document.querySelector('.status')?.textContent?.includes('triangles'), null, { timeout: 120000 });
console.log(await page.textContent('.status'));
await page.waitForTimeout(300);
await page.screenshot({ path: `${out}/app.png` });

async function saveResults(prefix) {
  const links = await page.$$eval('a[download]', (as) => as.map((a) => ({ href: a.href, name: a.download })));
  for (const l of links) {
    const bytes = await page.evaluate(async (h) => { const b = await (await fetch(h)).blob(); return await new Promise((r) => { const fr = new FileReader(); fr.onload = () => r(String(fr.result).split(',')[1]); fr.readAsDataURL(b); }); }, l.href);
    const file = `${out}/${prefix}-${l.name}`;
    writeFileSync(file, Buffer.from(bytes, 'base64'));
    console.log('  saved', file, Buffer.from(bytes, 'base64').length, 'bytes');
  }
  console.log('  ', (await page.textContent('.summary'))?.trim());
}

async function waitDone(timeout = 600000) {
  await page.waitForFunction(() => !document.querySelector('[role=progressbar]') && document.querySelector('a[download]'), null, { timeout });
}

if (only.includes('turntable')) {
  await page.click('role=tab[name="Turntable"]');
  await page.selectOption('#gsize', flag('--size', '960'));
  const videoOptions = await page.$$eval('#gvideo option', (os) => os.map((o) => o.value));
  console.log('video formats:', videoOptions.join(', '));
  if (videoOptions.includes('mp4-h264')) await page.selectOption('#gvideo', 'mp4-h264');
  const t = Date.now();
  await page.click('text=Render turntable');
  await waitDone();
  console.log('turntable', Date.now() - t, 'ms');
  await saveResults('turntable');
}
if (only.includes('still')) {
  await page.click('role=tab[name="Still"]');
  await page.selectOption('#ssize', '4096x4096');
  const t = Date.now();
  await page.click('text=Render still');
  await waitDone();
  console.log('still', Date.now() - t, 'ms');
  await saveResults('still');
}
if (only.includes('fourview')) {
  await page.click('role=tab[name="Four-view"]');
  const t = Date.now();
  await page.click('text=Render four-view sheet');
  await waitDone();
  console.log('fourview', Date.now() - t, 'ms');
  await saveResults('fourview');
}
await page.screenshot({ path: `${out}/app-final.png` });
console.log(logs.filter((l) => !l.includes('X4122') && !l.includes('404')).join('\n'));
await browser.close();
