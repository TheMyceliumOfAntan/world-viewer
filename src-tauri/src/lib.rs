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

    render::render_tile(cache, &region_dir, z, x, row, ymax)
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
    pub use crate::render::ChunkData;
    pub use crate::world::World;

    use std::path::{Path, PathBuf};

    pub fn open_world(save_dir: &Path) -> Result<World, String> {
        crate::world::open_world(save_dir, None)
    }

    pub fn load_chunk(region_dir: &Path, cx: i32, cz: i32) -> Result<Option<ChunkData>, String> {
        crate::render::load_chunk(region_dir, cx, cz)
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
        let mut cache = crate::render::TileCache::new(palette.clone(), 512);
        crate::render::render_tile(&mut cache, region_dir, zoom, tile_x, tile_row, ymax)
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
