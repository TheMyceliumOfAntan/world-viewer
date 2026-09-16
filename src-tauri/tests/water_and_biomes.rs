//! Water transparency and biome tinting, verified against real saves.
//!
//! Both features are easy to regress silently: water still renders *something*
//! blue when the floor scan breaks, and grass still renders *something* green
//! when the biome lookup always falls back to plains. These tests assert the
//! observable difference, not just "it produced pixels".

use std::path::PathBuf;

use world_viewer_lib::testing::{self, RenderOpts};

fn gtnh_save() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本")
}

fn modern_save() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\Aegis of the Frozen Sky\saves\新的世界")
}

fn decode(png: &[u8]) -> (usize, usize, Vec<u8>) {
    let decoder = png::Decoder::new(png);
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (info.width as usize, info.height as usize, buf)
}

fn opaque_pixels(buf: &[u8]) -> Vec<[u8; 3]> {
    buf.chunks(4)
        .filter(|p| p[3] > 0)
        .map(|p| [p[0], p[1], p[2]])
        .collect()
}

/// Bluish pixels: water-dominant. Used to isolate water pixels.
fn is_blue(p: [u8; 3]) -> bool {
    p[2] as i32 > p[0] as i32 + 20 && p[2] as i32 > p[1] as i32 + 20
}

/// The water toggle must change the rendered pixels: with `water` off a lake
/// is flat water colour, with it on the sea floor shows through.
#[test]
fn water_toggle_changes_the_render() {
    let save = gtnh_save();
    if !save.is_dir() {
        eprintln!("SKIP: GTNH save not present");
        return;
    }
    let world = testing::open_world(&save).expect("open world");
    let region_dir = save.join("region");

    // A tile near the player that the colours test already knows has water.
    let on = testing::render_tile_with_opts(
        &world.palette,
        &region_dir,
        0,
        -2,
        -2,
        255,
        RenderOpts {
            water: true,
            shading: true,
            altitude: true,
        },
    )
    .expect("render with water");
    let off = testing::render_tile_with_opts(
        &world.palette,
        &region_dir,
        0,
        -2,
        -2,
        255,
        RenderOpts {
            water: false,
            shading: true,
            altitude: true,
        },
    )
    .expect("render without water");

    assert_ne!(on, off, "the water toggle must change the tile");

    let (_, _, on_buf) = decode(&on);
    let (_, _, off_buf) = decode(&off);
    let on_px = opaque_pixels(&on_buf);
    let off_px = opaque_pixels(&off_buf);

    // With water off, lakes are a flat water colour: every water pixel is the
    // same. With it on, the sea floor shows through, so water pixels spread
    // across many more distinct colours.
    let water_like = is_blue;
    let on_variety: std::collections::HashSet<[u8; 3]> =
        on_px.iter().copied().filter(|p| water_like(*p)).collect();
    let off_variety: std::collections::HashSet<[u8; 3]> =
        off_px.iter().copied().filter(|p| water_like(*p)).collect();
    eprintln!(
        "distinct water-ish colours: on = {}, off = {}",
        on_variety.len(),
        off_variety.len()
    );
    assert!(
        on_variety.len() > off_variety.len(),
        "seeing the floor must add colour variety underwater: on {} vs off {}",
        on_variety.len(),
        off_variety.len()
    );

    // Blending the floor in also darkens the blue channel on average, since
    // the floor is never as blue as open water.
    let mean_b = |px: &[[u8; 3]]| -> f64 {
        let blue: Vec<f64> = px.iter().filter(|p| water_like(**p)).map(|p| p[2] as f64).collect();
        blue.iter().sum::<f64>() / blue.len().max(1) as f64
    };
    let (on_b, off_b) = (mean_b(&on_px), mean_b(&off_px));
    eprintln!("mean blue of water pixels: on = {:.1}, off = {:.1}", on_b, off_b);
    assert!(
        on_b < off_b,
        "water transparency must reduce blue dominance: on {:.1} vs off {:.1}",
        on_b,
        off_b
    );
}

