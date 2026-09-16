mod biome;
mod biome_tints;
mod legacy_ids;
mod nbt;
mod palette;
mod region;
mod render;
mod world;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tauri::http::Response;
use tauri::{Manager, State};

use render::TileCache;
use world::{World, WorldInfo};

/// Chunks held in the shared cache.
///
/// A zoom-0 viewport shows 5x5 tiles and each tile needs an 18x18-chunk block,
/// so a screen's working set is ~6500 chunks. At 4096 the cache held only 63%
/// of one screen and thrashed: measured 17% hit rate, and panning re-read from
/// disk constantly. At 8192 a screen fits, the hit rate jumps to ~53%, and
/// whole-viewport render time roughly halves. Chunks are ~9.7 KiB, so this is
/// ~78 MB — cheap next to the decoded tile bitmaps the frontend holds.
const CHUNK_CACHE_CAPACITY: usize = 8192;

pub struct AppState {
    world: Mutex<Option<World>>,
    /// `Arc` so a tile request can clone it and drop the lock before rendering.
    /// Holding the guard across `render_tile` would serialize every concurrent
    /// tile request, which is exactly what the frontend's 6-way concurrency is
    /// meant to avoid.
    cache: Mutex<Option<Arc<TileCache>>>,
}

impl Default for AppState {
    fn default() -> Self {
        AppState {
            world: Mutex::new(None),
            cache: Mutex::new(None),
        }
    }
}

#[derive(serde::Serialize)]
struct OpenResult {
    ok: bool,
    info: Option<WorldInfo>,
    error: Option<String>,
}

#[tauri::command]
fn open_world(path: String, state: State<AppState>) -> OpenResult {
    let save_dir = PathBuf::from(&path);
    match world::open_world(&save_dir, None) {
        Ok(w) => {
            let info = w.info();
            let pal = palette::Palette::load(&w.instance_root, &w.level_dat)
                .unwrap_or_else(|_| palette::Palette::empty());
            let cache = Arc::new(TileCache::new(pal, CHUNK_CACHE_CAPACITY));
            // Each lock is taken and released separately: nesting them here
            // while other commands take them in the opposite order would risk
            // a deadlock.
            *state.world.lock().unwrap() = Some(w);
            *state.cache.lock().unwrap() = Some(cache);
            // The previous world's region handles point at different files.
            region::clear_region_cache();
            OpenResult {
                ok: true,
                info: Some(info),
                error: None,
            }
        }
        Err(e) => OpenResult {
            ok: false,
            info: None,
            error: Some(e),
        },
    }
}

#[tauri::command]
fn get_world_info(state: State<AppState>) -> Option<WorldInfo> {
    state.world.lock().unwrap().as_ref().map(|w| w.info())
}

#[tauri::command]
fn invalidate_cache(state: State<AppState>) {
    if let Some(c) = state.cache.lock().unwrap().as_ref() {
        c.invalidate();
    }
    region::clear_region_cache();
}

#[tauri::command]
fn pick_save_folder() -> Option<String> {
    None // handled on frontend via dialog plugin
}

#[derive(serde::Serialize)]
struct BlockInfo {
    /// Block-y of the top-most non-air block, or None when the column is empty.
    y: Option<i32>,
    /// Human-readable name (JourneyMap display name when known).
    name: Option<String>,
    /// Raw block identifier: `name` or `id:meta` for legacy chunks.
    id: Option<String>,
}

/// Inspect the top-most non-air block of one world column, for the status bar.
///
/// The map is a 2D CRS.Simple plane, so block-y is not derivable on the
/// frontend; it has to come from the same column scan the renderer uses.
#[tauri::command]
fn probe_block(
    dim: i32,
    x: i32,
    z: i32,
    ymax_u: i64,
    state: State<AppState>,
) -> Result<BlockInfo, String> {
    use render::BlockRef;

    let world_guard = state.world.lock().unwrap();
    let world = world_guard.as_ref().ok_or("未加载世界")?;
    let dim_info = world.dimension(dim).ok_or("维度不存在")?;
    let region_dir = PathBuf::from(&dim_info.region_dir);
    let ymax = resolve_ymax(ymax_u, dim_info);

    let cache = state
        .cache
        .lock()
        .unwrap()
        .clone()
        .ok_or("缓存未初始化")?;

    // Reuse the renderer's chunk cache: the column almost always falls inside
    // a chunk a visible tile already loaded, so this is a HashMap hit.
    let key = (x >> 4, z >> 4);
    let Some(chunk) = cache.get_or_load(&region_dir, key) else {
        return Ok(BlockInfo { y: None, name: None, id: None });
    };

    // Chunk-local coordinates; the chunk covers x&15, z&15.
    let (lx, lz) = ((x & 15) as usize, (z & 15) as usize);
    let Some((block, by)) = chunk.top_block_ref(lx, lz, ymax) else {
        return Ok(BlockInfo { y: None, name: None, id: None });
    };

    let id = match block {
        render::BlockRefRef::Legacy(id, meta) => format!("{}:{}", id, meta),
        render::BlockRefRef::Named(name) => name.to_string(),
    };
    let owned: BlockRef = block.to_owned_ref();
    Ok(BlockInfo {
        y: Some(by),
        name: Some(cache.palette.display_name(&owned)),
        id: Some(id),
    })
}

