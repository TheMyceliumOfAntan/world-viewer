//! Measures how much memory one cached chunk costs, so the cache capacity can
//! be chosen against a real number instead of a guess.
//!   cargo test --release --test perf_chunk_memory -- --nocapture

use std::path::PathBuf;

use world_viewer_lib::testing;

fn save_dir() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\Aegis of the Frozen Sky\saves\新的世界")
}

#[test]
fn profile_chunk_memory() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let _world = testing::open_world(&dir).expect("open world");
    let region_dir = dir.join("region");

    // Load a solid block of real chunks.
    let mut loaded = Vec::new();
    for cz in 0..32 {
        for cx in 0..32 {
            if let Ok(Some(c)) = testing::load_chunk(&region_dir, cx, cz) {
                loaded.push(c);
            }
        }
    }
    assert!(!loaded.is_empty(), "no chunks loaded");

    // Count the bytes the chunk data actually owns. Vec capacity is what the
    // allocator reserved, which is the number that matters for memory.
    let bs_bytes = |bs: &testing::BlockStates| -> usize {
        let pal: usize = bs.palette.iter().map(|s| s.capacity()).sum();
        pal + bs.data.as_ref().map(|d| d.capacity() * 8).unwrap_or(0)
    };
    let lb_bytes = |lb: &testing::LegacyBiomes| -> usize {
        match lb {
            testing::LegacyBiomes::Columns(v) => v.capacity(),
            testing::LegacyBiomes::Grid(v) => v.capacity() * 2,
        }
    };

    let mut total = 0usize;
    let mut sections = 0usize;
    let mut nonempty_sections = 0usize;
    for c in &loaded {
        for s in &c.sections {
            sections += 1;
            let mut section_bytes = 0usize;
            for v in [&s.blocks16, &s.blocks, &s.data16, &s.data, &s.add]
                .into_iter()
                .flatten()
            {
                section_bytes += v.capacity();
            }
            if let Some(bs) = &s.block_states {
                section_bytes += bs_bytes(bs);
            }
            if let Some(b) = &s.biomes {
                section_bytes += bs_bytes(b);
            }
            if section_bytes > 0 {
                nonempty_sections += 1;
            }
            total += section_bytes;
        }
        if let Some(lb) = &c.legacy_biomes {
            total += lb_bytes(lb);
        }
    }

    let n = loaded.len() as f64;
    let per_chunk = total as f64 / n;
    eprintln!("chunks loaded        : {}", loaded.len());
    eprintln!(
        "sections             : {} total, {} with data ({:.1}/chunk)",
        sections,
        nonempty_sections,
        nonempty_sections as f64 / n
    );
    eprintln!("payload bytes        : {:.1} MB", total as f64 / 1_048_576.0);
    eprintln!("per chunk            : {:.1} KiB", per_chunk / 1024.0);

    // A z=0 viewport needs ~6724 chunks (measured in perf_sweep).
    for cap in [4096usize, 8192, 12288, 16384] {
        let mb = per_chunk * cap as f64 / 1_048_576.0;
        eprintln!("capacity {:>6} -> {:>7.1} MB payload", cap, mb);
    }
}
