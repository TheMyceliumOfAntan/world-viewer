// CDP performance probe for World Viewer's tile pipeline.
//
// Connects to the WebView2 instance over CDP (the app must be started with
// WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222), opens a
// world, and times the things the caches exist for.
//
// The frontend bitmap cache is cleared before every timed step so it cannot
// hide the server-side behaviour under test. Timing is wall-clock in the page,
// which is what the user feels: from "issue the view change" to "every tile
// request has settled".
//
// Scenarios
//   cold_sweep   visit N distinct areas, each never seen before -> disk read +
//                decompress + parse + render. Exercises the region header/file
//                cache (324 chunk opens per z=0 tile without it).
//   repeat_sweep the same N areas again -> tiles whose chunks are still in the
//                server chunk cache skip the disk entirely.
//   hot_recall   return to one hot area after enough cold excursions to
//                overflow the 4096-chunk cache. Random eviction drops hot
//                entries with the same probability as cold ones; LRU keeps
//                them.
//   pan_steady   continuous pan that stays inside generated terrain, per-step
//                timing -> steady-state cost and spikiness.
//
// Run: node scripts/measure-tiles.mjs <saveDir> <label> [outJson]
import { chromium } from "playwright";
import { writeFileSync } from "node:fs";

const saveDir = process.argv[2];
const label = process.argv[3] ?? "run";
const outJson = process.argv[4];

const CDP = "http://localhost:9222";

// The Aegis test world spans chunk -64..63, i.e. blocks -1024..1024. A z=0 tile
// is 256 blocks, so the generated area is 8x8 tiles. Every coordinate below is
// chosen to land inside it.
//
// 3x3 grid of tile-aligned areas, 2 tiles apart (512 blocks), covering the
// middle of the world. Each viewport shows ~15 tiles, so the 9 areas overlap
// heavily — that is deliberate: it makes the chunk cache do real work.
const GRID = [-512, 0, 512];
const AREAS = [];
for (const z of GRID) for (const x of GRID) AREAS.push([x, z]);

// Cold excursions used to overflow the cache before the hot recall. Each is a
// distinct, distant part of the world so it cannot share chunks with the hot
// area.
const EXCURSIONS = [
  [-768, -768],
  [-768, 768],
  [768, -768],
  [768, 768],
  [-768, 0],
  [768, 0],
];

async function main() {
  const browser = await chromium.connectOverCDP(CDP);
  const ctx = browser.contexts()[0];
  const page = ctx.pages()[0] ?? (await ctx.waitForEvent("page"));

  const url = `http://tauri.localhost/?save=${encodeURIComponent(saveDir)}`;
  await page.goto(url, { waitUntil: "domcontentloaded" });

  await page.waitForFunction(
    () => {
      const el = document.querySelector(".topbar .path span");
      return el && !el.textContent.includes("未加载");
    },
    { timeout: 180000 }
  );
  await page.waitForFunction(() => !!window.__tileLayer, { timeout: 30000 });

  // Leaflet keeps a back-reference to the map on every layer, so the map can
  // be driven without adding test-only globals to App.tsx.
  await page.evaluate(() => {
    window.__map = window.__tileLayer._map;
  });

  const idle = (ms = 180000) =>
    page.waitForFunction(
      () => {
        const s = window.__tileLayer.stats();
        return s.waiting === 0 && s.queued === 0 && s.active === 0;
      },
      { timeout: ms }
    );

  // Clear the frontend bitmap cache so the next redraw must ask the server.
  const clearFrontend = () =>
    page.evaluate(() => window.__tileLayer.clearCache());

  const goTo = (x, z, zoom) =>
    page.evaluate(
      ([x, z, zoom]) => {
        window.__map.setView([-z, x], zoom, { animate: false });
        window.__tileLayer.redraw();
      },
      [x, z, zoom]
    );

  // One timed step: clear the frontend cache, move, wait for all tiles.
  const step = async (name, x, z, zoom = 0) => {
    const t0 = Date.now();
    await clearFrontend();
    await goTo(x, z, zoom);
    await idle();
    return { name, ms: Date.now() - t0 };
  };

  const sum = (steps) => steps.reduce((a, s) => a + s.ms, 0);
  const median = (steps) => {
    const v = steps.map((s) => s.ms).sort((a, b) => a - b);
    return v[Math.floor(v.length / 2)];
  };

  const results = [];

  // Warm-up: JIT, first PNG decode, window layout. Not reported.
  await step("warmup", 0, 0, 0);

  // --- 1. cold sweep: every area is a fresh disk read ---
  const coldSteps = [];
  for (const [x, z] of AREAS) coldSteps.push(await step(`area_${x}_${z}`, x, z));
  results.push({ name: "cold_sweep", ms: sum(coldSteps), median: median(coldSteps), steps: coldSteps });

  // --- 2. repeat sweep: server chunk cache should absorb this ---
  const warmSteps = [];
  for (const [x, z] of AREAS) warmSteps.push(await step(`area_${x}_${z}`, x, z));
  results.push({ name: "repeat_sweep", ms: sum(warmSteps), median: median(warmSteps), steps: warmSteps });

  // --- 3. hot recall after cache pressure ---
  // Baseline for the hot area, taken while it is still resident.
  const hotBefore = await step("hot_before", 0, 0);
  // Overflow the 4096-chunk cache with terrain the hot area does not share.
  const excursionSteps = [];
  for (const [x, z] of EXCURSIONS) excursionSteps.push(await step(`exc_${x}_${z}`, x, z));
  // Same hot area again. Cold if its chunks were evicted, warm if retained.
  const hotAfter = await step("hot_after", 0, 0);
  results.push({
    name: "hot_recall",
    ms: hotAfter.ms,
    hotBefore,
    hotAfter,
    excursionSteps,
    retainedRatio: hotAfter.ms > 0 ? hotBefore.ms / hotAfter.ms : null,
  });

  // --- 4. continuous pan inside the world, per-step timing ---
  const panSteps = [];
  const tPan = Date.now();
  await step("pan_start", -768, 0);
  for (let i = 0; i < 10; i++) {
    panSteps.push(await step(`pan_${i}`, -768 + i * 128, 0));
  }
  results.push({ name: "pan_steady", ms: Date.now() - tPan, median: median(panSteps), steps: panSteps });

  const report = { label, saveDir, results };
  console.log(JSON.stringify(report, null, 2));
  if (outJson) writeFileSync(outJson, JSON.stringify(report, null, 2));
  await browser.close();
}

main().catch((e) => {
  console.error("FAILED:", e);
  process.exit(1);
});
