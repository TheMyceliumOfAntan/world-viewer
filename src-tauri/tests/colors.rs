//! Colour correctness: the reported bug was grass/foliage rendering grey
//! (1.7.10 partly) and entire worlds rendering grey (1.12.2, no id->name map).

use std::path::PathBuf;

use world_viewer_lib::testing;

fn gtnh_save() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本")
}

fn forge_1_12_2_save() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\1.12.2-Forge-14.23.5.2864\saves\新的世界")
}

/// The reported bug: in the 1.12.2 save every block was grey because no
/// id -> name mapping existed (that level.dat has no FML.ItemData).
#[test]
fn one_twelve_two_blocks_have_names_and_colours() {
    let save = forge_1_12_2_save();
    if !save.is_dir() {
        eprintln!("SKIP: 1.12.2 save not present");
        return;
    }
    let world = testing::open_world(&save).expect("open world");

    // Vanilla ids must resolve now.
    assert_eq!(
        world.palette.name_of(1),
        "minecraft:stone",
        "id 1 must map to stone"
    );
    assert_eq!(world.palette.name_of(2), "minecraft:grass");
    assert_eq!(world.palette.name_of(31), "minecraft:tallgrass");
    assert_eq!(world.palette.name_of(9), "minecraft:water");

    // And they must produce real colours, not the grey fallback.
    let fallback = world.palette.fallback;
    let (stone, src, _) = world.palette.color(1, 0);
    assert_ne!(stone, fallback, "stone must not be the fallback grey");
    assert_eq!(src, "vanilla", "stone should resolve via the vanilla table");

    // The rendered tile must contain colour, not just grey.
    let region_dir = save.join("region");
    let (png, has_data) =
        testing::render_tile_with_data_flag(&world.palette, &region_dir, 0, 0, 0, 255)
            .expect("render");
    assert!(has_data, "tile (0,0) must have data");

    let decoder = png::Decoder::new(&png[..]);
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut buf).unwrap();

    let mut grey = 0usize;
    let mut coloured = 0usize;
    for px in buf.chunks(4) {
        if px[3] == 0 {
            continue;
        }
        let (r, g, b) = (px[0], px[1], px[2]);
        // allow a little slack for shading
        if r.abs_diff(g) <= 3 && g.abs_diff(b) <= 3 {
            grey += 1;
        } else {
            coloured += 1;
        }
    }
    let total = grey + coloured;
    assert!(total > 1000, "tile too empty: {} px", total);
    let coloured_pct = 100.0 * coloured as f64 / total as f64;
    assert!(
        coloured_pct > 25.0,
        "1.12.2 tile is still mostly greyscale: {:.1}% coloured ({} grey / {} total)",
        coloured_pct,
        grey,
        total
    );
    eprintln!(
        "1.12.2 tile: {:.1}% coloured ({} of {} px)",
        coloured_pct, coloured, total
    );
}

