import { chromium } from 'playwright';
import { resolve } from 'node:path';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--enable-gpu', '--ignore-gpu-blocklist', '--use-angle=d3d11'] });
const page = await browser.newPage({ viewport: { width: 1600, height: 950 } });
page.on('pageerror', (e) => console.log('[pageerror]', e.message));
page.on('console', (m) => { if (m.type() === 'error') console.log('[console]', m.text().slice(0, 300)); });
await page.goto('http://localhost:5199/');
await page.setInputFiles('input[type=file]', resolve('../test-models/spiral egg - A.stl'));
await page.waitForFunction(() => /triangles/.test(document.querySelector('.status')?.textContent ?? ''));
const canvas = page.locator('canvas[data-testid=viewport]');
async function closeup(tag) {
  await page.getByRole('button', { name: 'Fit', exact: true }).click();
  await page.waitForTimeout(300);
  const box = await canvas.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height * 0.3);
  for (let i = 0; i < 7; i++) { await page.mouse.wheel(0, -300); await page.waitForTimeout(40); }
  await page.waitForTimeout(600);
  await canvas.screenshot({ path: `test-results/${tag}.png` });
}
await page.selectOption('#lprofile', 'adaptive');
await page.click('role=radio[name="Sliced"]');
await page.waitForFunction(() => [...document.querySelectorAll('p')].some((p) => /layers ·/.test(p.textContent ?? '')), null, { timeout: 120000 });
await page.waitForTimeout(500);
console.log('adaptive:', await page.locator('p', { hasText: /\d+ layers, / }).first().textContent(), '|', await page.locator('p', { hasText: 'layers ·' }).first().textContent());
await closeup('adaptive-sliced');
await page.click('role=radio[name="Shaded"]');
await page.waitForTimeout(800);
await closeup('adaptive-shaded');
await page.selectOption('#lprofile', 'gcode');
await page.setInputFiles('#lprofile ~ * input[type=file], input[accept*="bgcode"]:not([accept*="stl"])', resolve('test-results/gcode/spiral_egg_-_A.gcode'));
await page.waitForFunction(() => [...document.querySelectorAll('p')].some((p) => /^232 layers, /.test(p.textContent ?? '')), null, { timeout: 60000 });
console.log('gcode:', await page.locator('p', { hasText: /\d+ layers, / }).first().textContent());
await browser.close();
