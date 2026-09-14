import L from "leaflet";

/** A decoded tile plus whether the server says it has no generated chunks. */
type TileImage = { bitmap: ImageBitmap; isEmpty: boolean };

/**
 * A tile layer that never leaves holes while loading.
 *
 * Plain L.tileLayer requests every visible tile at once and shows blank space
 * until each one arrives, so panning and zooming flash empty squares. This
 * layer keeps an in-memory cache and falls back to the already-loaded parent
 * tile (drawn scaled and cropped to the right quadrant) so the map stays
 * continuous, just blurrier, until the sharp tile arrives.
 *
 * Leaflet may ask for the same tile again while its request is still in
 * flight (it rebuilds tile elements on zoom/pan). Every caller's `done`
 * callback must therefore be retained and fired together, otherwise the
 * tile stays permanently blank.
 */
export class CachedTileLayer extends L.GridLayer {
  private cache = new Map<string, ImageBitmap>();
  /** Tiles the server reported as covering no generated chunks. */
  private empty = new Set<string>();
  /** Pending callbacks per tile key; all fire when the image resolves. */
  private waiting = new Map<string, Array<(img: TileImage | null) => void>>();
  private queue: Array<() => void> = [];
  private active = 0;

  /** Keyed by `dim|ymax` — changing either invalidates every tile. */
  private urlTemplate = "";
  private maxConcurrent: number;
  private failures = new Set<string>();

  constructor(options?: L.GridLayerOptions & { maxConcurrent?: number }) {
    super({ tileSize: 256, ...options });
    this.maxConcurrent = options?.maxConcurrent ?? 6;
  }

  setUrlTemplate(template: string) {
    if (this.urlTemplate === template) return;
    this.urlTemplate = template;
    this.failures.clear();
    this.empty.clear();
    this.redraw();
  }

  getUrlTemplate() {
    return this.urlTemplate;
  }

  /** Drop cached tiles for the previous world/height without touching the map. */
  clearCache() {
    this.cache.clear();
    this.waiting.clear();
    this.failures.clear();
    this.empty.clear();
  }

  /** Diagnostics for tests and the status bar. */
  stats() {
    return {
      cached: this.cache.size,
      waiting: this.waiting.size,
      queued: this.queue.length,
      active: this.active,
      failed: this.failures.size,
      empty: this.empty.size,
    };
  }

  private urlFor(coords: { z: number; x: number; y: number }): string {
    return L.Util.template(this.urlTemplate, {
      z: coords.z,
      x: coords.x,
      y: coords.y,
    });
  }

  private pump() {
    while (this.active < this.maxConcurrent && this.queue.length > 0) {
      const job = this.queue.shift()!;
      this.active++;
      job();
    }
  }

  private enqueue(job: () => void) {
    this.queue.push(job);
    this.pump();
  }

  private done() {
    this.active = Math.max(0, this.active - 1);
    this.pump();
  }

  private resolveWaiters(key: string, img: TileImage | null) {
    const list = this.waiting.get(key);
    if (!list) return;
    this.waiting.delete(key);
    for (const cb of list) cb(img);
  }

  private load(key: string, url: string, onReady: (img: TileImage | null) => void) {
    const cached = this.cache.get(key);
    if (cached) {
      onReady({ bitmap: cached, isEmpty: this.empty.has(key) });
      return;
    }

    // Already requested: subscribe instead of dropping the callback.
    const pending = this.waiting.get(key);
    if (pending) {
      pending.push(onReady);
      return;
    }

    if (this.failures.has(key)) {
      onReady(null);
      return;
    }

    this.waiting.set(key, [onReady]);
    this.enqueue(() => {
      // Use fetch so the X-Tile-Empty header (no generated chunks here) is
      // readable; an <img> would hide it.
      fetch(url, { mode: "cors" })
        .then(async (resp) => {
          if (!resp.ok) throw new Error(String(resp.status));
          const isEmpty = resp.headers.get("X-Tile-Empty") === "1";
          const blob = await resp.blob();
          const bitmap = await createImageBitmap(blob);
          this.cache.set(key, bitmap);
          this.done();
          if (isEmpty) this.empty.add(key);
          else this.empty.delete(key);
          this.resolveWaiters(key, { bitmap, isEmpty });
        })
        .catch(() => {
          this.failures.add(key);
          this.done();
          this.resolveWaiters(key, null);
        });
    });
  }

  /** Draw a faint checkerboard: "this area was never generated". */
  private drawEmptyPattern(ctx: CanvasRenderingContext2D, size: L.Point) {
    const cell = 16;
    for (let y = 0; y < size.y; y += cell) {
      for (let x = 0; x < size.x; x += cell) {
        const odd = ((x / cell) + (y / cell)) % 2 === 0;
        ctx.fillStyle = odd ? "rgba(255,255,255,0.035)" : "rgba(255,255,255,0.07)";
        ctx.fillRect(x, y, cell, cell);
      }
    }
  }

  createTile(coords: L.Coords, done: L.DoneCallback): HTMLElement {
    const canvas = document.createElement("canvas");
    const size = this.getTileSize();
    canvas.width = size.x;
    canvas.height = size.y;
    canvas.style.opacity = "0";
    canvas.style.transition = "opacity 180ms ease-out";
    const ctx = canvas.getContext("2d")!;

    const key = this.urlFor(coords);

    // Draw the parent tile immediately (scaled 2x, cropped to this quadrant)
    // so there is never an empty hole while the sharp tile loads.
    if (coords.z > 0) {
      const px = Math.floor(coords.x / 2);
      const py = Math.floor(coords.y / 2);
      const qx = coords.x - px * 2;
      const qy = coords.y - py * 2;
      const parentKey = this.urlFor({ ...coords, z: coords.z - 1, x: px, y: py });
      const parent = this.cache.get(parentKey);
      if (parent) {
        ctx.imageSmoothingEnabled = true;
        ctx.drawImage(parent, -qx * size.x, -qy * size.y, size.x * 2, size.y * 2);
        canvas.style.opacity = "1";
      }
    }

    // Leaflet's GridLayer stores `this._tiles[key]` only *after* createTile
    // returns. When the tile is already cached, load() invokes this callback
    // synchronously, so done() would run before that store; _tileReady then
    // fails its `this._tiles[key]` lookup and never adds
    // 'leaflet-tile-loaded', leaving the tile visibility:hidden forever (the
    // map goes black after zooming out onto cached tiles). Defer one task so
    // the lookup succeeds.
    this.load(key, this.urlFor(coords), (img) => {
      setTimeout(() => {
        if (!img) {
          // Leave whatever the parent fallback drew (possibly nothing) and let
          // Leaflet know the request settled so it does not wait forever.
          done(undefined, canvas);
          return;
        }
        ctx.imageSmoothingEnabled = false;
        ctx.clearRect(0, 0, size.x, size.y);
        if (img.isEmpty) {
          // The area exists on the map but was never generated in this save.
          // Mark it so it is not confused with a tile that is still loading.
          this.drawEmptyPattern(ctx, size);
        } else {
          ctx.drawImage(img.bitmap, 0, 0, size.x, size.y);
        }
        canvas.style.opacity = "1";
        done(undefined, canvas);
      }, 0);
    });

    return canvas;
  }
}
