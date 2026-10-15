// Path-traced vs Studio close-up of layer ribs (Shaded mode).
import { chromium } from 'playwright';
import { writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
const stl = resolve(process.argv[2] ?? '../test-models/WandRing.stl');
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--enable-gpu', '--ignore-gpu-blocklist', '--use-angle=d3d11'] });
const page = await browser.newPage({ viewport: { width: 1600, height: 950 } });
page.on('pageerror', (e) => console.log('[pageerror]', e.message));
page.on('console', (m) => { if (m.type() === 'error') console.log('[console]', m.text().slice(0, 300)); });
await page.goto('http://localhost:5199/');
await page.setInputFiles('input[type=file]', stl);
await page.waitForFunction(() => /triangles/.test(document.querySelector('.status')?.textContent ?? ''));
const canvas = page.locator('canvas[data-testid=viewport]');
const box = await canvas.boundingBox();
await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
for (let i = 0; i < 5; i++) { await page.mouse.wheel(0, -300); await page.waitForTimeout(40); }
await page.waitForTimeout(400);
await page.click('role=tab[name="Still"]');
await page.selectOption('#ssize', '2048x2048');
async function render(engine, out) {
  await page.selectOption('#seng', engine);
  if (engine === 'pathtraced') { await page.selectOption('#ssamples', '64'); if (process.argv.includes('--no-denoise')) await page.uncheck('#seng ~ * input[type=checkbox], label:has-text("Denoise") input'); }
  await page.click('text=Render still');
  await page.waitForFunction(() => !document.querySelector('[role=progressbar]') && (document.querySelector('a[download]') || document.querySelector('.status.error')), null, { timeout: 600000, polling: 1000 });
  console.log(engine, '|', await page.textContent('.status'), '|', await page.textContent('.summary'));
  const href = await page.getAttribute('a[download]', 'href');
  const b64 = await page.evaluate(async (h) => { const b = await (await fetch(h)).blob(); return await new Promise((r) => { const fr = new FileReader(); fr.onload = () => r(String(fr.result).split(',')[1]); fr.readAsDataURL(b); }); }, href);
  writeFileSync(out, Buffer.from(b64, 'base64'));
}
await page.waitForFunction(() => !document.querySelector('#seng option[value=pathtraced]')?.hasAttribute('disabled'));
if (process.argv[3] === 'off') await page.evaluate(() => { window.__turninator.settings.layers.enabled = false; });
else if (process.argv[3]) await page.evaluate((d) => { window.__turninator.settings.layers.depthMm = Number(d); }, process.argv[3]);
await render('studio', 'test-results/ribs-studio.png');
await render('pathtraced', 'test-results/ribs-pt.png');
await browser.close();