/// The reported bug: GTNH grass rendered grey while stone/dirt looked right.
#[test]
fn gtnh_grass_and_foliage_are_green() {
    let save = gtnh_save();
    if !save.is_dir() {
        eprintln!("SKIP: GTNH save not present");
        return;
    }
    let world = testing::open_world(&save).expect("open world");

    // The palette itself stores grey for these (that is the root cause).
    let (grass_raw, _, _) = world.palette.color(2, 0);
    let grey_ish = grass_raw[0].abs_diff(grass_raw[1]) <= 6;
    eprintln!("palette grass raw = {:?} (greyscale: {})", grass_raw, grey_ish);

    let region_dir = save.join("region");

    // The surface near the player is mostly water, so measure over several
    // tiles and take the best one: what matters is that land renders green.
    let mut best_green = 0.0f64;
    let mut best_tile = (0, 0);
    for (tx, ty) in [(-2, -2), (-2, -1), (-1, -2), (-1, -1), (-3, -2), (-2, -3)] {
        let (png, has_data) =
            testing::render_tile_with_data_flag(&world.palette, &region_dir, 0, tx, ty, 255)
                .expect("render");
        if !has_data {
            continue;
        }
        let decoder = png::Decoder::new(&png[..]);
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        reader.next_frame(&mut buf).unwrap();

        let mut green = 0usize;
        let mut opaque = 0usize;
        for px in buf.chunks(4) {
            if px[3] == 0 {
                continue;
            }
            opaque += 1;
            let (r, g, b) = (px[0] as i32, px[1] as i32, px[2] as i32);
            if g > r + 12 && g > b + 12 {
                green += 1;
            }
        }
        let pct = 100.0 * green as f64 / opaque.max(1) as f64;
        eprintln!("tile ({},{}) green = {:.1}%", tx, ty, pct);
        if pct > best_green {
            best_green = pct;
            best_tile = (tx, ty);
        }
    }

    assert!(
        best_green > 8.0,
        "no GTNH tile showed meaningful green foliage; best was {:.1}% at {:?}",
        best_green,
        best_tile
    );
    eprintln!("best GTNH tile: {:.1}% green at {:?}", best_green, best_tile);
}

/// `is_foliage` must match the vanilla tint categories exactly: blocks that
/// the game tints with the biome colour, and nothing else. Tinting a flower
/// or a crop turns it green, which is wrong.
#[test]
fn foliage_classification_matches_vanilla_tint_categories() {
    // Vanilla `grass` tint: grass_block top, short/tall grass, ferns, reeds.
    // Vanilla `foliage` tint: leaves and vines.
    // Plus modded ground cover observed on the real GTNH surface.
    let must_tint = [
        "minecraft:grass",
        "minecraft:grass_block",
        "minecraft:tallgrass",
        "minecraft:short_grass",
        "minecraft:tall_grass",
        "minecraft:fern",
        "minecraft:large_fern",
        "minecraft:reeds",
        "minecraft:double_plant",
        "minecraft:leaves",
        "minecraft:leaves2",
        "minecraft:vine",
        "minecraft:oak_leaves",
        "minecraft:spruce_leaves",
        "minecraft:acacia_leaves",
        "BiomesOPlenty:foliage",
        "Thaumcraft:blockMagicalLeaves",
        "IC2:blockRubLeaves",
    ];
    // These have their own colours and must NOT be tinted green.
    let must_not_tint = [
        "minecraft:stone",
        "minecraft:dirt",
        "minecraft:water",
        "minecraft:sand",
        "minecraft:gravel",
        "minecraft:log",
        "gregtech:gt.blockores",
        "minecraft:red_flower",
        "minecraft:yellow_flower",
        "minecraft:wheat",
        "minecraft:carrots",
        "minecraft:brown_mushroom",
        "minecraft:red_mushroom",
        "minecraft:cactus",
        "minecraft:waterlily",
        "BiomesOPlenty:flowers",
        "BiomesOPlenty:lilyBop",
    ];
    for name in must_tint {
        assert!(
            testing::is_foliage(name),
            "{} must be tinted (vanilla tint category)",
            name
        );
    }
    for name in must_not_tint {
        assert!(
            !testing::is_foliage(name),
            "{} must NOT be tinted — it has its own colour",
            name
        );
    }
}

/// Tinting grey texture colours must yield a recognisable green.
#[test]
fn foliage_tint_produces_green() {
    // The real palette values for grass / tallgrass in the GTNH save.
    for grey in [[0x93u8, 0x93, 0x93], [0x87, 0x87, 0x87], [0x74, 0x74, 0x74]] {
        let out = testing::apply_foliage_tint(grey);
        let (r, g, b) = (out[0] as i32, out[1] as i32, out[2] as i32);
        assert!(
            g > r && g > b,
            "tinted {:?} must be green, got {:?}",
            grey,
            out
        );
        assert!(g > 80, "tinted {:?} too dark: {:?}", grey, out);
    }
}
