//! Pre-1.13 block names must resolve to a colour.
//!
//! The 1.13 flattening renamed much of the block set, so saves from 1.0–1.12.2
//! name their blocks the old way (`minecraft:grass`, `minecraft:leaves`,
//! `minecraft:stonebrick`). Without a name mapping those all fall through to
//! the fallback grey, which made every pre-1.13 save render as a grey map with
//! dark-tinted "grass".
//!
//! `vanilla_color` maps the legacy names onto their post-flattening
//! equivalents, so one table covers every version in that whole era rather
//! than needing a per-version patch.

use std::path::PathBuf;

use world_viewer_lib::testing;

/// Every legacy name in the 1.7.10 id table must resolve to a real colour.
#[test]
fn legacy_names_resolve_to_colours() {
    // (legacy name, expected rgb) — expected values are the modern
    // equivalents the alias table points at.
    let cases: &[(&str, [u8; 3])] = &[
        ("minecraft:grass", [125, 145, 78]),
        ("minecraft:tallgrass", [125, 145, 78]),
        ("minecraft:leaves", [72, 90, 36]),
        ("minecraft:leaves2", [43, 76, 24]),
        ("minecraft:log", [154, 125, 77]),
        ("minecraft:log2", [60, 46, 26]),
        ("minecraft:planks", [156, 127, 78]),
        ("minecraft:stonebrick", [122, 121, 121]),
        ("minecraft:brick_block", [150, 97, 83]),
        ("minecraft:nether_brick", [44, 22, 26]),
        ("minecraft:hardened_clay", [150, 92, 66]),
        ("minecraft:reeds", [110, 150, 70]),
        ("minecraft:waterlily", [40, 90, 35]),
        ("minecraft:deadbush", [145, 105, 55]),
        ("minecraft:web", [228, 234, 234]),
        ("minecraft:yellow_flower", [255, 216, 60]),
        ("minecraft:red_flower", [200, 45, 45]),
        ("minecraft:slime", [110, 190, 110]),
        ("minecraft:melon_block", [120, 150, 50]),
        ("minecraft:monster_egg", [110, 110, 110]),
        ("minecraft:noteblock", [120, 90, 60]),
        ("minecraft:mob_spawner", [30, 35, 40]),
        ("minecraft:snow_layer", [239, 251, 251]),
        ("minecraft:grass_path", [148, 122, 65]),
        ("minecraft:lit_furnace", [110, 110, 110]),
        ("minecraft:lit_redstone_lamp", [140, 100, 60]),
        ("minecraft:unlit_redstone_torch", [200, 60, 60]),
        ("minecraft:golden_rail", [180, 150, 90]),
        ("minecraft:fence", [156, 127, 78]),
        ("minecraft:fence_gate", [156, 127, 78]),
        ("minecraft:trapdoor", [156, 127, 78]),
        ("minecraft:wooden_door", [156, 127, 78]),
        ("minecraft:wooden_slab", [125, 125, 125]),
        ("minecraft:sapling", [75, 115, 50]),
        ("minecraft:unpowered_repeater", [160, 160, 160]),
        ("minecraft:unpowered_comparator", [160, 160, 160]),
        ("minecraft:skull", [200, 200, 190]),
    ];

    let palette = testing::Palette::empty();
    let mut unresolved = Vec::new();
    for (legacy, expect) in cases {
        let (rgb, src, _) = palette.color_ref(&testing::BlockRef::Named(legacy.to_string()));
        if src == "unknown" {
            unresolved.push(*legacy);
            continue;
        }
        assert_eq!(
            rgb, *expect,
            "{legacy} resolved to {rgb:?}, expected {expect:?}"
        );
    }
    assert!(
        unresolved.is_empty(),
        "these legacy names still have no colour: {unresolved:?}"
    );
}

/// The alias table must not break modern names: 1.13+ saves use the new
/// spelling directly and must still resolve.
#[test]
fn modern_names_still_resolve() {
    let palette = testing::Palette::empty();
    for (modern, expect) in [
        ("minecraft:grass_block", [125, 145, 78]),
        ("minecraft:short_grass", [125, 145, 78]),
        ("minecraft:oak_leaves", [72, 90, 36]),
        ("minecraft:oak_log", [154, 125, 77]),
        ("minecraft:stone_bricks", [122, 121, 121]),
        ("minecraft:dirt", [134, 96, 67]),
        ("minecraft:stone", [125, 125, 125]),
    ] {
        let (rgb, src, _) = palette.color_ref(&testing::BlockRef::Named(modern.to_string()));
        assert_eq!(rgb, expect, "{modern} regressed (src={src})");
    }
}

/// Real pre-1.13 saves: surface columns must no longer hit the fallback grey.
#[test]
fn real_legacy_saves_have_no_fallback_grey() {
    let saves = [
        (
            "1.6.4",
            PathBuf::from(r"C:\Minecraft\.minecraft\versions\1.6.4\saves\New World"),
        ),
        (
            "1.8.9",
            PathBuf::from(r"C:\Minecraft\.minecraft\versions\1.8.9\saves\新的世界"),
        ),
    ];

    for (label, save) in saves {
        if !save.is_dir() {
            eprintln!("SKIP {label}: save not present");
            continue;
        }
        let world = testing::open_world(&save).expect("open save");
        let dim = &world.dimensions[0];
        let region_dir = PathBuf::from(&dim.region_dir);

        let mut total = 0usize;
        let mut unknown: Vec<String> = Vec::new();
        let mut chunks_seen = 0;
        'scan: for cx in -8..8 {
            for cz in -8..8 {
                let Ok(Some(chunk)) = testing::load_chunk(&region_dir, cx, cz) else {
                    continue;
                };
                chunks_seen += 1;
                for z in 0..16 {
                    for x in 0..16 {
                        if let Some((block, _)) = chunk.top_block(x, z, 255) {
                            total += 1;
                            let (_, src, name) = world.palette.color_ref(&block);
                            if src == "unknown" {
                                unknown.push(name);
                            }
                        }
                    }
                }
                if chunks_seen >= 16 {
                    break 'scan;
                }
            }
        }
        assert!(total > 0, "{label}: sampled no columns");
        unknown.sort();
        unknown.dedup();
        eprintln!(
            "{label}: {total} columns from {chunks_seen} chunks, {} unknown names",
            unknown.len()
        );
        assert!(
            unknown.is_empty(),
            "{label}: {} columns still fall back to grey; names: {:?}",
            total,
            unknown
        );
    }
}
