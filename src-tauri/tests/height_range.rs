//! Dimension vertical extent detection.
//!
//! 1.13+ chunks store the full section list, so their extremes ARE the
//! dimension's limits (a 1.20 overworld reports -64..319). Pre-1.13 chunks
//! only store non-empty sections, so those must fall back to 0..255 rather
//! than reporting the sparse sample as the world height.
//!
//! Both cases are checked against real saves; each skips when absent.

use std::path::PathBuf;

use world_viewer_lib::testing;

fn modern_save() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\Aegis of the Frozen Sky\saves\新的世界")
}

fn legacy_save() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本")
}

/// 1.20 overworld: sections -4..19 => block Y -64..319.
#[test]
fn modern_dimension_reports_negative_and_high_limits() {
    if !modern_save().join("level.dat").is_file() {
        eprintln!("SKIP: modern save not present");
        return;
    }
    let world = testing::open_world(&modern_save()).expect("open world");
    let overworld = world
        .dimensions
        .iter()
        .find(|d| d.id == 0)
        .expect("overworld dimension");

    assert_eq!(
        overworld.min_y, -64,
        "1.20 overworld must start at Y=-64, got {}",
        overworld.min_y
    );
    assert_eq!(
        overworld.max_y, 319,
        "1.20 overworld must reach Y=319, got {}",
        overworld.max_y
    );
}

/// Legacy (1.12.2) chunks are sparse, so the extent must be the vanilla
/// 0..255 rather than the sampled non-empty sections (a GTNH chunk reports
/// only Y=0..4 => 0..79, which would truncate most of the world).
#[test]
fn legacy_dimension_falls_back_to_full_range() {
    if !legacy_save().join("level.dat").is_file() {
        eprintln!("SKIP: legacy save not present");
        return;
    }
    let world = testing::open_world(&legacy_save()).expect("open world");
    let overworld = world
        .dimensions
        .iter()
        .find(|d| d.id == 0)
        .expect("overworld dimension");

    assert_eq!(overworld.min_y, 0, "legacy worlds start at Y=0");
    assert_eq!(
        overworld.max_y, 255,
        "legacy worlds must not be truncated below 255, got {}",
        overworld.max_y
    );
}

/// The `u32::MAX` sentinel must resolve to the dimension's own ceiling.
///
/// A fixed 255 would silently hide every block above Y=255 in a 1.20+ world,
/// and would make the "full height" URL differ from the actual full height.
#[test]
fn full_height_sentinel_resolves_to_dimension_ceiling() {
    let modern = testing::DimensionInfo {
        id: 0,
        name: "overworld".into(),
        region_dir: String::new(),
        chunk_count: 1,
        has_data: true,
        min_y: -64,
        max_y: 319,
    };
    assert_eq!(
        testing::resolve_ymax(testing::YMAX_FULL, &modern),
        319,
        "full-height sentinel must reach the dimension ceiling, not 255"
    );
    // Explicit values are clamped into the dimension's range.
    assert_eq!(testing::resolve_ymax(70, &modern), 70);
    assert_eq!(testing::resolve_ymax(0, &modern), 0);
    assert_eq!(testing::resolve_ymax(1000, &modern), 319);
    // A value above i32::MAX must not wrap into a negative height.
    assert_eq!(testing::resolve_ymax(0x8000_0000, &modern), 319);

    // Negative heights are legal in 1.18+ worlds and must survive the filter
    // instead of being rejected as an unparsable u32 (which would silently
    // fall back to full height and ignore the user's choice).
    assert_eq!(testing::resolve_ymax(-64, &modern), -64);
    assert_eq!(testing::resolve_ymax(-1, &modern), -1);
    assert_eq!(testing::resolve_ymax(-1000, &modern), -64);

    let legacy = testing::DimensionInfo {
        min_y: 0,
        max_y: 255,
        ..modern.clone()
    };
    assert_eq!(testing::resolve_ymax(testing::YMAX_FULL, &legacy), 255);
    assert_eq!(testing::resolve_ymax(-1, &legacy), 0, "clamped to legacy floor");
}
