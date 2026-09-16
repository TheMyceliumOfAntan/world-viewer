//! Reproduces the CDP measurement's access pattern against the backend only,
//! with cache hit/miss/eviction counters, so the "second sweep is slower than
//! the first" anomaly can be attributed to a cause instead of guessed at.
//!   cargo test --release --test perf_sweep -- --nocapture

use std::path::PathBuf;
use std::time::Instant;

use world_viewer_lib::testing;

fn save_dir() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\Aegis of the Frozen Sky\saves\新的世界")
}

/// The same 3x3 grid of tile-aligned areas the CDP script uses.
const GRID: [i32; 3] = [-512, 0, 512];

/// One viewport's worth of tiles around an area, at z=0 (256 blocks/tile).
/// 5x5 tiles is a bit more than a 1360x860 window shows.
fn tiles_around(ax: i32, az: i32) -> Vec<(i32, i32)> {
    let tx = ax / 256;
    let ty = az / 256;
    let mut v = Vec::new();
    for dy in -2..=2 {
        for dx in -2..=2 {
            v.push((tx + dx, ty + dy));
        }
    }
    v
}

#[test]
fn profile_sweep() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let world = testing::open_world(&dir).expect("open world");
    let region_dir = dir.join("region");

    let areas: Vec<(i32, i32)> = GRID
        .iter()
        .flat_map(|&z| GRID.iter().map(move |&x| (x, z)))
        .collect();

    // Sweep capacity to find where the working set stops thrashing. Per chunk
    // is ~9.7 KiB, so capacity x 9.7 KiB is the memory cost.
    for capacity in [4096usize, 8192, 12288, 16384, 24576] {
        let cache = testing::new_tile_cache(world.palette.clone(), capacity);
        let run = |areas: &[(i32, i32)]| -> f64 {
            let mut total = 0.0f64;
            for &(ax, az) in areas {
                let t = Instant::now();
                for &(tx, ty) in &tiles_around(ax, az) {
                    let _ = testing::render_tile_in(&cache, &region_dir, 0, tx, ty, 255).unwrap();
                }
                total += t.elapsed().as_secs_f64() * 1000.0;
            }
            total
        };

        // Warm-up so both sweeps start from a settled cache.
        let _ = testing::render_tile_in(&cache, &region_dir, 0, 0, 0, 255);
        let cold = run(&areas);
        let (h, m, e, resident) = cache.stats();
        let warm = run(&areas);
        let (h2, m2, e2, _) = cache.stats();
        eprintln!(
            "cap {:>6} ({:>5.0} MB)  cold {:>7.1}  repeat {:>7.1}  ratio {:.2}x  \
             hit {:.0}%  evict {:>6}  resident {:>5}",
            capacity,
            capacity as f64 * 9.7 / 1024.0,
            cold,
            warm,
            warm / cold,
            100.0 * (h2 - h) as f64 / ((h2 - h) + (m2 - m)).max(1) as f64,
            e2 - e,
            resident
        );
    }
}
