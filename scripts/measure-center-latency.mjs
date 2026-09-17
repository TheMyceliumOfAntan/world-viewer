// Measures what "load the tile nearest the viewport centre first" actually
// targets: how long until the tile under the centre of the screen is decoded
// and drawable, after a view change onto unseen terrain.
//
// Total-time A/B is the wrong metric for this change — reordering requests
// does not reduce total work, only the order results arrive in. This probe
// clears the frontend cache, moves the view, and polls until the centre tile's
// key is present in the layer's bitmap cache.
//
// Run: node scripts/measure-center-latency.mjs <cdpPort> <label> [outJson]
import { chromium } from "playwright";
import { writeFileSync } from "node:fs";

const cdpPort = process.argv[2] ?? "9223";
const label = process.argv[3] ?? "run";
const outJson = process.argv[4];

const CDP = `http://localhost:${cdpPort}`;

// Same areas as measure-tiles-dev.mjs, so the terrain is comparable.
const AREAS = [];
for (const z of [-512, 0, 512]) for (const x of [-512, 0, 512]) AREAS.push([x, z]);

async function main() {
  const browser = await chromium.connectOverCDP(CDP);
  const ctx = browser.contexts()[0];
  const page = ctx.pages().find((p) => p.url().includes("1420")) ?? ctx.pages()[0];

  await page.waitForFunction(() => !!window.__tileLayer, { timeout: 120000 });

  // Polls inside the page so the sampling interval is not limited by CDP
  // round-trips. Resolves with the ms until the centre tile is in the cache.
  const measureCenter = (x, z, zoom) =>
    page.evaluate(
      ([x, z, zoom]) =>
        new Promise((resolve) => {
          const layer = window.__tileLayer;
          const map = layer._map;
          layer.clearCache();
          map.setView([-z, x], zoom, { animate: false });
          layer.redraw();

          const t0 = performance.now();
          const tz = layer._tileZoom;
          const size = layer.getTileSize();
          const p = map
            .project(map.getCenter(), tz)
            .divideBy(size.x)
            .floor();
          const template = layer.getUrlTemplate();
          const key = template
            .replace("{z}", String(tz))
            .replace("{x}", String(p.x))
            .replace("{y}", String(p.y));

          let ticks = 0;
          const tick = () => {
            ticks++;
            if (layer.cache.has(key)) {
              resolve({ ms: performance.now() - t0, ticks });
              return;
            }
            if (ticks > 2000) {
              resolve({ ms: -1, ticks, note: "timeout" });
              return;
            }
            requestAnimationFrame(tick);
          };
          requestAnimationFrame(tick);
        }),
      [x, z, zoom]
    );

  // Warm-up (JIT, first decode), not reported.
  await measureCenter(0, 0, 0);

  const samples = [];
  for (const [x, z] of AREAS) {
    samples.push({ x, z, ...(await measureCenter(x, z, 0)) });
  }

  const good = samples.filter((s) => s.ms >= 0).map((s) => s.ms).sort((a, b) => a - b);
  const median = good[Math.floor(good.length / 2)];
  const mean = good.reduce((a, b) => a + b, 0) / good.length;
  const report = {
    label,
    samples,
    median,
    mean: Math.round(mean),
    p90: good[Math.floor(good.length * 0.9)],
    count: good.length,
  };
  console.log(JSON.stringify(report, null, 2));
  if (outJson) writeFileSync(outJson, JSON.stringify(report, null, 2));
  await browser.close();
}

main().catch((e) => {
  console.error("FAILED:", e);
  process.exit(1);
});
