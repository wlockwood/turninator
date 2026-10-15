// Measures UI-thread responsiveness while the Raytrace preview runs.
import { chromium } from 'playwright';
import { resolve } from 'node:path';
const file = resolve(process.argv[2] ?? '../test-models/spiral egg - A.stl');
const url = process.argv[3] ?? 'http://localhost:5199/';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--enable-gpu', '--ignore-gpu-blocklist', '--use-angle=d3d11'] });
const page = await browser.newPage({ viewport: { width: 1500, height: 900 } });
page.on('pageerror', (e) => console.log('[pageerror]', e.message));
page.on('console', (m) => { if (m.type() === 'error') console.log('[console]', m.text().slice(0, 300)); });
await page.goto(url);
await page.setInputFiles('input[type=file][accept^=".stl"]', file);
await page.waitForFunction(() => /triangles|beads/.test(document.querySelector('.status')?.textContent ?? ''), null, { timeout: 120000 });
await page.waitForFunction(() => !document.querySelector('button[title^="Progressive"]')?.hasAttribute('disabled'));
await page.evaluate(() => {
  const w = window;
  w.__gaps = []; w.__long = [];
  let last = performance.now();
  const tick = (t) => { w.__gaps.push(t - last); last = t; requestAnimationFrame(tick); };
  requestAnimationFrame(tick);
  new PerformanceObserver((l) => { for (const e of l.getEntries()) w.__long.push(e.duration); }).observe({ entryTypes: ['longtask'] });
});
const t0 = Date.now();
await page.click('button[title^="Progressive"]');
await page.waitForFunction(() => { const m = /(\d+) spp/.exec(document.querySelector('.ptbadge')?.textContent ?? ''); return (m && Number(m[1]) >= 32) || /error/i.test(document.querySelector('.ptbadge')?.textContent ?? ''); }, null, { timeout: 300000, polling: 250 });
console.log('32 spp after', Date.now() - t0, 'ms |', await page.textContent('.ptbadge'));
// Orbit while tracing.
const box = await page.locator('.stage').boundingBox();
await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
await page.mouse.down();
for (let i = 0; i < 20; i++) { await page.mouse.move(box.x + box.width / 2 + i * 10, box.y + box.height / 2, { steps: 2 }); }
await page.mouse.up();
await page.waitForTimeout(2500);
const r = await page.evaluate(() => {
  const g = window.__gaps.slice(5);
  g.sort((a, b) => b - a);
  return { frames: g.length, worstFrameMs: g[0]?.toFixed(0), p95: g[Math.floor(g.length * 0.05)]?.toFixed(0), longTasks: window.__long.length, longestTaskMs: Math.max(0, ...window.__long).toFixed(0) };
});
console.log('UI thread:', JSON.stringify(r), '| badge:', await page.textContent('.ptbadge'));
await page.locator('.stage').screenshot({ path: 'test-results/ptresponsive.png' });
await browser.close();
