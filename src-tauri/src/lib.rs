mod nbt;
mod palette;
mod region;
mod render;
mod world;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use tauri::http::Response;
use tauri::{Manager, State};

use render::TileCache;
use world::{World, WorldInfo};

pub struct AppState {
    world: Mutex<Option<World>>,
    cache: Mutex<Option<TileCache>>,
}

impl Default for AppState {
    fn default() -> Self {
        AppState {
            world: Mutex::new(None),
            cache: Mutex::new(None),
        }
    }
}

const TILE_SIZE: u32 = 256;
const CHUNKS_PER_TILE: i32 = 16;

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
            *state.cache.lock().unwrap() = Some(TileCache::new(pal, 4096));
            *state.world.lock().unwrap() = Some(w);
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
    if let Some(c) = state.cache.lock().unwrap().as_mut() {
        c.invalidate();
    }
}

#[tauri::command]
fn pick_save_folder() -> Option<String> {
    None // handled on frontend via dialog plugin
}

fn render_tile_png(
    state: &AppState,
    dim: i32,
    z: i32,
    x: i32,
    row: i32,
    ymax_u: u32,
) -> Result<Vec<u8>, String> {
    let world_guard = state.world.lock().unwrap();
    let world = world_guard.as_ref().ok_or("未加载世界")?;
    let dim_info = world.dimension(dim).ok_or("维度不存在")?;
    let region_dir = PathBuf::from(&dim_info.region_dir);
    let ymax = if ymax_u == u32::MAX {
        255
    } else {
        ymax_u.min(255) as i32
    };

    let mut cache_guard = state.cache.lock().unwrap();
    let cache = cache_guard.as_mut().ok_or("缓存未初始化")?;

    if !(0..=4).contains(&z) {
        return Err(format!("zoom {} out of range 0..=4", z));
    }
    // z=0 -> 16 chunks/tile (1 px per block), z=4 -> 1 chunk/tile (16 px per block)
    let chunks_per_tile = CHUNKS_PER_TILE >> z;
    let chunk_size_px = TILE_SIZE as i32 / chunks_per_tile;
    let world_chunk_x = x * chunks_per_tile;
    let world_chunk_z = row * chunks_per_tile;

    for cz in 0..chunks_per_tile {
        for cx in 0..chunks_per_tile {
            cache.ensure(&region_dir, world_chunk_x + cx, world_chunk_z + cz);
        }
    }

    let mut img = vec![0u8; (crate::TILE_SIZE * crate::TILE_SIZE * 4) as usize];
    for cz in 0..chunks_per_tile {
        for cx in 0..chunks_per_tile {
            let gcx = world_chunk_x + cx;
            let gcz = world_chunk_z + cz;
            if let Some(chunk) = cache.chunks.get(&(gcx, gcz)).and_then(|o| o.as_ref()) {
                let buf = render::render_chunk(chunk, &cache.palette, ymax, true);
                let px0 = (cx * chunk_size_px) as usize;
                let py0 = (cz * chunk_size_px) as usize;
                let span = chunk_size_px as usize;
                for zz in 0..span {
                    for xx in 0..span {
                        let sx = xx * 16 / span.max(1);
                        let sz = zz * 16 / span.max(1);
                        let rgb = buf[sz.min(15) * 16 + sx.min(15)];
                        if rgb == [0, 0, 0] {
                            continue;
                        }
                        let px = px0 + xx;
                        let py = py0 + zz;
                        let idx = (py * crate::TILE_SIZE as usize + px) * 4;
                        img[idx] = rgb[0];
                        img[idx + 1] = rgb[1];
                        img[idx + 2] = rgb[2];
                        img[idx + 3] = 255;
                    }
                }
            }
        }
    }

    encode_png(&img, TILE_SIZE, TILE_SIZE)
}

