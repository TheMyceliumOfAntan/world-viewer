//! Measures how much of a tile render is spent resolving palette colours.
//!
//! Implement-3 (conic-style per-chunk colour pre-resolution cache) is only
//! worth adding if `Palette::color_ref` is actually a hot path. This probe
//! times a real z=0 tile, then replays the same column scan and colour lookups
//! in isolation, so the two numbers bound the win.
//!   cargo test --release --test perf_palette -- --nocapture

use std::path::PathBuf;
use std::time::Instant;

use world_viewer_lib::testing;

fn save_dir() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本")
}

#[test]
fn profile_palette_resolution() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let world = testing::open_world(&dir).expect("open world");
    let region_dir = dir.join("region");
    let cache = testing::new_tile_cache(world.palette.clone(), 8192);

    // Warm, then measure a real render end to end.
    let _ = testing::render_tile_in(&cache, &region_dir, 0, -2, -2, 255).unwrap();
    let t = Instant::now();
    for _ in 0..5 {
        let _ = testing::render_tile_in(&cache, &region_dir, 0, -2, -2, 255).unwrap();
    }
    let render_ms = t.elapsed().as_secs_f64() * 1000.0 / 5.0;

    // Replay one tile's worth of chunks (16x16 + 1 margin, as `fill` does) and
    // time only the colour lookups, to bound what a per-chunk cache could save.
    // Chunks are loaded first so disk IO and NBT parsing stay out of the timing.
    let scan_opts = testing::ScanOpts { water: true };
    let mut chunks = Vec::new();
    for cz in -1..=16 {
        for cx in -1..=16 {
            if let Ok(Some(chunk)) = testing::load_chunk(&region_dir, -32 + cx, -32 + cz) {
                chunks.push(chunk);
            }
        }
    }
    let mut lookups = 0usize;
    // A: scan only. The block ref is fed to black_box so the scan is not
    // optimised away, but no palette lookup happens.
    let t = Instant::now();
    for chunk in &chunks {
        for lz in 0..16usize {
            for lx in 0..16usize {
                let scan = chunk.scan_column(lx, lz, 255, &scan_opts);
                if let Some((block, _y)) = scan.block {
                    std::hint::black_box(block);
                    lookups += 1;
                }
            }
        }
    }
    let scan_ms = t.elapsed().as_secs_f64() * 1000.0;

    // B: scan + colour resolution. B - A is the palette cost.
    let t = Instant::now();
    for chunk in &chunks {
        for lz in 0..16usize {
            for lx in 0..16usize {
                let scan = chunk.scan_column(lx, lz, 255, &scan_opts);
                let Some((block, _y)) = scan.block else { continue };
                let _ = std::hint::black_box(world.palette.color_ref(&block.to_owned_ref()));
                if let Some((fb, _fy)) = scan.floor_block {
                    let _ =
                        std::hint::black_box(world.palette.color_ref(&fb.to_owned_ref()));
                }
            }
        }
    }
    let both_ms = t.elapsed().as_secs_f64() * 1000.0;
    let resolve_ms = both_ms - scan_ms;

    eprintln!("render (warm, per tile) : {:>8.2} ms", render_ms);
    eprintln!("A) column scan only     : {:>8.2} ms  ({} columns)", scan_ms, lookups);
    eprintln!("B) scan + colour        : {:>8.2} ms", both_ms);
    eprintln!(
        "=> colour resolution    : {:>8.2} ms  ({:.1}% of render)",
        resolve_ms,
        100.0 * resolve_ms / render_ms
    );
}
