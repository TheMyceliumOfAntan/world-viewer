//! Waypoint parsers for Xaero's minimap and VoxelMap, checked against the
//! real files on this machine. Tests skip when the files are absent.

use std::path::{Path, PathBuf};

use world_viewer_lib::testing;

fn xaero_instance() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\Create+")
}

fn voxelmap_instance() -> PathBuf {
    PathBuf::from(r"C:\.minecraft\versions\1.12.2-Forge-14.23.5.2864")
}

fn has_xaero() -> bool {
    xaero_instance().join("xaero").join("minimap").is_dir()
}

fn has_voxelmap() -> bool {
    voxelmap_instance().join("voxelmap").is_dir()
}

#[test]
fn parses_xaero_waypoints_from_real_files() {
    if !has_xaero() {
        eprintln!("SKIP: xaero data not present");
        return;
    }
    let wps = testing::load_waypoints(&xaero_instance(), "");
    let xaero: Vec<_> = wps.iter().filter(|w| w.source == "xaero").collect();
    assert!(
        !xaero.is_empty(),
        "expected Xaero waypoints, got none (all sources: {})",
        wps.len()
    );

    // The real file contains: overworld "home" (84,65,-701), two deathpoints,
    // a nether deathpoint and a nether waypoint with a non-ASCII name.
    let home = xaero
        .iter()
        .find(|w| w.name == "home")
        .expect("overworld 'home' waypoint must be parsed");
    assert_eq!(home.x, 84);
    assert_eq!(home.y, 65);
    assert_eq!(home.z, -701);
    assert_eq!(home.dimension, 0, "dim%0 must map to dimension 0");

    let deaths: Vec<_> = xaero.iter().filter(|w| w.kind == "Death").collect();
    assert!(
        !deaths.is_empty(),
        "expected at least one deathpoint in the nether"
    );
    assert!(
        deaths.iter().any(|w| w.dimension == -1),
        "deathpoint from dim%-1 must map to dimension -1, got {:?}",
        deaths.iter().map(|w| w.dimension).collect::<Vec<_>>()
    );

    // Colours must be real hex triples, not a palette index.
    for w in &xaero {
        assert!(
            w.color.starts_with('#') && w.color.len() == 7,
            "bad colour {:?} for {}",
            w.color,
            w.name
        );
    }
}

#[test]
fn xaero_dedupes_minimap_and_worldmap_trees() {
    if !has_xaero() {
        eprintln!("SKIP: xaero data not present");
        return;
    }
    let wps = testing::load_waypoints(&xaero_instance(), "");
    let mut seen = std::collections::HashSet::new();
    for w in wps.iter().filter(|w| w.source == "xaero") {
        let key = (w.name.clone(), w.x, w.y, w.z, w.dimension);
        assert!(
            seen.insert(key),
            "duplicate Xaero waypoint after merging minimap/world-map: {} at {},{},{} dim {}",
            w.name,
            w.x,
            w.y,
            w.z,
            w.dimension
        );
    }
}

#[test]
fn parses_voxelmap_waypoints_from_real_file() {
    if !has_voxelmap() {
        eprintln!("SKIP: voxelmap data not present");
        return;
    }
    // The file is named after the world; pass the matching name.
    let dir = voxelmap_instance().join("voxelmap");
    let stem = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .find_map(|e| {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("points") {
                p.file_stem().map(|s| s.to_string_lossy().into_owned())
            } else {
                None
            }
        })
        .expect("a .points file must exist");

    let wps = testing::load_waypoints(&voxelmap_instance(), &stem);
    let vm: Vec<_> = wps.iter().filter(|w| w.source == "voxelmap").collect();
    assert!(
        !vm.is_empty(),
        "expected VoxelMap waypoints for world {:?}",
        stem
    );

    // Real file has "point222" at x=256 z=233 y=63 dim 0.
    let p = vm
        .iter()
        .find(|w| w.name == "point222")
        .expect("'point222' must be parsed");
    assert_eq!(p.x, 256);
    assert_eq!(p.z, 233);
    assert_eq!(p.y, 63);
    assert_eq!(p.dimension, 0, "dimensions:0 must map to dimension 0");
    // green:1.0 red:0.399 green:0.239 blue:0.574 -> ~#663d92
    assert!(
        p.color.starts_with('#') && p.color.len() == 7,
        "bad colour {:?}",
        p.color
    );
}

#[test]
fn voxelmap_skips_disabled_waypoints() {
    if !has_voxelmap() {
        eprintln!("SKIP: voxelmap data not present");
        return;
    }
    let dir = voxelmap_instance().join("voxelmap");
    let stem = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .find_map(|e| {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("points") {
                p.file_stem().map(|s| s.to_string_lossy().into_owned())
            } else {
                None
            }
        })
        .unwrap();

    // Count enabled records in the raw file, compare with parsed count.
    let raw = std::fs::read_to_string(dir.join(format!("{}.points", stem))).unwrap();
    let enabled_in_file = raw
        .lines()
        .filter(|l| l.contains("name:") && l.contains("enabled:true"))
        .count();
    let parsed = testing::load_waypoints(&voxelmap_instance(), &stem)
        .iter()
        .filter(|w| w.source == "voxelmap")
        .count();
    assert_eq!(
        parsed, enabled_in_file,
        "parsed count must match enabled records in the file"
    );
}

#[test]
fn waypoint_sources_are_labelled() {
    // JourneyMap source label must survive the refactor.
    let save = PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本");
    if !save.is_dir() {
        eprintln!("SKIP: GTNH save not present");
        return;
    }
    let wps = testing::load_waypoints(Path::new(r"C:\.minecraft\versions\GTNH 2.8.4"), "新的世界");
    assert!(!wps.is_empty(), "expected JourneyMap waypoints");
    assert!(
        wps.iter().all(|w| w.source == "journeymap"),
        "GTNH instance only has JourneyMap data"
    );
}
