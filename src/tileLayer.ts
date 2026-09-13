import L from "leaflet";

/**
 * A tile layer that never leaves holes while loading.
 *
 * Plain L.tileLayer requests every visible tile at once and shows blank space
 * until each one arrives, so panning and zooming flash empty squares. This
 * layer keeps an in-memory cache and falls back to the already-loaded parent
 * tile (drawn scaled and cropped to the right quadrant) so the map stays
 * continuous, just blurrier, until the sharp tile arrives.
 */
export class CachedTileLayer extends L.GridLayer {
  private cache = new Map<string, HTMLImageElement>();
  private inflight = new Set<string>();
  private queue: Array<() => void> = [];
  private active = 0;

  /** Keyed by `dim|ymax` — changing either invalidates every tile. */
  private urlTemplate = "";
  private maxConcurrent: number;

  constructor(options?: L.GridLayerOptions & { maxConcurrent?: number }) {
    super({ tileSize: 256, ...options });
    this.maxConcurrent = options?.maxConcurrent ?? 6;
  }

  setUrlTemplate(template: string) {
    if (this.urlTemplate === template) return;
    this.urlTemplate = template;
    this.redraw();
  }

  getUrlTemplate() {
    return this.urlTemplate;
  }

  /** Drop cached tiles for the previous world/height without touching the map. */
  clearCache() {
    this.cache.clear();
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

  private load(key: string, url: string, onReady: (img: HTMLImageElement | null) => void) {
    const cached = this.cache.get(key);
    if (cached) {
      onReady(cached);
      return;
    }
    if (this.inflight.has(key)) return;
    this.inflight.add(key);
    this.enqueue(() => {
      const img = new Image();
      img.onload = () => {
        this.cache.set(key, img);
        this.inflight.delete(key);
        this.done();
        onReady(img);
      };
      img.onerror = () => {
        this.inflight.delete(key);
        this.done();
        onReady(null);
      };
      img.src = url;
    });
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

    this.load(key, this.urlFor(coords), (img) => {
      if (!img) {
        // Nothing rendered yet and the tile failed: leave it transparent.
        done(undefined, canvas);
        return;
      }
      ctx.imageSmoothingEnabled = false;
      ctx.clearRect(0, 0, size.x, size.y);
      ctx.drawImage(img, 0, 0, size.x, size.y);
      canvas.style.opacity = "1";
      done(undefined, canvas);
    });

    return canvas;
  }
}
