//! Measures whether concurrent tile requests actually run in parallel, or
//! serialize behind the single `Mutex<TileCache>`.
//!
//! If the backend serializes, wall time for N concurrent tiles equals N times
//! one tile. If it parallelizes, wall time stays near one tile until the cores
//! saturate.
//!   cargo test --release --test perf_concurrency -- --nocapture

use std::path::PathBuf;
use std::sync::{Arc, Barrier, Mutex};
use std::time::Instant;

use world_viewer_lib::testing;

fn save_dir() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\Aegis of the Frozen Sky\saves\新的世界")
}

/// A stand-in for `AppState`: the same `Mutex<Option<Arc<TileCache>>>` shape the
/// Tauri commands use, so the locking behaviour under test is the real one.
struct Shared {
    cache: Mutex<Option<Arc<testing::TileCache>>>,
}

#[test]
fn profile_concurrency() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let world = testing::open_world(&dir).expect("open world");
    let region_dir = dir.join("region");

    // Tiles far enough apart that they share no chunks, so this measures
    // scheduling, not cache sharing.
    let tiles: Vec<(i32, i32)> = (0..6).map(|i| (-2 + i, -2)).collect();

    let shared = Arc::new(Shared {
        cache: Mutex::new(Some(Arc::new(testing::new_tile_cache(
            world.palette.clone(),
            4096,
        )))),
    });

    // --- sequential: one tile at a time ---
    let t = Instant::now();
    for &(x, y) in &tiles {
        let cache = shared.cache.lock().unwrap().clone().unwrap();
        let _ = testing::render_tile_in(&cache, &region_dir, 0, x, y, 255).unwrap();
    }
    let seq_ms = t.elapsed().as_secs_f64() * 1000.0;

    // --- concurrent: all at once through the shared state ---
    shared.cache.lock().unwrap().as_ref().unwrap().invalidate();
    let barrier = Arc::new(Barrier::new(tiles.len()));
    let t = Instant::now();
    std::thread::scope(|scope| {
        for &(x, y) in &tiles {
            let shared = Arc::clone(&shared);
            let region_dir = region_dir.clone();
            let barrier = Arc::clone(&barrier);
            scope.spawn(move || {
                barrier.wait();
                // Same pattern as `render_tile_png`: clone the Arc and drop the
                // lock, so only the cache's own map access is serialized.
                let cache = shared.cache.lock().unwrap().clone().unwrap();
                let _ = testing::render_tile_in(&cache, &region_dir, 0, x, y, 255).unwrap();
            });
        }
    });
    let par_ms = t.elapsed().as_secs_f64() * 1000.0;

    let n = tiles.len() as f64;
    eprintln!("cores available        : {}", std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1));
    eprintln!("tiles                  : {}", tiles.len());
    eprintln!("sequential             : {:>7.1} ms  ({:.1} ms/tile)", seq_ms, seq_ms / n);
    eprintln!("concurrent (6 threads) : {:>7.1} ms  ({:.1} ms/tile)", par_ms, par_ms / n);
    eprintln!("speedup                : {:.2}x", seq_ms / par_ms);
    eprintln!(
        "=> if speedup ~1.0 the shared lock serializes rendering; \
         if >1 the work already runs in parallel"
    );
}

/// Rendering must not depend on what other threads are doing.
///
/// `acquire` releases the cache lock while loading, so a chunk can be evicted
/// before it is handed out. If that happened, the tile would silently render
/// with holes — the same coordinates would produce a different image depending
/// on timing. This renders the same tile from many threads at once, under cache
/// pressure, and requires every result to be byte-identical to the single
/// threaded render.
#[test]
fn concurrent_render_is_deterministic() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let world = testing::open_world(&dir).expect("open world");
    let region_dir = dir.join("region");

    // A capacity far below one tile's chunk count (~324), so every acquire has
    // to evict and the race window is hit as often as possible.
    let cache = Arc::new(testing::new_tile_cache(world.palette.clone(), 64));

    let tile = (0i32, 0i32);
    let reference = testing::render_tile_in(&cache, &region_dir, 0, tile.0, tile.1, 255)
        .expect("reference render");

    let cache = Arc::new(testing::new_tile_cache(world.palette.clone(), 64));
    let mismatches = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(8));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let cache = Arc::clone(&cache);
            let region_dir = region_dir.clone();
            let mismatches = Arc::clone(&mismatches);
            let barrier = Arc::clone(&barrier);
            let reference = reference.clone();
            scope.spawn(move || {
                barrier.wait();
                for _ in 0..3 {
                    let got = testing::render_tile_in(&cache, &region_dir, 0, tile.0, tile.1, 255)
                        .expect("concurrent render");
                    if got != reference {
                        mismatches.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            });
        }
    });

    let bad = mismatches.load(std::sync::atomic::Ordering::Relaxed);
    eprintln!("mismatching renders: {} / 24", bad);
    assert_eq!(bad, 0, "concurrent renders differed from the reference");
}
