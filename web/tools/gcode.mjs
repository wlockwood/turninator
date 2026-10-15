// G-code preview check. Usage: node tools/gcode.mjs [gcode] [tag] [--export]
import { chromium } from 'playwright';
import { writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
const file = resolve(process.argv[2] ?? 'test-results/gcode/spiral_egg_-_A.gcode');
const tag = process.argv[3] ?? 'gcode';
const doExport = process.argv.includes('--export');
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--enable-gpu', '--ignore-gpu-blocklist', '--use-angle=d3d11'] });
const page = await browser.newPage({ viewport: { width: 1600, height: 950 } });
const logs = [];
page.on('console', (m) => { if (m.type() === 'error' || m.type() === 'warning') logs.push(`[${m.type()}] ${m.text().slice(0, 400)}`); });
page.on('pageerror', (e) => logs.push(`[pageerror] ${e.message}`));
await page.goto('http://localhost:5199/');
const t = Date.now();
await page.setInputFiles('input[type=file]', file);
await page.waitForFunction(() => /beads|Could not/.test(document.querySelector('.status')?.textContent ?? ''), null, { timeout: 120000 });
console.log('loaded in', Date.now() - t, 'ms:', await page.textContent('.status'));
const canvas = page.locator('canvas[data-testid=viewport]');
await page.waitForTimeout(500);
await page.screenshot({ path: `test-results/${tag}-app.png` });
const box = await canvas.boundingBox();
await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
for (let i = 0; i < 8; i++) { await page.mouse.wheel(0, -300); await page.waitForTimeout(40); }
await page.waitForTimeout(500);
await canvas.screenshot({ path: `test-results/${tag}-close.png` });
await page.click('text=Fit');
await page.selectOption('#gcolor', 'feature');
await page.locator('input[aria-label="Build progress"]').fill('0.55');
await page.waitForTimeout(500);
await canvas.screenshot({ path: `test-results/${tag}-features-55.png` });
await page.selectOption('#gcolor', 'material');
await page.locator('input[aria-label="Build progress"]').fill('1');
if (doExport) {
  await page.selectOption('#gsize', '480');
  await page.selectOption('#gmotion', 'build-rotate');
  const t2 = Date.now();
  await page.click('text=Render turntable');
  await page.waitForFunction(() => !document.querySelector('[role=progressbar]') && (document.querySelector('a[download]') || document.querySelector('.status.error')), null, { timeout: 600000 });
  console.log('build-up GIF in', Date.now() - t2, 'ms |', await page.textContent('.status'));
  const href = await page.getAttribute('a[download]', 'href');
  const b64 = await page.evaluate(async (h) => { const b = await (await fetch(h)).blob(); return await new Promise((r) => { const fr = new FileReader(); fr.onload = () => r(String(fr.result).split(',')[1]); fr.readAsDataURL(b); }); }, href);
  writeFileSync(`test-results/${tag}-buildup.gif`, Buffer.from(b64, 'base64'));
}
console.log(logs.filter((l) => !l.includes('404') && !l.includes('X4122')).slice(0, 15).join('\n') || 'no errors');
await browser.close();
