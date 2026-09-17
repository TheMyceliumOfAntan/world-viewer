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
  /**
   * Upper bound on cached bitmaps. A 256x256 RGBA bitmap is ~256 KiB, so an
   * unbounded cache reached ~256 MB after a thousand tiles of panning. The
   * frontend cache is a fast path in front of the server's chunk cache; it does
   * not need to hold the whole world, only the tiles near the viewport.
   */
  private maxCached: number;

  constructor(options?: L.GridLayerOptions & { maxConcurrent?: number; maxCached?: number }) {
    super({ tileSize: 256, ...options });
    this.maxConcurrent = options?.maxConcurrent ?? 6;
    this.maxCached = options?.maxCached ?? 512;
  }

  /**
   * Move `key` to the most-recently-used end and drop the oldest entries.
   *
   * `Map` iterates in insertion order, so re-inserting on use makes iteration
   * order equal recency order; eviction is then just the first key. This is the
   * whole LRU — no separate bookkeeping, since the key is the URL and the value
   * is only reachable through it.
   */
  /**
   * Release an evicted bitmap's GPU memory.
   *
   * `ImageBitmap.close()` is immediate, and a bitmap may still be referenced
   * by a `createTile` callback sitting in the `setTimeout` queue (both the
   * parent-fallback draw and the sharp-tile draw run there). Closing in the
   * same turn would make that `drawImage` throw `InvalidStateError`, so the
   * close is deferred one macrotask, after every already-queued draw has run.
   *
   * The deferral opens a race: the same bitmap can be re-remembered (cache
   * hit) before the timeout fires. Re-check membership at close time so a
   * bitmap that is live again is not closed out from under the cache.
   */
  private closeSoon(bitmap: ImageBitmap) {
    setTimeout(() => {
      for (const live of this.cache.values()) {
        if (live === bitmap) return;
      }
      bitmap.close();
    }, 0);
  }

  private remember(key: string, bitmap: ImageBitmap) {
    this.cache.delete(key);
    this.cache.set(key, bitmap);
    while (this.cache.size > this.maxCached) {
      const oldest = this.cache.keys().next().value;
      if (oldest === undefined) break;
      const evicted = this.cache.get(oldest);
      this.cache.delete(oldest);
      if (evicted) this.closeSoon(evicted);
      // The empty-set is a sibling of the cache and must not outgrow it.
      this.empty.delete(oldest);
    }
  }

  private recall(key: string): ImageBitmap | undefined {
    const bitmap = this.cache.get(key);
    if (bitmap) {
      this.cache.delete(key);
      this.cache.set(key, bitmap);
    }
    return bitmap;
  }

  setUrlTemplate(template: string) {
    if (this.urlTemplate === template) return;
    this.urlTemplate = template;
    this.failures.clear();
    this.empty.clear();
    // Queued jobs captured the old template's URLs, and redraw() is about to
    // remove every tile element that was waiting on them. Dropping the jobs
    // without dropping their waiters would strand callbacks whose key can be
    // hit again if the user returns to the same height, leaving those tiles
    // permanently blank. Dragging the height slider fires many template
    // changes, so keeping the jobs would also stack thousands of stale
    // requests ahead of the current ones and leave the map black for tens of
    // seconds until the backlog drains.
    this.queue.length = 0;
    this.waiting.clear();
    this.redraw();
  }

  getUrlTemplate() {
    return this.urlTemplate;
  }

  /**
   * Leaflet's `_setView` rounds the zoom before clamping, but `redraw()` and
   * `_update()` pass the raw map zoom straight through. The map uses
   * `zoomSnap: 0.25`, so a wheel zoom can sit on a fractional zoom (2.5);
   * those two paths then set `_tileZoom = 2.5` and request tiles at z=2.5.
   * The backend parses the zoom as an integer and 404s, so every tile fails
   * and the map stays black until the next integer-zoom event. Rounding in
   * the one clamp shared by all three call sites keeps `_tileZoom` integral.
   */
  protected _clampZoom(zoom: number): number {
    const base = L.GridLayer.prototype as unknown as {
      _clampZoom(zoom: number): number;
    };
    return base._clampZoom.call(this, Math.round(zoom));
  }

  /** Drop cached tiles for the previous world/height without touching the map. */
  clearCache() {
    for (const bitmap of this.cache.values()) {
      this.closeSoon(bitmap);
    }
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
    const cached = this.recall(key);
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
          this.remember(key, bitmap);
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
      const parent = this.recall(parentKey);
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
