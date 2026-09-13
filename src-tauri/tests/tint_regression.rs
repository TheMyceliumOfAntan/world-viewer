//! Confirm the tint regression: color_ref returns the JourneyMap *display*
//! name, so is_foliage never matches real block ids.

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
    eprintln!("is_foliage({:?}) = {}", name, testing::is_foliage(&name));
    eprintln!("palette.name_of(2) = {:?}", world.palette.name_of(2));

    // The name returned by color_ref MUST be classifiable. If it is a
    // JourneyMap display name like "草方块", tinting silently stops working.
    assert!(
        testing::is_foliage(&name),
        "color_ref returned {:?}, which is_foliage cannot classify. \
         The renderer relies on this name to decide whether to apply the \
         biome tint, so returning a display name breaks grass colouring.",
        name
    );
}
