// Close-up capture to inspect layer shading. Usage: node tools/closeup.mjs [stl] [wheelSteps]
import { chromium } from 'playwright';
import { resolve } from 'node:path';
const stl = resolve(process.argv[2] ?? '../test-models/spiral egg - A.stl');
const steps = Number(process.argv[3] ?? 8);
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--enable-gpu', '--ignore-gpu-blocklist', '--use-angle=d3d11'] });
const page = await browser.newPage({ viewport: { width: 1600, height: 950 } });
await page.goto('http://localhost:5199/');
await page.setInputFiles('input[type=file]', stl);
await page.waitForFunction(() => document.querySelector('.status')?.textContent?.includes('triangles'));
const box = await page.locator('canvas[data-testid=viewport]').boundingBox();
await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
for (let i = 0; i < steps; i++) { await page.mouse.wheel(0, -300); await page.waitForTimeout(60); }
await page.waitForTimeout(800);
await page.locator('canvas[data-testid=viewport]').screenshot({ path: 'test-results/closeup.png' });
await browser.close();
