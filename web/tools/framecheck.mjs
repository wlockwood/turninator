// Verify the viewport export frame matches rendered output.
import { chromium } from 'playwright';
import { writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--enable-gpu', '--ignore-gpu-blocklist', '--use-angle=d3d11'] });
const page = await browser.newPage({ viewport: { width: 1500, height: 900 } });
page.on('pageerror', (e) => console.log('[pageerror]', e.message));
await page.goto('http://localhost:5199/');
await page.setInputFiles('input[type=file][accept^=".stl"]', resolve('../test-models/spiral egg - A.stl'));
await page.waitForFunction(() => /triangles/.test(document.querySelector('.status')?.textContent ?? ''));
const save = async (out) => {
  await page.waitForFunction(() => !document.querySelector('[role=progressbar]') && document.querySelector('a[download]'), null, { timeout: 300000 });
  const href = await page.getAttribute('a[download]', 'href');
  const b64 = await page.evaluate(async (h) => { const b = await (await fetch(h)).blob(); return await new Promise((r) => { const fr = new FileReader(); fr.onload = () => r(String(fr.result).split(',')[1]); fr.readAsDataURL(b); }); }, href);
  writeFileSync(out, Buffer.from(b64, 'base64'));
};
const frameShot = async (out) => {
  await page.waitForTimeout(400);
  const f = await page.locator('.frame').boundingBox();
  await page.screenshot({ path: out, clip: f });
  console.log('frame', JSON.stringify(f));
};
// Turntable (square), slightly zoomed so framing matters.
await page.selectOption('#gsize', '480');
await page.selectOption('#gdur', '3');
await frameShot('test-results/frame-tt-viewport.png');
await page.click('text=Render turntable');
await save('test-results/frame-tt.gif');
// Still 16:9
await page.click('role=tab[name="Still"]');
await page.selectOption('#ssize', '1920x1080');
await frameShot('test-results/frame-still-viewport.png');
await page.click('text=Render still');
await save('test-results/frame-still.png');
await browser.close();
