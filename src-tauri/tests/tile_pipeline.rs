//! End-to-end tile pipeline test: renders real tiles to disk so we can visually inspect.

use std::path::PathBuf;

use world_viewer_lib::testing;

#[test]
fn render_full_tile_png_for_real_world() {
    let dir = PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本");
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let world = testing::open_world(&dir).expect("open world");
    let region_dir = dir.join("region");

    // Player is around X=-415, Z=-286 -> chunk (-26, -18)
    // z=0 tile covers 16 chunks -> tile x = -26/16 = -2, row = -18/16 = -2
    let tile = testing::render_tile(
        &world.palette,
        &region_dir,
        0,   // dim
        0,   // zoom
        -2,  // tile x
        -2,  // tile row
        255, // ymax
    )
    .expect("render tile");

    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("tiles");
    std::fs::create_dir_all(&out_dir).unwrap();
    let out_path = out_dir.join("dim0_z0_x-2_y-2.png");
    std::fs::write(&out_path, &tile).unwrap();

    // PNG must be valid and non-trivial
    assert!(tile.len() > 1000, "tile suspiciously small: {} bytes", tile.len());
    let decoder = png::Decoder::new(&tile[..]);
    let mut reader = decoder.read_info().expect("valid png");
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(info.width, 256);
    assert_eq!(info.height, 256);
    // Count non-transparent pixels
    let opaque = buf.chunks(4).filter(|p| p[3] > 0).count();
    assert!(
        opaque > 5000,
        "tile mostly empty: {} opaque pixels of 65536; wrote {}",
        opaque,
        out_path.display()
    );
    eprintln!("wrote {} ({} bytes, {} opaque px)", out_path.display(), tile.len(), opaque);
}

#[test]
fn render_cave_slice_differs_from_surface() {
    let dir = PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本");
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let world = testing::open_world(&dir).expect("open world");
    let region_dir = dir.join("region");

    let surface = testing::render_tile(&world.palette, &region_dir, 0, 0, -2, -2, 255).unwrap();
    let cave = testing::render_tile(&world.palette, &region_dir, 0, 0, -2, -2, 30).unwrap();
    assert_ne!(
        surface, cave,
        "Y<=30 slice must differ from full-height render"
    );
    eprintln!("surface {} bytes vs cave {} bytes", surface.len(), cave.len());
}

/// A tile whose own area is empty must be flagged empty even when its
/// hillshading margin overlaps generated chunks. Getting this wrong makes the
/// frontend show a fully transparent tile as if it were still loading.
#[test]
fn empty_tile_is_flagged_even_with_populated_margin() {
    let dir = PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本");
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let world = testing::open_world(&dir).expect("open world");
    let region_dir = dir.join("region");

    // Find one tile with data and one without, at the same zoom, and assert
    // the flag matches what the PNG actually contains.
    let mut saw_data = false;
    let mut saw_empty = false;
    for tx in -4..=2 {
        for ty in -8..=-1 {
            let (png, has_data) = testing::render_tile_with_data_flag(
                &world.palette,
                &region_dir,
                2,
                tx,
                ty,
                255,
            )
            .unwrap();
            let opaque = count_opaque(&png);
            if has_data {
                assert!(
                    opaque > 0,
                    "tile ({},{}) flagged has_data but PNG has no opaque pixels",
                    tx,
                    ty
                );
                saw_data = true;
            } else {
                assert_eq!(
                    opaque, 0,
                    "tile ({},{}) flagged empty but PNG has {} opaque pixels",
                    tx, ty, opaque
                );
                saw_empty = true;
            }
        }
    }
    assert!(saw_data, "expected at least one populated tile in range");
    assert!(saw_empty, "expected at least one empty tile in range");
    eprintln!("has_data flag matches PNG contents for both populated and empty tiles");
}

fn count_opaque(png: &[u8]) -> usize {
    let decoder = png::Decoder::new(png);
    let mut reader = decoder.read_info().expect("valid png");
    let mut buf = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut buf).unwrap();
    buf.chunks(4).filter(|p| p[3] > 0).count()
}

/// Relief shading must produce visible variation in real terrain.
/// Without it, flat plains collapse to a single flat colour (the reported bug).
#[test]
fn relief_shading_produces_height_variation() {
    let dir = PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本");
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let world = testing::open_world(&dir).expect("open world");
    let region_dir = dir.join("region");
    let png = testing::render_tile(&world.palette, &region_dir, 0, 0, -2, -2, 255).unwrap();

    let decoder = png::Decoder::new(&png[..]);
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut buf).unwrap();

    // Count distinct colours among opaque pixels: relief shading must create
    // many shades of the same base block colour.
    let mut colors = std::collections::HashSet::new();
    for px in buf.chunks(4) {
        if px[3] > 0 {
            colors.insert([px[0], px[1], px[2]]);
        }
    }
    assert!(
        colors.len() > 400,
        "expected rich shading variation, got only {} distinct colours",
        colors.len()
    );

    // Luminance spread must be non-trivial (i.e. not one flat value).
    let lums: Vec<u32> = buf
        .chunks(4)
        .filter(|p| p[3] > 0)
        .map(|p| (p[0] as u32 * 30 + p[1] as u32 * 59 + p[2] as u32 * 11) / 100)
        .collect();
    let min = *lums.iter().min().unwrap();
    let max = *lums.iter().max().unwrap();
    assert!(
        max - min > 60,
        "luminance range too flat: {}..{} (relief shading not applied?)",
        min,
        max
    );
    eprintln!(
        "relief check: {} distinct colours, luminance {}..{}",
        colors.len(),
        min,
        max
    );
}
