//! Correctness + speed of the targeted chunk parser vs the generic one.

use std::path::PathBuf;
use std::time::Instant;

use world_viewer_lib::testing;

fn save_dir() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本")
}

/// The fast parser must produce byte-identical sections to the generic one.
#[test]
fn fast_parser_matches_generic_parser() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let region_dir = dir.join("region");

    let mut compared = 0;
    for cx in 0..8 {
        for cz in 0..8 {
            let raw = match testing::read_chunk_nbt(&region_dir, cx, cz) {
                Ok(Some(r)) => r,
                _ => continue,
            };
            let generic = testing::parse_sections_generic(&raw)
                .unwrap_or_else(|e| panic!("generic parse ({},{}) failed: {}", cx, cz, e));
            let fast = testing::parse_sections_fast(&raw)
                .unwrap_or_else(|e| panic!("fast parse ({},{}) failed: {}", cx, cz, e));

            assert_eq!(
                generic.len(),
                fast.len(),
                "section count differs at ({},{})",
                cx,
                cz
            );
            for (g, f) in generic.iter().zip(fast.iter()) {
                assert_eq!(g.y, f.y, "section Y differs at ({},{})", cx, cz);
                assert_eq!(
                    g.blocks16, f.blocks16,
                    "Blocks16 differs at ({},{}) y={}",
                    cx, cz, g.y
                );
                assert_eq!(
                    g.blocks, f.blocks,
                    "Blocks differs at ({},{}) y={}",
                    cx, cz, g.y
                );
                assert_eq!(
                    g.data16, f.data16,
                    "Data16 differs at ({},{}) y={}",
                    cx, cz, g.y
                );
                assert_eq!(g.data, f.data, "Data differs at ({},{}) y={}", cx, cz, g.y);
                assert_eq!(g.add, f.add, "Add differs at ({},{}) y={}", cx, cz, g.y);
            }
            compared += 1;
        }
    }
    assert!(compared > 20, "expected to compare real chunks, got {}", compared);
    eprintln!("compared {} chunks: generic == fast", compared);
}

#[test]
fn fast_parser_is_faster() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let region_dir = dir.join("region");

    let mut raws = Vec::new();
    for cx in 0..8 {
        for cz in 0..8 {
            if let Ok(Some(r)) = testing::read_chunk_nbt(&region_dir, cx, cz) {
                raws.push(r);
            }
        }
    }
    if raws.is_empty() {
        eprintln!("SKIP: no chunks read");
        return;
    }
    let n = raws.len() as f64;

    let t = Instant::now();
    for raw in &raws {
        let _ = testing::parse_sections_generic(raw);
    }
    let generic_ms = t.elapsed().as_secs_f64() * 1000.0;

    let t = Instant::now();
    for raw in &raws {
        let _ = testing::parse_sections_fast(raw);
    }
    let fast_ms = t.elapsed().as_secs_f64() * 1000.0;

    eprintln!(
        "parse {} chunks: generic {:>7.1} ms ({:.3} ms/chunk) | fast {:>7.1} ms ({:.3} ms/chunk) | speedup {:.1}x",
        raws.len(),
        generic_ms,
        generic_ms / n,
        fast_ms,
        fast_ms / n,
        generic_ms / fast_ms
    );
    assert!(
        fast_ms < generic_ms,
        "fast parser should be faster: {} vs {}",
        fast_ms,
        generic_ms
    );
}
