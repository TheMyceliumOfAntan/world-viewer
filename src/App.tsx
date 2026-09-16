import { useEffect, useRef, useState } from "react";
import L from "leaflet";
import "leaflet/dist/leaflet.css";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { CachedTileLayer } from "./tileLayer";

type Dimension = {
  id: number;
  name: string;
  region_dir: string;
  chunk_count: number;
  has_data: boolean;
  min_y: number;
  max_y: number;
};

type Waypoint = {
  name: string;
  x: number;
  y: number;
  z: number;
  dimension: number;
  color: string;
  kind: string;
  source: string;
};

type Player = { x: number; y: number; z: number; dimension: number; name: string };

type WorldInfo = {
  save_dir: string;
  save_name: string;
  level_name: string;
  world_seed: string;
  instance_root: string;
  dimensions: Dimension[];
  player: Player | null;
  waypoints: Waypoint[];
  palette_entries: number;
  palette_mapped_blocks: number;
};

type OpenResult = { ok: boolean; info: WorldInfo | null; error: string | null };

/** Top-most non-air block of the hovered column (see `probe_block`). */
type BlockInfo = { y: number | null; name: string | null; id: string | null };

const DEFAULT_SAVE = "C:\\.minecraft\\versions\\GTNH 2.8.4\\saves\\新的世界 - 副本";

/** Render toggles, mirrored in the tile URL so each tile renders per setting. */
type RenderFlags = { water: boolean; shading: boolean; altitude: boolean };

const DEFAULT_FLAGS: RenderFlags = { water: true, shading: true, altitude: true };

function tileUrl(
  dim: number,
  ymax: number,
  maxY: number,
  worldKey: string,
  flags: RenderFlags
) {
  const yPart = ymax >= maxY ? "4294967295" : String(ymax);
  const f = `water=${flags.water ? 1 : 0}&shade=${flags.shading ? 1 : 0}&alt=${
    flags.altitude ? 1 : 0
  }`;
  return `http://tile.localhost/${worldKey}/${dim}/{z}/{x}/{y}.png?ymax=${yPart}&${f}`;
}

/** Short, filesystem-safe key identifying a save, so tile URLs differ per world. */
function worldKeyOf(saveDir: string): string {
  let h = 0;
  for (let i = 0; i < saveDir.length; i++) {
    h = (Math.imul(31, h) + saveDir.charCodeAt(i)) | 0;
  }
  return "w" + (h >>> 0).toString(36);
}

/** Vertical extent of a dimension, falling back to the legacy range. */
function rangeOf(dimensions: Dimension[], id: number): { min: number; max: number } {
  const d = dimensions.find((x) => x.id === id);
  return { min: d?.min_y ?? 0, max: d?.max_y ?? 255 };
}

/** Per-source waypoint counts for the current dimension. */
function WaypointSources({ world, dim }: { world: WorldInfo; dim: number }) {
  const counts = new Map<string, number>();
  for (const w of world.waypoints) {
    if (w.dimension !== dim) continue;
    counts.set(w.source, (counts.get(w.source) ?? 0) + 1);
  }
  if (counts.size === 0) {
    return <p className="hint">当前维度没有路径点</p>;
  }
  const label: Record<string, string> = {
    journeymap: "JourneyMap",
    xaero: "Xaero",
    voxelmap: "VoxelMap",
  };
  return (
    <ul className="sources">
      {[...counts.entries()].map(([src, n]) => (
        <li key={src}>
          <span className="srcname">{label[src] ?? src}</span>
          <span className="srcnum">{n}</span>
        </li>
      ))}
    </ul>
  );
}

