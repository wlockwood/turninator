// Path-traced G-code still. Usage: node tools/pt-gcode.mjs [gcode] [preset] [progress] [samples] [colorMode]
import { chromium } from 'playwright';
import { writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
const file = resolve(process.argv[2] ?? 'test-results/gcode/spiral_egg_-_A.gcode');
const preset = process.argv[3] ?? 'Clear resin';
const progress = process.argv[4] ?? '0.6';
const samples = process.argv[5] ?? '64';
const colorMode = process.argv[6] ?? 'material';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--enable-gpu', '--ignore-gpu-blocklist', '--use-angle=d3d11'] });
const page = await browser.newPage({ viewport: { width: 1600, height: 950 } });
const logs = [];
page.on('console', (m) => { if (m.type() !== 'debug') console.log('[console]', m.type(), m.text().slice(0, 300)); });
page.on('pageerror', (e) => logs.push(`[pageerror] ${e.message}`));
await page.goto('http://localhost:5199/');
await page.setInputFiles('input[type=file]', file);
await page.waitForFunction(() => /beads|triangles|Could not/.test(document.querySelector('.status')?.textContent ?? ''), null, { timeout: 120000 });
await page.click(`button.swatch[title="${preset}"]`);
if (await page.$('#gcolor')) await page.selectOption('#gcolor', colorMode);
await page.locator('input[aria-label="Build progress"]').fill(progress);
await page.click('role=tab[name="Still"]');
await page.waitForFunction(() => !document.querySelector('#seng option[value=pathtraced]')?.hasAttribute('disabled'));
await page.selectOption('#ssize', '2048x2048');
await page.selectOption('#seng', 'pathtraced');
await page.selectOption('#ssamples', samples);
const t = Date.now();
await page.click('text=Render still');
const ticker = setInterval(async () => {
  const st = await page.textContent('.status').catch(() => '');
  const pr = await page.locator('[role=progressbar] + p').textContent().catch(() => '');
  console.log(`[${((Date.now() - t) / 1000).toFixed(0)} s] ${st} | ${pr}`);
}, 10000);
await page.waitForFunction(() => !document.querySelector('[role=progressbar]') && (document.querySelector('a[download]') || document.querySelector('.status.error')), null, { timeout: 1200000, polling: 1000 });
clearInterval(ticker);
console.log('done in', Date.now() - t, 'ms |', await page.textContent('.status'), '|', await page.textContent('.summary').catch(() => ''));
const href = await page.getAttribute('a[download]', 'href').catch(() => null);
if (href) {
  const b64 = await page.evaluate(async (h) => { const b = await (await fetch(h)).blob(); return await new Promise((r) => { const fr = new FileReader(); fr.onload = () => r(String(fr.result).split(',')[1]); fr.readAsDataURL(b); }); }, href);
  writeFileSync(`test-results/pt-gcode-${preset.replace(/\W+/g, '-')}-${progress}.png`, Buffer.from(b64, 'base64'));
}
console.log(logs.slice(0, 10).join('\n') || 'no errors');
await browser.close();