fn encode_png(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    }
    Ok(out)
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
            pick_save_folder
        ])
        .register_asynchronous_uri_scheme_protocol("tile", |ctx, request, responder| {
            let state: State<AppState> = ctx.app_handle().state();
            let uri = request.uri().clone();
            let result = tile_from_uri(&state, &uri);
            match result {
                Ok(png) => {
                    responder.respond(
                        Response::builder()
                            .header("Content-Type", "image/png")
                            .header("Access-Control-Allow-Origin", "*")
                            .header("Cache-Control", "no-store")
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
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// tile://localhost/{dim}/{z}/{x}/{row}.png?ymax=N
fn tile_from_uri(state: &AppState, uri: &tauri::http::Uri) -> Result<Vec<u8>, String> {
    let path = uri.path().trim_start_matches('/');
    let path = path.trim_end_matches(".png");
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() != 4 {
        return Err(format!("bad tile path: {}", uri.path()));
    }
    let dim: i32 = parts[0].parse().map_err(|_| "bad dim".to_string())?;
    let z: i32 = parts[1].parse().map_err(|_| "bad z".to_string())?;
    let x: i32 = parts[2].parse().map_err(|_| "bad x".to_string())?;
    let row: i32 = parts[3].parse().map_err(|_| "bad row".to_string())?;
    let ymax = uri
        .query()
        .and_then(|q| {
            q.split('&')
                .filter_map(|kv| kv.split_once('='))
                .find(|(k, _)| *k == "ymax")
                .and_then(|(_, v)| v.parse::<u32>().ok())
        })
        .unwrap_or(u32::MAX);
    render_tile_png(state, dim, z, x, row, ymax)
}

/// Exposed for tests
pub mod testing {
    pub use crate::palette::Palette;
    pub use crate::render::{render_chunk as render_chunk_impl, ChunkData};
    pub use crate::world::World;

    use std::path::{Path, PathBuf};

    pub fn open_world(save_dir: &Path) -> Result<World, String> {
        crate::world::open_world(save_dir, None)
    }

    pub fn load_chunk(region_dir: &Path, cx: i32, cz: i32) -> Result<Option<ChunkData>, String> {
        crate::render::load_chunk(region_dir, cx, cz)
    }

    pub fn render_chunk(chunk: &ChunkData, palette: &Palette, ymax: i32) -> Vec<[u8; 3]> {
        render_chunk_impl(chunk, palette, ymax, true)
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
        let chunks_per_tile = crate::CHUNKS_PER_TILE >> zoom;
        if chunks_per_tile < 1 {
            return Err("zoom out of range".into());
        }
        let chunk_size_px = crate::TILE_SIZE as i32 / chunks_per_tile;
        let world_chunk_x = tile_x * chunks_per_tile;
        let world_chunk_z = tile_row * chunks_per_tile;

        let mut img = vec![0u8; (crate::TILE_SIZE * crate::TILE_SIZE * 4) as usize];
        for cz in 0..chunks_per_tile {
            for cx in 0..chunks_per_tile {
                let gcx = world_chunk_x + cx;
                let gcz = world_chunk_z + cz;
                if let Some(chunk) = load_chunk(region_dir, gcx, gcz)? {
                    let buf = render_chunk_impl(&chunk, palette, ymax, true);
                    let px0 = (cx * chunk_size_px) as usize;
                    let py0 = (cz * chunk_size_px) as usize;
                    let span = chunk_size_px as usize;
                    for zz in 0..span {
                        for xx in 0..span {
                            let sx = xx * 16 / span.max(1);
                            let sz = zz * 16 / span.max(1);
                            let rgb = buf[sz.min(15) * 16 + sx.min(15)];
                            if rgb == [0, 0, 0] {
                                continue;
                            }
                            let px = px0 + xx;
                            let py = py0 + zz;
                            let idx = (py * crate::TILE_SIZE as usize + px) * 4;
                            img[idx] = rgb[0];
                            img[idx + 1] = rgb[1];
                            img[idx + 2] = rgb[2];
                            img[idx + 3] = 255;
                        }
                    }
                }
            }
        }
        crate::encode_png(&img, crate::TILE_SIZE, crate::TILE_SIZE)
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
