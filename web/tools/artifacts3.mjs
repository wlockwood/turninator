import { chromium } from 'playwright';
import { resolve } from 'node:path';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--enable-gpu', '--ignore-gpu-blocklist', '--use-angle=d3d11'] });
const page = await browser.newPage({ viewport: { width: 1430, height: 910 } });
page.on('pageerror', (e) => console.log('[pageerror]', e.message));
await page.goto('http://localhost:5199/');
await page.click('text=Dark');
await page.click('button.swatch[title="Translucent PETG"]');
await page.setInputFiles('input[type=file][accept^=".stl"]', resolve('test-results/fixtures/trays.stl'));
await page.waitForFunction(() => /triangles/.test(document.querySelector('.status')?.textContent ?? ''));
await page.getByRole('button', { name: 'Iso', exact: true }).click();
await page.evaluate(() => { const s = window.__turninator.studio; const p = s.camera.position, t = s.controls.target; s.applyPerspective([p.x * 0.75 - 30, p.y * 0.75, p.z * 0.75], [t.x - 20, t.y - 15, t.z]); });
const canvas = page.locator('canvas[data-testid=viewport]');
for (const [tag, fn] of [['ao', () => {}], ['noao', () => { window.__turninator.settings.studio.ao = false; }], ['noshadow', () => { window.__turninator.settings.studio.shadows = false; }]]) {
  await page.evaluate(fn);
  await page.waitForTimeout(500);
  await canvas.screenshot({ path: `test-results/art3-${tag}.png` });
}
await browser.close();
