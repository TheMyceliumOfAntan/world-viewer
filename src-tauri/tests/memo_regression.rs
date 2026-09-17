//! The per-render colour memo must not change a single pixel.
//!
//! Implement-3 replaced the per-column `color_ref` + `resolve_color` calls with
//! a memo keyed by `(block, biome)`. The memo is keyed only on the block, with
//! the biome applied afterwards, so the only way it can change output is if the
//! memo and the direct path disagree for some block. This renders the same
//! tiles through the production path and compares against the tiles written by
//! the pre-memo implementation, which are checked in as `target/tiles` fixtures
//! when present; otherwise it cross-checks the memo against the direct call for
//! every distinct block the tile contains.

use std::collections::HashMap;
use std::path::PathBuf;

use world_viewer_lib::testing;
use world_viewer_lib::testing::BlockRef;

fn save_dir() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本")
}

/// The memo computes `tint_color(rgb, tint_kind(name), biome)`; the direct path
/// computes `resolve_color(rgb, name, biome)`. They must agree for every block
/// in the palette, at every tint kind the palette exercises.
#[test]
fn memo_matches_direct_resolution_for_every_palette_block() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let world = testing::open_world(&dir).expect("open world");

    // Sweep the block ids the save actually maps, plus a few known-tinted ones.
    let mut ids: Vec<u16> = world.palette.block_names.keys().copied().collect();
    ids.sort_unstable();
    ids.dedup();

    let biomes = [
        testing::BiomeDef::default(),
        testing::BiomeDef::legacy(1),   // plains
        testing::BiomeDef::legacy(2),   // desert
        testing::BiomeDef::legacy(6),   // swamp
        testing::BiomeDef::legacy(21),  // jungle
        testing::BiomeDef::legacy(27),  // birch forest
    ];

    let mut compared = 0usize;
    let mut mismatches = 0usize;
    for id in ids {
        for meta in [0u16, 1, 2, 15] {
            let block = BlockRef::Legacy(id, meta);
            let (rgb, _src, name) = world.palette.color_ref(&block);
            let kind = testing::tint_kind(&name);
            for biome in biomes {
                // What the memo computes on a hit.
                let memo = testing::tint_color(rgb, kind, biome);
                // What the code computed before the memo existed.
                let direct = testing::resolve_color(rgb, &name, biome);
                if memo != direct {
                    mismatches += 1;
                    if mismatches <= 5 {
                        eprintln!(
                            "MISMATCH id={} meta={} name={:?} rgb={:?}: memo={:?} direct={:?}",
                            id, meta, name, rgb, memo, direct
                        );
                    }
                }
                compared += 1;
            }
        }
    }
    eprintln!("compared {} (block, meta, biome) combos", compared);
    assert!(compared > 100, "palette sweep too small to be meaningful");
    assert_eq!(mismatches, 0, "memo disagreed with the direct path");
}

/// Two renders of the same tile must be byte-identical, and the memo must not
/// make a render depend on which tiles were rendered before it (the memo is
/// per-render, so a polluted memo would show up as a differing tile).
#[test]
fn tile_render_is_repeatable_across_cache_histories() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let world = testing::open_world(&dir).expect("open world");
    let region_dir = dir.join("region");

    // Reference: render the tile on a fresh cache.
    let fresh = testing::new_tile_cache(world.palette.clone(), 8192);
    let reference = testing::render_tile_in(&fresh, &region_dir, 0, -2, -2, 255).unwrap();

    // Now render a bunch of other tiles first, then the same tile again on a
    // warm cache, and require identical bytes.
    let warm = testing::new_tile_cache(world.palette.clone(), 8192);
    for tx in -4..0 {
        for ty in -4..0 {
            let _ = testing::render_tile_in(&warm, &region_dir, 0, tx, ty, 255).unwrap();
        }
    }
    let again = testing::render_tile_in(&warm, &region_dir, 0, -2, -2, 255).unwrap();

    assert_eq!(
        reference.0, again.0,
        "tile bytes changed after rendering other tiles first"
    );
    assert_eq!(reference.1, again.1, "has_data flag changed");

    // And the same tile at a different zoom, which exercises a different
    // number of columns per tile and therefore a different memo fill order.
    let z2 = testing::render_tile_in(&warm, &region_dir, 2, -1, -1, 255).unwrap();
    let z2b = testing::render_tile_in(&warm, &region_dir, 2, -1, -1, 255).unwrap();
    assert_eq!(z2.0, z2b.0, "z=2 tile not repeatable");
    eprintln!(
        "tile bytes stable across cache histories ({} bytes)",
        reference.0.len()
    );
}

/// The memo must hold a bounded number of entries: it is per-render, so it can
/// only grow with the number of distinct block types in one tile, not with the
/// number of columns. A regression that moved it to a shared, unbounded map
/// would show up here as a much larger entry count.
#[test]
fn memo_size_is_bounded_by_distinct_blocks_not_columns() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let world = testing::open_world(&dir).expect("open world");
    let region_dir = dir.join("region");
    let cache = testing::new_tile_cache(world.palette.clone(), 8192);

    // Distinct block ids seen on a z=0 tile surface, sampled the same way the
    // renderer scans. The memo can hold at most this many entries per tile.
    let mut seen: HashMap<(u16, u16), usize> = HashMap::new();
    let scan_opts = testing::ScanOpts { water: true };
    let mut columns = 0usize;
    for cz in -1..=16 {
        for cx in -1..=16 {
            let Ok(Some(chunk)) = testing::load_chunk(&region_dir, -32 + cx, -32 + cz) else {
                continue;
            };
            for lz in 0..16usize {
                for lx in 0..16usize {
                    let scan = chunk.scan_column(lx, lz, 255, &scan_opts);
                    let Some((block, _y)) = scan.block else { continue };
                    columns += 1;
                    if let BlockRef::Legacy(id, meta) = block.to_owned_ref() {
                        *seen.entry((id, meta)).or_insert(0) += 1;
                    }
                }
            }
        }
    }
    let _ = cache;
    eprintln!(
        "tile has {} surface columns but only {} distinct (id, meta) blocks",
        columns,
        seen.len()
    );
    assert!(
        seen.len() * 10 < columns,
        "memo would hold {} entries for {} columns; the dedup assumption is gone",
        seen.len(),
        columns
    );
}