export default function App() {
  const mapRef = useRef<L.Map | null>(null);
  const mapDivRef = useRef<HTMLDivElement | null>(null);
  const layerRef = useRef<CachedTileLayer | null>(null);
  const worldKeyRef = useRef<string>("none");
  const markersRef = useRef<L.LayerGroup | null>(null);
  const [info, setInfo] = useState<WorldInfo | null>(null);
  const [error, setError] = useState<string>("");
  const [dim, setDim] = useState<number>(0);
  const [ymax, setYmax] = useState<number>(255);
  const [loading, setLoading] = useState(false);
  const [mouse, setMouse] = useState<{ x: number; z: number } | null>(null);
  const [block, setBlock] = useState<BlockInfo | null>(null);
  const [flags, setFlags] = useState<RenderFlags>(DEFAULT_FLAGS);
  // Vertical extent of the current dimension. Modern saves reach -64..319,
  // legacy ones 0..255, and modded datapacks can move both ends, so the
  // slider range and the "full height" sentinel follow the world's own limits
  // instead of a fixed 255.
  const dimInfo = info?.dimensions.find((d) => d.id === dim);
  const range = { min: dimInfo?.min_y ?? 0, max: dimInfo?.max_y ?? 255 };
  const atFullHeight = ymax >= range.max;
  // Latest probe request, so a slow earlier reply cannot overwrite a newer one.
  const probeSeqRef = useRef(0);
  const probeTimerRef = useRef<number | null>(null);
  // The mousemove handler is registered once, so it must read the current
  // world/dim/height through a ref rather than a stale closure.
  const latestRef = useRef({ loaded: false, dim: 0, ymax: 255, maxY: 255 });
  latestRef.current = { loaded: !!info, dim, ymax, maxY: range.max };
  const lastProbeRef = useRef<{ x: number; z: number } | null>(null);

  const runProbe = async (x: number, z: number) => {
    lastProbeRef.current = { x, z };
    const { loaded, dim: d, ymax: y, maxY } = latestRef.current;
    if (!loaded) {
      setBlock(null);
      return;
    }
    const seq = ++probeSeqRef.current;
    try {
      const r = await invoke<BlockInfo>("probe_block", {
        dim: d,
        x,
        z,
        ymaxU: y >= maxY ? 4294967295 : y,
      });
      if (seq === probeSeqRef.current) setBlock(r);
    } catch {
      if (seq === probeSeqRef.current) setBlock(null);
    }
  };

  const scheduleProbe = (x: number, z: number) => {
    lastProbeRef.current = { x, z };
    if (probeTimerRef.current !== null) return;
    probeTimerRef.current = window.setTimeout(() => {
      probeTimerRef.current = null;
      const p = lastProbeRef.current;
      if (p) void runProbe(p.x, p.z);
    }, 150);
  };

  useEffect(() => {
    return () => {
      if (probeTimerRef.current !== null) window.clearTimeout(probeTimerRef.current);
    };
  }, []);

  const ensureMap = () => {
    if (mapRef.current || !mapDivRef.current) return mapRef.current;
    const map = L.map(mapDivRef.current, {
      crs: L.CRS.Simple,
      minZoom: 0,
      maxZoom: 4,
      zoomControl: true,
      attributionControl: false,
      zoomSnap: 0.25,
    });
    map.setView([0, 0], 1);
    mapRef.current = map;
    map.on("mousemove", (e: L.LeafletMouseEvent) => {
      // Leaflet CRS.Simple: lat = -worldZ, lng = worldX (in blocks at zoom 0 scale)
      const x = Math.round(e.latlng.lng);
      const z = Math.round(-e.latlng.lat);
      setMouse({ x, z });
      scheduleProbe(x, z);
    });
    return map;
  };

  const buildTileLayer = (
    map: L.Map,
    dimension: number,
    ymaxVal: number,
    maxY: number,
    f: RenderFlags
  ) => {
    const template = tileUrl(dimension, ymaxVal, maxY, worldKeyRef.current, f);
    if (layerRef.current) {
      // Reuse the layer (and its cache) unless the world, the height filter or
      // a render toggle changed. All of them are part of the URL, so switching
      // worlds is detected here and drops the previous world's tiles.
      const changed = layerRef.current.getUrlTemplate() !== template;
      if (changed) {
        layerRef.current.clearCache();
      }
      layerRef.current.setUrlTemplate(template);
      layerRef.current.redraw();
      return;
    }
    const layer = new CachedTileLayer({
      minZoom: 0,
      maxZoom: 4,
      noWrap: true,
      maxConcurrent: 6,
      keepBuffer: 3,
      updateWhenIdle: false,
      updateWhenZooming: true,
    });
    layer.setUrlTemplate(template);
    layer.addTo(map);
    layerRef.current = layer;
    // Exposed for diagnostics / automated tests.
    (window as unknown as { __tileLayer?: CachedTileLayer }).__tileLayer = layer;
  };

  const drawMarkers = (map: L.Map, world: WorldInfo, dimension: number) => {
    if (markersRef.current) {
      map.removeLayer(markersRef.current);
    }
    const group = L.layerGroup();
    const sourceLabel: Record<string, string> = {
      journeymap: "JourneyMap",
      xaero: "Xaero",
      voxelmap: "VoxelMap",
    };
    for (const wp of world.waypoints.filter((w) => w.dimension === dimension)) {
      const marker = L.circleMarker([-wp.z, wp.x], {
        radius: 7,
        color: "#000",
        weight: 1.5,
        fillColor: wp.color,
        fillOpacity: 0.95,
      });
      const src = sourceLabel[wp.source] ?? wp.source;
      marker.bindPopup(
        `<b>${wp.name}</b><br/>${wp.kind} · ${src}<br/>X=${wp.x} Y=${wp.y} Z=${wp.z}`
      );
      marker.addTo(group);
    }
    if (world.player && world.player.dimension === dimension) {
      const p = world.player;
      const marker = L.circleMarker([-p.z, p.x], {
        radius: 9,
        color: "#fff",
        weight: 3,
        fillColor: "#e33",
        fillOpacity: 1,
      });
      marker.bindPopup(
        `<b>${p.name}</b> (最后位置)<br/>X=${p.x.toFixed(1)} Y=${p.y.toFixed(1)} Z=${p.z.toFixed(1)}`
      );
      marker.addTo(group);
    }
    group.addTo(map);
    markersRef.current = group;
  };

  const loadWorld = async (path: string) => {
    setLoading(true);
    setError("");
    try {
      const res = await invoke<OpenResult>("open_world", { path });
      if (!res.ok) {
        setError(res.error ?? "打开失败");
        setInfo(null);
        return;
      }
      setInfo(res.info);
      const map = ensureMap();
      if (!map || !res.info) return;
      // Key tiles by save so switching worlds cannot reuse stale tiles.
      // Clearing here (rather than relying on the URL differing) makes the
      // invariant explicit: a freshly opened world starts with no tiles.
      worldKeyRef.current = worldKeyOf(res.info.save_dir);
      layerRef.current?.clearCache();
      // Prefer the first dimension that actually has chunks, so a save whose
      // first listed dimension is empty does not open on a blank map.
      const firstDim =
        res.info.dimensions.find((d) => d.has_data)?.id ??
        res.info.dimensions[0]?.id ??
        0;
      setDim(firstDim);
      const firstRange = rangeOf(res.info.dimensions, firstDim);
      setYmax(firstRange.max);
      buildTileLayer(map, firstDim, firstRange.max, firstRange.max, flags);
      drawMarkers(map, res.info, firstDim);
      // center on player if present, else on spawn-ish origin
      const p = res.info.player;
      if (p) {
        map.setView([-p.z, p.x], 2);
      } else {
        map.setView([0, 0], 1);
      }
      // force tile refresh
      layerRef.current?.redraw();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const switchDim = (id: number) => {
    setDim(id);
    const map = mapRef.current;
    if (!map) return;
    // Each dimension has its own vertical extent, so reset the height filter
    // to the new dimension's ceiling instead of carrying the old value over.
    const nextRange = rangeOf(info?.dimensions ?? [], id);
    const nextYmax = Math.min(ymax, nextRange.max);
    setYmax(nextYmax);
    buildTileLayer(map, id, nextYmax, nextRange.max, flags);
    if (info) drawMarkers(map, info, id);
    // Fly to the first marker in this dimension, else to origin
    const wps = info?.waypoints.filter((w) => w.dimension === id) ?? [];
    const p = info?.player && info.player.dimension === id ? info.player : null;
    if (p) {
      map.setView([-p.z, p.x], 2);
    } else if (wps.length > 0) {
      map.setView([-wps[0].z, wps[0].x], 2);
    } else {
      map.setView([0, 0], 1);
    }
  };

  const applyYmax = (v: number) => {
    setYmax(v);
    const map = mapRef.current;
    if (!map) return;
    buildTileLayer(map, dim, v, range.max, flags);
  };

  const toggleFlag = (key: keyof RenderFlags) => {
    const next = { ...flags, [key]: !flags[key] };
    setFlags(next);
    const map = mapRef.current;
    if (!map) return;
    buildTileLayer(map, dim, ymax, range.max, next);
  };

  const pickFolder = async () => {
    const selected = await open({ directory: true, title: "选择 Minecraft 存档目录" });
    if (typeof selected === "string") {
      await loadWorld(selected);
    }
  };

  useEffect(() => {
    // Support ?save=<path> so a world can be opened directly (and so the UI
    // can be reloaded by automated tests without losing the loaded world).
    const params = new URLSearchParams(window.location.search);
    const save = params.get("save");
    if (save) {
      void loadWorld(save);
    }
    return () => {
      mapRef.current?.remove();
      mapRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="app">
      <header className="topbar">
        <div className="title">World Viewer</div>
        <div className="path">
          {info ? (
            <span title={info.save_dir}>
              {info.level_name} · {info.dimensions.length} 个维度 · 方块映射 {info.palette_mapped_blocks} 个 ·
              调色板 {info.palette_entries} 项
            </span>
          ) : (
            <span className="dim">未加载存档</span>
          )}
        </div>
        <div className="actions">
          <button onClick={pickFolder} disabled={loading}>
            选择存档…
          </button>
          <button onClick={() => loadWorld(DEFAULT_SAVE)} disabled={loading}>
            加载 GTNH 测试存档
          </button>
        </div>
      </header>

      <div className="body">
        <aside className="sidebar">
          <h3>维度</h3>
          {info ? (
            <ul>
              {info.dimensions.map((d) => (
                <li
                  key={d.id}
                  className={d.id === dim ? "active" : ""}
                  onClick={() => switchDim(d.id)}
                >
                  <span className="dimname">{d.name}</span>
                  <span className="dimmeta">ID {d.id} · {d.chunk_count} 区块</span>
                </li>
              ))}
            </ul>
          ) : (
            <p className="hint">加载存档后显示可用维度</p>
          )}

          <h3>高度切层</h3>
          <div className="slider">
            <input
              type="range"
              min={range.min}
              max={range.max}
              value={ymax}
              disabled={!info}
              onChange={(e) => applyYmax(Number(e.target.value))}
            />
            <div className="sliderval">
              Y ≤ {atFullHeight ? "全高" : ymax}
              <span className="dimmeta">
                {" "}
                （{range.min}..{range.max}）
              </span>
            </div>
          </div>
          <p className="hint">向下拖动可查看地下结构（洞穴/矿道）</p>

          <h3>渲染</h3>
          <ul className="toggles">
            <li>
              <label>
                <input
                  type="checkbox"
                  checked={flags.water}
                  disabled={!info}
                  onChange={() => toggleFlag("water")}
                />
                透视水面（显示水底）
              </label>
            </li>
            <li>
              <label>
                <input
                  type="checkbox"
                  checked={flags.shading}
                  disabled={!info}
                  onChange={() => toggleFlag("shading")}
                />
                地形阴影
              </label>
            </li>
            <li>
              <label>
                <input
                  type="checkbox"
                  checked={flags.altitude}
                  disabled={!info || !flags.shading}
                  onChange={() => toggleFlag("altitude")}
                />
                高度明暗
              </label>
            </li>
          </ul>

          <h3>图例</h3>
          <ul className="legend">
            <li>
              <span className="dot player" /> 玩家最后位置
            </li>
            <li>
              <span className="dot wp" /> 路径点（JourneyMap / Xaero / VoxelMap）
            </li>
          </ul>
          {info && <WaypointSources world={info} dim={dim} />}
        </aside>

        <main className="mapwrap">
          <div ref={mapDivRef} className="map" />
          {error && <div className="error">{error}</div>}
          {loading && <div className="loading">加载中…</div>}
        </main>
      </div>

      <footer className="statusbar">
        <span>
          坐标: {mouse ? `X=${mouse.x} ${block?.y != null ? `Y=${block.y} ` : ""}Z=${mouse.z}` : "—"}
        </span>
        <span>
          方块:{" "}
          {block?.name
            ? `${block.name}${block.id && block.id !== block.name ? ` (${block.id})` : ""}`
            : "—"}
        </span>
        <span>维度: {info?.dimensions.find((d) => d.id === dim)?.name ?? "—"}</span>
        <span>层: {atFullHeight ? "全高" : `Y ≤ ${ymax}`}</span>
        <span>种子: {info?.world_seed || "—"}</span>
        <span>{info ? `存档: ${info.save_name}` : ""}</span>
      </footer>
    </div>
  );
}