/// Resolve the `ymax` query/sentinel against a dimension's real height range.
fn resolve_ymax(ymax_u: i64, dim_info: &world::DimensionInfo) -> i32 {
    world::resolve_ymax(ymax_u, dim_info)
}

fn render_tile_png(
    state: &AppState,
    dim: i32,
    z: i32,
    x: i32,
    row: i32,
    ymax_u: i64,
    opts: render::RenderOpts,
) -> Result<(Vec<u8>, bool), String> {
    let world_guard = state.world.lock().unwrap();
    let world = world_guard.as_ref().ok_or("未加载世界")?;
    let dim_info = world.dimension(dim).ok_or("维度不存在")?;
    let region_dir = PathBuf::from(&dim_info.region_dir);
    let ymax = resolve_ymax(ymax_u, dim_info);

    // Clone the Arc and drop the world lock before rendering. `render_tile`
    // takes tens of milliseconds; holding this lock would block `open_world`
    // and every other tile request for that whole time.
    let cache = state
        .cache
        .lock()
        .unwrap()
        .clone()
        .ok_or("缓存未初始化")?;
    drop(world_guard);

    let (png, has_data) = render::render_tile(&cache, &region_dir, z, x, row, ymax, opts)?;
    Ok((png, has_data))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            open_world,
            get_world_info,
            invalidate_cache,
            pick_save_folder,
            probe_block
        ])
        .register_asynchronous_uri_scheme_protocol("tile", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let uri = request.uri().clone();
            // Render off the main thread. Doing this synchronously in the
            // protocol handler blocks the event loop and makes the window
            // stutter on every pan/zoom.
            std::thread::spawn(move || {
                let state: State<AppState> = app.state();
                match tile_from_uri(&state, &uri) {
                    Ok((png, has_data)) => {
                        responder.respond(
                            Response::builder()
                                .header("Content-Type", "image/png")
                                .header("Access-Control-Allow-Origin", "*")
                                .header("Cache-Control", "no-store")
                                // Lets the frontend distinguish "nothing generated
                                // here" from "still loading".
                                .header("X-Tile-Empty", if has_data { "0" } else { "1" })
                                .header("Access-Control-Expose-Headers", "X-Tile-Empty")
                                .body(png)
                                .unwrap(),
                        );
                    }
                    Err(e) => {
                        responder.respond(
                            Response::builder()
                                .status(404)
                                .header("Content-Type", "text/plain; charset=utf-8")
                                .body(e.into_bytes())
                                .unwrap(),
                        );
                    }
                }
            });
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// tile://localhost/{dim}/{z}/{x}/{row}.png?ymax=N
/// tile://localhost/{worldKey}/{dim}/{z}/{x}/{row}.png?ymax=N
///
/// `worldKey` identifies the loaded save. It is not used for lookup — the
/// app only ever has one world open — but it must be present so that the
/// browser's tile cache and the frontend's tile cache are keyed per world.
/// Without it, switching saves would keep serving the previous world's tiles.
fn tile_from_uri(state: &AppState, uri: &tauri::http::Uri) -> Result<(Vec<u8>, bool), String> {
    let path = uri.path().trim_start_matches('/');
    let path = path.trim_end_matches(".png");
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() != 5 {
        return Err(format!("bad tile path: {}", uri.path()));
    }
    let dim: i32 = parts[1].parse().map_err(|_| "bad dim".to_string())?;
    let z: i32 = parts[2].parse().map_err(|_| "bad z".to_string())?;
    let x: i32 = parts[3].parse().map_err(|_| "bad x".to_string())?;
    let row: i32 = parts[4].parse().map_err(|_| "bad row".to_string())?;
    let query = uri.query().unwrap_or("");
    let lookup = |k: &str| -> Option<String> {
        query
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .find(|(key, _)| *key == k)
            .map(|(_, v)| v.to_string())
    };
    let ymax = lookup("ymax")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(world::YMAX_FULL);
    let opts = render::RenderOpts::from_query(lookup);
    render_tile_png(state, dim, z, x, row, ymax, opts)
}

/// Exposed for tests
pub mod testing {
    pub use crate::biome::{apply_tint, tint_color, BiomeDef, TintKind};
    pub use crate::palette::Palette;
    pub use crate::region::{BlockStates, LegacyBiomes, RegionFile, Section};
    pub use crate::render::{BlockRef, ChunkData, RenderOpts, ScanOpts};
    pub use crate::world::{resolve_ymax, DimensionInfo, World, YMAX_FULL};

    /// Which biome tint a block name takes.
    pub fn tint_kind(name: &str) -> crate::biome::TintKind {
        crate::render::tint_kind(name)
    }

    /// Resolve a palette colour for a biome.
    pub fn resolve_color(c: [u8; 3], name: &str, biome: crate::biome::BiomeDef) -> [u8; 3] {
        crate::render::resolve_color(c, name, biome)
    }

    use std::path::{Path, PathBuf};

    pub fn open_world(save_dir: &Path) -> Result<World, String> {
        crate::world::open_world(save_dir, None)
    }

    pub fn load_chunk(region_dir: &Path, cx: i32, cz: i32) -> Result<Option<ChunkData>, String> {
        crate::render::load_chunk(region_dir, cx, cz)
    }

    /// Raw decompressed chunk NBT bytes (stage 1 of the pipeline).
    pub fn read_chunk_nbt(
        region_dir: &Path,
        cx: i32,
        cz: i32,
    ) -> Result<Option<Vec<u8>>, String> {
        crate::region::read_chunk_nbt(region_dir, cx, cz)
    }

    /// Drop the cached region handles (forces the next read to reopen files).
    pub fn clear_region_cache() {
        crate::region::clear_region_cache()
    }

    /// Full NBT parse (stage 2 of the pipeline).
    pub fn parse_nbt(data: &[u8]) -> Result<(), String> {
        crate::nbt::parse(data).map(|_| ())
    }

    /// Generic (whole-tree) section parser, used to validate the fast one.
    pub fn parse_sections_generic(data: &[u8]) -> Result<Vec<crate::region::Section>, String> {
        crate::region::parse_sections(data)
    }

    /// Targeted section parser used in production.
    pub fn parse_sections_fast(data: &[u8]) -> Result<Vec<crate::region::Section>, String> {
        crate::region::parse_sections_fast(data)
    }

    /// Load waypoints from all supported mods under an instance root.
    pub fn load_waypoints(
        instance_root: &Path,
        save_name: &str,
    ) -> Vec<crate::palette::Waypoint> {
        crate::palette::load_waypoints(instance_root, save_name)
    }

    /// Render a single 256x256 tile the same way the tile:// protocol does.
    pub fn render_tile(
        palette: &Palette,
        region_dir: &Path,
        _dim: i32,
        zoom: i32,
        tile_x: i32,
        tile_row: i32,
        ymax: i32,
    ) -> Result<Vec<u8>, String> {
        render_tile_with_opts(
            palette,
            region_dir,
            zoom,
            tile_x,
            tile_row,
            ymax,
            RenderOpts::default(),
        )
    }

    /// Like `render_tile` but also reports whether the tile covers any
    /// generated chunks (used to distinguish "no data" from "not loaded").
    pub fn render_tile_with_data_flag(
        palette: &Palette,
        region_dir: &Path,
        zoom: i32,
        tile_x: i32,
        tile_row: i32,
        ymax: i32,
    ) -> Result<(Vec<u8>, bool), String> {
        render_tile_full(
            palette,
            region_dir,
            zoom,
            tile_x,
            tile_row,
            ymax,
            RenderOpts::default(),
        )
    }

    /// Render with explicit render options.
    pub fn render_tile_with_opts(
        palette: &Palette,
        region_dir: &Path,
        zoom: i32,
        tile_x: i32,
        tile_row: i32,
        ymax: i32,
        opts: RenderOpts,
    ) -> Result<Vec<u8>, String> {
        render_tile_full(palette, region_dir, zoom, tile_x, tile_row, ymax, opts)
            .map(|(png, _has_data)| png)
    }

    /// Render with explicit options, reporting the data flag too.
    pub fn render_tile_full(
        palette: &Palette,
        region_dir: &Path,
        zoom: i32,
        tile_x: i32,
        tile_row: i32,
        ymax: i32,
        opts: RenderOpts,
    ) -> Result<(Vec<u8>, bool), String> {
        let cache = crate::render::TileCache::new(palette.clone(), 512);
        crate::render::render_tile(&cache, region_dir, zoom, tile_x, tile_row, ymax, opts)
    }

    pub use crate::render::TileCache;

    /// A fresh cache with the production capacity, for concurrency tests.
    pub fn new_tile_cache(palette: Palette, capacity: usize) -> TileCache {
        crate::render::TileCache::new(palette, capacity)
    }

    /// Render one tile through a caller-provided cache, so a test can hold the
    /// same lock the Tauri commands hold.
    pub fn render_tile_in(
        cache: &TileCache,
        region_dir: &Path,
        zoom: i32,
        tile_x: i32,
        tile_row: i32,
        ymax: i32,
    ) -> Result<(Vec<u8>, bool), String> {
        crate::render::render_tile(
            cache,
            region_dir,
            zoom,
            tile_x,
            tile_row,
            ymax,
            RenderOpts::default(),
        )
    }

    #[allow(dead_code)]
    pub fn path_buf(s: &str) -> PathBuf {
        PathBuf::from(s)
    }
}

/// Exposed for tests
pub fn dims_by_id(world: &World) -> HashMap<i32, String> {
    world::dims_by_id(world)
}
