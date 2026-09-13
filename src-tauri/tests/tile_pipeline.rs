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
