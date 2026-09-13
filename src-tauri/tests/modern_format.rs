//! 1.13+ flattened chunk format (`sections[].block_states`), checked against
//! the real 1.20+ save. Tests skip when the save is absent.

use std::path::PathBuf;

use world_viewer_lib::testing;

fn modern_save() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\Aegis of the Frozen Sky\saves\新的世界")
}

fn has_modern() -> bool {
    modern_save().join("level.dat").is_file()
}

#[test]
fn parses_modern_sections_with_palette_and_data() {
    if !has_modern() {
        eprintln!("SKIP: modern save not present");
        return;
    }
    let region_dir = modern_save().join("region");
    let chunk = testing::load_chunk(&region_dir, 0, 0)
        .expect("read ok")
        .expect("chunk (0,0) must exist");

    assert!(!chunk.sections.is_empty(), "modern chunk has no sections");
    // 1.20+ overworld chunks have sections below y=0
    assert!(
        chunk.sections.iter().any(|s| s.y < 0),
        "expected negative-Y sections, got {:?}",
        chunk.sections.iter().map(|s| s.y).collect::<Vec<_>>()
    );

    // Every section must have a palette.
    for s in &chunk.sections {
        let bs = s.block_states.as_ref().expect("section must use block_states");
        assert!(!bs.palette.is_empty(), "section Y={} has empty palette", s.y);
        // Names must be namespaced block ids, not empty strings.
        assert!(
            bs.palette.iter().all(|n| n.contains(':')),
            "section Y={} palette has non-namespaced entries: {:?}",
            s.y,
            bs.palette
        );
    }

    // Top block at spawn column must be a real block with a plausible height.
    let (block, y) = chunk
        .top_visible(0, 0, 255)
        .expect("spawn column must have a top block");
    let name = match &block {
        testing::BlockRef::Named(n) => n.clone(),
        other => panic!("modern chunk must yield named blocks, got {:?}", other),
    };
    assert!(name.contains(':'), "bad block name {:?}", name);
    assert!(
        (0..=320).contains(&y),
        "implausible block height {} for {}",
        y,
        name
    );
    eprintln!("spawn column top: {} at y={}", name, y);
}

#[test]
fn modern_blocks_resolve_to_real_colours() {
    if !has_modern() {
        eprintln!("SKIP: modern save not present");
        return;
    }
    let save = modern_save();
    let world = testing::open_world(&save).expect("open world");
    let region_dir = save.join("region");

    let (png, has_data) = testing::render_tile_with_data_flag(
        &world.palette,
        &region_dir,
        0,
        0,
        0,
        255,
    )
    .expect("render tile");
    assert!(has_data, "tile (0,0) at zoom 0 must contain terrain");

    let decoder = png::Decoder::new(&png[..]);
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut buf).unwrap();

    let opaque = buf.chunks(4).filter(|p| p[3] > 0).count();
    assert!(opaque > 1000, "tile mostly empty: {} opaque px", opaque);

    // Terrain must be coloured, not uniform grey from the fallback.
    let mut colors = std::collections::HashSet::new();
    for px in buf.chunks(4) {
        if px[3] > 0 {
            colors.insert([px[0], px[1], px[2]]);
        }
    }
    assert!(
        colors.len() > 200,
        "expected varied terrain colours, got {} distinct",
        colors.len()
    );

    // The built-in vanilla table must produce recognisable hues: the tile
    // should contain greenish pixels (grass/leaves) rather than only grey.
    let greenish = buf
        .chunks(4)
        .filter(|p| p[3] > 0 && p[1] > p[0].saturating_add(8) && p[1] > p[2].saturating_add(8))
        .count();
    assert!(
        greenish > 50,
        "expected green vegetation pixels, found {}",
        greenish
    );
    eprintln!(
        "modern tile: {} opaque px, {} distinct colours, {} greenish",
        opaque,
        colors.len(),
        greenish
    );
}

#[test]
fn modern_and_legacy_parsers_do_not_cross_contaminate() {
    // A legacy chunk must still parse through the legacy path, and a modern
    // chunk through the modern one, with the auto-detection in load_chunk.
    let legacy = PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本\region");
    let modern = modern_save().join("region");
    if !legacy.is_dir() || !has_modern() {
        eprintln!("SKIP: saves not present");
        return;
    }

    let lc = testing::load_chunk(&legacy, 1, 1).unwrap().unwrap();
    assert!(
        lc.sections.iter().any(|s| s.blocks16.is_some() || s.blocks.is_some()),
        "legacy chunk must use legacy arrays"
    );
    assert!(
        lc.sections.iter().all(|s| s.block_states.is_none()),
        "legacy chunk must not produce block_states"
    );

    let mc = testing::load_chunk(&modern, 0, 0).unwrap().unwrap();
    assert!(
        mc.sections.iter().any(|s| s.block_states.is_some()),
        "modern chunk must use block_states"
    );
    assert!(
        mc.sections.iter().all(|s| s.blocks16.is_none() && s.blocks.is_none()),
        "modern chunk must not produce legacy arrays"
    );
}

#[test]
fn parses_1_12_2_legacy_add_format() {
    let region =
        PathBuf::from(r"C:\.minecraft\versions\1.12.2-Forge-14.23.5.2864\saves\新的世界\region");
    if !region.is_dir() {
        eprintln!("SKIP: 1.12.2 save not present");
        return;
    }
    let chunk = match testing::load_chunk(&region, 0, 0).expect("read ok") {
        Some(c) => c,
        None => {
            // Chunk (0,0) may not be generated; find any present chunk.
            let mut found = None;
            'outer: for cx in 0..8 {
                for cz in -8..8 {
                    if let Some(c) = testing::load_chunk(&region, cx, cz).expect("read ok") {
                        found = Some(c);
                        break 'outer;
                    }
                }
            }
            found.expect("at least one chunk must exist in the 1.12.2 save")
        }
    };
    assert!(!chunk.sections.is_empty());

    // 1.12.2 uses Blocks plus (when needed) Add for ids above 255.
    let has_blocks = chunk.sections.iter().any(|s| s.blocks.is_some());
    assert!(has_blocks, "1.12.2 sections must use Blocks");

    let (block, y) = chunk
        .top_visible(8, 8, 255)
        .expect("column must have a top block");
    assert!(!block.is_air(), "top block must not be air");
    assert!((0..=255).contains(&y));
}
