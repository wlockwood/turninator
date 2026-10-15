// Sliced-mode visual check. Usage: node tools/sliced.mjs [stl] [wheelSteps] [tag]
import { chromium } from 'playwright';
import { resolve } from 'node:path';
const stl = resolve(process.argv[2] ?? '../test-models/spiral egg - A.stl');
const steps = Number(process.argv[3] ?? 6);
const tag = process.argv[4] ?? 'sliced';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--enable-gpu', '--ignore-gpu-blocklist', '--use-angle=d3d11'] });
const page = await browser.newPage({ viewport: { width: 1600, height: 950 } });
const logs = [];
page.on('console', (m) => { if (m.type() === 'error') logs.push(m.text().slice(0, 300)); });
page.on('pageerror', (e) => logs.push(`[pageerror] ${e.message}`));
await page.goto('http://localhost:5199/');
await page.setInputFiles('input[type=file]', stl);
await page.waitForFunction(() => document.querySelector('.status')?.textContent?.includes('triangles'));
await page.click('role=radio[name="Sliced"]');
const t = Date.now();
await page.waitForFunction(() => [...document.querySelectorAll('p')].some((p) => /layers ·/.test(p.textContent ?? '')), null, { timeout: 120000 });
console.log('sliced in', Date.now() - t, 'ms:', await page.locator('p', { hasText: 'layers ·' }).first().textContent());
await page.click('text=Iso');
const canvas = page.locator('canvas[data-testid=viewport]');
await page.waitForTimeout(400);
await canvas.screenshot({ path: `test-results/${tag}-full.png` });
const box = await canvas.boundingBox();
await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
for (let i = 0; i < steps; i++) { await page.mouse.wheel(0, -300); await page.waitForTimeout(50); }
await page.waitForTimeout(600);
await canvas.screenshot({ path: `test-results/${tag}-close.png` });
console.log(logs.join('\n') || 'no errors');
await browser.close();
