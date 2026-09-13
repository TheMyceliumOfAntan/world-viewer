//! Integration tests against the real GTNH save (read-only).
//! These skip gracefully when the save is not present (e.g. CI).

use std::path::PathBuf;

use world_viewer_lib::testing;

fn save_dir() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本")
}

fn instance_root() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4")
}

#[test]
fn opens_real_world_and_finds_dimensions() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let world = testing::open_world(&dir).expect("open_world failed");
    assert_eq!(world.dimensions[0].id, 0, "first dimension must be overworld");
    assert!(
        world.dimensions.iter().any(|d| d.id == 112),
        "DIM112 (ExtraUtilities last millennium) must be listed, got: {:?}",
        world
            .dimensions
            .iter()
            .map(|d| d.id)
            .collect::<Vec<_>>()
    );
    for d in &world.dimensions {
        assert!(d.has_data, "dimension {} listed but has no data", d.id);
        assert!(d.chunk_count > 0, "dimension {} has 0 chunks", d.id);
    }
}

#[test]
fn block_id_to_name_mapping_works() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let world = testing::open_world(&dir).expect("open_world failed");
    let p = &world.palette;
    assert_eq!(p.name_of(1), "minecraft:stone");
    assert_eq!(p.name_of(9), "minecraft:water");
    assert_eq!(p.name_of(2289), "etfuturum:deepslate");
    assert_eq!(p.name_of(2711), "gregtech:gt.blockores");
    let (rgb, src, _) = p.color(1, 0);
    assert_eq!(src, "exact", "stone must resolve exactly from JM palette");
    assert_eq!(rgb, [0x7d, 0x7d, 0x7d]);
}

#[test]
fn reads_chunk_sections_and_top_block() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let region_dir = dir.join("region");
    let chunk = testing::load_chunk(&region_dir, 1, 1)
        .expect("read failed")
        .expect("chunk (1,1) must exist in r.0.0.mca");
    assert!(!chunk.sections.is_empty(), "chunk has no sections");
    let mut non_air = 0;
    for z in 0..16 {
        for x in 0..16 {
            if chunk.top_block(x, z, 255).is_some() {
                non_air += 1;
            }
        }
    }
    assert!(non_air > 200, "expected a populated chunk, non_air={}", non_air);
}

#[test]
fn waypoints_and_player_are_loaded() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let world = testing::open_world(&dir).expect("open_world failed");
    assert!(
        !world.waypoints.is_empty(),
        "expected at least one JourneyMap waypoint"
    );
    let wp = &world.waypoints[0];
    assert!(wp.x != 0 || wp.z != 0, "waypoint coordinates look empty");
    let player = world.player.as_ref().expect("player must be in level.dat");
    assert_eq!(player.dimension, 0);
    assert!((player.x + 415.0).abs() < 2.0, "player x={}", player.x);
}

#[test]
fn renders_chunk_to_non_empty_image() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let region_dir = dir.join("region");
    let world = testing::open_world(&dir).expect("open world");
    // chunk (1,1) lives in tile (0,0) at zoom 0
    let png = testing::render_tile(&world.palette, &region_dir, 0, 0, 0, 0, 255)
        .expect("render tile");
    let decoder = png::Decoder::new(&png[..]);
    let mut reader = decoder.read_info().expect("valid png");
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    let opaque = buf.chunks(4).filter(|p| p[3] > 0).count();
    assert!(
        opaque > 200,
        "rendered tile mostly empty: {} opaque pixels ({}x{})",
        opaque,
        info.width,
        info.height
    );
}

#[test]
fn instance_root_detected_for_waypoints() {
    let dir = save_dir();
    if !dir.is_dir() {
        eprintln!("SKIP: test save not present");
        return;
    }
    let world = testing::open_world(&dir).expect("open world");
    assert_eq!(
        world.instance_root,
        instance_root(),
        "instance root must walk up to the GTNH folder"
    );
}
