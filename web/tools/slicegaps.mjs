// Sliced-mode gap check with a translucent material on a dark background.
import { chromium } from 'playwright';
import { resolve } from 'node:path';
const stl = resolve(process.argv[2] ?? '../test-models/Technics_work_support.stl');
const tag = process.argv[3] ?? 'gaps';
const preset = process.argv[4] ?? 'Clear resin';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--enable-gpu', '--ignore-gpu-blocklist', '--use-angle=d3d11'] });
const page = await browser.newPage({ viewport: { width: 1430, height: 910 } });
page.on('pageerror', (e) => console.log('[pageerror]', e.message));
await page.goto('http://localhost:5199/');
await page.click('text=Dark');
await page.setInputFiles('input[type=file][accept^=".stl"]', stl);
await page.waitForFunction(() => /triangles/.test(document.querySelector('.status')?.textContent ?? ''));
await page.click(`button.swatch[title="${preset}"]`);
await page.evaluate(() => { const l = window.__turninator.settings.layers; l.enabled = true; l.profile = 'uniform'; l.heightMm = 0.2; });
await page.click('role=radio[name="Sliced"]');
await page.waitForFunction(() => [...document.querySelectorAll('p')].some((p) => /layers ·/.test(p.textContent ?? '')), null, { timeout: 120000 });
console.log(await page.locator('p', { hasText: 'layers ·' }).first().textContent());
await page.getByRole('button', { name: 'Iso', exact: true }).click();
const canvas = page.locator('canvas[data-testid=viewport]');
const box = await canvas.boundingBox();
await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
for (let i = 0; i < 3; i++) { await page.mouse.wheel(0, -300); await page.waitForTimeout(40); }
await page.waitForTimeout(800);
await canvas.screenshot({ path: `test-results/${tag}.png` });
await browser.close();
