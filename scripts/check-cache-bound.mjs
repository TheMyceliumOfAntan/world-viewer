// Verifies the frontend bitmap cache stays within its bound under heavy
// panning, i.e. the LRU added to CachedTileLayer actually evicts.
// Run: node scripts/check-cache-bound.mjs <saveDir>
import { chromium } from "playwright";

const saveDir = process.argv[2];

const browser = await chromium.connectOverCDP("http://localhost:9222");
const ctx = browser.contexts()[0];
const page = ctx.pages()[0] ?? (await ctx.waitForEvent("page"));

await page.goto(`http://tauri.localhost/?save=${encodeURIComponent(saveDir)}`, {
  waitUntil: "domcontentloaded",
});
await page.waitForFunction(
  () => {
    const el = document.querySelector(".topbar .path span");
    return el && !el.textContent.includes("未加载");
  },
  { timeout: 180000 }
);
await page.waitForFunction(() => !!window.__tileLayer, { timeout: 30000 });
await page.evaluate(() => {
  window.__map = window.__tileLayer._map;
});

const idle = () =>
  page.waitForFunction(
    () => {
      const s = window.__tileLayer.stats();
      return s.waiting === 0 && s.queued === 0 && s.active === 0;
    },
    { timeout: 180000 }
  );

const bound = await page.evaluate(() => window.__tileLayer.maxCached);

// Sweep across many distinct tiles to force far more entries than the bound.
// z=4 is 16 blocks/tile, so this world (2048 blocks) is 128x128 = 16384 tiles
// there — plenty to overflow the cache.
let peak = 0;
for (let i = 0; i < 16; i++) {
  await page.evaluate(
    ([x, z, zoom]) => {
      window.__map.setView([-z, x], zoom, { animate: false });
      window.__tileLayer.redraw();
    },
    [-896 + i * 128, ((i % 5) - 2) * 128, 4]
  );
  await idle();
  const s = await page.evaluate(() => window.__tileLayer.stats());
  peak = Math.max(peak, s.cached);
}

console.log(JSON.stringify({ bound, peak, withinBound: peak <= bound }, null, 2));
await browser.close();
