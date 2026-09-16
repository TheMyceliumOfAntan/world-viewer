//! Isolates the cost of opening a region file and reading its 8 KiB header,
//! which the old `read_chunk_nbt` paid once per chunk.
//!
//! The control arm reimplements the old algorithm inline (open + seek + read
//! per chunk) rather than calling the cached production path, so the two arms
//! really differ in the thing being measured.
//!   cargo test --release --test perf_region_io -- --nocapture

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Instant;

use flate2::read::{GzDecoder, ZlibDecoder};

fn save_dir() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本")
}

/// The old implementation: reopen the file and re-read the 8 KiB header for
/// every single chunk.
fn old_read_chunk_nbt(region_dir: &Path, cx: i32, cz: i32) -> Result<Option<Vec<u8>>, String> {
    let path = region_dir.join(format!("r.{}.{}.mca", cx >> 5, cz >> 5));
    let mut f = match File::open(&path) {
        Ok(f) => f,
        Err(_) => return Ok(None),
    };
    let mut header = [0u8; 8192];
    if f.read_exact(&mut header).is_err() {
        return Ok(None);
    }
    let lx = (cx & 31) as usize;
    let lz = (cz & 31) as usize;
    let idx = (lx + lz * 32) * 4;
    let offset = ((header[idx] as usize) << 16 | (header[idx + 1] as usize) << 8
        | header[idx + 2] as usize)
        * 4096;
    if offset == 0 {
        return Ok(None);
    }
    f.seek(SeekFrom::Start(offset as u64))
        .map_err(|e| e.to_string())?;
    let mut len_buf = [0u8; 5];
    if f.read_exact(&mut len_buf).is_err() {
        return Ok(None);
    }
    let len = u32::from_be_bytes([len_buf[0], len_buf[1], len_buf[2], len_buf[3]]) as usize;
    if len <= 1 {
        return Ok(None);
    }
    let comp = len_buf[4];
    let mut raw = vec![0u8; len - 1];
    if f.read_exact(&mut raw).is_err() {
        return Ok(None);
    }
    match comp {
        1 => {
            let mut out = Vec::new();
            GzDecoder::new(&raw[..])
                .read_to_end(&mut out)
                .map_err(|e| e.to_string())?;
            Ok(Some(out))
        }
        2 => {
            let mut out = Vec::new();
            ZlibDecoder::new(&raw[..])
                .read_to_end(&mut out)
                .map_err(|e| e.to_string())?;
            Ok(Some(out))
        }
        3 => Ok(Some(raw)),
        other => Err(format!("unknown region compression {}", other)),
    }
}

#[test]
fn profile_region_io() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let region_dir = dir.join("region");

    // One z=0 tile's worth of chunks: 18x18 with the 1-chunk margin.
    let mut coords = Vec::new();
    for cz in -1..17 {
        for cx in -1..17 {
            coords.push((cx, cz));
        }
    }

    let mut regions: Vec<(i32, i32)> = coords.iter().map(|&(x, z)| (x >> 5, z >> 5)).collect();
    regions.sort();
    regions.dedup();

    // Warm the OS page cache so this measures our own overhead, not cold disk.
    for &(cx, cz) in &coords {
        let _ = old_read_chunk_nbt(&region_dir, cx, cz);
    }

    // A) old path: open + read header per chunk
    let t = Instant::now();
    let mut hit = 0;
    for &(cx, cz) in &coords {
        if let Ok(Some(_)) = old_read_chunk_nbt(&region_dir, cx, cz) {
            hit += 1;
        }
    }
    let old_ms = t.elapsed().as_secs_f64() * 1000.0;

    // B) production path: cached handle + cached header
    world_viewer_lib::testing::clear_region_cache();
    let t = Instant::now();
    let mut hit2 = 0;
    for &(cx, cz) in &coords {
        if let Ok(Some(_)) = world_viewer_lib::testing::read_chunk_nbt(&region_dir, cx, cz) {
            hit2 += 1;
        }
    }
    let new_ms = t.elapsed().as_secs_f64() * 1000.0;

    let n = coords.len() as f64;
    eprintln!("chunks                  : {}", coords.len());
    eprintln!("distinct region files   : {}", regions.len());
    eprintln!(
        "A) old: open+header/chunk: {:>7.1} ms  ({:.3} ms/chunk, {} hits)",
        old_ms,
        old_ms / n,
        hit
    );
    eprintln!(
        "B) new: cached handle     : {:>7.1} ms  ({:.3} ms/chunk, {} hits)",
        new_ms,
        new_ms / n,
        hit2
    );
    eprintln!(
        "=> saving: {:.1} ms ({:.0}%)",
        old_ms - new_ms,
        100.0 * (old_ms - new_ms) / old_ms
    );
    assert_eq!(hit, hit2, "both paths must see the same chunks");
}

/// The cached-handle reader must return byte-identical data to the original
/// open-and-seek implementation, or the tile cache would render different
/// pixels than before the change.
#[test]
fn cached_reader_matches_original_bytes() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let region_dir = dir.join("region");

    // Clear the handle cache so this exercises the open path too, then check
    // both a cold and a warm read (cached handle) against the original.
    world_viewer_lib::testing::clear_region_cache();

    let mut compared = 0;
    for cz in -8..8 {
        for cx in -8..8 {
            let old = old_read_chunk_nbt(&region_dir, cx, cz).unwrap();
            let new = world_viewer_lib::testing::read_chunk_nbt(&region_dir, cx, cz).unwrap();
            assert_eq!(old, new, "chunk ({}, {}) differs", cx, cz);
            compared += 1;
        }
    }
    eprintln!("byte-identical chunks   : {}", compared);
    assert!(compared > 0);
}
