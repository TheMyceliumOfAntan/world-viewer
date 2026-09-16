//! Confirm the tint contract: `color_ref` returns a *block id* name, so the
//! renderer can classify it and pick a biome tint. A display name breaks this.

use std::path::PathBuf;

use world_viewer_lib::testing;

#[test]
fn color_ref_name_is_usable_for_classification() {
    let save = PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本");
    if !save.is_dir() {
        eprintln!("SKIP: GTNH save not present");
        return;
    }
    let world = testing::open_world(&save).expect("open world");

    // Grass id 2 in 1.7.10.
    let block = testing::BlockRef::Legacy(2, 0);
    let (_rgb, src, name) = world.palette.color_ref(&block);
    eprintln!("color_ref(grass) -> src={} name={:?}", src, name);
    eprintln!("tint_kind({:?}) = {:?}", name, testing::tint_kind(&name));
    eprintln!("palette.name_of(2) = {:?}", world.palette.name_of(2));

    // The name returned by color_ref MUST be classifiable. If it is a
    // JourneyMap display name like "草方块", tinting silently stops working.
    assert_eq!(
        testing::tint_kind(&name),
        testing::TintKind::Grass,
        "color_ref returned {:?}, which the tint classifier cannot resolve. \
         The renderer relies on this name to decide which biome tint to apply, \
         so returning a display name breaks grass colouring.",
        name
    );
}

/// The full chain must produce green grass from a real save's grey palette.
#[test]
fn real_palette_grass_renders_green() {
    let save = PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本");
    if !save.is_dir() {
        eprintln!("SKIP: GTNH save not present");
        return;
    }
    let world = testing::open_world(&save).expect("open world");

    let block = testing::BlockRef::Legacy(2, 0);
    let (rgb, _, name) = world.palette.color_ref(&block);
    let out = testing::resolve_color(rgb, &name, testing::BiomeDef::legacy(1));
    eprintln!("grass palette {:?} -> rendered {:?}", rgb, out);
    let (r, g, b) = (out[0] as i32, out[1] as i32, out[2] as i32);
    assert!(
        g > r && g > b,
        "grass must render green, got {:?} from palette {:?}",
        out,
        rgb
    );
}
