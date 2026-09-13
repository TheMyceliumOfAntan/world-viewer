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
};

type Waypoint = {
  name: string;
  x: number;
  y: number;
  z: number;
  dimension: number;
  color: string;
  kind: string;
};

type Player = { x: number; y: number; z: number; dimension: number; name: string };

type WorldInfo = {
  save_dir: string;
  save_name: string;
  level_name: string;
  instance_root: string;
  dimensions: Dimension[];
  player: Player | null;
  waypoints: Waypoint[];
  palette_entries: number;
  palette_mapped_blocks: number;
};

type OpenResult = { ok: boolean; info: WorldInfo | null; error: string | null };

const DEFAULT_SAVE = "C:\\.minecraft\\versions\\GTNH 2.8.4\\saves\\新的世界 - 副本";

function tileUrl(dim: number, ymax: number) {
  const yPart = ymax >= 255 ? "4294967295" : String(ymax);
  return `http://tile.localhost/${dim}/{z}/{x}/{y}.png?ymax=${yPart}`;
}

export default function App() {
  const mapRef = useRef<L.Map | null>(null);
  const mapDivRef = useRef<HTMLDivElement | null>(null);
  const layerRef = useRef<CachedTileLayer | null>(null);
  const markersRef = useRef<L.LayerGroup | null>(null);
  const [info, setInfo] = useState<WorldInfo | null>(null);
  const [error, setError] = useState<string>("");
  const [dim, setDim] = useState<number>(0);
  const [ymax, setYmax] = useState<number>(255);
  const [loading, setLoading] = useState(false);
  const [mouse, setMouse] = useState<{ x: number; z: number } | null>(null);

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
      setMouse({ x: Math.round(e.latlng.lng), z: Math.round(-e.latlng.lat) });
    });
    return map;
  };

  const buildTileLayer = (map: L.Map, dimension: number, ymaxVal: number) => {
    const template = tileUrl(dimension, ymaxVal);
    if (layerRef.current) {
      // Reuse the layer (and its cache) unless the world changed.
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
    for (const wp of world.waypoints.filter((w) => w.dimension === dimension)) {
      const marker = L.circleMarker([-wp.z, wp.x], {
        radius: 7,
        color: "#000",
        weight: 1.5,
        fillColor: wp.color,
        fillOpacity: 0.95,
      });
      marker.bindPopup(
        `<b>${wp.name}</b><br/>${wp.kind}<br/>X=${wp.x} Y=${wp.y} Z=${wp.z}`
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
      const firstDim = res.info.dimensions[0]?.id ?? 0;
      setDim(firstDim);
      setYmax(255);
      buildTileLayer(map, firstDim, 255);
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
    buildTileLayer(map, id, ymax);
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
    buildTileLayer(map, dim, v);
  };

  const pickFolder = async () => {
    const selected = await open({ directory: true, title: "选择 Minecraft 存档目录" });
    if (typeof selected === "string") {
      await loadWorld(selected);
    }
  };

  useEffect(() => {
    // no auto-load: avoid touching disk until user acts
    return () => {
      mapRef.current?.remove();
      mapRef.current = null;
    };
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
              min={0}
              max={255}
              value={ymax}
              disabled={!info}
              onChange={(e) => applyYmax(Number(e.target.value))}
            />
            <div className="sliderval">Y ≤ {ymax === 255 ? "全高" : ymax}</div>
          </div>
          <p className="hint">向下拖动可查看地下结构（洞穴/矿道）</p>

          <h3>图例</h3>
          <ul className="legend">
            <li>
              <span className="dot player" /> 玩家最后位置
            </li>
            <li>
              <span className="dot wp" /> JourneyMap 路径点
            </li>
          </ul>
        </aside>

        <main className="mapwrap">
          <div ref={mapDivRef} className="map" />
          {error && <div className="error">{error}</div>}
          {loading && <div className="loading">加载中…</div>}
        </main>
      </div>

      <footer className="statusbar">
        <span>坐标: {mouse ? `X=${mouse.x} Z=${mouse.z}` : "—"}</span>
        <span>维度: {info?.dimensions.find((d) => d.id === dim)?.name ?? "—"}</span>
        <span>层: {ymax === 255 ? "全高" : `Y ≤ ${ymax}`}</span>
        <span>{info ? `存档: ${info.save_name}` : ""}</span>
      </footer>
    </div>
  );
}
