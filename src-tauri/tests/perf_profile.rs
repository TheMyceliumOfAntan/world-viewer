//! Timing profile for the tile pipeline. Run with:
//!   cargo test --test perf_profile -- --nocapture

use std::path::PathBuf;
use std::time::Instant;

use world_viewer_lib::testing;

fn save_dir() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本")
}

#[test]
fn profile_tile_pipeline() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let world = testing::open_world(&dir).expect("open world");
    let region_dir = dir.join("region");

    // --- single chunk (cold) ---
    let t = Instant::now();
    let c = testing::load_chunk(&region_dir, 1, 1).unwrap();
    let chunk_ms = t.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "1 chunk cold load+parse : {:>8.2} ms  (present={})",
        chunk_ms,
        c.is_some()
    );

    // --- z=0 tile (16x16 chunks + 1 margin = 324 chunks) ---
    let t = Instant::now();
    let z0 = testing::render_tile(&world.palette, &region_dir, 0, 0, -2, -2, 255).unwrap();
    let z0_ms = t.elapsed().as_secs_f64() * 1000.0;
    eprintln!("z=0 tile (324 chunks)   : {:>8.2} ms  ({} bytes)", z0_ms, z0.len());

    // --- z=4 tile (1 chunk + margin = 9 chunks) ---
    let t = Instant::now();
    let z4 = testing::render_tile(&world.palette, &region_dir, 0, 4, -30, -30, 255).unwrap();
    let z4_ms = t.elapsed().as_secs_f64() * 1000.0;
    eprintln!("z=4 tile (9 chunks)     : {:>8.2} ms  ({} bytes)", z4_ms, z4.len());

    // --- a viewport's worth: 16 z=0 tiles through one shared cache ---
    // Sharing the cache is the point: separate caches per tile would measure
    // 16 independent cold renders and hide the effect of cache reuse between
    // neighbouring tiles. Capacity matches production (8192).
    let cache = testing::new_tile_cache(world.palette.clone(), 8192);
    let t = Instant::now();
    let mut total = 0usize;
    for tx in -4..0 {
        for ty in -4..0 {
            let (p, _) =
                testing::render_tile_in(&cache, &region_dir, 0, tx, ty, 255).unwrap();
            total += p.len();
        }
    }
    let batch_ms = t.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "16 z=0 tiles (viewport) : {:>8.2} ms total, {:.1} ms/tile avg ({} bytes)",
        batch_ms,
        batch_ms / 16.0,
        total
    );

    eprintln!(
        "\nNOTE: a z=0 tile needs {} chunks and a viewport spans ~6500, so the",
        18 * 18
    );
    eprintln!("server cache is sized at 8192 (CHUNK_CACHE_CAPACITY) to hold one screen.");
}