/// The floor must actually be reached: a water column's rendered colour has to
/// depend on what is underneath it.
#[test]
fn water_columns_see_their_floor() {
    let save = gtnh_save();
    if !save.is_dir() {
        eprintln!("SKIP: GTNH save not present");
        return;
    }
    let region_dir = save.join("region");

    // Scan the region for water columns and confirm the floor scan resolves
    // them (rather than bailing out at the surface).
    let mut water_columns = 0usize;
    let mut with_floor = 0usize;
    let mut distinct_floors = std::collections::HashSet::new();
    'scan: for cx in -16..16 {
        for cz in -16..16 {
            let Ok(Some(chunk)) = testing::load_chunk(&region_dir, cx, cz) else {
                continue;
            };
            for z in 0..16usize {
                for x in 0..16usize {
                    let scan = chunk.scan_column(x, z, 255, &testing::ScanOpts { water: true });
                    if scan.water_top.is_none() {
                        continue;
                    }
                    water_columns += 1;
                    if let Some((fb, _)) = scan.floor_block {
                        with_floor += 1;
                        distinct_floors.insert(fb.to_owned_ref());
                    }
                }
            }
            if water_columns > 2000 {
                break 'scan;
            }
        }
    }

    eprintln!(
        "water columns: {}, with floor: {}, distinct floor blocks: {}",
        water_columns,
        with_floor,
        distinct_floors.len()
    );
    assert!(water_columns > 100, "expected water in this save");
    assert_eq!(
        water_columns, with_floor,
        "every water column must resolve a floor (the scan must not stop at the surface)"
    );
    assert!(
        distinct_floors.len() > 1,
        "floors must vary (sand, gravel, stone, dirt), got {}",
        distinct_floors.len()
    );
}

/// Grass on different biomes must render different colours. Before biome
/// tinting the whole map used one fixed plains green.
#[test]
fn biome_tint_varies_the_grass_colour() {
    let save = gtnh_save();
    if !save.is_dir() {
        eprintln!("SKIP: GTNH save not present");
        return;
    }
    let region_dir = save.join("region");
    let world = testing::open_world(&save).expect("open world");

    // Collect the biome id of every grass-topped column and the colour it
    // renders. More than one biome must appear, and their colours must differ.
    let mut by_biome: std::collections::HashMap<u16, [u8; 3]> = std::collections::HashMap::new();
    'scan: for cx in -16..16 {
        for cz in -16..16 {
            let Ok(Some(chunk)) = testing::load_chunk(&region_dir, cx, cz) else {
                continue;
            };
            for z in 0..16usize {
                for x in 0..16usize {
                    let scan = chunk.scan_column(x, z, 255, &testing::ScanOpts { water: false });
                    let Some((block, y)) = scan.block else { continue };
                    let (rgb, _, name) = world.palette.color_ref(&block.to_owned_ref());
                    if testing::tint_kind(&name) != testing::TintKind::Grass {
                        continue;
                    }
                    let id = match &chunk.legacy_biomes {
                        Some(b) => b.biome_id(x, z, y),
                        None => continue,
                    };
                    let rendered = testing::resolve_color(rgb, &name, scan.tint);
                    by_biome.insert(id, rendered);
                }
            }
            if by_biome.len() >= 3 {
                break 'scan;
            }
        }
    }

    eprintln!("grass biomes found: {:?}", by_biome);
    assert!(
        by_biome.len() >= 2,
        "expected grass in more than one biome, found {:?}",
        by_biome
    );
    let colours: std::collections::HashSet<[u8; 3]> = by_biome.values().copied().collect();
    assert!(
        colours.len() >= 2,
        "different biomes must render different grass colours, got {:?}",
        colours
    );
}

/// The biome data must survive parsing for the modern (1.18+) layout too, and
/// the biome palette must yield namespaced names.
#[test]
fn modern_sections_carry_biome_palettes() {
    let save = modern_save();
    if !save.is_dir() {
        eprintln!("SKIP: modern save not present");
        return;
    }
    let region_dir = save.join("region");
    let chunk = match testing::load_chunk(&region_dir, 0, 0).expect("read ok") {
        Some(c) => c,
        None => {
            eprintln!("SKIP: chunk (0,0) not generated");
            return;
        }
    };

    let with_biomes = chunk
        .sections
        .iter()
        .filter(|s| s.biomes.is_some())
        .count();
    eprintln!(
        "{} of {} sections carry biomes",
        with_biomes,
        chunk.sections.len()
    );
    assert!(
        with_biomes > 0,
        "1.18+ sections must carry a biome palette"
    );

    // Biome names must be namespaced, and resolve to real tints.
    let mut names = std::collections::HashSet::new();
    for s in &chunk.sections {
        if let Some(b) = &s.biomes {
            for n in &b.palette {
                if n.contains(':') {
                    names.insert(n.clone());
                }
            }
        }
    }
    eprintln!("biome names: {:?}", names.iter().take(5).collect::<Vec<_>>());
    assert!(!names.is_empty(), "biome palette must contain namespaced ids");
    for n in &names {
        let def = testing::BiomeDef::modern(n);
        // A resolved biome must not be the all-zero default.
        assert_ne!(
            def,
            testing::BiomeDef::default(),
            "biome {} has no tint data",
            n
        );
    }
}

