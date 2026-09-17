// Same measurements as measure-tiles.mjs, but against a dev-mode app: it
// reuses the already-open page instead of navigating to tauri.localhost, and
// takes the CDP port as an argument (9222 is occupied by an unrelated app on
// this machine).
// Run: node scripts/measure-tiles-dev.mjs <cdpPort> <label> [outJson]
import { chromium } from "playwright";
import { writeFileSync } from "node:fs";

const cdpPort = process.argv[2] ?? "9223";
const label = process.argv[3] ?? "run";
const outJson = process.argv[4];

const CDP = `http://localhost:${cdpPort}`;

// The Aegis test world spans chunk -64..63, i.e. blocks -1024..1024. A z=0 tile
// is 256 blocks, so the generated area is 8x8 tiles.
const GRID = [-512, 0, 512];
const AREAS = [];
for (const z of GRID) for (const x of GRID) AREAS.push([x, z]);

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
  const page = ctx.pages().find((p) => p.url().includes("1420")) ?? ctx.pages()[0];

  // The app may still be on its start screen; wait for the map layer to exist.
  await page.waitForFunction(() => !!window.__tileLayer, { timeout: 120000 });
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

  const clearFrontend = () => page.evaluate(() => window.__tileLayer.clearCache());

  const goTo = (x, z, zoom) =>
    page.evaluate(
      ([x, z, zoom]) => {
        window.__map.setView([-z, x], zoom, { animate: false });
        window.__tileLayer.redraw();
      },
      [x, z, zoom]
    );

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

  await step("warmup", 0, 0, 0);

  const coldSteps = [];
  for (const [x, z] of AREAS) coldSteps.push(await step(`area_${x}_${z}`, x, z));
  results.push({ name: "cold_sweep", ms: sum(coldSteps), median: median(coldSteps), steps: coldSteps });

  const warmSteps = [];
  for (const [x, z] of AREAS) warmSteps.push(await step(`area_${x}_${z}`, x, z));
  results.push({ name: "repeat_sweep", ms: sum(warmSteps), median: median(warmSteps), steps: warmSteps });

  const hotBefore = await step("hot_before", 0, 0);
  const excursionSteps = [];
  for (const [x, z] of EXCURSIONS) excursionSteps.push(await step(`exc_${x}_${z}`, x, z));
  const hotAfter = await step("hot_after", 0, 0);
  results.push({
    name: "hot_recall",
    ms: hotAfter.ms,
    hotBefore,
    hotAfter,
    excursionSteps,
    retainedRatio: hotAfter.ms > 0 ? hotBefore.ms / hotAfter.ms : null,
  });

  const panSteps = [];
  const tPan = Date.now();
  await step("pan_start", -768, 0);
  for (let i = 0; i < 10; i++) {
    panSteps.push(await step(`pan_${i}`, -768 + i * 128, 0));
  }
  results.push({ name: "pan_steady", ms: Date.now() - tPan, median: median(panSteps), steps: panSteps });

  const report = { label, results };
  console.log(JSON.stringify(report, null, 2));
  if (outJson) writeFileSync(outJson, JSON.stringify(report, null, 2));
  await browser.close();
}

main().catch((e) => {
  console.error("FAILED:", e);
  process.exit(1);
});
