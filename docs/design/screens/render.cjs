// Renders docs/design/screens/mock.html to PNGs at 2x.
// Usage: node docs/design/screens/render.cjs <out-dir>   (from the repo root, after pnpm install)
// Set CHROMIUM to a Chromium binary if Playwright's own browser is not installed.
const { chromium } = require('@playwright/test');
const path = require('node:path');
(async () => {
  const out = path.resolve(process.argv[2] || '.');
  const mock = 'file://' + path.resolve(__dirname, 'mock.html');
  const b = await chromium.launch(process.env.CHROMIUM ? { executablePath: process.env.CHROMIUM } : {});
  const p = await b.newPage({ viewport: { width: 1400, height: 900 }, deviceScaleFactor: 2 });
  p.on('pageerror', e => console.log('ERR', e.message));
  for (const v of ['library-1280', 'intake-1280', 'mutant-1280', 'review-1280', 'stick-1280', 'loading-1280', 'tray-1280', 'plan-1280', 'library-640', 'intake-640', 'waves-1280']) {
    await p.goto(mock + '#' + v); await p.reload(); await p.waitForTimeout(700);
    await p.locator('#stage').screenshot({ path: path.join(out, v + '.png') });
  }
  await b.close();
})();