/// Shading toggles must change the render, and altitude shading must be a
/// no-op on flat terrain while slope shading is not.
#[test]
fn shading_toggles_change_the_render() {
    let save = gtnh_save();
    if !save.is_dir() {
        eprintln!("SKIP: GTNH save not present");
        return;
    }
    let world = testing::open_world(&save).expect("open world");
    let region_dir = save.join("region");
    let base = |shading, altitude| RenderOpts {
        water: true,
        shading,
        altitude,
    };

    let full = testing::render_tile_with_opts(
        &world.palette, &region_dir, 0, -2, -2, 255, base(true, true),
    )
    .unwrap();
    let no_shade = testing::render_tile_with_opts(
        &world.palette, &region_dir, 0, -2, -2, 255, base(false, false),
    )
    .unwrap();
    let slope_only = testing::render_tile_with_opts(
        &world.palette, &region_dir, 0, -2, -2, 255, base(true, false),
    )
    .unwrap();

    assert_ne!(full, no_shade, "shading must change the tile");
    assert_ne!(full, slope_only, "altitude shading must change the tile");
}

/// All three legacy biome layouts must be decoded, and every biome id they
/// contain must resolve to real tint data. This is the coverage that keeps
/// 1.14 (256-column), 1.16 (1024-cell grid) and 1.6/1.12 (256-column) saves
/// from silently falling back to the plains tint.
#[test]
fn legacy_biome_layouts_decode_and_resolve() {
    // (label, save dir) — each exercises a different on-disk biome encoding.
    let saves: &[(&str, &str)] = &[
        ("1.14.4 columns", r"C:\Minecraft\.minecraft\versions\1.14.4\saves\新的世界"),
        ("1.16.5 grid", r"C:\.minecraft\versions\1.16.5\saves\新的世界"),
        (
            "1.12.2 columns",
            r"C:\.minecraft\versions\1.12.2-Forge-14.23.5.2864\saves\新的世界",
        ),
        ("1.6.4 columns", r"C:\Minecraft\.minecraft\versions\1.6.4\saves\New World"),
    ];

    let mut checked = 0usize;
    for (label, path) in saves {
        let dir = PathBuf::from(path);
        if !dir.is_dir() {
            eprintln!("SKIP {label}: save not present");
            continue;
        }
        let world = testing::open_world(&dir).expect("open world");
        let Some(dim) = world.dimensions.iter().find(|d| d.has_data) else {
            eprintln!("SKIP {label}: no dimension with data");
            continue;
        };
        let region_dir = PathBuf::from(&dim.region_dir);

        let mut kinds = std::collections::HashSet::new();
        let mut ids = std::collections::HashSet::new();
        for cx in 0..8 {
            for cz in 0..8 {
                let Ok(Some(chunk)) = testing::load_chunk(&region_dir, cx, cz) else {
                    continue;
                };
                match &chunk.legacy_biomes {
                    Some(testing::LegacyBiomes::Columns(v)) => {
                        kinds.insert("columns");
                        ids.extend(v.iter().map(|&x| x as u16));
                    }
                    Some(testing::LegacyBiomes::Grid(v)) => {
                        kinds.insert("grid");
                        ids.extend(v.iter().copied());
                    }
                    None => {}
                }
            }
        }

        assert!(
            !kinds.is_empty(),
            "{label}: no chunk carried legacy biomes — the parser regressed"
        );
        // Every decoded id must have a tint row, otherwise that biome renders
        // as plains regardless of what the world says.
        let unresolved: Vec<u16> = ids
            .iter()
            .copied()
            .filter(|&i| testing::BiomeDef::legacy(i) == testing::BiomeDef::default())
            .collect();
        eprintln!(
            "{label}: kinds={:?} ids={} unresolved={:?}",
            kinds,
            ids.len(),
            unresolved
        );
        assert!(
            unresolved.is_empty(),
            "{label}: biome ids with no tint data: {:?}",
            unresolved
        );
        checked += 1;
    }
    assert!(checked > 0, "no legacy save was available to check");
}

/// The 1.16-style 1024-cell grid must be indexed by the block's Y, not just
/// its column: a biome sampled at the surface and one sampled deep underground
/// can legitimately differ.
#[test]
fn grid_biomes_are_indexed_by_height() {
    use testing::LegacyBiomes;

    // 1024 cells, indexed (y>>2)*16 + (z>>2)*4 + (x>>2). Give one cell a
    // distinctive id so we can prove the Y component is used.
    let mut cells = vec![7u16; 1024];
    cells[(64 >> 2) * 16 + (9 >> 2) * 4 + (5 >> 2)] = 12;
    let biomes = LegacyBiomes::Grid(cells);

    assert_eq!(biomes.biome_id(5, 9, 64), 12, "cell must be found by (x,z,y)");
    assert_eq!(biomes.biome_id(5, 9, 0), 7, "a different Y is a different cell");
    assert_eq!(biomes.biome_id(0, 0, 64), 7);
    // Y is clamped into 0..255 so 1.18-style heights still resolve.
    assert_eq!(biomes.biome_id(5, 9, -64), 7);
    assert_eq!(biomes.biome_id(5, 9, 400), 7);
}
