import { chromium } from 'playwright';
import { resolve } from 'node:path';
const models = ['Deck Post Jig', 'Technics_work_support', 'Single Sword Mount', 'WandHandHexy', 'Amcrest IP5m Camera Mount'];
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--enable-gpu', '--ignore-gpu-blocklist', '--use-angle=d3d11'] });
const page = await browser.newPage({ viewport: { width: 1430, height: 910 } });
page.on('pageerror', (e) => console.log('[pageerror]', e.message));
page.on('console', (m) => { if (m.type() === 'error' || m.type() === 'warning') console.log('[console]', m.text().slice(0, 200)); });
await page.goto('http://localhost:5199/');
await page.click('text=Dark');
await page.click('button.swatch[title="Translucent PETG"]');
for (const m of models) {
  await page.setInputFiles('input[type=file][accept^=".stl"]', resolve(`../test-models/${m}.stl`));
  await page.waitForFunction((n) => (document.querySelector('.status')?.textContent ?? '').startsWith(n), m);
  await page.getByRole('button', { name: 'Iso', exact: true }).click();
  const canvas = page.locator('canvas[data-testid=viewport]');
  const box = await canvas.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  for (let i = 0; i < 3; i++) { await page.mouse.wheel(0, -300); await page.waitForTimeout(40); }
  await page.waitForTimeout(700);
  await canvas.screenshot({ path: `test-results/art2-${m.replace(/\W+/g, '_')}.png` });
}
await browser.close();
