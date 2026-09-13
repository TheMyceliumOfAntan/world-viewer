//! Stage-by-stage timing to find where the 2.2 ms/chunk goes.
//!   cargo test --test perf_stages -- --nocapture

use std::path::PathBuf;
use std::time::Instant;

fn save_dir() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本")
}

#[test]
fn profile_stages() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let region_dir = dir.join("region");

    // pick 30 real chunks from r.0.0.mca
    let mut coords = Vec::new();
    for cx in 0..8 {
        for cz in 0..8 {
            coords.push((cx, cz));
        }
    }

    // stage 1: open + read + decompress
    let t = Instant::now();
    let mut raws = Vec::new();
    for &(cx, cz) in &coords {
        if let Ok(Some(raw)) = world_viewer_lib::testing::read_chunk_nbt(&region_dir, cx, cz) {
            raws.push(raw);
        }
    }
    let s1 = t.elapsed().as_secs_f64() * 1000.0;

    // stage 2: full NBT parse
    let t = Instant::now();
    let mut parsed = 0;
    for raw in &raws {
        if world_viewer_lib::testing::parse_nbt(raw).is_ok() {
            parsed += 1;
        }
    }
    let s2 = t.elapsed().as_secs_f64() * 1000.0;

    let n = raws.len().max(1) as f64;
    let total_mb: f64 = raws.iter().map(|r| r.len() as f64).sum::<f64>() / 1_048_576.0;
    eprintln!("chunks sampled          : {}", raws.len());
    eprintln!("decompressed total      : {:.1} MB", total_mb);
    eprintln!(
        "1) open+read+decompress  : {:>7.1} ms total  ({:.3} ms/chunk)",
        s1,
        s1 / n
    );
    eprintln!(
        "2) full NBT parse        : {:>7.1} ms total  ({:.3} ms/chunk)  [{} parsed]",
        s2,
        s2 / n,
        parsed
    );
    eprintln!(
        "   => parse is {:.0}% of the per-chunk cost",
        100.0 * s2 / (s1 + s2)
    );
}
